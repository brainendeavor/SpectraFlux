use http_body_util::Full;
use hyper::{Request, Response};
use matchit::Router;
use std::collections::HashMap;
use std::sync::Arc;

pub mod handlers;

pub const ADMIN_HTML: &str = include_str!("assets/admin.html");
pub const FAVICON_SVG: &str = include_str!("assets/favicon.svg");

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RouteDefinition {
    pub method: String,
    #[serde(alias = "path")]
    pub relative_path: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

impl RouteDefinition {
    pub fn new<S1: Into<String>, S2: Into<String>, S3: Into<String>>(
        method: S1,
        relative_path: S2,
        description: S3,
    ) -> Self {
        Self {
            method: method.into(),
            relative_path: relative_path.into(),
            description: description.into(),
            timeout_ms: None,
        }
    }

    pub fn with_timeout<S1: Into<String>, S2: Into<String>, S3: Into<String>>(
        method: S1,
        relative_path: S2,
        description: S3,
        timeout_ms: u64,
    ) -> Self {
        Self {
            method: method.into(),
            relative_path: relative_path.into(),
            description: description.into(),
            timeout_ms: Some(timeout_ms),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RegisteredRoute {
    pub fluxcell_name: String,
    pub mount_path: String,
    pub method: String,
    pub relative_path: String,
    pub full_path: String,
}

#[derive(Debug, Clone)]
pub struct RouteMatch<'a> {
    pub fluxcell_name: &'a str,
    pub mount_path: &'a str,
    pub relative_path: &'a str,
    pub full_path: &'a str,
    pub params: HashMap<String, String>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RouterError {
    #[error("Route collision: path '{path}' already registered by fluxcell '{existing_fluxcell}' (attempted by '{new_fluxcell}')")]
    Collision {
        path: String,
        existing_fluxcell: String,
        new_fluxcell: String,
    },
    #[error("Route not found: {method} {path}")]
    NotFound {
        method: String,
        path: String,
    },
    #[error("Method not allowed: {method} for {path} (Allowed: {})", allowed.join(", "))]
    MethodNotAllowed {
        method: String,
        path: String,
        allowed: Vec<String>,
    },
    #[error("Invalid route pattern: {0}")]
    InvalidPattern(String),
}

pub struct FluxRouter {
    // Map of METHOD -> matchit::Router<RegisteredRoute>
    routers: HashMap<String, Router<RegisteredRoute>>,
    // Set of all full paths mapped to the registering fluxcell for quick collision reporting
    registered_paths: HashMap<String, String>,
    // Map of fluxcell_name -> (mount_path, Vec<RouteDefinition>) for rebuild on unregister
    fluxcell_routes: HashMap<String, (String, Vec<RouteDefinition>)>,
}

impl FluxRouter {
    pub fn new() -> Self {
        Self {
            routers: HashMap::new(),
            registered_paths: HashMap::new(),
            fluxcell_routes: HashMap::new(),
        }
    }

    pub fn is_reserved_mount_path(mount_path: &str) -> bool {
        let clean = clean_path_prefix(mount_path);
        clean == "/admin"
            || clean.starts_with("/admin/")
            || clean == "/graphql"
            || clean == "/gql"
            || clean.starts_with("/_flux")
            || clean == "/healthz"
            || clean == "/readyz"
            || clean == "/metrics"
    }

    pub fn register_fluxcell_routes(
        &mut self,
        fluxcell_name: &str,
        mount_path: &str,
        routes: &[RouteDefinition],
    ) -> Result<(), RouterError> {
        let clean_mount = clean_path_prefix(mount_path);

        if Self::is_reserved_mount_path(&clean_mount) {
            return Err(RouterError::InvalidPattern(format!(
                "Mount path '{}' is reserved for system infrastructure",
                clean_mount
            )));
        }

        for route in routes {
            let clean_rel = clean_path_suffix(&route.relative_path);
            let full_path = if clean_mount.is_empty() && clean_rel.is_empty() {
                "/".to_string()
            } else {
                format!("{}{}", clean_mount, clean_rel)
            };
            let method = route.method.to_uppercase();

            let route_key = format!("{} {}", method, full_path);

            if let Some(existing_fluxcell) = self.registered_paths.get(&route_key) {
                return Err(RouterError::Collision {
                    path: full_path,
                    existing_fluxcell: existing_fluxcell.clone(),
                    new_fluxcell: fluxcell_name.to_string(),
                });
            }

            let reg = RegisteredRoute {
                fluxcell_name: fluxcell_name.to_string(),
                mount_path: if clean_mount.is_empty() { "/".to_string() } else { clean_mount.clone() },
                method: method.clone(),
                relative_path: if clean_rel.is_empty() { "/".to_string() } else { clean_rel },
                full_path: full_path.clone(),
            };

            let router = self.routers.entry(method).or_default();
            router
                .insert(&full_path, reg)
                .map_err(|e| RouterError::InvalidPattern(e.to_string()))?;

            self.registered_paths
                .insert(route_key, fluxcell_name.to_string());
        }

        self.fluxcell_routes
            .insert(fluxcell_name.to_string(), (mount_path.to_string(), routes.to_vec()));

        Ok(())
    }

    pub fn unregister_fluxcell_routes(&mut self, fluxcell_name: &str) -> bool {
        if self.fluxcell_routes.remove(fluxcell_name).is_some() {
            self.rebuild_routers();
            true
        } else {
            false
        }
    }

    fn rebuild_routers(&mut self) {
        self.routers.clear();
        self.registered_paths.clear();
        let old_routes = std::mem::take(&mut self.fluxcell_routes);
        for (name, (mount, routes)) in old_routes {
            let _ = self.register_fluxcell_routes(&name, &mount, &routes);
        }
    }

    pub fn list_registered_routes(&self) -> Vec<RegisteredRoute> {
        let mut list = Vec::new();
        for (name, (mount, routes)) in &self.fluxcell_routes {
            let clean_mount = clean_path_prefix(mount);
            for r in routes {
                let clean_rel = clean_path_suffix(&r.relative_path);
                let full_path = if clean_mount.is_empty() && clean_rel.is_empty() {
                    "/".to_string()
                } else {
                    format!("{}{}", clean_mount, clean_rel)
                };
                list.push(RegisteredRoute {
                    fluxcell_name: name.clone(),
                    mount_path: mount.clone(),
                    method: r.method.to_uppercase(),
                    relative_path: r.relative_path.clone(),
                    full_path,
                });
            }
        }
        list.sort_by(|a, b| a.full_path.cmp(&b.full_path));
        list
    }

    pub fn lookup<'a>(&'a self, method: &str, path: &str) -> Result<RouteMatch<'a>, RouterError> {
        let method_upper = method.to_uppercase();
        let normalized_path = if path.len() > 1 && path.ends_with('/') {
            path.trim_end_matches('/')
        } else {
            path
        };

        if let Some(router) = self.routers.get(&method_upper) {
            if let Ok(matched) = router.at(normalized_path) {
                let mut params = HashMap::new();
                for (k, v) in matched.params.iter() {
                    params.insert(k.to_string(), v.to_string());
                }
                return Ok(RouteMatch {
                    fluxcell_name: &matched.value.fluxcell_name,
                    mount_path: &matched.value.mount_path,
                    relative_path: &matched.value.relative_path,
                    full_path: &matched.value.full_path,
                    params,
                });
            }
        }

        // Check if any other registered method supports this route for RFC 9110 MethodNotAllowed
        let mut allowed = Vec::new();
        for (m, router) in &self.routers {
            if m != &method_upper && router.at(normalized_path).is_ok() {
                allowed.push(m.clone());
            }
        }

        if !allowed.is_empty() {
            allowed.sort();
            return Err(RouterError::MethodNotAllowed {
                method: method_upper,
                path: path.to_string(),
                allowed,
            });
        }

        Err(RouterError::NotFound {
            method: method_upper,
            path: path.to_string(),
        })
    }

    pub fn get_fluxcells_summary(&self) -> serde_json::Value {
        let mut cells = serde_json::Map::new();
        for (name, (mount, routes)) in &self.fluxcell_routes {
            let routes_json: Vec<serde_json::Value> = routes
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "method": r.method,
                        "path": r.relative_path,
                        "relative_path": r.relative_path,
                        "description": r.description,
                        "timeoutMs": r.timeout_ms,
                    })
                })
                .collect();
            cells.insert(
                name.clone(),
                serde_json::json!({
                    "mountPath": mount,
                    "routesCount": routes.len(),
                    "routes": routes_json,
                }),
            );
        }
        serde_json::Value::Object(cells)
    }
}

