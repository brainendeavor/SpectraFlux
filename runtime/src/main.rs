use anyhow::{Context, Result};
use hyper_util::rt::TokioIo;
use hyper_util::server::conn::auto::Builder as ServerBuilder;
use spectra_flux::broker::create_broker;
use spectra_flux::config::FluxConfig;
use spectra_flux::db::{DatabaseRegistry, PostgresDb};
use spectra_flux::deployer::{DeployerGuard, DeployerRegistry, FluxcellDeployer, FluxcellStatus};
use spectra_flux::http::{handle_request, FluxRouter, RouteDefinition};
use spectra_flux::storage::create_storage;
use spectra_flux::telemetry::TelemetryClient;
use spectra_flux::wasm::{CircuitBreakerConfig, FluxcellWasmConfig, WasmHost};
use std::net::SocketAddr;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    log::info!("🚀 Initializing Spectral Flux Engine...");

    // 1. Load Configuration
    let config_path = std::env::var("FLUX_CONFIG").unwrap_or_else(|_| "spectral-flux.toml".to_string());
    let config = match FluxConfig::load_from_file(&config_path) {
        Ok(c) => {
            log::info!("Loaded configuration from '{}'", config_path);
            c
        }
        Err(e) => {
            log::warn!("Could not load '{}' ({}), using default configuration", config_path, e);
            FluxConfig::default_local()
        }
    };

    let worker_id = format!("spectral-flux-{}", uuid::Uuid::now_v7());
    log::info!("Instance Worker ID: {}", worker_id);

    // 2. Initialize Telemetry Client
    let telemetry = Arc::new(TelemetryClient::new(
        worker_id.clone(),
        config.broker.method.clone(),
        config.broker.stream.clone(),
        200,
    ));

    telemetry.record_log("INFO", "Spectral Flux engine initializing", None);

    // Start heartbeat reporter if gateway admin URL configured
    let stop_signal = Arc::new(AtomicBool::new(false));
    if let Some(admin_url) = &config.gateway_admin_url {
        log::info!("Configuring telemetry heartbeat to Gateway at '{}'...", admin_url);
        telemetry.clone().start_heartbeat_task(
            admin_url.clone(),
            Duration::from_secs(3),
            stop_signal.clone(),
        );
    }

    // 3. Initialize Unified Storage
    log::info!("Initializing storage backend '{}'...", config.storage.backend);
    let storage = create_storage(&config.storage.backend, config.storage.addr.as_deref())
        .await
        .context("Failed to initialize storage engine")?;
    telemetry.record_log("INFO", &format!("Storage backend '{}' ready", config.storage.backend), None);
    let trace_storage = Arc::new(spectra_flux::telemetry::DomainTraceStorage::new(storage.clone()));

    // 3.5. Initialize Database Registry
    let mut db_registry = DatabaseRegistry::new();
    for (name, db_cfg) in &config.databases {
        log::info!("Connecting to database '{}'...", name);
        match PostgresDb::new(&db_cfg.url, db_cfg.max_connections) {
            Ok(db) => {
                let is_default = name == "default";
                db_registry.register(name, Arc::new(db), is_default);
                log::info!("Database '{}' registered", name);
            }
            Err(e) => {
                log::warn!("Failed to initialize database '{}': {:#}", name, e);
            }
        }
    }
    if db_registry.is_empty() {
        if let Some(db_url) = &config.database.url {
            match PostgresDb::new(db_url, config.database.max_connections) {
                Ok(db) => {
                    db_registry.register("default", Arc::new(db), true);
                    log::info!("Registered default database from config.database");
                }
                Err(e) => {
                    log::warn!("Failed to initialize default database: {:#}", e);
                }
            }
        }
    }

    // 4. Initialize WASM Host
    log::info!("Initializing Wasmtime Host Engine with epoch interruption...");
    let wasm_host = Arc::new(
        WasmHost::with_db(5, Some(storage.clone()), Some(Arc::new(db_registry)))
            .context("Failed to initialize WASM host engine")?,
    );

    // 5. Initialize Router with Collision Detection
    let mut router = FluxRouter::new();

    for (name, cell_cfg) in &config.fluxcells {
        if !cell_cfg.enabled {
            continue;
        }

        log::info!("Loading fluxcell '{}' mounted at '{}'...", name, cell_cfg.mount_path);

        // Check if wasm module file exists
        if std::path::Path::new(&cell_cfg.wasm_module).exists() {
            let bytes = std::fs::read(&cell_cfg.wasm_module)
                .with_context(|| format!("Failed to read WASM module '{}'", cell_cfg.wasm_module))?;

            let resolved = config.resolve_cell_execution(name, cell_cfg, None, None, None)?;
            let wasm_cfg = FluxcellWasmConfig {
                profile: resolved.profile,
                timeout_ms: resolved.timeout_ms,
                max_memory_bytes: resolved.max_memory_bytes,
                max_instances: resolved.max_instances,
                offload: resolved.offload,
                circuit_breaker: CircuitBreakerConfig::default(),
            };

            wasm_host
                .register_wasm_bytes_with_subs(
                    name,
                    &bytes,
                    wasm_cfg,
                    cell_cfg.subscriptions.clone(),
                )
                .with_context(|| format!("Failed to register WASM module '{}'", cell_cfg.wasm_module))?;

            if let Some(routes) = wasm_host.get_fluxcell_routes(name) {
                let route_count = routes.len();
                router
                    .register_fluxcell_routes(name, &cell_cfg.mount_path, &routes)
                    .with_context(|| format!("Route collision detected for fluxcell '{}'", name))?;
                log::info!("Registered {} WASM routes for '{}' at '{}'", route_count, name, cell_cfg.mount_path);
                telemetry.record_log(
                    "INFO",
                    &format!("✨ Fluxcell '{}' is ALIVE: mounted at '{}' with {} WASM route(s)", name, cell_cfg.mount_path, route_count),
                    None,
                );
            } else {
                log::warn!("No routes exported by WASM fluxcell '{}'", name);
                telemetry.record_log(
                    "WARN",
                    &format!("Fluxcell '{}' loaded but exported 0 routes", name),
                    None,
                );
            }
        } else {
            // Built-in fallback routes for out-of-the-box fluxcells
            let routes = match name.as_str() {
                "magic_link" | "magic-link" => vec![
                    RouteDefinition::new("GET", "/verify", "Verify magic link token"),
                    RouteDefinition::new("POST", "/verify", "Redeem magic link token"),
                    RouteDefinition::new("GET", "/status", "Auth service status"),
                ],
                "webhook" => vec![
                    RouteDefinition::new("GET", "/health", "Webhook service health"),
                    RouteDefinition::new("GET", "/dlq", "Dead-letter queue status"),
                    RouteDefinition::new("POST", "/test", "Test webhook delivery"),
                ],
                _ => Vec::new(),
            };

            if !routes.is_empty() {
                router
                    .register_fluxcell_routes(name, &cell_cfg.mount_path, &routes)
                    .with_context(|| format!("Route collision detected for fluxcell '{}'", name))?;
                log::info!("Registered {} built-in routes for fluxcell '{}'", routes.len(), name);
                telemetry.record_log(
                    "INFO",
                    &format!("✨ Fluxcell '{}' is ALIVE: mounted at '{}' with {} built-in route(s)", name, cell_cfg.mount_path, routes.len()),
                    None,
                );
            }
        }
    }

    let shared_router = Arc::new(RwLock::new(router));

    // 6. Initialize Deployer Subsystem (if enabled)
    let deployer: Option<Arc<FluxcellDeployer>> = if config.deployer.enabled {
        log::info!("Initializing Fluxcell Deployer subsystem (storage: {:?})...", config.deployer.storage_dir);
        let guard = Arc::new(DeployerGuard::new(
            config.deployer.external_deploy_enabled,
            config.deployer.dev_upload_enabled,
        ));
        let registry = Arc::new(DeployerRegistry::new(&config.deployer.storage_dir)?);
        let dep = Arc::new(FluxcellDeployer::new(
            config.deployer.clone(),
            guard,
            registry.clone(),
            wasm_host.clone(),
            shared_router.clone(),
        ));

        // Boot-load persisted active cells from fluxcells.json
        let persisted = registry.list_records();
        for record in persisted {
            if record.status == FluxcellStatus::Active {
                let wasm_file = registry.storage_dir().join(&record.wasm_file);
                if wasm_file.exists() {
                    match std::fs::read(&wasm_file) {
                        Ok(bytes) => {
                            let wasm_cfg = FluxcellWasmConfig {
                                profile: record.profile.clone(),
                                timeout_ms: record.timeout_ms,
                                max_memory_bytes: record.max_memory_bytes,
                                max_instances: record.max_instances,
                                offload: record.offload,
                                circuit_breaker: CircuitBreakerConfig::default(),
                            };
                            if let Err(e) = wasm_host.register_wasm_bytes(&record.name, &bytes, wasm_cfg) {
                                log::error!("Failed to register persisted fluxcell '{}': {}", record.name, e);
                            } else {
                                let mut router_lock = shared_router.write().unwrap();
                                if let Err(e) = router_lock.register_fluxcell_routes(&record.name, &record.mount_path, &record.routes) {
                                    log::error!("Failed to mount routes for persisted fluxcell '{}': {}", record.name, e);
                                } else {
                                    log::info!("Restored active fluxcell '{}' (v{}) mounted at '{}'", record.name, record.version, record.mount_path);
                                }
                            }
                        }
                        Err(e) => log::error!("Failed to read persisted wasm file {:?}: {}", wasm_file, e),
                    }
                }
            }
        }

        Some(dep)
    } else {
        log::info!("Fluxcell Deployer subsystem is disabled in configuration.");
        None
    };

    // 7. Connect Broker Consumer Loop
    log::info!("Connecting to broker '{}' at '{}'...", config.broker.method, config.broker.addr);
    match create_broker(&config.broker).await {
        Ok(broker) => {
            let tele_clone = telemetry.clone();
            let storage_clone = storage.clone();
            let dep_broker = deployer.clone();
            let wasm_clone = wasm_host.clone();
            let resilience_clone = config.resilience.clone();
            let trace_store_clone = trace_storage.clone();
            let worker_id_broker = worker_id.clone();
            let subjects = vec![
                "mutation.>".to_string(),
                "webhook.>".to_string(),
                "auth.>".to_string(),
                "deployer.>".to_string(),
            ];
            let group = config.broker.consumer_group.clone();

            tokio::spawn(async move {
                match broker.subscribe(&subjects, &group).await {
                    Ok(mut stream) => {
                        use futures_util::StreamExt;
                        log::info!("Broker consumer subscribed to subjects: {:?}", subjects);
                        while let Some(msg) = stream.next().await {
                            tele_clone.increment_processed(None);
                            tele_clone.record_log(
                                "INFO",
                                &format!("Processed event id={} topic={}", msg.id, msg.topic),
                                None,
                            );

                            let start_trace = std::time::Instant::now();
                            let started_at = chrono::Utc::now().to_rfc3339();

                            let (command_id, hlc, operation_name, initial_input) = if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&msg.payload) {
                                let cid = val.pointer("/requestId")
                                    .or_else(|| val.pointer("/commandId"))
                                    .or_else(|| val.pointer("/id"))
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.to_string())
                                    .unwrap_or_else(|| if !msg.id.is_empty() { msg.id.clone() } else { uuid::Uuid::now_v7().to_string() });
                                let h = val.pointer("/hlc")
                                    .map(|v| if v.is_string() { v.as_str().unwrap().to_string() } else { v.to_string() })
                                    .unwrap_or_else(|| format!("{}", chrono::Utc::now().timestamp_micros()));
                                let op = val.pointer("/gql/operationName")
                                    .or_else(|| val.pointer("/operationName"))
                                    .or_else(|| val.pointer("/operation"))
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.to_string())
                                    .unwrap_or_else(|| {
                                        msg.topic.split('.').last().unwrap_or("unknown").to_string()
                                    });
                                let inp = val.pointer("/gql/jsonBody")
                                    .or_else(|| val.pointer("/gql/variables"))
                                    .or_else(|| val.pointer("/variables"))
                                    .or_else(|| val.pointer("/request/gql/jsonBody"))
                                    .cloned()
                                    .unwrap_or_else(|| val.clone());
                                (cid, h, op, inp)
                            } else {
                                let cid = if !msg.id.is_empty() { msg.id.clone() } else { uuid::Uuid::now_v7().to_string() };
                                let h = format!("{}", chrono::Utc::now().timestamp_micros());
                                let op = msg.topic.split('.').last().unwrap_or("unknown").to_string();
                                (cid, h, op, serde_json::Value::Null)
                            };

                            let mut steps: Vec<spectra_flux::telemetry::FluxcellStepSpan> = Vec::new();

                            // Handle deployment events
                            if (msg.topic.ends_with("deployfluxcell") || msg.topic == "deployer.deploy") && dep_broker.is_some() {
                                if let Some(dep) = &dep_broker {
                                    if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&msg.payload) {
                                        let name = val.pointer("/request/gql/jsonBody/variables/name")
                                            .or_else(|| val.pointer("/variables/name"))
                                            .or_else(|| val.pointer("/name"))
                                            .and_then(|v| v.as_str())
                                            .unwrap_or_default();
                                        let artifact_url = val.pointer("/request/gql/jsonBody/variables/artifactUrl")
                                            .or_else(|| val.pointer("/request/gql/jsonBody/variables/artifact_url"))
                                            .or_else(|| val.pointer("/variables/artifactUrl"))
                                            .or_else(|| val.pointer("/artifact_url"))
                                            .or_else(|| val.pointer("/artifactUrl"))
                                            .and_then(|v| v.as_str())
                                            .unwrap_or_default();
                                        let sha256 = val.pointer("/request/gql/jsonBody/variables/sha256")
                                            .or_else(|| val.pointer("/variables/sha256"))
                                            .or_else(|| val.pointer("/sha256"))
                                            .and_then(|v| v.as_str())
                                            .unwrap_or_default();
                                        let mount_path = val.pointer("/request/gql/jsonBody/variables/mountPath")
                                            .or_else(|| val.pointer("/request/gql/jsonBody/variables/mount_path"))
                                            .or_else(|| val.pointer("/variables/mountPath"))
                                            .or_else(|| val.pointer("/mount_path"))
                                            .or_else(|| val.pointer("/mountPath"))
                                            .and_then(|v| v.as_str())
                                            .unwrap_or_default();

                                        if !name.is_empty() && !artifact_url.is_empty() && !sha256.is_empty() {
                                            log::info!("Received deployFluxcell event for '{}' from '{}'", name, artifact_url);
                                            let dep_clone = dep.clone();
                                            let name = name.to_string();
                                            let artifact_url = artifact_url.to_string();
                                            let sha256 = sha256.to_string();
                                            let mount_path = mount_path.to_string();
                                            tokio::spawn(async move {
                                                match dep_clone.stage_remote_artifact(&name, &artifact_url, &sha256, &mount_path, None, None, None).await {
                                                    Ok(rec) => log::info!("Successfully staged remote fluxcell '{}' (status: {:?})", rec.name, rec.status),
                                                    Err(e) => log::error!("Failed to stage remote fluxcell '{}': {}", name, e),
                                                }
                                            });
                                        }
                                    }
                                }
                            } else if (msg.topic.ends_with("activatefluxcell") || msg.topic == "deployer.activate") && dep_broker.is_some() {
                                if let Some(dep) = &dep_broker {
                                    if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&msg.payload) {
                                        let name = val.pointer("/request/gql/jsonBody/variables/name")
                                            .or_else(|| val.pointer("/variables/name"))
                                            .or_else(|| val.pointer("/name"))
                                            .and_then(|v| v.as_str())
                                            .unwrap_or_default();
                                        let sha256 = val.pointer("/request/gql/jsonBody/variables/sha256")
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

                            // Handle built-in fluxcell event routing
                            if msg.topic.ends_with("requestmagiclink") || msg.topic == "auth.magic_link" {
                                let step_start = std::time::Instant::now();
                                let step_start_iso = chrono::Utc::now().to_rfc3339();
                                let email = if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&msg.payload) {
                                    val.pointer("/request/gql/jsonBody/variables/email")
                                        .or_else(|| val.pointer("/variables/email"))
                                        .or_else(|| val.pointer("/email"))
                                        .and_then(|v| v.as_str())
                                        .map(|s| s.to_string())
                                        .unwrap_or_else(|| "user@example.com".to_string())
                                } else {
                                    "user@example.com".to_string()
                                };
                                let token = fluxcell_magic_link::mint_magic_token(&email);
                                let kv_start = std::time::Instant::now();
                                let kv_res = storage_clone.set(&format!("magic_token:{}", token), &email, 900).await;
                                let kv_duration_us = kv_start.elapsed().as_micros() as u64;

                                let host_calls = vec![
                                    spectra_flux::telemetry::HostCallSpan {
                                        call_type: "kv:set".to_string(),
                                        target: format!("magic_token:{}", token),
                                        duration_us: kv_duration_us,
                                        status: if kv_res.is_ok() { "ok".to_string() } else { "error".to_string() },
                                        detail: Some(format!("ttl: 900s, email: {}", email)),
                                    }
                                ];

                                let step_duration_ms = step_start.elapsed().as_secs_f64() * 1000.0;
                                steps.push(spectra_flux::telemetry::FluxcellStepSpan {
                                    fluxcell_name: "magic_link".to_string(),
                                    topic: msg.topic.clone(),
                                    function_name: "mint_magic_token".to_string(),
                                    start_time: step_start_iso,
                                    duration_ms: step_duration_ms,
                                    status: "ok".to_string(),
                                    input_preview: Some(serde_json::json!({ "email": email })),
                                    output_preview: Some(serde_json::json!({ "status": "minted", "token": token })),
                                    error: None,
                                    host_calls,
                                });

                                tele_clone.record_log(
                                    "INFO",
                                    &format!("Minted magic link token for {} in storage (token: {})", email, token),
                                    None,
                                );
                            }

                            // Dispatch event to matching WASM fluxcells
                            let subscribed_cells = wasm_clone.find_subscribed_fluxcells(&msg.topic);
                            let mut all_succeeded = true;
                            let mut dlq_requested = false;
                            let mut failure_reason = String::new();

                            for cell_name in subscribed_cells {
                                if let Ok(payload_val) = serde_json::from_slice::<serde_json::Value>(&msg.payload) {
                                    let wasm_exec = wasm_clone.clone();
                                    let tele_exec = tele_clone.clone();
                                    let topic = msg.topic.clone();
                                    let cell = cell_name.clone();

                                    let offload = wasm_exec
                                        .get_fluxcell_offload_strategy(&cell)
                                        .unwrap_or_default();

                                    let step_start = std::time::Instant::now();
                                    let step_start_iso = chrono::Utc::now().to_rfc3339();

                                    let (res, host_calls) = match offload {
                                        spectra_flux::config::OffloadStrategy::BlockingPool
                                        | spectra_flux::config::OffloadStrategy::DedicatedWorker => {
                                            let payload_clone = payload_val.clone();
                                            tokio::task::spawn_blocking(move || {
                                                wasm_exec.invoke_event_with_spans(&cell, &payload_clone)
                                            })
                                            .await
                                            .unwrap_or_else(|e| (Err(anyhow::anyhow!("Join error: {}", e)), Vec::new()))
                                        }
                                        spectra_flux::config::OffloadStrategy::Inline => {
                                            wasm_exec.invoke_event_with_spans(&cell, &payload_val)
                                        }
                                    };

                                    let step_duration_ms = step_start.elapsed().as_secs_f64() * 1000.0;

                                    let (step_status, step_error, output_preview) = match &res {
                                        Ok(r) => {
                                            let status_str = r.get("status").and_then(|s| s.as_str()).unwrap_or("ok");
                                            if status_str == "dead_letter" || status_str == "dead-letter" {
                                                dlq_requested = true;
                                                failure_reason = format!("Fluxcell '{}' requested dead_letter: {:?}", cell_name, r.get("error"));
                                                all_succeeded = false;
                                                ("dead_letter".to_string(), Some(failure_reason.clone()), Some(r.clone()))
                                            } else if status_str == "nack" || status_str == "error" {
                                                all_succeeded = false;
                                                failure_reason = format!("Fluxcell '{}' returned nack: {:?}", cell_name, r.get("error"));
                                                ("error".to_string(), Some(failure_reason.clone()), Some(r.clone()))
                                            } else {
                                                tele_exec.record_log(
                                                    "INFO",
                                                    &format!("Fluxcell '{}' processed event topic='{}': status={}", cell_name, topic, status_str),
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
                                                &format!("Fluxcell '{}' failed to process event topic='{}': {}", cell_name, topic, e),
                                                None,
                                            );
                                            ("error".to_string(), Some(e.to_string()), None)
                                        }
                                    };

                                    steps.push(spectra_flux::telemetry::FluxcellStepSpan {
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

                            // Record Domain Operation Trace
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

                            let trace = spectra_flux::telemetry::DomainOperationTrace {
                                command_id,
                                hlc,
                                topic: msg.topic.clone(),
                                operation_name,
                                ingress: "QUEUE".to_string(),
                                worker_id: worker_id_broker.clone(),
                                status: trace_status,
                                total_duration_ms,
                                started_at,
                                completed_at,
                                initial_input,
                                steps,
                                terminal_output,
                                error: if all_succeeded && !dlq_requested { None } else { Some(failure_reason.clone()) },
                            };

                            let _ = trace_store_clone.record_trace(&trace).await;

                            if all_succeeded && !dlq_requested {
                                let _ = broker.ack(&msg).await;
                            } else if dlq_requested || msg.delivery_attempt >= resilience_clone.max_retries {
                                // Route to Dead-Letter Queue if enabled
                                if resilience_clone.dlq_enabled {
                                    let dlq_topic = format!("{}{}", resilience_clone.dlq_topic_prefix, msg.topic);
                                    let envelope = serde_json::json!({
                                        "messageId": msg.id,
                                        "originalTopic": msg.topic,
                                        "payload": serde_json::from_slice::<serde_json::Value>(&msg.payload).unwrap_or(serde_json::Value::Null),
                                        "deliveryAttempts": msg.delivery_attempt,
                                        "reason": failure_reason,
                                        "failedAt": chrono::Utc::now().to_rfc3339(),
                                    });
                                    if let Ok(dlq_bytes) = serde_json::to_vec(&envelope) {
                                        let _ = broker.publish(&dlq_topic, &dlq_bytes).await;
                                        tele_clone.record_log(
                                            "WARN",
                                            &format!("Event '{}' routed to DLQ topic '{}' after {} attempts: {}", msg.id, dlq_topic, msg.delivery_attempt, failure_reason),
                                            None,
                                        );
                                    }
                                }
                                let _ = broker.ack(&msg).await;
                            } else {
                                // Compute exponential backoff with jitter
                                let exp_factor = 2u64.saturating_pow(msg.delivery_attempt.saturating_sub(1));
                                let base_delay = resilience_clone.backoff_initial_ms.saturating_mul(exp_factor);
                                let capped_delay = base_delay.min(resilience_clone.backoff_max_ms);
                                let jitter = (std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .subsec_nanos() as u64 % 50)
                                    .min(capped_delay / 4);
                                let delay = std::time::Duration::from_millis(capped_delay + jitter);
                                tele_clone.record_log(
                                    "WARN",
                                    &format!("Event '{}' nacked (attempt {}/{}), retrying in {:?}: {}", msg.id, msg.delivery_attempt, resilience_clone.max_retries, delay, failure_reason),
                                    None,
                                );
                                let _ = broker.nack(&msg, delay).await;
                            }
                        }
                    }
                    Err(e) => {
                        log::warn!("Broker subscription could not be established: {}", e);
                    }
                }
            });
        }
        Err(e) => {
            log::warn!("Broker connection failed: {}. Continuing in standalone API mode.", e);
        }
    }

    // 8. Start HTTP Server (:8081)
    let addr: SocketAddr = format!("{}:{}", config.host, config.port).parse()?;
    let listener = TcpListener::bind(addr).await?;
    log::info!("🚀 Spectral Flux HTTP Support Server listening on http://{}", addr);
    telemetry.record_log("INFO", &format!("🚀 Spectral Flux HTTP server listening on http://{}", addr), None);

    let router_arc = shared_router.clone();
    let telemetry_arc = telemetry.clone();
    let wasm_dispatcher = wasm_host.clone();
    let deployer_arc = deployer.clone();
    let trace_storage_arc = trace_storage.clone();

    tokio::select! {
        _ = async {
            loop {
                let (stream, _) = match listener.accept().await {
                    Ok(res) => res,
                    Err(e) => {
                        log::error!("TCP accept error: {}", e);
                        continue;
                    }
                };

                let io = TokioIo::new(stream);
                let router_clone = router_arc.clone();
                let tele_clone = telemetry_arc.clone();
                let dispatcher_clone = wasm_dispatcher.clone();
                let deployer_clone = deployer_arc.clone();
                let trace_storage_clone = trace_storage_arc.clone();

                tokio::spawn(async move {
                    let service = hyper::service::service_fn(move |req| {
                        handle_request(
                            req,
                            router_clone.clone(),
                            tele_clone.clone(),
                            dispatcher_clone.clone(),
                            deployer_clone.clone(),
                            Some(trace_storage_clone.clone()),
                        )
                    });

                    if let Err(err) = ServerBuilder::new(hyper_util::rt::TokioExecutor::new())
                        .serve_connection(io, service)
                        .await
                    {
                        log::debug!("HTTP connection closed: {:?}", err);
                    }
                });
            }
        } => {}
        _ = tokio::signal::ctrl_c() => {
            log::info!("Shutting down Spectral Flux Engine gracefully...");
            stop_signal.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    log::info!("Spectral Flux Engine shutdown complete.");
    Ok(())
}
