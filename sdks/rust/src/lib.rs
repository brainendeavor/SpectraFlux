//! Canonical Guest SDK for SpectraFlux WebAssembly Fluxcells
//!
//! Provides zero-unsafe abstractions, RAII database transactions,
//! HTTP response builders, monotonic HLC causality utilities,
//! and the `export_fluxcell!` entrypoint macro.

pub mod abi;
pub mod broker;
pub mod causal;
pub mod db;
pub mod dedup;
pub mod event;
pub mod hlc;
pub mod http;
pub mod macros;
pub mod prelude;
pub mod telemetry;

use serde::{Deserialize, Serialize};

/// Build and version metadata exposed by the Fluxcell.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FluxcellMetadata {
    pub version: String,
    pub git_hash: String,
    pub build_time: String,
    pub description: String,
}

impl FluxcellMetadata {
    pub fn new(version: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            version: version.into(),
            git_hash: option_env!("GIT_HASH").unwrap_or("unknown").to_string(),
            build_time: option_env!("BUILD_TIME").unwrap_or("").to_string(),
            description: description.into(),
        }
    }
}

/// The core trait defining a Fluxcell's capabilities, routing, and event handlers.
pub trait Fluxcell {
    /// Returns compile-time metadata for the Fluxcell.
    fn metadata() -> FluxcellMetadata;

    /// Returns the HTTP endpoints dynamically mounted by this Fluxcell.
    fn routes() -> Vec<http::RouteMeta> {
        Vec::new()
    }

    /// Returns the event topics this Fluxcell subscribes to on the broker stream.
    fn subscriptions() -> Vec<String> {
        Vec::new()
    }

    /// Handles an incoming synchronous HTTP request.
    fn handle_http(_req: http::HttpRequest) -> http::HttpResponse {
        http::HttpResponse::not_found("No HTTP routes handled by this fluxcell")
    }

    /// Handles an incoming asynchronous broker event.
    fn handle_event(_event: event::EventContext) -> serde_json::Value {
        serde_json::json!({ "status": "ok" })
    }
}