impl Default for FluxRouter {
    fn default() -> Self {
        Self::new()
    }
}

pub fn clean_path_prefix(prefix: &str) -> String {
    let p = prefix.trim().trim_matches('/');
    if p.is_empty() {
        "".to_string()
    } else {
        format!("/{}", p)
    }
}

pub fn clean_path_suffix(suffix: &str) -> String {
    let s = suffix.trim().trim_matches('/');
    if s.is_empty() {
        "".to_string()
    } else {
        format!("/{}", s)
    }
}

pub fn simple_url_decode(input: &str) -> String {
    let mut result = String::new();
    let mut chars = input.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            let h1 = chars.next();
            let h2 = chars.next();
            if let (Some(h1), Some(h2)) = (h1, h2) {
                if let Ok(byte) = u8::from_str_radix(&format!("{}{}", h1, h2), 16) {
                    result.push(byte as char);
                    continue;
                }
            }
        } else if c == '+' {
            result.push(' ');
        } else {
            result.push(c);
        }
    }
    result
}

#[async_trait::async_trait]
pub trait FluxcellHttpDispatcher: Send + Sync {
    async fn dispatch(
        &self,
        fluxcell_name: &str,
        relative_path: &str,
        method: &str,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    ) -> Result<(u16, Vec<(String, String)>, Vec<u8>), anyhow::Error>;

    async fn dispatch_with_spans(
        &self,
        fluxcell_name: &str,
        relative_path: &str,
        method: &str,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    ) -> Result<(u16, Vec<(String, String)>, Vec<u8>, Vec<crate::telemetry::HostCallSpan>), anyhow::Error> {
        let (status, resp_headers, resp_body) = self
            .dispatch(fluxcell_name, relative_path, method, headers, body)
            .await?;
        Ok((status, resp_headers, resp_body, Vec::new()))
    }
}

pub async fn handle_request<B>(
    req: Request<B>,
    router: Arc<std::sync::RwLock<FluxRouter>>,
    telemetry: Arc<crate::telemetry::TelemetryClient>,
    dispatcher: Arc<dyn FluxcellHttpDispatcher>,
    deployer: Option<Arc<crate::deployer::FluxcellDeployer>>,
    trace_storage: Option<Arc<crate::telemetry::DomainTraceStorage>>,
) -> Result<Response<Full<bytes::Bytes>>, std::convert::Infallible>
where
    B: hyper::body::Body + Send + 'static,
    B::Data: Send,
    B::Error: std::error::Error + Send + Sync + 'static,
{
    handle_request_with_config(req, router, telemetry, dispatcher, deployer, trace_storage, None).await
}

