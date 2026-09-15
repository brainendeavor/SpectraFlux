//! Barebones Starter Fluxcell for SpectraFlux
//!
//! A minimal, idiomatic WebAssembly micro-unit built on the official `fluxcell-sdk`.

use fluxcell_sdk::prelude::*;

pub const FLUXCELL_VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct StarterFluxcell;

impl Fluxcell for StarterFluxcell {
    fn metadata() -> FluxcellMetadata {
        FluxcellMetadata::new(FLUXCELL_VERSION, "Barebones Starter Fluxcell")
    }

    fn routes() -> Vec<RouteMeta> {
        vec![
            RouteMeta::get("/health", "Health check endpoint"),
            RouteMeta::post("/echo", "Echoes incoming payload or executes transactional command"),
        ]
    }

    fn subscriptions() -> Vec<String> {
        vec!["events.sample".to_string()]
    }

    fn handle_http(req: HttpRequest) -> HttpResponse {
        match req.path.as_str() {
            "/health" => HttpResponse::json(&serde_json::json!({
                "status": "healthy",
                "version": FLUXCELL_VERSION,
                "git_hash": env!("GIT_HASH"),
                "build_timestamp": env!("BUILD_TIMESTAMP"),
            })),
            "/echo" => {
                let body_json: serde_json::Value = req.json().unwrap_or(serde_json::Value::Null);
                HttpResponse::json(&serde_json::json!({
                    "status": "ok",
                    "echo": body_json,
                }))
            }
            _ => HttpResponse::not_found(format!("Endpoint '{}' not found", req.path)),
        }
    }

    fn handle_event(event: EventContext) -> serde_json::Value {
        serde_json::json!({
            "status": "ok",
            "topic": event.topic,
            "id": event.event_id,
            "hlc": event.hlc,
        })
    }
}

export_fluxcell!(StarterFluxcell);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_starter_routes_and_subscriptions() {
        let routes = StarterFluxcell::routes();
        assert_eq!(routes.len(), 2);
        assert_eq!(routes[0].path, "/health");

        let subs = StarterFluxcell::subscriptions();
        assert_eq!(subs, vec!["events.sample"]);
    }

    #[test]
    fn test_starter_health_endpoint() {
        let req = HttpRequest {
            method: "GET".to_string(),
            path: "/health".to_string(),
            ..Default::default()
        };
        let resp = StarterFluxcell::handle_http(req);
        assert_eq!(resp.status, 200);
    }

    #[test]
    fn test_starter_echo_endpoint() {
        let req = HttpRequest {
            method: "POST".to_string(),
            path: "/echo".to_string(),
            body: b"{\"hello\":\"world\"}".to_vec(),
            ..Default::default()
        };
        let resp = StarterFluxcell::handle_http(req);
        assert_eq!(resp.status, 200);
    }
}
