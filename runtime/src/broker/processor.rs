use crate::broker::{BrokerConsumerAdapter, BrokerMessage};
use crate::config::ResilienceConfig;
use crate::deployer::FluxcellDeployer;
use crate::storage::FluxStorage;
use crate::telemetry::{DomainOperationTrace, DomainTraceStorage, FluxcellStepSpan, HostCallSpan, TelemetryClient};
use crate::wasm::WasmHost;
use futures_util::StreamExt;
use std::sync::Arc;
use std::time::Duration;

pub struct EventProcessor {
    broker: Arc<dyn BrokerConsumerAdapter>,
    wasm_host: Arc<WasmHost>,
    telemetry: Arc<TelemetryClient>,
    storage: Arc<dyn FluxStorage>,
    deployer: Option<Arc<FluxcellDeployer>>,
    trace_storage: Option<Arc<DomainTraceStorage>>,
    resilience: ResilienceConfig,
    worker_id: String,
}

impl EventProcessor {
    pub fn new(
        broker: Arc<dyn BrokerConsumerAdapter>,
        wasm_host: Arc<WasmHost>,
        telemetry: Arc<TelemetryClient>,
        storage: Arc<dyn FluxStorage>,
        deployer: Option<Arc<FluxcellDeployer>>,
        trace_storage: Option<Arc<DomainTraceStorage>>,
        resilience: ResilienceConfig,
        worker_id: String,
    ) -> Self {
        Self {
            broker,
            wasm_host,
            telemetry,
            storage,
            deployer,
            trace_storage,
            resilience,
            worker_id,
        }
    }

    pub fn spawn_consumer_loop(self: Arc<Self>, subjects: Vec<String>, group: String) {
        tokio::spawn(async move {
            match self.broker.subscribe(&subjects, &group).await {
                Ok(mut stream) => {
                    log::info!("Broker consumer subscribed to subjects: {:?}", subjects);
                    while let Some(msg) = stream.next().await {
                        self.process_message(&msg).await;
                    }
                }
                Err(e) => {
                    log::warn!("Broker subscription could not be established: {}", e);
                }
            }
        });
    }

