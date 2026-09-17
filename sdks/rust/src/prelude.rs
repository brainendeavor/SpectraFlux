//! Ergonomic Prelude for Fluxcell Development
//!
//! Conveniently re-exports the most commonly used types, traits, and macros:
//!
//! ```rust
//! use fluxcell_sdk::prelude::*;
//! ```

pub use crate::broker::publish_event;
pub use crate::causal::{advance_db_watermark, CausalGuard, CausalVerdict};
pub use crate::db::{Database, Transaction};
pub use crate::dedup::DeduplicationBuffer;
pub use crate::event::{EventContext, EventVerdict};
pub use crate::export_fluxcell;
pub use crate::hlc::{is_stale as is_hlc_stale, parse_hlc};
pub use crate::http::{HttpRequest, HttpResponse, RouteMeta};
pub use crate::telemetry::{TelemetryBuffer, TelemetryLogEntry};
pub use crate::{Fluxcell, FluxcellMetadata};
