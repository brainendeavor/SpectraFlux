use super::json_response;
use crate::deployer::FluxcellDeployer;
use crate::http::simple_url_decode;
use http_body_util::{BodyExt, Full};
use hyper::{Request, Response, StatusCode};
use std::sync::Arc;

pub fn handle_status(
    deployer: Option<&Arc<FluxcellDeployer>>,
) -> Response<Full<bytes::Bytes>> {
    if let Some(dep) = deployer {
        let records = dep.registry().list_records();
        let body = serde_json::json!({
            "external_deploy_enabled": dep.guard().is_external_deploy_allowed(),
            "dev_upload_enabled": dep.guard().is_dev_upload_allowed(),
            "auto_activate": dep.config().auto_activate,
            "fluxcells": records,
        });
        json_response(StatusCode::OK, body.to_string())
    } else {
        json_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "{\"error\":\"DEPLOYER_NOT_ENABLED\"}",
        )
    }
}

pub fn handle_history(
    deployer: Option<&Arc<FluxcellDeployer>>,
) -> Response<Full<bytes::Bytes>> {
    if let Some(dep) = deployer {
        let events = dep.registry().list_audit_events();
        let body = serde_json::json!({ "events": events });
        json_response(StatusCode::OK, body.to_string())
    } else {
        json_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "{\"error\":\"DEPLOYER_NOT_ENABLED\"}",
        )
    }
}

pub async fn handle_upload<B>(
    deployer: Option<&Arc<FluxcellDeployer>>,
    req: Request<B>,
    query_string: Option<&str>,
) -> Response<Full<bytes::Bytes>>
where
    B: hyper::body::Body + Send + 'static,
    B::Data: Send,
    B::Error: std::error::Error + Send + Sync + 'static,
{
    let dep = match deployer {
        Some(d) => d,
        None => {
            return json_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "{\"error\":\"DEPLOYER_NOT_ENABLED\"}",
            );
        }
    };

    let mut name = req
        .headers()
        .get("X-Fluxcell-Name")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let mut mount = req
        .headers()
        .get("X-Fluxcell-Mount")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let mut auto_activate = req
        .headers()
        .get("X-Fluxcell-Auto-Activate")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.eq_ignore_ascii_case("true") || s == "1")
        .unwrap_or(false);

    let mut timeout_ms = None;

    if let Some(q) = query_string {
        for param in q.split('&') {
            let mut kv = param.split('=');
            if let (Some(k), Some(v)) = (kv.next(), kv.next()) {
                if k == "name" && name.is_none() {
                    name = Some(simple_url_decode(v));
                } else if (k == "mount" || k == "mount_path") && mount.is_none() {
                    mount = Some(simple_url_decode(v));
                } else if k == "auto_activate" || k == "activate" {
                    auto_activate = v.eq_ignore_ascii_case("true") || v == "1";
                } else if k == "timeout_ms" {
                    timeout_ms = v.parse::<u64>().ok();
                }
            }
        }
    }

    let cell_name = match name {
        Some(n) if !n.trim().is_empty() => n.trim().to_string(),
        _ => {
            return json_response(
                StatusCode::BAD_REQUEST,
                "{\"error\":\"Missing required 'name' parameter or 'X-Fluxcell-Name' header\"}",
            );
        }
    };

    let mount_path = match mount {
        Some(m) if !m.trim().is_empty() => m.trim().to_string(),
        _ => format!("/api/{}", cell_name),
    };

    let body_bytes = match req.into_body().collect().await {
        Ok(collected) => collected.to_bytes().to_vec(),
        Err(e) => {
            return json_response(
                StatusCode::BAD_REQUEST,
                format!("{{\"error\":\"BODY_READ_ERROR\",\"message\":\"{}\"}}", e),
            );
        }
    };

    let effective_timeout = timeout_ms.or(Some(10_000));
    match dep.stage_uploaded_artifact(
        &cell_name,
        body_bytes,
        &mount_path,
        effective_timeout,
        None,
    ) {
        Ok(mut record) => {
            if auto_activate {
                match dep.activate(&cell_name, &record.sha256) {
                    Ok(active_rec) => {
                        record = active_rec;
                    }
                    Err(e) => {
                        log::warn!(
                            "Artifact staged but auto-activation failed for '{}': {}",
                            cell_name,
                            e
                        );
                    }
                }
            }
            let body = serde_json::to_string(&record).unwrap_or_default();
            json_response(StatusCode::CREATED, body)
        }
        Err(e) => {
            let status = if !dep.guard().is_dev_upload_allowed() {
                StatusCode::FORBIDDEN
            } else {
                StatusCode::BAD_REQUEST
            };
            json_response(
                status,
                serde_json::json!({
                    "error": "DEPLOY_ERROR",
                    "message": e.to_string()
                })
                .to_string(),
            )
        }
    }
}

