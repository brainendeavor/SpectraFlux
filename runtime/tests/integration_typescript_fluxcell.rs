use spectra_flux::broker::{BrokerConsumerAdapter, InMemoryBroker};
use spectra_flux::wasm::{FluxcellWasmConfig, WasmHost};
use std::path::PathBuf;
use std::sync::Arc;

#[tokio::test]
async fn test_typescript_magic_link_fluxcell_full_lifecycle() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let wasm_path = manifest_dir
        .parent()
        .expect("Workspace root")
        .join("fluxcells/typescript/magic-link/build/release.wasm");

    // Ensure the TypeScript fluxcell is built before testing
    if !wasm_path.exists() {
        let status = std::process::Command::new("bun")
            .args(["run", "build"])
            .current_dir(wasm_path.parent().unwrap().parent().unwrap())
            .status()
            .expect("Failed to execute bun run build");
        assert!(status.success(), "Failed to build TypeScript magic-link fluxcell");
    }

    let wasm_bytes = std::fs::read(&wasm_path)
        .unwrap_or_else(|e| panic!("Failed to read {}: {}", wasm_path.display(), e));

    // Zero-bloat invariant: < 50 KB
    assert!(
        wasm_bytes.len() < 50 * 1024,
        "WASM binary size {} exceeds 50 KB limit",
        wasm_bytes.len()
    );

    let (broker, _tx) = InMemoryBroker::new(32);
    let broker = Arc::new(broker);
    use futures_util::StreamExt;
    let mut broker_stream = broker
        .subscribe(&["auth.magic_link_dispatched".to_string()], "test-group")
        .await
        .expect("Failed to subscribe to broker");

    let host = Arc::new(
        WasmHost::with_capabilities(5, None, None, None, Some(broker.clone()))
            .expect("Failed to initialize WasmHost"),
    );

    let cfg = FluxcellWasmConfig {
        timeout_ms: 5000,
        ..Default::default()
    };

    host.register_wasm_bytes("magic-link-ts", &wasm_bytes, cfg)
        .expect("Failed to register TypeScript WASM fluxcell");

    // 1. Verify reflected subscriptions
    let subs = host
        .get_fluxcell_subscriptions("magic-link-ts")
        .expect("Missing subscriptions");
    assert!(subs.contains(&"auth.magic_link".to_string()));
    assert!(subs.contains(&"mutation.requestmagiclink".to_string()));

    // 2. Verify reflected routes
    let routes = host
        .get_fluxcell_routes("magic-link-ts")
        .expect("Missing routes");
    assert!(routes.iter().any(|r| r.relative_path == "/verify" && r.method == "GET"));
    assert!(routes.iter().any(|r| r.relative_path == "/verify" && r.method == "POST"));
    assert!(routes.iter().any(|r| r.relative_path == "/status" && r.method == "GET"));

    // 3. HTTP: GET /status
    let (status, _, body) = host
        .invoke_http("magic-link-ts", "/status", "GET", vec![], vec![])
        .expect("Failed to invoke /status");
    assert_eq!(status, 200);
    let body_str = String::from_utf8(body).expect("Valid UTF-8");
    assert!(body_str.contains("\"magic_link_ts\""));

    // 4. HTTP: GET /verify without token -> 401 Unauthorized
    let (status, _, body) = host
        .invoke_http("magic-link-ts", "/verify", "GET", vec![], vec![])
        .expect("Failed to invoke /verify without token");
    assert_eq!(status, 401);
    let body_str = String::from_utf8(body).expect("Valid UTF-8");
    assert!(body_str.contains("INVALID_OR_EXPIRED_TOKEN"));

    // 5. HTTP: GET /verify?token=tok-query-123 -> 200 Verified
    let (status, _, body) = host
        .invoke_http("magic-link-ts", "/verify?token=tok-query-123", "GET", vec![], vec![])
        .expect("Failed to invoke /verify with query token");
    assert_eq!(status, 200);
    let body_str = String::from_utf8(body).expect("Valid UTF-8");
    assert!(body_str.contains("\"VERIFIED\""));
    assert!(body_str.contains("tok-query-123"));

    // 6. HTTP: POST /verify with JSON body -> 200 Verified
    let post_body = serde_json::json!({ "token": "tok-json-456" });
    let (status, _, body) = host
        .invoke_http(
            "magic-link-ts",
            "/verify",
            "POST",
            vec![("content-type".to_string(), "application/json".to_string())],
            serde_json::to_vec(&post_body).unwrap(),
        )
        .expect("Failed to invoke POST /verify");
    assert_eq!(status, 200);
    let body_str = String::from_utf8(body).expect("Valid UTF-8");
    assert!(body_str.contains("\"VERIFIED\""));
    assert!(body_str.contains("tok-json-456"));

    // 7. Event Dispatching & Outbound Broker Publish
    let event = serde_json::json!({
        "event_id": "evt-auth-789",
        "hlc": "1740000000000.0001",
        "topic": "mutation.requestmagiclink",
        "payload_json": "{\"email\":\"user@example.com\"}"
    });

    let res = host
        .invoke_event("magic-link-ts", &event)
        .expect("Failed to invoke event");
    assert_eq!(res.get("status").and_then(|v| v.as_str()), Some("ok"));
    assert_eq!(res.get("processed").and_then(|v| v.as_bool()), Some(true));

    // Verify outbound broker message dispatched by guest!
    let published_msg = broker_stream
        .next()
        .await
        .expect("Expected outbound broker message from guest publish()");
    assert_eq!(published_msg.topic, "auth.magic_link_dispatched");
    let pub_payload: serde_json::Value =
        serde_json::from_slice(&published_msg.payload).expect("Valid payload JSON");
    assert_eq!(pub_payload["event_id"], "evt-auth-789");
}
