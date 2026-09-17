use super::{html_response, json_response};
use crate::http::{FluxRouter, ADMIN_HTML};
use crate::telemetry::{DomainTraceStorage, TelemetryClient};
use http_body_util::Full;
use hyper::{Response, StatusCode};
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};

pub fn handle_dashboard() -> Response<Full<bytes::Bytes>> {
    html_response(StatusCode::OK, ADMIN_HTML)
}

pub fn handle_logs(telemetry: &TelemetryClient) -> Response<Full<bytes::Bytes>> {
    let logs = telemetry.get_recent_logs();
    let body = serde_json::to_string(&logs).unwrap_or_else(|_| "[]".to_string());
    json_response(StatusCode::OK, body)
}

pub fn handle_overview(
    telemetry: &TelemetryClient,
    router: &RwLock<FluxRouter>,
) -> Response<Full<bytes::Bytes>> {
    let uptime = telemetry.started_at.elapsed().as_secs();
    let processed = telemetry.processed_events.load(Ordering::Relaxed);
    let errors = telemetry.error_count.load(Ordering::Relaxed);
    let cells = {
        let r = router.read().unwrap_or_else(|e| e.into_inner());
        r.get_fluxcells_summary()
    };
    let body = serde_json::json!({
        "workerId": telemetry.worker_id,
        "uptimeSeconds": uptime,
        "processedEvents": processed,
        "errors": errors,
        "fluxcells": cells,
    });
    json_response(StatusCode::OK, body.to_string())
}

pub async fn handle_traces(
    trace_storage: Option<&Arc<DomainTraceStorage>>,
    query_string: Option<&str>,
) -> Response<Full<bytes::Bytes>> {
    if let Some(ts) = trace_storage {
        let limit = query_string
            .and_then(|q| {
                for param in q.split('&') {
                    let mut kv = param.split('=');
                    if let (Some("limit"), Some(v)) = (kv.next(), kv.next()) {
                        return v.parse::<usize>().ok();
                    }
                }
                None
            })
            .unwrap_or(50);

        let summaries = ts.list_recent_traces(limit).await.unwrap_or_default();
        let body = serde_json::to_string(&summaries).unwrap_or_else(|_| "[]".to_string());
        json_response(StatusCode::OK, body)
    } else {
        json_response(StatusCode::OK, "[]")
    }
}

pub async fn handle_trace_by_id(
    trace_storage: Option<&Arc<DomainTraceStorage>>,
    cmd_id: &str,
) -> Response<Full<bytes::Bytes>> {
    if let Some(ts) = trace_storage {
        match ts.get_trace(cmd_id).await {
            Ok(Some(trace)) => {
                let body = serde_json::to_string(&trace).unwrap_or_default();
                json_response(StatusCode::OK, body)
            }
            Ok(None) => json_response(
                StatusCode::NOT_FOUND,
                serde_json::json!({
                    "error": "TRACE_NOT_FOUND",
                    "commandId": cmd_id
                })
                .to_string(),
            ),
            Err(e) => json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                serde_json::json!({
                    "error": "STORAGE_ERROR",
                    "message": e.to_string()
                })
                .to_string(),
            ),
        }
    } else {
        json_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "{\"error\":\"TRACE_STORAGE_NOT_ENABLED\"}",
        )
    }
}

pub fn handle_checkpoints(cmd_id: &str) -> Response<Full<bytes::Bytes>> {
    let body = serde_json::json!({
        "commandId": cmd_id,
        "status": "active",
        "memoized": true
    });
    json_response(StatusCode::OK, body.to_string())
}

pub fn handle_lockdown(
    deployer: Option<&Arc<crate::deployer::FluxcellDeployer>>,
) -> Response<Full<bytes::Bytes>> {
    if let Some(dep) = deployer {
        dep.guard().emergency_lockdown();
        let body = serde_json::json!({
            "status": "locked_down",
            "external_deploy_enabled": dep.guard().is_external_deploy_allowed(),
            "dev_upload_enabled": dep.guard().is_dev_upload_allowed(),
            "message": "Emergency lockdown activated. All deployments frozen."
        });
        json_response(StatusCode::OK, body.to_string())
    } else {
        json_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "{\"error\":\"DEPLOYER_NOT_ENABLED\"}",
        )
    }
}