pub async fn handle_request_with_config<B>(
    req: Request<B>,
    router: Arc<std::sync::RwLock<FluxRouter>>,
    telemetry: Arc<crate::telemetry::TelemetryClient>,
    dispatcher: Arc<dyn FluxcellHttpDispatcher>,
    deployer: Option<Arc<crate::deployer::FluxcellDeployer>>,
    trace_storage: Option<Arc<crate::telemetry::DomainTraceStorage>>,
    config_summary: Option<Arc<serde_json::Value>>,
) -> Result<Response<Full<bytes::Bytes>>, std::convert::Infallible>
where
    B: hyper::body::Body + Send + 'static,
    B::Data: Send,
    B::Error: std::error::Error + Send + Sync + 'static,
{
    let method = req.method().to_string();
    let raw_path = req.uri().path().to_string();
    let query_string = req.uri().query().map(|q| q.to_string());
    let path = if raw_path.len() > 1 && raw_path.ends_with('/') {
        raw_path.trim_end_matches('/').to_string()
    } else {
        raw_path
    };

    // 1. Probes
    if path == "/healthz" {
        return Ok(handlers::probes::handle_healthz(&telemetry));
    }
    if path == "/readyz" {
        return Ok(handlers::probes::handle_readyz());
    }
    if path == "/metrics" || path == "/admin/api/v1/metrics" {
        return Ok(handlers::probes::handle_metrics(&telemetry));
    }

    // Favicon & Admin Assets
    if path == "/favicon.ico"
        || path == "/favicon.svg"
        || path == "/admin/favicon.ico"
        || path == "/admin/favicon.svg"
    {
        return Ok(handlers::admin::handle_favicon());
    }

    // 2. Admin & Observability
    if path == "/admin/logs" || path == "/admin/api/v1/logs" {
        return Ok(handlers::admin::handle_logs(&telemetry));
    }
    if path == "/admin" || path == "/admin/" || path == "/dashboard" {
        return Ok(handlers::admin::handle_dashboard());
    }
    if path == "/admin/api/v1/overview" {
        return Ok(handlers::admin::handle_overview(&telemetry, &router, config_summary.as_deref()));
    }
    if path == "/admin/api/v1/config" && method == "GET" {
        return Ok(handlers::admin::handle_config(config_summary.as_deref()));
    }
    if path == "/admin/api/v1/traces" && method == "GET" {
        return Ok(handlers::admin::handle_traces(trace_storage.as_ref(), query_string.as_deref()).await);
    }
    if path.starts_with("/admin/api/v1/traces/") && method == "GET" {
        let cmd_id = path.strip_prefix("/admin/api/v1/traces/").unwrap_or("").trim_matches('/');
        return Ok(handlers::admin::handle_trace_by_id(trace_storage.as_ref(), cmd_id).await);
    }
    if path.starts_with("/admin/api/v1/checkpoints") {
        let cmd_id = path.strip_prefix("/admin/api/v1/checkpoints/").unwrap_or("");
        return Ok(handlers::admin::handle_checkpoints(cmd_id));
    }
    if path == "/admin/api/v1/security/lockdown" && method == "POST" {
        return Ok(handlers::admin::handle_lockdown(deployer.as_ref()));
    }

    // 3. Deployer Subsystem
    if path == "/_flux/deployer/status" && method == "GET" {
        return Ok(handlers::deployer::handle_status(deployer.as_ref()));
    }
    if path == "/_flux/deployer/history" && method == "GET" {
        return Ok(handlers::deployer::handle_history(deployer.as_ref()));
    }
    if path == "/_flux/deployer/upload" && method == "POST" {
        return Ok(handlers::deployer::handle_upload(deployer.as_ref(), req, query_string.as_deref()).await);
    }
    if path == "/_flux/deployer/activate" && method == "POST" {
        return Ok(handlers::deployer::handle_activate(deployer.as_ref(), req).await);
    }
    if path.starts_with("/_flux/deployer/fluxcells") && method == "DELETE" {
        return Ok(handlers::deployer::handle_remove(deployer.as_ref(), &path, query_string.as_deref()));
    }

    // 4. Fluxcell Route Dispatch
    Ok(handlers::dispatch::handle_fluxcell_dispatch(
        req,
        &path,
        query_string.as_deref(),
        &method,
        &router,
        &telemetry,
        dispatcher.as_ref(),
        trace_storage.as_ref(),
    ).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;
    use hyper::StatusCode;

    struct MockDispatcher {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
        should_fail: bool,
    }

    #[async_trait::async_trait]
    impl FluxcellHttpDispatcher for MockDispatcher {
        async fn dispatch(
            &self,
            _fluxcell_name: &str,
            _relative_path: &str,
            _method: &str,
            _headers: Vec<(String, String)>,
            _body: Vec<u8>,
        ) -> Result<(u16, Vec<(String, String)>, Vec<u8>), anyhow::Error> {
            if self.should_fail {
                Err(anyhow::anyhow!("Mock execution error"))
            } else {
                Ok((self.status, self.headers.clone(), self.body.clone()))
            }
        }
    }

    #[test]
    fn test_router_registration_and_lookup() {
        let mut router = FluxRouter::new();

        let magic_link_routes = vec![
            RouteDefinition::new("GET", "/verify", "Verify token"),
            RouteDefinition::new("POST", "/verify", "Redeem token"),
            RouteDefinition::new("GET", "/status", "Status"),
        ];

        let webhook_routes = vec![
            RouteDefinition::new("GET", "/health", "Health"),
            RouteDefinition::new("GET", "/dlq", "DLQ status"),
        ];

        router.register_fluxcell_routes("magic-link", "/auth", &magic_link_routes).unwrap();
        router.register_fluxcell_routes("webhook", "/api/webhooks", &webhook_routes).unwrap();

        // Verify lookup
        let m = router.lookup("GET", "/auth/verify").unwrap();
        assert_eq!(m.fluxcell_name, "magic-link");
        assert_eq!(m.mount_path, "/auth");
        assert_eq!(m.relative_path, "/verify");

        let m2 = router.lookup("GET", "/api/webhooks/dlq").unwrap();
        assert_eq!(m2.fluxcell_name, "webhook");
        assert_eq!(m2.mount_path, "/api/webhooks");
        assert_eq!(m2.relative_path, "/dlq");
    }

    #[test]
    fn test_router_collision_detection() {
        let mut router = FluxRouter::new();

        let fluxcell1_routes = vec![RouteDefinition::new("GET", "/verify", "First")];
        let fluxcell2_routes = vec![RouteDefinition::new("GET", "/verify", "Conflicting")];

        // Mount first to /auth
        router.register_fluxcell_routes("auth-v1", "/auth", &fluxcell1_routes).unwrap();

        // Attempt to mount second to /auth with overlapping route
        let err = router.register_fluxcell_routes("auth-v2", "/auth", &fluxcell2_routes).unwrap_err();
        
        match err {
            RouterError::Collision { path, existing_fluxcell, new_fluxcell } => {
                assert_eq!(path, "/auth/verify");
                assert_eq!(existing_fluxcell, "auth-v1");
                assert_eq!(new_fluxcell, "auth-v2");
            }
            other => panic!("Expected Collision error, got: {:?}", other),
        }
    }

    #[test]
    fn test_router_method_not_allowed_and_rfc9110_allow_header() {
        let mut router = FluxRouter::new();

        let routes = vec![
            RouteDefinition::new("GET", "/verify", "Check token"),
            RouteDefinition::new("POST", "/verify", "Redeem token"),
        ];

        router.register_fluxcell_routes("auth", "/auth", &routes).unwrap();

        // DELETE /auth/verify should return MethodNotAllowed with allowed: ["GET", "POST"]
        let err = router.lookup("DELETE", "/auth/verify").unwrap_err();
        match err {
            RouterError::MethodNotAllowed { method, path, allowed } => {
                assert_eq!(method, "DELETE");
                assert_eq!(path, "/auth/verify");
                assert_eq!(allowed, vec!["GET".to_string(), "POST".to_string()]);
            }
            other => panic!("Expected MethodNotAllowed, got: {:?}", other),
        }

        // Truly nonexistent path returns NotFound
        let err_not_found = router.lookup("GET", "/auth/nonexistent").unwrap_err();
        assert!(matches!(err_not_found, RouterError::NotFound { .. }));
    }

    #[test]
    fn test_router_trailing_slash_normalization() {
        let mut router = FluxRouter::new();

        let routes = vec![RouteDefinition::new("GET", "/verify", "Verify")];

        router.register_fluxcell_routes("auth", "/auth", &routes).unwrap();

        // Should resolve both with and without trailing slash
        let m1 = router.lookup("GET", "/auth/verify").unwrap();
        let m2 = router.lookup("GET", "/auth/verify/").unwrap();
        assert_eq!(m1.full_path, m2.full_path);
        assert_eq!(m1.relative_path, m2.relative_path);
    }

    #[test]
    fn test_router_root_mount_path_cleaning() {
        let mut router = FluxRouter::new();

        let routes = vec![RouteDefinition::new("GET", "/health", "Health")];

        // Mount at root "" or "/"
        router.register_fluxcell_routes("root_cell", "", &routes).unwrap();

        let m = router.lookup("GET", "/health").unwrap();
        assert_eq!(m.full_path, "/health");
        assert_eq!(m.relative_path, "/health");
    }

    #[tokio::test]
    async fn test_handle_request_probes() {
        let router = Arc::new(std::sync::RwLock::new(FluxRouter::new()));
        let telemetry = Arc::new(crate::telemetry::TelemetryClient::new(
            "worker-test-1".to_string(),
            "nats".to_string(),
            None,
            100,
        ));
        telemetry.record_log("INFO", "Initialized test", None);

        let dispatcher = Arc::new(MockDispatcher {
            status: 200,
            headers: vec![],
            body: vec![],
            should_fail: false,
        });

        // 1. /healthz
        let req = Request::builder().uri("/healthz").body(Full::new(bytes::Bytes::new())).unwrap();
        let resp = handle_request(req, router.clone(), telemetry.clone(), dispatcher.clone(), None, None).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(json["status"], "ok");

        // 2. /healthz/ (with trailing slash)
        let req = Request::builder().uri("/healthz/").body(Full::new(bytes::Bytes::new())).unwrap();
        let resp = handle_request(req, router.clone(), telemetry.clone(), dispatcher.clone(), None, None).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // 3. /readyz
        let req = Request::builder().uri("/readyz").body(Full::new(bytes::Bytes::new())).unwrap();
        let resp = handle_request(req, router.clone(), telemetry.clone(), dispatcher.clone(), None, None).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // 4. /metrics
        let req = Request::builder().uri("/metrics").body(Full::new(bytes::Bytes::new())).unwrap();
        let resp = handle_request(req, router.clone(), telemetry.clone(), dispatcher.clone(), None, None).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(json["worker_id"], "worker-test-1");

        // 5. /admin/logs
        let req = Request::builder().uri("/admin/logs").body(Full::new(bytes::Bytes::new())).unwrap();
        let resp = handle_request(req, router.clone(), telemetry.clone(), dispatcher.clone(), None, None).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let json: Vec<serde_json::Value> = serde_json::from_slice(&body_bytes).unwrap();
        assert!(!json.is_empty());
        assert_eq!(json[0]["message"], "Initialized test");

        // 6. /admin and /dashboard HTML UI
        let req = Request::builder().uri("/admin").body(Full::new(bytes::Bytes::new())).unwrap();
        let resp = handle_request(req, router.clone(), telemetry.clone(), dispatcher.clone(), None, None).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get("Content-Type").unwrap(), "text/html; charset=utf-8");
        let html_bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let html_str = String::from_utf8_lossy(&html_bytes);
        assert!(html_str.contains("Live Logs Active"));

        let req = Request::builder().uri("/dashboard").body(Full::new(bytes::Bytes::new())).unwrap();
        let resp = handle_request(req, router.clone(), telemetry.clone(), dispatcher.clone(), None, None).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // 7. /admin/api/v1/overview
        let req = Request::builder().uri("/admin/api/v1/overview").body(Full::new(bytes::Bytes::new())).unwrap();
        let resp = handle_request(req, router.clone(), telemetry.clone(), dispatcher.clone(), None, None).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let overview: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(overview["workerId"], "worker-test-1");
        assert!(overview.get("fluxcells").is_some());

        // 8. /admin/api/v1/logs
        let req = Request::builder().uri("/admin/api/v1/logs").body(Full::new(bytes::Bytes::new())).unwrap();
        let resp = handle_request(req, router.clone(), telemetry.clone(), dispatcher.clone(), None, None).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let logs: Vec<serde_json::Value> = serde_json::from_slice(&body_bytes).unwrap();
        assert!(!logs.is_empty());

        // 9. /admin/api/v1/metrics
        let req = Request::builder().uri("/admin/api/v1/metrics").body(Full::new(bytes::Bytes::new())).unwrap();
        let resp = handle_request(req, router.clone(), telemetry.clone(), dispatcher.clone(), None, None).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // 10. /admin/api/v1/checkpoints/:cmd_id
        let req = Request::builder().uri("/admin/api/v1/checkpoints/018f3a2b-1234").body(Full::new(bytes::Bytes::new())).unwrap();
        let resp = handle_request(req, router.clone(), telemetry.clone(), dispatcher.clone(), None, None).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let chk: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(chk["commandId"], "018f3a2b-1234");

        // 11. Favicon endpoints (/favicon.ico, /favicon.svg, /admin/favicon.ico, /admin/favicon.svg)
        for fav_uri in ["/favicon.ico", "/favicon.svg", "/admin/favicon.ico", "/admin/favicon.svg"] {
            let req = Request::builder().uri(fav_uri).body(Full::new(bytes::Bytes::new())).unwrap();
            let resp = handle_request(req, router.clone(), telemetry.clone(), dispatcher.clone(), None, None).await.unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
            assert_eq!(resp.headers().get("Content-Type").unwrap(), "image/svg+xml");
            assert_eq!(resp.headers().get("Cache-Control").unwrap(), "public, max-age=86400, immutable");
            let body = resp.into_body().collect().await.unwrap().to_bytes();
            assert_eq!(body.as_ref(), FAVICON_SVG.as_bytes());
        }
        // Ensure no error count incremented
        assert_eq!(telemetry.error_count.load(std::sync::atomic::Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn test_handle_request_route_dispatch_success_with_custom_headers() {
        let mut router = FluxRouter::new();
        router.register_fluxcell_routes("auth", "/auth", &[RouteDefinition::new(
            "POST",
            "/login",
            "Login redirect",
        )]).unwrap();

        let router = Arc::new(std::sync::RwLock::new(router));
        let telemetry = Arc::new(crate::telemetry::TelemetryClient::new(
            "worker-test".to_string(),
            "nats".to_string(),
            None,
            100,
        ));

        let dispatcher = Arc::new(MockDispatcher {
            status: 302,
            headers: vec![
                ("Location".to_string(), "/dashboard".to_string()),
                ("Set-Cookie".to_string(), "session=abc123xyz; Path=/".to_string()),
            ],
            body: b"Redirecting...".to_vec(),
            should_fail: false,
        });

        let req = Request::builder()
            .method("POST")
            .uri("/auth/login")
            .body(Full::new(bytes::Bytes::from("{\"username\":\"admin\"}")))
            .unwrap();

        let resp = handle_request(req, router, telemetry, dispatcher, None, None).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FOUND);
        assert_eq!(resp.headers().get("Location").unwrap(), "/dashboard");
        assert_eq!(resp.headers().get("Set-Cookie").unwrap(), "session=abc123xyz; Path=/");

        let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(body_bytes.as_ref(), b"Redirecting...");
    }

    #[tokio::test]
    async fn test_handle_request_method_not_allowed_header() {
        let mut router = FluxRouter::new();
        router.register_fluxcell_routes("auth", "/auth", &[RouteDefinition::new(
            "GET",
            "/verify",
            "Verify",
        )]).unwrap();

        let router = Arc::new(std::sync::RwLock::new(router));
        let telemetry = Arc::new(crate::telemetry::TelemetryClient::new(
            "worker-test".to_string(),
            "nats".to_string(),
            None,
            100,
        ));
        let dispatcher = Arc::new(MockDispatcher {
            status: 200,
            headers: vec![],
            body: vec![],
            should_fail: false,
        });

        let req = Request::builder()
            .method("DELETE")
            .uri("/auth/verify")
            .body(Full::new(bytes::Bytes::new()))
            .unwrap();

        let resp = handle_request(req, router, telemetry, dispatcher, None, None).await.unwrap();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(resp.headers().get("Allow").unwrap(), "GET");
    }

    #[tokio::test]
    async fn test_handle_request_dispatcher_error_increments_telemetry() {
        let mut router = FluxRouter::new();
        router.register_fluxcell_routes("flaky", "/flaky", &[RouteDefinition::new(
            "GET",
            "/fail",
            "Failing route",
        )]).unwrap();

        let router = Arc::new(std::sync::RwLock::new(router));
        let telemetry = Arc::new(crate::telemetry::TelemetryClient::new(
            "worker-test".to_string(),
            "nats".to_string(),
            None,
            100,
        ));
        let dispatcher = Arc::new(MockDispatcher {
            status: 200,
            headers: vec![],
            body: vec![],
            should_fail: true,
        });

        let req = Request::builder()
            .method("GET")
            .uri("/flaky/fail")
            .body(Full::new(bytes::Bytes::new()))
            .unwrap();

        let resp = handle_request(req, router, telemetry.clone(), dispatcher, None, None).await.unwrap();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(telemetry.error_count.load(std::sync::atomic::Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn test_http_adversarial_oversized_body() {
        let mut router = FluxRouter::new();
        router.register_fluxcell_routes("echo", "/echo", &[RouteDefinition::new(
            "POST",
            "/data",
            "Echo data",
        )]).unwrap();

        let router = Arc::new(std::sync::RwLock::new(router));
        let telemetry = Arc::new(crate::telemetry::TelemetryClient::new(
            "worker-test".to_string(),
            "nats".to_string(),
            None,
            100,
        ));

        // 3MB payload
        let big_body = vec![b'Z'; 3 * 1024 * 1024];
        let big_body_clone = big_body.clone();

        struct EchoDispatcher;
        #[async_trait::async_trait]
        impl FluxcellHttpDispatcher for EchoDispatcher {
            async fn dispatch(
                &self,
                _fluxcell_name: &str,
                _relative_path: &str,
                _method: &str,
                _headers: Vec<(String, String)>,
                body: Vec<u8>,
            ) -> Result<(u16, Vec<(String, String)>, Vec<u8>), anyhow::Error> {
                Ok((200, vec![], body))
            }
        }

        let req = Request::builder()
            .method("POST")
            .uri("/echo/data")
            .body(Full::new(bytes::Bytes::from(big_body)))
            .unwrap();

        let resp = handle_request(req, router, telemetry, Arc::new(EchoDispatcher), None, None).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let resp_bytes = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(resp_bytes.len(), big_body_clone.len());
    }

    #[tokio::test]
    async fn test_http_adversarial_path_traversal_attempts() {
        let mut router = FluxRouter::new();
        router.register_fluxcell_routes("auth", "/auth", &[RouteDefinition::new(
            "GET",
            "/verify",
            "Verify",
        )]).unwrap();

        let router = Arc::new(std::sync::RwLock::new(router));
        let telemetry = Arc::new(crate::telemetry::TelemetryClient::new(
            "worker-test".to_string(),
            "nats".to_string(),
            None,
            100,
        ));
        let dispatcher = Arc::new(MockDispatcher {
            status: 200,
            headers: vec![],
            body: vec![],
            should_fail: false,
        });

        let malicious_uris = vec![
            "/auth/../../etc/passwd",
            "/auth/..%2f..%2fetc/shadow",
            "/api/webhooks/../../secret.key",
            "/auth/%00/verify",
        ];

        for uri in malicious_uris {
            let req = Request::builder()
                .method("GET")
                .uri(uri)
                .body(Full::new(bytes::Bytes::new()))
                .unwrap();

            let resp = handle_request(req, router.clone(), telemetry.clone(), dispatcher.clone(), None, None).await.unwrap();
            // Should be safely rejected with 404 Not Found without panicking or path traversal
            assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        }
    }

    #[tokio::test]
    async fn test_handle_request_deployer_governance_and_lockdown() {
        let temp_dir = std::env::temp_dir().join(format!("spectra_deploy_http_{}", uuid::Uuid::new_v4()));
        let mut dep_cfg = crate::config::DeployerConfig::default();
        dep_cfg.enabled = true;
        dep_cfg.storage_dir = temp_dir.to_string_lossy().to_string();
        dep_cfg.external_deploy_enabled = true;
        dep_cfg.dev_upload_enabled = true;

        let guard = Arc::new(crate::deployer::DeployerGuard::new(true, true));
        let registry = Arc::new(crate::deployer::DeployerRegistry::new(&temp_dir).unwrap());
        let wasm_host = Arc::new(crate::wasm::WasmHost::new(5, None).unwrap());
        let router = Arc::new(std::sync::RwLock::new(FluxRouter::new()));

        let deployer = Arc::new(crate::deployer::FluxcellDeployer::new(
            dep_cfg,
            guard.clone(),
            registry,
            wasm_host,
            router.clone(),
        ));

        let telemetry = Arc::new(crate::telemetry::TelemetryClient::new(
            "test-worker".to_string(),
            "nats".to_string(),
            None,
            100,
        ));
        let dispatcher = Arc::new(MockDispatcher {
            status: 200,
            headers: vec![],
            body: vec![],
            should_fail: false,
        });

        // 1. GET /_flux/deployer/status
        let req = Request::builder()
            .method("GET")
            .uri("/_flux/deployer/status")
            .body(Full::new(bytes::Bytes::new()))
            .unwrap();
        let resp = handle_request(req, router.clone(), telemetry.clone(), dispatcher.clone(), Some(deployer.clone()), None).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["external_deploy_enabled"], true);
        assert_eq!(json["dev_upload_enabled"], true);

        // 2. POST /admin/api/v1/security/lockdown
        let req = Request::builder()
            .method("POST")
            .uri("/admin/api/v1/security/lockdown")
            .body(Full::new(bytes::Bytes::new()))
            .unwrap();
        let resp = handle_request(req, router.clone(), telemetry.clone(), dispatcher.clone(), Some(deployer.clone()), None).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "locked_down");
        assert_eq!(json["external_deploy_enabled"], false);
        assert_eq!(json["dev_upload_enabled"], false);

        // Verify guard state changed immediately
        assert!(!guard.is_external_deploy_allowed());
        assert!(!guard.is_dev_upload_allowed());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_handle_request_deployer_upload_with_mount_path_and_auto_activate() {
        let temp_dir = std::env::temp_dir().join(format!("spectra_deploy_upload_{}", uuid::Uuid::new_v4()));
        let mut dep_cfg = crate::config::DeployerConfig::default();
        dep_cfg.enabled = true;
        dep_cfg.storage_dir = temp_dir.to_string_lossy().to_string();
        dep_cfg.external_deploy_enabled = true;
        dep_cfg.dev_upload_enabled = true;

        let guard = Arc::new(crate::deployer::DeployerGuard::new(true, true));
        let registry = Arc::new(crate::deployer::DeployerRegistry::new(&temp_dir).unwrap());
        let wasm_host = Arc::new(crate::wasm::WasmHost::new(5, None).unwrap());
        let router = Arc::new(std::sync::RwLock::new(FluxRouter::new()));

        let deployer = Arc::new(crate::deployer::FluxcellDeployer::new(
            dep_cfg,
            guard,
            registry.clone(),
            wasm_host,
            router.clone(),
        ));

        let telemetry = Arc::new(crate::telemetry::TelemetryClient::new(
            "test-worker".to_string(),
            "nats".to_string(),
            None,
            100,
        ));
        let dispatcher = Arc::new(MockDispatcher {
            status: 200,
            headers: vec![],
            body: vec![],
            should_fail: false,
        });

        // Valid minimal WASM module bytes
        let wasm_bytes = wat::parse_str(
            r#"(module
                (memory (export "memory") 1)
                (func (export "allocate") (param i32) (result i32) i32.const 0)
                (func (export "deallocate") (param i32 i32))
            )"#,
        ).unwrap();

        // Upload using mount_path and auto_activate=true query params (matching CLI behavior)
        let req = Request::builder()
            .method("POST")
            .uri("/_flux/deployer/upload?name=test-upload-cell&mount_path=/custom/api&auto_activate=true")
            .body(Full::new(bytes::Bytes::from(wasm_bytes)))
            .unwrap();

        let resp = handle_request(req, router.clone(), telemetry.clone(), dispatcher.clone(), Some(deployer.clone()), None).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["name"], "test-upload-cell");
        assert_eq!(json["mount_path"], "/custom/api");
        // Status must be Active because auto_activate=true was specified
        assert_eq!(json["status"], "Active");

        // Verify registry record
        let record = registry.get_record("test-upload-cell").expect("Record must be saved in registry");
        assert_eq!(record.mount_path, "/custom/api");
        assert_eq!(record.status, crate::deployer::FluxcellStatus::Active);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_admin_traces_api() {
        let router = Arc::new(std::sync::RwLock::new(FluxRouter::new()));
        let telemetry = Arc::new(crate::telemetry::TelemetryClient::new(
            "worker-test".to_string(),
            "nats".to_string(),
            None,
            100,
        ));
        let dispatcher = Arc::new(MockDispatcher {
            status: 200,
            headers: vec![],
            body: vec![],
            should_fail: false,
        });

        // 1. Without trace storage
        let req = Request::builder()
            .method("GET")
            .uri("/admin/api/v1/traces")
            .body(Full::new(bytes::Bytes::new()))
            .unwrap();
        let resp = handle_request(req, router.clone(), telemetry.clone(), dispatcher.clone(), None, None).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(body_bytes.as_ref(), b"[]");

        // 2. With trace storage
        let mem_storage = Arc::new(crate::storage::KevyStorage::new_in_memory().unwrap());
        let trace_storage = Arc::new(crate::telemetry::DomainTraceStorage::new(mem_storage));

        let trace = crate::telemetry::DomainOperationTrace {
            command_id: "0191f6a1-0000-7000-8000-000000000001".to_string(),
            hlc: "2026-09-16T12:00:00.000Z-0001".to_string(),
            topic: "mutation.recordvote".to_string(),
            operation_name: "recordVote".to_string(),
            ingress: "QUEUE".to_string(),
            worker_id: "worker-1".to_string(),
            status: "completed".to_string(),
            total_duration_ms: 1.25,
            started_at: "2026-09-16T12:00:00Z".to_string(),
            completed_at: "2026-09-16T12:00:00.001Z".to_string(),
            initial_input: serde_json::json!({"proposalId": "p-123", "vote": "YES"}),
            steps: vec![
                crate::telemetry::FluxcellStepSpan {
                    fluxcell_name: "voting_engine".to_string(),
                    topic: "mutation.recordvote".to_string(),
                    function_name: "on_event".to_string(),
                    start_time: "2026-09-16T12:00:00Z".to_string(),
                    duration_ms: 1.2,
                    status: "ok".to_string(),
                    input_preview: Some(serde_json::json!({"proposalId": "p-123"})),
                    output_preview: Some(serde_json::json!({"tallied": true})),
                    error: None,
                    host_calls: vec![
                        crate::telemetry::HostCallSpan {
                            call_type: "db:execute".to_string(),
                            target: "INSERT INTO votes ...".to_string(),
                            duration_us: 800,
                            status: "ok".to_string(),
                            detail: Some("rows: 1".to_string()),
                        }
                    ],
                }
            ],
            terminal_output: Some(serde_json::json!({"voteRecorded": true})),
            error: None,
        };
        trace_storage.record_trace(&trace).await.unwrap();

        // Query list
        let req = Request::builder()
            .method("GET")
            .uri("/admin/api/v1/traces")
            .body(Full::new(bytes::Bytes::new()))
            .unwrap();
        let resp = handle_request(req, router.clone(), telemetry.clone(), dispatcher.clone(), None, Some(trace_storage.clone())).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let list: Vec<crate::telemetry::RecentTraceSummary> = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].command_id, "0191f6a1-0000-7000-8000-000000000001");
        assert_eq!(list[0].step_count, 1);
        assert_eq!(list[0].host_call_count, 1);

        // Query detail
        let req = Request::builder()
            .method("GET")
            .uri("/admin/api/v1/traces/0191f6a1-0000-7000-8000-000000000001")
            .body(Full::new(bytes::Bytes::new()))
            .unwrap();
        let resp = handle_request(req, router.clone(), telemetry.clone(), dispatcher.clone(), None, Some(trace_storage.clone())).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let detail: crate::telemetry::DomainOperationTrace = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(detail.command_id, "0191f6a1-0000-7000-8000-000000000001");
        assert_eq!(detail.steps[0].host_calls[0].call_type, "db:execute");

        // Query non-existent detail
        let req = Request::builder()
            .method("GET")
            .uri("/admin/api/v1/traces/non-existent")
            .body(Full::new(bytes::Bytes::new()))
            .unwrap();
        let resp = handle_request(req, router.clone(), telemetry.clone(), dispatcher.clone(), None, Some(trace_storage.clone())).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_handle_request_http_fluxcell_records_trace() {
        let mut router = FluxRouter::new();
        router.register_fluxcell_routes(
            "vote_cell",
            "/votes",
            &[RouteDefinition::new("POST", "/mutate", "Cast vote")],
        ).unwrap();
        let router = Arc::new(std::sync::RwLock::new(router));

        let telemetry = Arc::new(crate::telemetry::TelemetryClient::new(
            "worker-http-test".to_string(),
            "nats".to_string(),
            None,
            100,
        ));

        struct SpanMockDispatcher;
        #[async_trait::async_trait]
        impl FluxcellHttpDispatcher for SpanMockDispatcher {
            async fn dispatch(
                &self,
                _fluxcell_name: &str,
                _relative_path: &str,
                _method: &str,
                _headers: Vec<(String, String)>,
                _body: Vec<u8>,
            ) -> Result<(u16, Vec<(String, String)>, Vec<u8>), anyhow::Error> {
                Ok((200, vec![("content-type".to_string(), "application/json".to_string())], b"{\"success\":true}".to_vec()))
            }

            async fn dispatch_with_spans(
                &self,
                _fluxcell_name: &str,
                _relative_path: &str,
                _method: &str,
                _headers: Vec<(String, String)>,
                _body: Vec<u8>,
            ) -> Result<(u16, Vec<(String, String)>, Vec<u8>, Vec<crate::telemetry::HostCallSpan>), anyhow::Error> {
                let spans = vec![
                    crate::telemetry::HostCallSpan {
                        call_type: "db:execute".to_string(),
                        target: "INSERT INTO votes ...".to_string(),
                        duration_us: 420,
                        status: "ok".to_string(),
                        detail: Some("rows: 1".to_string()),
                    }
                ];
                Ok((200, vec![("content-type".to_string(), "application/json".to_string())], b"{\"success\":true}".to_vec(), spans))
            }
        }

        let mem_storage = Arc::new(crate::storage::KevyStorage::new_in_memory().unwrap());
        let trace_storage = Arc::new(crate::telemetry::DomainTraceStorage::new(mem_storage));

        let req = Request::builder()
            .method("POST")
            .uri("/votes/mutate")
            .header("x-command-id", "0191f6a1-http-0000-8000-000000000002")
            .header("x-hlc", "2026-09-17T00:00:00.000Z-0001")
            .header("content-type", "application/json")
            .body(Full::new(bytes::Bytes::from(r#"{"operationName":"recordVote","proposalId":"p-999","value":1}"#)))
            .unwrap();

        let resp = handle_request(
            req,
            router.clone(),
            telemetry.clone(),
            Arc::new(SpanMockDispatcher),
            None,
            Some(trace_storage.clone()),
        ).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);

        // Allow spawned trace recording task to complete
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let trace = trace_storage.get_trace("0191f6a1-http-0000-8000-000000000002").await.unwrap();
        assert!(trace.is_some(), "Expected trace to be recorded in DomainTraceStorage");
        let trace = trace.unwrap();

        assert_eq!(trace.command_id, "0191f6a1-http-0000-8000-000000000002");
        assert_eq!(trace.hlc, "2026-09-17T00:00:00.000Z-0001");
        assert_eq!(trace.ingress, "HTTP");
        assert_eq!(trace.topic, "http:/votes/mutate");
        assert_eq!(trace.operation_name, "recordVote");
        assert_eq!(trace.status, "completed");
        assert_eq!(trace.steps.len(), 1);
        assert_eq!(trace.steps[0].fluxcell_name, "vote_cell");
        assert_eq!(trace.steps[0].host_calls.len(), 1);
        assert_eq!(trace.steps[0].host_calls[0].call_type, "db:execute");
    }

    #[tokio::test]
    async fn test_handle_request_404_records_trace() {
        let router = Arc::new(std::sync::RwLock::new(FluxRouter::new()));
        let telemetry = Arc::new(crate::telemetry::TelemetryClient::new(
            "worker-test".to_string(),
            "in_memory".to_string(),
            None,
            100,
        ));
        let mem_storage = Arc::new(crate::storage::KevyStorage::new_in_memory().unwrap());
        let trace_storage = Arc::new(crate::telemetry::DomainTraceStorage::new(mem_storage));

        let req = Request::builder()
            .method("POST")
            .uri("/nonexistent/endpoint")
            .header("x-command-id", "0191f6a1-4040-7000-8000-000000000404")
            .header("x-hlc", "2026-09-17T00:00:00.000Z-0002")
            .header("content-type", "application/json")
            .body(Full::new(bytes::Bytes::from(r#"{"operationName":"missingOp","data":"none"}"#)))
            .unwrap();

        let resp = handle_request(
            req,
            router.clone(),
            telemetry.clone(),
            Arc::new(MockDispatcher {
                status: 200,
                headers: vec![],
                body: vec![],
                should_fail: false,
            }),
            None,
            Some(trace_storage.clone()),
        ).await.unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        // Allow trace task to finish
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let trace = trace_storage.get_trace("0191f6a1-4040-7000-8000-000000000404").await.unwrap();
        assert!(trace.is_some(), "Expected 404 request to be recorded in DomainTraceStorage");
        let trace = trace.unwrap();

        assert_eq!(trace.command_id, "0191f6a1-4040-7000-8000-000000000404");
        assert_eq!(trace.ingress, "HTTP");
        assert_eq!(trace.topic, "http:/nonexistent/endpoint");
        assert_eq!(trace.operation_name, "missingOp");
        assert_eq!(trace.status, "failed");
        assert!(trace.error.unwrap().contains("404 NOT_FOUND"));
    }

    #[tokio::test]
    async fn test_admin_config_endpoint() {
        let router = Arc::new(std::sync::RwLock::new(FluxRouter::new()));
        let telemetry = Arc::new(crate::telemetry::TelemetryClient::new(
            "worker-cfg-test".to_string(),
            "in_memory".to_string(),
            None,
            100,
        ));
        let dispatcher = Arc::new(MockDispatcher {
            status: 200,
            headers: vec![],
            body: vec![],
            should_fail: false,
        });

        let cfg = crate::config::FluxConfig::default_local();
        let config_summary = Arc::new(cfg.to_sanitized_json("test-config.toml"));

        // Test GET /admin/api/v1/config
        let req = Request::builder()
            .method("GET")
            .uri("/admin/api/v1/config")
            .body(Full::new(bytes::Bytes::new()))
            .unwrap();

        let resp = handle_request_with_config(
            req,
            router.clone(),
            telemetry.clone(),
            dispatcher.clone(),
            None,
            None,
            Some(config_summary.clone()),
        ).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(json["configSource"], "test-config.toml");
        assert_eq!(json["host"], "0.0.0.0");
        assert_eq!(json["port"], 8081);
        assert_eq!(json["broker"]["method"], "in_memory");
        assert!(json["profiles"]["standard"].is_object());

        // Test GET /admin/api/v1/overview includes config
        let req = Request::builder()
            .method("GET")
            .uri("/admin/api/v1/overview")
            .body(Full::new(bytes::Bytes::new()))
            .unwrap();

        let resp = handle_request_with_config(
            req,
            router.clone(),
            telemetry.clone(),
            dispatcher.clone(),
            None,
            None,
            Some(config_summary.clone()),
        ).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(json["workerId"], "worker-cfg-test");
        assert_eq!(json["config"]["configSource"], "test-config.toml");
    }
}