    pub async fn process_message(&self, msg: &BrokerMessage) {
        self.telemetry.increment_processed(None);
        self.telemetry.record_log(
            "INFO",
            &format!("Processed event id={} topic={}", msg.id, msg.topic),
            None,
        );

        let start_trace = std::time::Instant::now();
        let started_at = chrono::Utc::now().to_rfc3339();

        let (command_id, hlc, operation_name, initial_input) =
            if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&msg.payload) {
                let cid = val
                    .pointer("/requestId")
                    .or_else(|| val.pointer("/commandId"))
                    .or_else(|| val.pointer("/id"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| {
                        if !msg.id.is_empty() {
                            msg.id.clone()
                        } else {
                            uuid::Uuid::now_v7().to_string()
                        }
                    });
                let h = val
                    .pointer("/hlc")
                    .map(|v| {
                        if v.is_string() {
                            v.as_str().unwrap_or_default().to_string()
                        } else {
                            v.to_string()
                        }
                    })
                    .unwrap_or_else(|| format!("{}", chrono::Utc::now().timestamp_micros()));
                let op = val
                    .pointer("/gql/operationName")
                    .or_else(|| val.pointer("/operationName"))
                    .or_else(|| val.pointer("/operation"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| {
                        msg.topic.split('.').last().unwrap_or("unknown").to_string()
                    });
                let inp = val
                    .pointer("/gql/jsonBody")
                    .or_else(|| val.pointer("/gql/variables"))
                    .or_else(|| val.pointer("/variables"))
                    .or_else(|| val.pointer("/request/gql/jsonBody"))
                    .cloned()
                    .unwrap_or_else(|| val.clone());
                (cid, h, op, inp)
            } else {
                let cid = if !msg.id.is_empty() {
                    msg.id.clone()
                } else {
                    uuid::Uuid::now_v7().to_string()
                };
                let h = format!("{}", chrono::Utc::now().timestamp_micros());
                let op = msg.topic.split('.').last().unwrap_or("unknown").to_string();
                (cid, h, op, serde_json::Value::Null)
            };

        let mut steps: Vec<FluxcellStepSpan> = Vec::new();

        // 1. Handle remote deployment events
        self.handle_deployment_event(msg).await;

        // 2. Handle built-in fluxcell event routing
        self.handle_builtin_events(msg, &mut steps).await;

        // 3. Dispatch event to matching WASM fluxcells
        let subscribed_cells = self.wasm_host.find_subscribed_fluxcells(&msg.topic);
        let mut all_succeeded = !steps.iter().any(|s| s.error.is_some());
        let mut dlq_requested = false;
        let mut failure_reason = steps.iter().find_map(|s| s.error.clone()).unwrap_or_default();

        for cell_name in subscribed_cells {
            if let Ok(payload_val) = serde_json::from_slice::<serde_json::Value>(&msg.payload) {
                let wasm_exec = self.wasm_host.clone();
                let tele_exec = self.telemetry.clone();
                let topic = msg.topic.clone();
                let cell = cell_name.clone();

                let offload = wasm_exec
                    .get_fluxcell_offload_strategy(&cell)
                    .unwrap_or_default();

                let step_start = std::time::Instant::now();
                let step_start_iso = chrono::Utc::now().to_rfc3339();

                let (res, host_calls) = match offload {
                    crate::config::OffloadStrategy::BlockingPool
                    | crate::config::OffloadStrategy::DedicatedWorker => {
                        let payload_clone = payload_val.clone();
                        tokio::task::spawn_blocking(move || {
                            wasm_exec.invoke_event_with_spans(&cell, &payload_clone)
                        })
                        .await
                        .unwrap_or_else(|e| (Err(anyhow::anyhow!("Join error: {}", e)), Vec::new()))
                    }
                    crate::config::OffloadStrategy::Inline => {
                        wasm_exec.invoke_event_with_spans(&cell, &payload_val)
                    }
                };

                let step_duration_ms = step_start.elapsed().as_secs_f64() * 1000.0;

                let (step_status, step_error, output_preview) = match &res {
                    Ok(r) => {
                        let status_str = r.get("status").and_then(|s| s.as_str()).unwrap_or("ok");
                        if status_str == "dead_letter" || status_str == "dead-letter" {
                            dlq_requested = true;
                            failure_reason = format!(
                                "Fluxcell '{}' requested dead_letter: {:?}",
                                cell_name,
                                r.get("error")
                            );
                            all_succeeded = false;
                            ("dead_letter".to_string(), Some(failure_reason.clone()), Some(r.clone()))
                        } else if status_str == "nack" || status_str == "error" {
                            all_succeeded = false;
                            failure_reason = format!(
                                "Fluxcell '{}' returned nack: {:?}",
                                cell_name,
                                r.get("error")
                            );
                            ("error".to_string(), Some(failure_reason.clone()), Some(r.clone()))
                        } else {
                            tele_exec.record_log(
                                "INFO",
                                &format!(
                                    "Fluxcell '{}' processed event topic='{}': status={}",
                                    cell_name, topic, status_str
                                ),
                                None,
                            );
                            ("ok".to_string(), None, Some(r.clone()))
                        }
                    }
                    Err(e) => {
                        all_succeeded = false;
                        failure_reason = format!("Fluxcell '{}' failed: {}", cell_name, e);
                        tele_exec.increment_error();
                        tele_exec.record_log(
                            "ERROR",
                            &format!(
                                "Fluxcell '{}' failed to process event topic='{}': {}",
                                cell_name, topic, e
                            ),
                            None,
                        );
                        ("error".to_string(), Some(e.to_string()), None)
                    }
                };

                steps.push(FluxcellStepSpan {
                    fluxcell_name: cell_name.clone(),
                    topic: topic.clone(),
                    function_name: "on_event".to_string(),
                    start_time: step_start_iso,
                    duration_ms: step_duration_ms,
                    status: step_status,
                    input_preview: Some(payload_val.clone()),
                    output_preview,
                    error: step_error,
                    host_calls,
                });

                if !all_succeeded {
                    break;
                }
            }
        }

        // 4. Record Domain Operation Trace
        let total_duration_ms = start_trace.elapsed().as_secs_f64() * 1000.0;
        let completed_at = chrono::Utc::now().to_rfc3339();
        let trace_status = if all_succeeded && !dlq_requested {
            "completed".to_string()
        } else if dlq_requested {
            "dlq".to_string()
        } else {
            "failed".to_string()
        };

        let terminal_output = steps.last().and_then(|s| s.output_preview.clone());

        let trace = DomainOperationTrace {
            command_id,
            hlc,
            topic: msg.topic.clone(),
            operation_name,
            ingress: "QUEUE".to_string(),
            worker_id: self.worker_id.clone(),
            status: trace_status,
            total_duration_ms,
            started_at,
            completed_at,
            initial_input,
            steps,
            terminal_output,
            error: if all_succeeded && !dlq_requested {
                None
            } else {
                Some(failure_reason.clone())
            },
        };

        if let Some(store) = &self.trace_storage {
            let _ = store.record_trace(&trace).await;
        }

        // 5. Broker Acknowledgment & Resilient Retry / DLQ
        if all_succeeded && !dlq_requested {
            let _ = self.broker.ack(msg).await;
        } else if dlq_requested || msg.delivery_attempt >= self.resilience.max_retries {
            if self.resilience.dlq_enabled {
                let dlq_topic = format!("{}{}", self.resilience.dlq_topic_prefix, msg.topic);
                let envelope = serde_json::json!({
                    "messageId": msg.id,
                    "originalTopic": msg.topic,
                    "payload": serde_json::from_slice::<serde_json::Value>(&msg.payload).unwrap_or(serde_json::Value::Null),
                    "deliveryAttempts": msg.delivery_attempt,
                    "reason": failure_reason,
                    "failedAt": chrono::Utc::now().to_rfc3339(),
                });
                if let Ok(dlq_bytes) = serde_json::to_vec(&envelope) {
                    let _ = self.broker.publish(&dlq_topic, &dlq_bytes).await;
                    self.telemetry.record_log(
                        "WARN",
                        &format!(
                            "Event '{}' routed to DLQ topic '{}' after {} attempts: {}",
                            msg.id, dlq_topic, msg.delivery_attempt, failure_reason
                        ),
                        None,
                    );
                }
            }
            let _ = self.broker.ack(msg).await;
        } else {
            let exp_factor = 2u64.saturating_pow(msg.delivery_attempt.saturating_sub(1));
            let base_delay = self.resilience.backoff_initial_ms.saturating_mul(exp_factor);
            let capped_delay = base_delay.min(self.resilience.backoff_max_ms);
            let jitter = (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_nanos() as u64 % 50)
                .min(capped_delay / 4);
            let delay = Duration::from_millis(capped_delay + jitter);
            self.telemetry.record_log(
                "WARN",
                &format!(
                    "Event '{}' nacked (attempt {}/{}), retrying in {:?}: {}",
                    msg.id, msg.delivery_attempt, self.resilience.max_retries, delay, failure_reason
                ),
                None,
            );
            let _ = self.broker.nack(msg, delay).await;
        }
    }

    async fn handle_deployment_event(&self, msg: &BrokerMessage) {
        if let Some(dep) = &self.deployer {
            if msg.topic.ends_with("deployfluxcell") || msg.topic == "deployer.deploy" {
                if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&msg.payload) {
                    let name = val
                        .pointer("/request/gql/jsonBody/variables/name")
                        .or_else(|| val.pointer("/variables/name"))
                        .or_else(|| val.pointer("/name"))
                        .and_then(|v| v.as_str())
                        .unwrap_or_default();
                    let artifact_url = val
                        .pointer("/request/gql/jsonBody/variables/artifactUrl")
                        .or_else(|| val.pointer("/request/gql/jsonBody/variables/artifact_url"))
                        .or_else(|| val.pointer("/variables/artifactUrl"))
                        .or_else(|| val.pointer("/artifact_url"))
                        .or_else(|| val.pointer("/artifactUrl"))
                        .and_then(|v| v.as_str())
                        .unwrap_or_default();
                    let sha256 = val
                        .pointer("/request/gql/jsonBody/variables/sha256")
                        .or_else(|| val.pointer("/variables/sha256"))
                        .or_else(|| val.pointer("/sha256"))
                        .and_then(|v| v.as_str())
                        .unwrap_or_default();
                    let mount_path = val
                        .pointer("/request/gql/jsonBody/variables/mountPath")
                        .or_else(|| val.pointer("/request/gql/jsonBody/variables/mount_path"))
                        .or_else(|| val.pointer("/variables/mountPath"))
                        .or_else(|| val.pointer("/mount_path"))
                        .or_else(|| val.pointer("/mountPath"))
                        .and_then(|v| v.as_str())
                        .unwrap_or_default();

                    if !name.is_empty() && !artifact_url.is_empty() && !sha256.is_empty() {
                        log::info!(
                            "Received deployFluxcell event for '{}' from '{}'",
                            name,
                            artifact_url
                        );
                        let dep_clone = dep.clone();
                        let name = name.to_string();
                        let artifact_url = artifact_url.to_string();
                        let sha256 = sha256.to_string();
                        let mount_path = mount_path.to_string();
                        tokio::spawn(async move {
                            match dep_clone
                                .stage_remote_artifact(
                                    &name,
                                    &artifact_url,
                                    &sha256,
                                    &mount_path,
                                    None,
                                    None,
                                    None,
                                )
                                .await
                            {
                                Ok(rec) => log::info!(
                                    "Successfully staged remote fluxcell '{}' (status: {:?})",
                                    rec.name,
                                    rec.status
                                ),
                                Err(e) => log::error!("Failed to stage remote fluxcell '{}': {}", name, e),
                            }
                        });
                    }
                }
            } else if msg.topic.ends_with("activatefluxcell") || msg.topic == "deployer.activate" {
                if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&msg.payload) {
                    let name = val
                        .pointer("/request/gql/jsonBody/variables/name")
                        .or_else(|| val.pointer("/variables/name"))
                        .or_else(|| val.pointer("/name"))
                        .and_then(|v| v.as_str())
                        .unwrap_or_default();
                    let sha256 = val
                        .pointer("/request/gql/jsonBody/variables/sha256")
                        .or_else(|| val.pointer("/variables/sha256"))
                        .or_else(|| val.pointer("/sha256"))
                        .and_then(|v| v.as_str())
                        .unwrap_or_default();
                    if !name.is_empty() && !sha256.is_empty() {
                        match dep.activate(name, sha256) {
                            Ok(rec) => log::info!("Activated fluxcell '{}' into production", rec.name),
                            Err(e) => log::error!("Failed to activate fluxcell '{}': {}", name, e),
                        }
                    }
                }
            }
        }
    }

    async fn handle_builtin_events(&self, msg: &BrokerMessage, steps: &mut Vec<FluxcellStepSpan>) {
        if msg.topic.ends_with("requestmagiclink") || msg.topic == "auth.magic_link" {
            let step_start = std::time::Instant::now();
            let step_start_iso = chrono::Utc::now().to_rfc3339();

            let payload_val: Option<serde_json::Value> = serde_json::from_slice(&msg.payload).ok();
            let payload_ref = payload_val.as_ref();

            let email = payload_ref.and_then(|val| {
                val.pointer("/request/gql/jsonBody/variables/email")
                    .or_else(|| val.pointer("/variables/email"))
                    .or_else(|| val.pointer("/email"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            }).unwrap_or_else(|| "user@example.com".to_string());

            let app_name_override = payload_ref.and_then(|val| {
                val.pointer("/request/gql/jsonBody/variables/appName")
                    .or_else(|| val.pointer("/variables/appName"))
                    .or_else(|| val.pointer("/branding/appName"))
                    .or_else(|| val.pointer("/appName"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            });

            let logo_url_override = payload_ref.and_then(|val| {
                val.pointer("/request/gql/jsonBody/variables/logoUrl")
                    .or_else(|| val.pointer("/variables/logoUrl"))
                    .or_else(|| val.pointer("/branding/logoUrl"))
                    .or_else(|| val.pointer("/logoUrl"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            });

            let accent_color_override = payload_ref.and_then(|val| {
                val.pointer("/request/gql/jsonBody/variables/accentColor")
                    .or_else(|| val.pointer("/variables/accentColor"))
                    .or_else(|| val.pointer("/branding/accentColor"))
                    .or_else(|| val.pointer("/accentColor"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            });

            let token = fluxcell_magic_link::mint_magic_token(&email);
            let code = fluxcell_magic_link::mint_magic_code();

            let kv_start = std::time::Instant::now();
            let kv_res_token = self
                .storage
                .set(&format!("magic_token:{}", token), &email, 900)
                .await;
            let kv_res_code = self
                .storage
                .set(&format!("magic_token:{}", code), &email, 900)
                .await;
            let kv_duration_us = kv_start.elapsed().as_micros() as u64;

            let host_calls = vec![
                HostCallSpan {
                    call_type: "kv:set".to_string(),
                    target: format!("magic_token:{}", token),
                    duration_us: kv_duration_us / 2,
                    status: if kv_res_token.is_ok() {
                        "ok".to_string()
                    } else {
                        "error".to_string()
                    },
                    detail: Some(format!("ttl: 900s, email: {}", email)),
                },
                HostCallSpan {
                    call_type: "kv:set".to_string(),
                    target: format!("magic_token:{}", code),
                    duration_us: kv_duration_us / 2,
                    status: if kv_res_code.is_ok() {
                        "ok".to_string()
                    } else {
                        "error".to_string()
                    },
                    detail: Some(format!("ttl: 900s, otp_code: {}", code)),
                },
            ];

            // Resolve dynamic email branding (payload override > 12-factor mailer_cfg defaults)
            let mailer_cfg = crate::mailer::MailerConfig::from_env();
            let branding = fluxcell_magic_link::EmailBranding {
                app_name: app_name_override.unwrap_or_else(|| mailer_cfg.app_name.clone()),
                logo_url: logo_url_override.or_else(|| mailer_cfg.logo_url.clone()),
                accent_color: accent_color_override.or_else(|| mailer_cfg.accent_color.clone()),
                support_email: None,
            };

            let verify_url = format!("{}/auth/verify?token={}", mailer_cfg.base_url, token);
            let rendered = fluxcell_magic_link::render_email_templates_branded(
                &email,
                &verify_url,
                Some(&code),
                Some(&branding),
            );
            let mail_res = crate::mailer::send_transactional_email(
                &mailer_cfg,
                &email,
                &rendered.subject,
                &rendered.html_body,
                &rendered.text_body,
            )
            .await;

            let step_duration_ms = step_start.elapsed().as_secs_f64() * 1000.0;
            let mail_err = mail_res.err();
            steps.push(FluxcellStepSpan {
                fluxcell_name: "magic_link".to_string(),
                topic: msg.topic.clone(),
                function_name: "mint_magic_token".to_string(),
                start_time: step_start_iso,
                duration_ms: step_duration_ms,
                status: if mail_err.is_none() { "ok".to_string() } else { "email_dispatch_error".to_string() },
                input_preview: Some(serde_json::json!({
                    "email": email,
                    "appName": branding.app_name,
                    "provider": format!("{:?}", mailer_cfg.provider)
                })),
                output_preview: Some(serde_json::json!({
                    "status": "minted",
                    "token": token,
                    "code": code,
                    "appName": branding.app_name,
                    "verify_url": verify_url
                })),
                error: mail_err.clone(),
                host_calls,
            });

            self.telemetry.record_log(
                if mail_err.is_none() { "INFO" } else { "WARN" },
                &format!(
                    "Minted magic link token for {} in storage (token: {}, provider: {:?}, status: {})",
                    email,
                    token,
                    mailer_cfg.provider,
                    if let Some(e) = mail_err { format!("mail_failed: {}", e) } else { "mail_dispatched".to_string() }
                ),
                None,
            );
        }
    }
}
