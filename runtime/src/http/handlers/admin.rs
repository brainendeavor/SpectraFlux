use super::{html_response, json_response, svg_response};
use crate::http::{FluxRouter, ADMIN_HTML, FAVICON_SVG};
use crate::telemetry::{DomainTraceStorage, TelemetryClient};
use http_body_util::{BodyExt, Full};
use hyper::{Request, Response, StatusCode};
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};

pub fn handle_dashboard() -> Response<Full<bytes::Bytes>> {
    html_response(StatusCode::OK, ADMIN_HTML)
}

pub fn handle_favicon() -> Response<Full<bytes::Bytes>> {
    svg_response(StatusCode::OK, FAVICON_SVG)
}

pub fn handle_logs(telemetry: &TelemetryClient) -> Response<Full<bytes::Bytes>> {
    let logs = telemetry.get_recent_logs();
    let body = serde_json::to_string(&logs).unwrap_or_else(|_| "[]".to_string());
    json_response(StatusCode::OK, body)
}

pub fn handle_overview(
    telemetry: &TelemetryClient,
    router: &RwLock<FluxRouter>,
    config_summary: Option<&serde_json::Value>,
) -> Response<Full<bytes::Bytes>> {
    let uptime = telemetry.started_at.elapsed().as_secs();
    let processed = telemetry.processed_events.load(Ordering::Relaxed);
    let errors = telemetry.error_count.load(Ordering::Relaxed);
    let cells = {
        let r = router.read().unwrap_or_else(|e| e.into_inner());
        r.get_fluxcells_summary()
    };
    let rss_bytes = get_process_rss_bytes().unwrap_or(0);
    let rss_mb = (rss_bytes as f64) / (1024.0 * 1024.0);
    let mut val = serde_json::json!({
        "workerId": telemetry.worker_id,
        "uptimeSeconds": uptime,
        "processedEvents": processed,
        "errors": errors,
        "fluxcells": cells,
        "memory": {
            "processRssBytes": rss_bytes,
            "processRssMb": (rss_mb * 100.0).round() / 100.0,
        }
    });
    if let Some(cfg) = config_summary {
        val["config"] = cfg.clone();
    }
    json_response(StatusCode::OK, val.to_string())
}

#[cfg(target_os = "linux")]
fn get_process_rss_bytes() -> Option<u64> {
    std::fs::read_to_string("/proc/self/statm").ok().and_then(|s| {
        s.split_whitespace().nth(1)?.parse::<u64>().ok().map(|pages| pages * 4096)
    })
}

