//! Canonical Reference Fluxcell: Invoice Mailer & Webhook Service
//!
//! Demonstrates the complete guest interface for SpectraGQL / Spectral Flux:
//! - Automated semantic versioning and git SHA derivation
//! - Dynamic HTTP routes (`POST /send`, `GET /status`, `GET /templates`)
//! - Event stream subscriptions (`billing.invoice.created`, `mutation.sendinvoice`)
//! - Sandboxed execution and JSON envelope packing

use serde::{Deserialize, Serialize};

pub const FLUXCELL_VERSION: &str = env!("FLUXCELL_VERSION");
pub const GIT_HASH: &str = env!("GIT_HASH");
pub const BUILD_TIME: &str = env!("BUILD_TIME");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteMeta {
    pub method: String,
    pub path: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    pub version: String,
    pub git_hash: String,
    pub build_time: String,
    pub description: String,
    pub profile: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_memory_mb: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FluxcellConfigEnvelope {
    pub profile: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_memory_mb: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoicePayload {
    pub invoice_id: String,
    pub customer_email: String,
    pub amount_cents: u64,
    pub currency: String,
    pub due_date: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailReceipt {
    pub status: String,
    pub recipient: String,
    pub subject: String,
    pub delivered_at: String,
}

/// Returns the event topics this fluxcell subscribes to on the broker stream.
pub fn subscriptions() -> Vec<String> {
    vec![
        "billing.invoice.created".to_string(),
        "mutation.sendinvoice".to_string(),
    ]
}

/// Returns the HTTP endpoints dynamically mounted by this fluxcell.
pub fn routes() -> Vec<RouteMeta> {
    vec![
        RouteMeta {
            method: "POST".to_string(),
            path: "/mutate".to_string(),
            description: "Internal synchronous GraphQL mutation handler (Mode A passthrough)".to_string(),
            timeout_ms: Some(10_000),
        },
        RouteMeta {
            method: "POST".to_string(),
            path: "/send".to_string(),
            description: "Directly trigger invoice email delivery".to_string(),
            timeout_ms: Some(120_000),
        },
        RouteMeta {
            method: "GET".to_string(),
            path: "/status".to_string(),
            description: "Check mailer service health and delivery statistics".to_string(),
            timeout_ms: Some(5_000),
        },
        RouteMeta {
            method: "GET".to_string(),
            path: "/templates".to_string(),
            description: "List available invoice email HTML templates".to_string(),
            timeout_ms: Some(5_000),
        },
    ]
}

/// Returns compile-time build and version metadata.
pub fn metadata() -> Metadata {
    Metadata {
        version: FLUXCELL_VERSION.to_string(),
        git_hash: GIT_HASH.to_string(),
        build_time: BUILD_TIME.to_string(),
        description: "Asynchronous Invoice Delivery & Notification Service".to_string(),
        profile: "extended".to_string(),
        timeout_ms: Some(120_000),
        max_memory_mb: Some(32),
    }
}

/// Renders a responsive plaintext and HTML invoice receipt email.
pub fn render_invoice_email(invoice: &InvoicePayload) -> (String, String) {
    let subject = format!("Invoice {} for {} {}", invoice.invoice_id, invoice.currency, invoice.amount_cents as f64 / 100.0);
    let body = format!(
        "<!DOCTYPE html><html><body><h2>Invoice {}</h2><p>Amount: {} {:.2}</p><p>Due: {}</p></body></html>",
        invoice.invoice_id,
        invoice.currency,
        invoice.amount_cents as f64 / 100.0,
        invoice.due_date
    );
    (subject, body)
}

/// Returns the cell execution profile and hardware limits.
pub fn config() -> FluxcellConfigEnvelope {
    FluxcellConfigEnvelope {
        profile: "extended".to_string(),
        timeout_ms: Some(120_000),
        max_memory_mb: Some(32),
    }
}

// ============================================================================
// WASM Guest ABI Exports (Packed Pointers for Host Reflection)
// ============================================================================

static mut LAST_ALLOC: Option<Vec<u8>> = None;

#[unsafe(no_mangle)]
pub extern "C" fn get_metadata() -> u64 {
    let meta = metadata();
    let json = serde_json::to_string(&meta).unwrap_or_else(|_| "{}".to_string());
    pack_string(json)
}

#[unsafe(no_mangle)]
pub extern "C" fn get_config() -> u64 {
    let cfg = config();
    let json = serde_json::to_string(&cfg).unwrap_or_else(|_| "{}".to_string());
    pack_string(json)
}

#[unsafe(no_mangle)]
pub extern "C" fn get_subscriptions() -> u64 {
    let subs = subscriptions();
    let json = serde_json::to_string(&subs).unwrap_or_else(|_| "[]".to_string());
    pack_string(json)
}

#[unsafe(no_mangle)]
pub extern "C" fn get_routes() -> u64 {
    let r = routes();
    let json = serde_json::to_string(&r).unwrap_or_else(|_| "[]".to_string());
    pack_string(json)
}

fn pack_string(s: String) -> u64 {
    let bytes = s.into_bytes();
    let len = bytes.len() as u64;
    let ptr = bytes.as_ptr() as usize as u64;
    unsafe {
        LAST_ALLOC = Some(bytes);
    }
    (ptr << 32) | (len & 0xFFFFFFFF)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metadata_and_routes() {
        let meta = metadata();
        assert!(!meta.version.is_empty());
        assert!(!meta.git_hash.is_empty());
        assert_eq!(meta.profile, "extended");
        assert_eq!(meta.timeout_ms, Some(120_000));

        let r = routes();
        assert_eq!(r.len(), 4);
        assert_eq!(r[0].path, "/mutate");
        assert_eq!(r[0].method, "POST");
        assert_eq!(r[0].timeout_ms, Some(10_000));
        assert_eq!(r[1].path, "/send");
        assert_eq!(r[1].method, "POST");
        assert_eq!(r[1].timeout_ms, Some(120_000));

        let subs = subscriptions();
        assert_eq!(subs.len(), 2);
    }

    #[test]
    fn test_config_envelope() {
        let cfg = config();
        assert_eq!(cfg.profile, "extended");
        assert_eq!(cfg.timeout_ms, Some(120_000));
        assert_eq!(cfg.max_memory_mb, Some(32));
    }

    #[test]
    fn test_render_invoice_email() {
        let inv = InvoicePayload {
            invoice_id: "INV-2026-001".to_string(),
            customer_email: "billing@client.com".to_string(),
            amount_cents: 25000,
            currency: "USD".to_string(),
            due_date: "2026-10-01".to_string(),
        };
        let (subject, html) = render_invoice_email(&inv);
        assert!(subject.contains("INV-2026-001"));
        assert!(html.contains("250.00"));
    }
}
