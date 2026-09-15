//! HTTP Protocol Envelopes & Response Builders
//!
//! Provides ergonomic, JSON-friendly HTTP request decoding and response building
//! matching the SpectraFlux chassis HTTP router specification.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Metadata declaring a dynamically mounted HTTP route in the chassis router.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RouteMeta {
    pub method: String,
    pub path: String,
    pub description: String,
}

impl RouteMeta {
    pub fn get(path: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            method: "GET".to_string(),
            path: path.into(),
            description: description.into(),
        }
    }

    pub fn post(path: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            method: "POST".to_string(),
            path: path.into(),
            description: description.into(),
        }
    }

    pub fn put(path: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            method: "PUT".to_string(),
            path: path.into(),
            description: description.into(),
        }
    }

    pub fn delete(path: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            method: "DELETE".to_string(),
            path: path.into(),
            description: description.into(),
        }
    }
}

/// An incoming HTTP request dispatched by the host chassis.
#[derive(Debug, Clone, Default)]
pub struct HttpRequest {
    pub method: String,
    pub path: String,
    pub raw_path: String,
    pub query_params: HashMap<String, String>,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl HttpRequest {
    /// Decodes an `HttpRequest` from the JSON payload passed by the host chassis.
    pub fn from_json_val(val: &serde_json::Value) -> Self {
        let raw_path = val.get("path").and_then(|p| p.as_str()).unwrap_or("/");
        let clean_path = raw_path.split('?').next().unwrap_or(raw_path).to_string();
        let query_str = raw_path.split_once('?').map(|(_, q)| q).unwrap_or("");

        let mut query_params = HashMap::new();
        for pair in query_str.split('&') {
            if let Some((k, v)) = pair.split_once('=') {
                query_params.insert(k.to_string(), v.to_string());
            }
        }

        let method = val.get("method").and_then(|m| m.as_str()).unwrap_or("GET").to_string();

        let mut headers = HashMap::new();
        if let Some(h_arr) = val.get("headers").and_then(|h| h.as_array()) {
            for item in h_arr {
                if let (Some(k), Some(v)) = (item.get(0).and_then(|k| k.as_str()), item.get(1).and_then(|v| v.as_str())) {
                    headers.insert(k.to_lowercase(), v.to_string());
                }
            }
        }

        let body = if let Some(body_str) = val.get("body").and_then(|b| b.as_str()) {
            body_str.as_bytes().to_vec()
        } else if let Some(body_arr) = val.get("body").and_then(|b| b.as_array()) {
            body_arr.iter().filter_map(|b| b.as_u64().map(|v| v as u8)).collect()
        } else {
            Vec::new()
        };

        Self {
            method,
            path: clean_path,
            raw_path: raw_path.to_string(),
            query_params,
            headers,
            body,
        }
    }

    /// Deserializes the request body as JSON into `T`.
    pub fn json<T: DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_slice(&self.body)
    }

    /// Returns a query parameter value if present.
    pub fn query(&self, key: &str) -> Option<&str> {
        self.query_params.get(key).map(|s| s.as_str())
    }

    /// Returns a header value if present (case-insensitive lookup).
    pub fn header(&self, key: &str) -> Option<&str> {
        self.headers.get(&key.to_lowercase()).map(|s| s.as_str())
    }
}

/// An outgoing HTTP response returned to the host chassis.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    /// Constructs a standard HTTP 200 OK response with an empty body.
    pub fn ok() -> Self {
        Self {
            status: 200,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            body: b"{\"status\":\"ok\"}".to_vec(),
        }
    }

    /// Constructs an HTTP 200 OK response containing a JSON-serialized payload.
    pub fn json<T: Serialize>(data: &T) -> Self {
        Self::json_with_status(200, data)
    }

    /// Constructs an HTTP response with a specific status code containing a JSON-serialized payload.
    pub fn json_with_status<T: Serialize>(status: u16, data: &T) -> Self {
        let body = serde_json::to_vec(data).unwrap_or_else(|_| b"{}".to_vec());
        Self {
            status,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            body,
        }
    }

    /// Constructs an HTTP 400 Bad Request response with a structured JSON error message.
    pub fn bad_request(message: impl Into<String>) -> Self {
        let msg = message.into();
        Self::json_with_status(400, &serde_json::json!({
            "error": "BAD_REQUEST",
            "message": msg
        }))
    }

    /// Constructs an HTTP 404 Not Found response with a structured JSON error message.
    pub fn not_found(message: impl Into<String>) -> Self {
        let msg = message.into();
        Self::json_with_status(404, &serde_json::json!({
            "error": "NOT_FOUND",
            "message": msg
        }))
    }

    /// Constructs an HTTP 500 Internal Server Error response with a structured JSON error message.
    pub fn error(message: impl Into<String>) -> Self {
        let msg = message.into();
        Self::json_with_status(500, &serde_json::json!({
            "error": "INTERNAL_SERVER_ERROR",
            "message": msg
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_http_request_decoding() {
        let json_val = serde_json::json!({
            "path": "/stats?company_id=abc-123&metric=score",
            "method": "POST",
            "body": "{\"value\": 10}"
        });
        let req = HttpRequest::from_json_val(&json_val);
        assert_eq!(req.path, "/stats");
        assert_eq!(req.method, "POST");
        assert_eq!(req.query("company_id"), Some("abc-123"));
        assert_eq!(req.query("metric"), Some("score"));

        #[derive(Deserialize)]
        struct Body { value: i32 }
        let body: Body = req.json().unwrap();
        assert_eq!(body.value, 10);
    }

    #[test]
    fn test_http_response_builders() {
        let resp = HttpResponse::json(&serde_json::json!({ "score": 42 }));
        assert_eq!(resp.status, 200);
        assert_eq!(resp.headers[0].0, "content-type");

        let err = HttpResponse::bad_request("invalid id");
        assert_eq!(err.status, 400);
    }
}
