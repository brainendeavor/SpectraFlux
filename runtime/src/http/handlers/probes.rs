use super::json_response;
use crate::telemetry::TelemetryClient;
use http_body_util::Full;
use hyper::{Response, StatusCode};
use std::sync::atomic::Ordering;

pub fn handle_healthz(telemetry: &TelemetryClient) -> Response<Full<bytes::Bytes>> {
    let uptime = telemetry.started_at.elapsed().as_secs();
    let body = serde_json::json!({
        "status": "ok",
        "uptime_seconds": uptime,
        "processed_events": telemetry.processed_events.load(Ordering::Relaxed),
        "errors": telemetry.error_count.load(Ordering::Relaxed),
    });
    json_response(StatusCode::OK, body.to_string())
}

pub fn handle_readyz() -> Response<Full<bytes::Bytes>> {
    json_response(StatusCode::OK, "{\"status\":\"ready\"}")
}

pub fn handle_metrics(telemetry: &TelemetryClient) -> Response<Full<bytes::Bytes>> {
    let uptime = telemetry.started_at.elapsed().as_secs();
    let body = serde_json::json!({
        "worker_id": telemetry.worker_id,
        "uptime_seconds": uptime,
        "processed_events": telemetry.processed_events.load(Ordering::Relaxed),
        "errors": telemetry.error_count.load(Ordering::Relaxed),
    });
    json_response(StatusCode::OK, body.to_string())
}