#[cfg(target_os = "macos")]
fn get_process_rss_bytes() -> Option<u64> {
    std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()
        .and_then(|out| {
            let s = String::from_utf8_lossy(&out.stdout);
            s.trim().parse::<u64>().ok().map(|kb| kb * 1024)
        })
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn get_process_rss_bytes() -> Option<u64> {
    None
}


pub fn is_admin_authorized(headers: &hyper::HeaderMap, query_string: Option<&str>) -> bool {
    let expected_token = std::env::var("FLUX_ADMIN_TOKEN")
        .or_else(|_| std::env::var("FLUX_DEPLOY_TOKEN"))
        .ok();

    let expected = match expected_token {
        Some(t) if !t.trim().is_empty() => t,
        _ => {
            log::warn!("Admin write operation authorized without configured token (set FLUX_ADMIN_TOKEN for authenticated control plane)");
            return true;
        }
    };

    if let Some(auth) = headers.get(hyper::header::AUTHORIZATION).and_then(|v| v.to_str().ok()) {
        if let Some(token) = auth.strip_prefix("Bearer ").or_else(|| auth.strip_prefix("bearer ")) {
            if token.trim() == expected.trim() {
                return true;
            }
        }
    }

    for header_name in ["x-flux-admin-token", "x-admin-token", "x-deploy-token"] {
        if let Some(val) = headers.get(header_name).and_then(|v| v.to_str().ok()) {
            if val.trim() == expected.trim() {
                return true;
            }
        }
    }

    if let Some(query) = query_string {
        for param in query.split('&') {
            if let Some((k, v)) = param.split_once('=') {
                if (k == "token" || k == "admin_token") && v.trim() == expected.trim() {
                    return true;
                }
            }
        }
    }

    false
}

pub fn handle_config(config_summary: Option<&serde_json::Value>) -> Response<Full<bytes::Bytes>> {
    handle_config_get(None, config_summary)
}

pub fn handle_config_get(
    dynamic_state: Option<&Arc<arc_swap::ArcSwap<crate::dynamic_state::DynamicChassisState>>>,
    config_summary: Option<&serde_json::Value>,
) -> Response<Full<bytes::Bytes>> {
    if let Some(ds) = dynamic_state {
        let state = ds.load();
        let mut sanitized = state.config.to_sanitized_json(&state.config_path);
        if let Some(obj) = sanitized.as_object_mut() {
            obj.insert("backend".to_string(), serde_json::json!("disk"));
            obj.insert("descriptor".to_string(), serde_json::json!(format!("file://{}", state.config_path)));
            obj.insert("version".to_string(), serde_json::json!(state.version));
            obj.insert("updated_at_epoch_ms".to_string(), serde_json::json!(state.updated_at_epoch_ms));
            obj.insert("hash".to_string(), serde_json::json!(state.config_hash));
            obj.insert("content".to_string(), serde_json::json!(state.raw_config.as_str()));
            obj.insert("sourcePath".to_string(), serde_json::json!(state.config_path));
        }
        json_response(StatusCode::OK, sanitized.to_string())
    } else if let Some(cfg) = config_summary {
        let mut val = cfg.clone();
        if let Some(obj) = val.as_object_mut() {
            obj.insert("backend".to_string(), serde_json::json!("disk"));
            obj.insert("descriptor".to_string(), serde_json::json!("file://spectra-flux.toml"));
            obj.insert("version".to_string(), serde_json::json!(1));
            obj.insert("updated_at_epoch_ms".to_string(), serde_json::json!(0));
            obj.insert("hash".to_string(), serde_json::json!("untracked"));
            obj.insert("content".to_string(), serde_json::json!(""));
        }
        json_response(StatusCode::OK, val.to_string())
    } else {
        json_response(StatusCode::OK, "{}".to_string())
    }
}

pub async fn handle_config_validate<B>(req: Request<B>) -> Response<Full<bytes::Bytes>>
where
    B: hyper::body::Body + Send + 'static,
    B::Data: Send,
    B::Error: std::error::Error + Send + Sync + 'static,
{
    let body_bytes = match req.into_body().collect().await {
        Ok(c) => c.to_bytes(),
        Err(e) => {
            return json_response(
                StatusCode::BAD_REQUEST,
                serde_json::json!({
                    "valid": false,
                    "errors": [format!("Failed to read request body: {}", e)],
                    "warnings": [],
                    "summary": null
                }).to_string(),
            );
        }
    };

    let content = if let Ok(json_req) = serde_json::from_slice::<serde_json::Value>(&body_bytes) {
        if let Some(c) = json_req.get("content").and_then(|v| v.as_str()) {
            c.to_string()
        } else {
            String::from_utf8_lossy(&body_bytes).to_string()
        }
    } else {
        String::from_utf8_lossy(&body_bytes).to_string()
    };

    match crate::config::FluxConfig::from_toml_str(&content) {
        Ok(cfg) => {
            let mut warnings = Vec::new();
            if cfg.fluxcells.is_empty() {
                warnings.push("No fluxcells configured".to_string());
            }

            let mut tenants: Vec<String> = cfg.mailer.tenants.keys().cloned().collect();
            tenants.sort();

            let summary = serde_json::json!({
                "host": cfg.host,
                "port": cfg.port,
                "broker_method": cfg.broker.method,
                "broker_addr": cfg.broker.addr,
                "storage_backend": cfg.storage.backend,
                "database_configured": cfg.database.url.is_some() || !cfg.databases.is_empty(),
                "named_databases_count": cfg.databases.len(),
                "fluxcells_count": cfg.fluxcells.len(),
                "profiles_count": cfg.profiles.len(),
                "mailer_provider": cfg.mailer.provider.as_deref().unwrap_or("console"),
                "tenants_count": cfg.mailer.tenants.len(),
                "tenants": tenants,
            });

            json_response(
                StatusCode::OK,
                serde_json::json!({
                    "valid": true,
                    "errors": [],
                    "warnings": warnings,
                    "summary": summary,
                }).to_string(),
            )
        }
        Err(e) => {
            json_response(
                StatusCode::OK,
                serde_json::json!({
                    "valid": false,
                    "errors": [e.to_string()],
                    "warnings": [],
                    "summary": null,
                }).to_string(),
            )
        }
    }
}

pub async fn handle_config_update<B>(
    req: Request<B>,
    query_string: Option<&str>,
    dynamic_state: Option<&Arc<arc_swap::ArcSwap<crate::dynamic_state::DynamicChassisState>>>,
    telemetry: &crate::telemetry::TelemetryClient,
) -> Response<Full<bytes::Bytes>>
where
    B: hyper::body::Body + Send + 'static,
    B::Data: Send,
    B::Error: std::error::Error + Send + Sync + 'static,
{
    if !is_admin_authorized(req.headers(), query_string) {
        return json_response(
            StatusCode::UNAUTHORIZED,
            serde_json::json!({
                "error": "Unauthorized",
                "message": "Missing or invalid admin authorization token (set FLUX_ADMIN_TOKEN or provide valid Authorization: Bearer <token>)"
            }).to_string(),
        );
    }

    let ds = match dynamic_state {
        Some(ds) => ds,
        None => {
            return json_response(
                StatusCode::SERVICE_UNAVAILABLE,
                serde_json::json!({
                    "error": "DynamicStateUnavailable",
                    "message": "Chassis dynamic state is not active on this node"
                }).to_string(),
            );
        }
    };

    let body_bytes = match req.into_body().collect().await {
        Ok(c) => c.to_bytes(),
        Err(e) => {
            return json_response(
                StatusCode::BAD_REQUEST,
                serde_json::json!({
                    "error": "Bad Request",
                    "message": format!("Failed to read request body: {}", e)
                }).to_string(),
            );
        }
    };

    let (content, reload) = if let Ok(json_req) = serde_json::from_slice::<serde_json::Value>(&body_bytes) {
        let content = json_req.get("content").and_then(|v| v.as_str()).map(|s| s.to_string())
            .unwrap_or_else(|| String::from_utf8_lossy(&body_bytes).to_string());
        let reload = json_req.get("reload").and_then(|v| v.as_bool()).unwrap_or(true);
        (content, reload)
    } else {
        (String::from_utf8_lossy(&body_bytes).to_string(), true)
    };

    let parsed_cfg = match crate::config::FluxConfig::from_toml_str(&content) {
        Ok(cfg) => cfg,
        Err(e) => {
            return json_response(
                StatusCode::BAD_REQUEST,
                serde_json::json!({
                    "error": "Invalid Configuration",
                    "message": format!("Configuration parsing error: {}", e)
                }).to_string(),
            );
        }
    };

    let current_state = ds.load();
    let target_path = if !current_state.config_path.is_empty() && current_state.config_path != "built-in default (in-memory)" {
        current_state.config_path.clone()
    } else {
        "spectra-flux.toml".to_string()
    };

    // Persist to disk
    if let Err(e) = std::fs::write(&target_path, &content) {
        log::error!("Failed to persist configuration to disk at '{}': {}", target_path, e);
        return json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            serde_json::json!({
                "error": "Storage Error",
                "message": format!("Failed to persist configuration to disk ({}): {}", target_path, e)
            }).to_string(),
        );
    }

    log::info!("Admin Control Plane: Successfully persisted configuration to '{}'", target_path);

    let (version, hash, updated_at) = if reload {
        let new_version = current_state.version + 1;
        let new_state = crate::dynamic_state::DynamicChassisState::new(
            parsed_cfg,
            content,
            target_path,
            new_version,
        );
        let v = new_state.version;
        let h = new_state.config_hash.clone();
        let u = new_state.updated_at_epoch_ms;
        ds.store(Arc::new(new_state));
        telemetry.record_log("INFO", &format!("Admin: Hot-reloaded configuration to revision #{}", v), None);
        log::info!("Admin: Hot-reloaded configuration to revision #{}", v);
        (v, h, u)
    } else {
        (current_state.version, current_state.config_hash.clone(), current_state.updated_at_epoch_ms)
    };

    json_response(
        StatusCode::OK,
        serde_json::json!({
            "success": true,
            "version": version,
            "hash": hash,
            "updated_at_epoch_ms": updated_at,
            "reloaded": reload,
            "message": if reload {
                "Configuration saved to disk and runtime hot-reloaded successfully".to_string()
            } else {
                "Configuration saved to disk (hot-reload skipped)".to_string()
            },
        }).to_string(),
    )
}

