use fluxcell_sdk::causal::{evaluate_causality, CausalGuard, CausalVerdict};
use fluxcell_sdk::dedup::DeduplicationBuffer;
use fluxcell_sdk::event::{EventContext, EventVerdict};
use fluxcell_sdk::hlc::{is_stale, parse_hlc};
use fluxcell_sdk::http::{HttpRequest, HttpResponse};
use serde_json::json;

#[test]
fn test_event_context_parsing_edge_cases() {
    // 1. Valid JSON payload with full metadata
    let payload = json!({
        "id": "evt-12345",
        "topic": "mutation.coeval.recordvote",
        "hlc": "1726000000000-0001",
        "data": { "voter": "alice", "vote": 1 }
    });
    let bytes = serde_json::to_vec(&payload).unwrap();
    let ctx = EventContext::from_bytes(&bytes);

    assert_eq!(ctx.event_id, "evt-12345");
    assert_eq!(ctx.topic, "mutation.coeval.recordvote");
    assert_eq!(ctx.hlc, "1726000000000-0001");

    #[derive(serde::Deserialize)]
    struct VoteData {
        voter: String,
        vote: i32,
    }
    #[derive(serde::Deserialize)]
    struct Wrapper {
        data: VoteData,
    }
    let parsed: Wrapper = ctx.json().unwrap();
    assert_eq!(parsed.data.voter, "alice");
    assert_eq!(parsed.data.vote, 1);

    // 2. Non-JSON raw binary bytes
    let raw_bytes = b"\x00\xFF\xAA\xBB\xCC";
    let raw_ctx = EventContext::from_bytes(raw_bytes);
    assert_eq!(raw_ctx.event_id, "");
    assert_eq!(raw_ctx.topic, "");
    assert_eq!(raw_ctx.hlc, "");
    assert_eq!(raw_ctx.payload, raw_bytes);
    assert!(raw_ctx.json::<Wrapper>().is_err());

    // 3. Alternative field name: event_id instead of id
    let alt_json = json!({
        "event_id": "alt-999",
        "topic": "audit.logs"
    });
    let alt_ctx = EventContext::from_bytes(&serde_json::to_vec(&alt_json).unwrap());
    assert_eq!(alt_ctx.event_id, "alt-999");
    assert_eq!(alt_ctx.topic, "audit.logs");
}

#[test]
fn test_event_verdicts_json_parity() {
    assert_eq!(EventVerdict::Ack.to_json(), json!({ "status": "ok" }));
    assert_eq!(
        EventVerdict::Nack("connection timed out".to_string()).to_json(),
        json!({ "status": "nack", "error": "connection timed out" })
    );
    assert_eq!(
        EventVerdict::DeadLetter("poison pill payload".to_string()).to_json(),
        json!({ "status": "dead_letter", "error": "poison pill payload" })
    );
    assert_eq!(
        EventVerdict::IgnoredStaleHlc.to_json(),
        json!({ "status": "IGNORED_STALE_HLC" })
    );
}

#[test]
fn test_http_request_and_response_builders() {
    let req_json = json!({
        "path": "/api/v1/votes",
        "method": "POST",
        "headers": [["content-type", "application/json"], ["authorization", "Bearer secret"]],
        "body": "{\"choice\":\"A\"}"
    });

    let req = HttpRequest::from_json_val(&req_json);
    assert_eq!(req.path, "/api/v1/votes");
    assert_eq!(req.method, "POST");
    assert_eq!(req.header("content-type"), Some("application/json"));
    assert_eq!(req.header("authorization"), Some("Bearer secret"));
    assert_eq!(req.header("non-existent"), None);

    // Response builders
    let ok_resp = HttpResponse::ok();
    assert_eq!(ok_resp.status, 200);
    assert_eq!(ok_resp.body, b"{\"status\":\"ok\"}");

    let json_resp = HttpResponse::json(&json!({ "acknowledged": true }));
    assert_eq!(json_resp.status, 200);
    assert!(json_resp.headers.iter().any(|(k, v)| k.eq_ignore_ascii_case("content-type") && v == "application/json"));

    let err_resp = HttpResponse::error("internal server error");
    assert_eq!(err_resp.status, 500);

    let not_found = HttpResponse::not_found("not found");
    assert_eq!(not_found.status, 404);
}

#[test]
fn test_causal_guard_hlc_monotonicity() {
    let guard: CausalGuard<String> = CausalGuard::new(10);
    let key = "orders".to_string();

    // First event at HLC 100.1 -> Fresh
    assert_eq!(guard.evaluate_and_advance(&key, "100.1"), CausalVerdict::Fresh);

    // Duplicate event at HLC 100.1 -> Duplicate
    assert_eq!(guard.evaluate_and_advance(&key, "100.1"), CausalVerdict::Duplicate);

    // Stale event at HLC 90.1 -> Stale
    assert_eq!(guard.evaluate_and_advance(&key, "90.1"), CausalVerdict::Stale);

    // Newer event at HLC 100.2 -> Fresh
    assert_eq!(guard.evaluate_and_advance(&key, "100.2"), CausalVerdict::Fresh);

    // Pure evaluation function
    assert_eq!(evaluate_causality("100.3", "100.2"), CausalVerdict::Fresh);
    assert_eq!(evaluate_causality("100.2", "100.2"), CausalVerdict::Duplicate);
    assert_eq!(evaluate_causality("100.1", "100.2"), CausalVerdict::Stale);
}

#[test]
fn test_hlc_parsing_and_staleness() {
    let parsed = parse_hlc("1726000000000.0042").unwrap();
    assert_eq!(parsed.0, 1726000000000);
    assert_eq!(parsed.1, 42);

    assert!(is_stale("100.1", "100.2"));
    assert!(is_stale("99.99", "100.0"));
    assert!(!is_stale("100.2", "100.1"));
    assert!(is_stale("100.1", "100.1")); // Equal is stale/duplicate
}

#[test]
fn test_deduplication_buffer_monotonic_filtering() {
    let dedup: DeduplicationBuffer<String, ()> = DeduplicationBuffer::new(4);
    let key = "msg-1".to_string();

    // Initial message
    assert!(dedup.check_and_update(&key, "100.1", ()));

    // Stale message rejected
    assert!(!dedup.check_and_update(&key, "90.1", ()));

    // Duplicate message rejected
    assert!(!dedup.check_and_update(&key, "100.1", ()));

    // Fresh message accepted
    assert!(dedup.check_and_update(&key, "100.2", ()));
}
