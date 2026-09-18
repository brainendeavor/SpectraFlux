pub mod admin;
pub mod deployer;
pub mod dispatch;
pub mod probes;

use http_body_util::Full;
use hyper::{Response, StatusCode};

pub fn json_response(status: StatusCode, body: impl Into<String>) -> Response<Full<bytes::Bytes>> {
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Full::new(bytes::Bytes::from(body.into())))
        .unwrap_or_else(|_| Response::new(Full::new(bytes::Bytes::new())))
}

pub fn html_response(status: StatusCode, body: &'static str) -> Response<Full<bytes::Bytes>> {
    Response::builder()
        .status(status)
        .header("Content-Type", "text/html; charset=utf-8")
        .body(Full::new(bytes::Bytes::from(body)))
        .unwrap_or_else(|_| Response::new(Full::new(bytes::Bytes::new())))
}

pub fn svg_response(status: StatusCode, body: &'static str) -> Response<Full<bytes::Bytes>> {
    Response::builder()
        .status(status)
        .header("Content-Type", "image/svg+xml")
        .header("Cache-Control", "public, max-age=86400, immutable")
        .body(Full::new(bytes::Bytes::from_static(body.as_bytes())))
        .unwrap_or_else(|_| Response::new(Full::new(bytes::Bytes::new())))
}

pub fn error_response(status: StatusCode, err_code: &str, message: &str) -> Response<Full<bytes::Bytes>> {
    let payload = serde_json::json!({
        "error": err_code,
        "message": message,
    });
    json_response(status, payload.to_string())
}