pub async fn handle_config_reload<B>(
    req: Request<B>,
    query_string: Option<&str>,
    dynamic_state: Option<&Arc<arc_swap::ArcSwap<crate::dynamic_state::DynamicChassisState>>>,
    telemetry: &crate::telemetry::TelemetryClient,
) -> Response<Full<bytes::Bytes>>
where
    B: hyper::body::Body + Send + 'static,
    B::Data: Send,
    B::Error: std::error::Error + Send + Sync + 'static,
{
    if !is_admin_authorized(req.headers(), query_string) {
        return json_response(
            StatusCode::UNAUTHORIZED,
            serde_json::json!({
                "error": "Unauthorized",
                "message": "Missing or invalid admin authorization token (set FLUX_ADMIN_TOKEN or provide valid Authorization: Bearer <token>)"
            }).to_string(),
        );
    }

    let ds = match dynamic_state {
        Some(ds) => ds,
        None => {
            return json_response(
                StatusCode::SERVICE_UNAVAILABLE,
                serde_json::json!({
                    "error": "DynamicStateUnavailable",
                    "message": "Chassis dynamic state is not active on this node"
                }).to_string(),
            );
        }
    };

    let current_state = ds.load();
    let target_path = if !current_state.config_path.is_empty() && current_state.config_path != "built-in default (in-memory)" {
        current_state.config_path.clone()
    } else {
        "spectra-flux.toml".to_string()
    };

    let content = match std::fs::read_to_string(&target_path) {
        Ok(c) => c,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                serde_json::json!({
                    "error": "Read Error",
                    "message": format!("Failed to read configuration file '{}': {}", target_path, e)
                }).to_string(),
            );
        }
    };

    let parsed_cfg = match crate::config::FluxConfig::from_toml_str(&content) {
        Ok(cfg) => cfg,
        Err(e) => {
            return json_response(
                StatusCode::BAD_REQUEST,
                serde_json::json!({
                    "error": "Invalid Configuration",
                    "message": format!("Configuration parsing error: {}", e)
                }).to_string(),
            );
        }
    };

    let new_version = current_state.version + 1;
    let new_state = crate::dynamic_state::DynamicChassisState::new(
        parsed_cfg,
        content,
        target_path,
        new_version,
    );
    let v = new_state.version;
    let h = new_state.config_hash.clone();
    let u = new_state.updated_at_epoch_ms;
    ds.store(Arc::new(new_state));
    telemetry.record_log("INFO", &format!("Admin: Hot-reloaded configuration from disk to revision #{}", v), None);
    log::info!("Admin: Hot-reloaded configuration from disk to revision #{}", v);

    json_response(
        StatusCode::OK,
        serde_json::json!({
            "success": true,
            "version": v,
            "hash": h,
            "updated_at_epoch_ms": u,
            "reloaded": true,
            "message": "Configuration reloaded from disk and applied successfully"
        }).to_string(),
    )
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