pub async fn handle_activate<B>(
    deployer: Option<&Arc<FluxcellDeployer>>,
    req: Request<B>,
) -> Response<Full<bytes::Bytes>>
where
    B: hyper::body::Body + Send + 'static,
    B::Data: Send,
    B::Error: std::error::Error + Send + Sync + 'static,
{
    let dep = match deployer {
        Some(d) => d,
        None => {
            return json_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "{\"error\":\"DEPLOYER_NOT_ENABLED\"}",
            );
        }
    };

    let body_bytes = match req.into_body().collect().await {
        Ok(collected) => collected.to_bytes().to_vec(),
        Err(e) => {
            return json_response(
                StatusCode::BAD_REQUEST,
                format!("{{\"error\":\"BODY_READ_ERROR\",\"message\":\"{}\"}}", e),
            );
        }
    };

    #[derive(serde::Deserialize)]
    struct ActivateRequest {
        name: String,
        sha256: String,
    }

    let activate_req: ActivateRequest = match serde_json::from_slice(&body_bytes) {
        Ok(r) => r,
        Err(e) => {
            return json_response(
                StatusCode::BAD_REQUEST,
                format!("{{\"error\":\"INVALID_PAYLOAD\",\"message\":\"{}\"}}", e),
            );
        }
    };

    match dep.activate(&activate_req.name, &activate_req.sha256) {
        Ok(record) => {
            let body = serde_json::to_string(&record).unwrap_or_default();
            json_response(StatusCode::OK, body)
        }
        Err(e) => json_response(
            StatusCode::BAD_REQUEST,
            serde_json::json!({
                "error": "ACTIVATION_ERROR",
                "message": e.to_string()
            })
            .to_string(),
        ),
    }
}

pub fn handle_remove(
    deployer: Option<&Arc<FluxcellDeployer>>,
    path: &str,
    query_string: Option<&str>,
) -> Response<Full<bytes::Bytes>> {
    let dep = match deployer {
        Some(d) => d,
        None => {
            return json_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "{\"error\":\"DEPLOYER_NOT_ENABLED\"}",
            );
        }
    };

    let cell_name = if path.len() > "/_flux/deployer/fluxcells/".len() {
        path["/_flux/deployer/fluxcells/".len()..].trim_matches('/').to_string()
    } else {
        let mut q_name = None;
        if let Some(q) = query_string {
            for param in q.split('&') {
                let mut kv = param.split('=');
                if let (Some("name"), Some(v)) = (kv.next(), kv.next()) {
                    q_name = Some(v.to_string());
                }
            }
        }
        q_name.unwrap_or_default()
    };

    if cell_name.is_empty() {
        return json_response(
            StatusCode::BAD_REQUEST,
            "{\"error\":\"Fluxcell name required\"}",
        );
    }

    match dep.remove(&cell_name) {
        Ok(record) => json_response(
            StatusCode::OK,
            serde_json::json!({
                "status": "removed",
                "fluxcell": record
            })
            .to_string(),
        ),
        Err(e) => json_response(
            StatusCode::BAD_REQUEST,
            serde_json::json!({
                "error": "REMOVE_ERROR",
                "message": e.to_string()
            })
            .to_string(),
        ),
    }
}
