use super::json_response;
use crate::http::{FluxRouter, FluxcellHttpDispatcher, RouterError};
use crate::telemetry::{DomainTraceStorage, TelemetryClient};
use http_body_util::{BodyExt, Full};
use hyper::{Request, Response, StatusCode};
use std::sync::{Arc, RwLock};

pub async fn handle_fluxcell_dispatch<B>(
    req: Request<B>,
    path: &str,
    query_string: Option<&str>,
    method: &str,
    router: &RwLock<FluxRouter>,
    telemetry: &TelemetryClient,
    dispatcher: &dyn FluxcellHttpDispatcher,
    trace_storage: Option<&Arc<DomainTraceStorage>>,
) -> Response<Full<bytes::Bytes>>
where
    B: hyper::body::Body + Send + 'static,
    B::Data: Send,
    B::Error: std::error::Error + Send + Sync + 'static,
{
    // 1. Lookup route in FluxRouter (poison-safe lock recovery)
    let (fluxcell_name, relative_path_matched) = {
        let router_lock = router.read().unwrap_or_else(|e| e.into_inner());
        match router_lock.lookup(method, path) {
            Ok(m) => (m.fluxcell_name.to_string(), m.relative_path.to_string()),
            Err(RouterError::NotFound { .. }) => {
                return json_response(
                    StatusCode::NOT_FOUND,
                    "{\"error\":\"NOT_FOUND\",\"message\":\"No fluxcell route matches request\"}",
                );
            }
            Err(RouterError::MethodNotAllowed { allowed, .. }) => {
                let allow_header = allowed.join(", ");
                return Response::builder()
                    .status(StatusCode::METHOD_NOT_ALLOWED)
                    .header("Content-Type", "application/json")
                    .header("Allow", allow_header)
                    .body(Full::new(bytes::Bytes::from(
                        "{\"error\":\"METHOD_NOT_ALLOWED\"}",
                    )))
                    .unwrap_or_else(|_| Response::new(Full::new(bytes::Bytes::new())));
            }
            Err(e) => {
                return json_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("{{\"error\":\"ROUTER_ERROR\",\"message\":\"{}\"}}", e),
                );
            }
        }
    };

    // 2. Collect headers & body
    let headers: Vec<(String, String)> = req
        .headers()
        .iter()
        .filter_map(|(k, v)| v.to_str().ok().map(|val| (k.as_str().to_string(), val.to_string())))
        .collect();

    let body_bytes = match req.into_body().collect().await {
        Ok(collected) => collected.to_bytes().to_vec(),
        Err(e) => {
            return json_response(
                StatusCode::BAD_REQUEST,
                format!("{{\"error\":\"BODY_READ_ERROR\",\"message\":\"{}\"}}", e),
            );
        }
    };

    // 3. Prepare path and tracing metadata
    let relative_path = if let Some(q) = query_string {
        format!("{}?{}", relative_path_matched, q)
    } else {
        relative_path_matched
    };

    let start_instant = std::time::Instant::now();
    let started_at = chrono::Utc::now().to_rfc3339();

    let payload_val: serde_json::Value =
        serde_json::from_slice(&body_bytes).unwrap_or(serde_json::Value::Null);

    let command_id = headers
        .iter()
        .find(|(k, _)| {
            k.eq_ignore_ascii_case("x-command-id")
                || k.eq_ignore_ascii_case("x-spectra-request-id")
                || k.eq_ignore_ascii_case("x-request-id")
                || k.eq_ignore_ascii_case("idempotency-key")
        })
        .map(|(_, v)| v.clone())
        .or_else(|| {
            payload_val
                .get("commandId")
                .or_else(|| payload_val.get("command_id"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| uuid::Uuid::now_v7().to_string());

    let hlc = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("x-spectra-hlc") || k.eq_ignore_ascii_case("x-hlc"))
        .map(|(_, v)| v.clone())
        .or_else(|| {
            payload_val
                .get("hlc")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| {
            format!(
                "{}.000001",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis()
            )
        });

    let operation_name = payload_val
        .get("operationName")
        .or_else(|| payload_val.get("operation_name"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("{} {}", method, path));

    // 4. Dispatch to Fluxcell guest via dispatcher
    match dispatcher
        .dispatch_with_spans(&fluxcell_name, &relative_path, method, headers, body_bytes)
        .await
    {
        Ok((status_code, resp_headers, resp_body, host_calls)) => {
            let duration_ms = start_instant.elapsed().as_secs_f64() * 1000.0;
            let completed_at = chrono::Utc::now().to_rfc3339();
            let is_ok = status_code < 400;

            if is_ok {
                telemetry.increment_processed(None);
            } else {
                telemetry.increment_error();
            }

            telemetry.record_log(
                if is_ok { "INFO" } else { "ERROR" },
                &format!(
                    "HTTP {} '{}' [{}] -> HTTP {} ({:.2}ms)",
                    method, path, operation_name, status_code, duration_ms
                ),
                Some(hlc.clone()),
            );

            if let Some(store) = trace_storage {
                let resp_val: serde_json::Value =
                    serde_json::from_slice(&resp_body).unwrap_or_else(|_| {
                        if let Ok(s) = std::str::from_utf8(&resp_body) {
                            serde_json::Value::String(s.to_string())
                        } else {
                            serde_json::Value::Null
                        }
                    });

                let step_status = if is_ok {
                    "ok".to_string()
                } else {
                    "error".to_string()
                };
                let step = crate::telemetry::FluxcellStepSpan {
                    fluxcell_name: fluxcell_name.clone(),
                    topic: format!("http:{}", path),
                    function_name: "handle_http".to_string(),
                    start_time: started_at.clone(),
                    duration_ms,
                    status: step_status,
                    input_preview: if payload_val.is_null() {
                        None
                    } else {
                        Some(payload_val.clone())
                    },
                    output_preview: Some(resp_val.clone()),
                    error: if is_ok {
                        None
                    } else {
                        Some(format!("HTTP {}", status_code))
                    },
                    host_calls,
                };

                let trace = crate::telemetry::DomainOperationTrace {
                    command_id,
                    hlc,
                    topic: format!("http:{}", path),
                    operation_name,
                    ingress: "HTTP".to_string(),
                    worker_id: "chassis-http".to_string(),
                    status: if is_ok {
                        "completed".to_string()
                    } else {
                        "failed".to_string()
                    },
                    total_duration_ms: duration_ms,
                    started_at,
                    completed_at,
                    initial_input: payload_val,
                    steps: vec![step],
                    terminal_output: Some(resp_val),
                    error: if is_ok {
                        None
                    } else {
                        Some(format!("HTTP {}", status_code))
                    },
                };

                let store_clone = (*store).clone();
                tokio::spawn(async move {
                    let _ = store_clone.record_trace(&trace).await;
                });
            }

            let mut builder = Response::builder().status(status_code);
            for (k, v) in resp_headers {
                builder = builder.header(k, v);
            }
            builder
                .body(Full::new(bytes::Bytes::from(resp_body)))
                .unwrap_or_else(|_| Response::new(Full::new(bytes::Bytes::new())))
        }
        Err(e) => {
            telemetry.increment_error();
            let duration_ms = start_instant.elapsed().as_secs_f64() * 1000.0;
            let completed_at = chrono::Utc::now().to_rfc3339();

            telemetry.record_log(
                "ERROR",
                &format!(
                    "HTTP {} '{}' [{}] dispatch failed: {}",
                    method, path, operation_name, e
                ),
                Some(hlc.clone()),
            );

            if let Some(store) = trace_storage {
                let step = crate::telemetry::FluxcellStepSpan {
                    fluxcell_name: fluxcell_name.clone(),
                    topic: format!("http:{}", path),
                    function_name: "handle_http".to_string(),
                    start_time: started_at.clone(),
                    duration_ms,
                    status: "error".to_string(),
                    input_preview: if payload_val.is_null() {
                        None
                    } else {
                        Some(payload_val.clone())
                    },
                    output_preview: None,
                    error: Some(e.to_string()),
                    host_calls: Vec::new(),
                };

                let trace = crate::telemetry::DomainOperationTrace {
                    command_id,
                    hlc,
                    topic: format!("http:{}", path),
                    operation_name,
                    ingress: "HTTP".to_string(),
                    worker_id: "chassis-http".to_string(),
                    status: "failed".to_string(),
                    total_duration_ms: duration_ms,
                    started_at,
                    completed_at,
                    initial_input: payload_val,
                    steps: vec![step],
                    terminal_output: None,
                    error: Some(e.to_string()),
                };

                let store_clone = (*store).clone();
                tokio::spawn(async move {
                    let _ = store_clone.record_trace(&trace).await;
                });
            }

            json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!(
                    "{{\"error\":\"FLUXCELL_DISPATCH_ERROR\",\"message\":\"{}\"}}",
                    e
                ),
            )
        }
    }
}
