# `fluxcell-sdk`: Canonical Rust Guest SDK for SpectraFlux

[![Crates.io](https://img.shields.io/badge/crates.io-v0.1.0-blue.svg)](Cargo.toml)
[![License: MIT](https://img.shields.io/badge/License-MIT-black.svg)](LICENSE)
[![WASM Target](https://img.shields.io/badge/target-wasm32--wasip1-cyan.svg)](https://webassembly.org/)

`fluxcell-sdk` is the canonical Rust guest SDK for developing ultra-high-performance WebAssembly **Fluxcells** running inside the **SpectraFlux** micro-VM runtime chassis.

It provides zero-unsafe guest abstractions, RAII-guaranteed database transactions with automatic rollback on failure, monotonic Hybrid Logical Clock (HLC) causality guards, fast in-memory deduplication buffers, ergonomic HTTP request/response builders, and low-overhead event processing envelopes.

---

## Table of Contents

- [Architectural Tenets](#architectural-tenets)
- [Quick Start](#quick-start)
- [Core Abstractions](#core-abstractions)
  - [The `Fluxcell` Trait](#the-fluxcell-trait)
  - [The `export_fluxcell!` Macro](#the-export_fluxcell-macro)
  - [HTTP Routing & Responses](#http-routing--responses)
  - [Event Stream Handlers & Verdicts](#event-stream-handlers--verdicts)
  - [RAII Host Database Client (`Database` & `Transaction`)](#raii-host-database-client-database--transaction)
  - [Causal Monotonicity & Watermark Engine (`CausalGuard`)](#causal-monotonicity--watermark-engine-causalguard)
  - [In-Memory Deduplication Buffer (`DeduplicationBuffer`)](#in-memory-deduplication-buffer-deduplicationbuffer)
  - [Direct Telemetry Buffer (`TelemetryBuffer`)](#direct-telemetry-buffer-telemetrybuffer)
- [Compilation & Toolchain Setup](#compilation--toolchain-setup)
- [Example Fluxcell Implementations](#example-fluxcell-implementations)
- [Security & Invariants](#security--invariants)

---

## Architectural Tenets

1. **Zero Unsafe Guest Logic:** All memory allocations, string packing, and low-level host ABI interactions are encapsulated safely by the SDK.
2. **Sub-Millisecond Wire Speed:** Zero redundant parsing, zero edge-thread pretty printing, compact JSON serialization, and sub-millisecond execution.
3. **RAII Transactional Integrity:** Transactions automatically trigger an asynchronous host `ROLLBACK` on `Drop` if not explicitly committed, eliminating lock leaks.
4. **Causal Monotonicity:** Protects target databases lacking high-precision clock support from out-of-order broker deliveries using monotonic HLC guards.

---

## Quick Start

Add `fluxcell-sdk` to your `Cargo.toml`:

```toml
[dependencies]
fluxcell-sdk = "0.1.0"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"

[lib]
crate-type = ["cdylib", "rlib"]
```

Import the ergonomic prelude:

```rust
use fluxcell_sdk::prelude::*;
```

Define your cell and export it:

```rust
use fluxcell_sdk::prelude::*;

pub struct EchoCell;

impl Fluxcell for EchoCell {
    fn metadata() -> FluxcellMetadata {
        FluxcellMetadata::new("1.0.0", "Minimal Echo Cell")
    }

    fn routes() -> Vec<RouteMeta> {
        vec![
            RouteMeta::get("/echo", "Echo request parameters"),
            RouteMeta::post("/echo", "Echo request body"),
        ]
    }

    fn handle_http(req: HttpRequest) -> HttpResponse {
        if req.path == "/echo" {
            return HttpResponse::json(&serde_json::json!({
                "method": req.method,
                "path": req.path,
                "body_len": req.body.len(),
            }));
        }
        HttpResponse::not_found("Endpoint not found")
    }
}

export_fluxcell!(EchoCell);
```

---

## Core Abstractions

### The `Fluxcell` Trait

The central trait implemented by every guest cell:

```rust
pub trait Fluxcell {
    /// Returns compile-time metadata (version, git hash, description).
    fn metadata() -> FluxcellMetadata;

    /// Returns dynamic HTTP endpoints exposed by this cell.
    fn routes() -> Vec<RouteMeta> { Vec::new() }

    /// Returns event stream topics subscribed to on the broker.
    fn subscriptions() -> Vec<String> { Vec::new() }

    /// Handles synchronous incoming HTTP requests.
    fn handle_http(_req: HttpRequest) -> HttpResponse {
        HttpResponse::not_found("No HTTP routes handled")
    }

    /// Handles asynchronous incoming broker events.
    fn handle_event(_event: EventContext) -> serde_json::Value {
        serde_json::json!({ "status": "ok" })
    }
}
```

### The `export_fluxcell!` Macro

Generates the low-level `extern "C"` functions required by the SpectraFlux Wasmtime host:
* `allocate(size: usize) -> *mut u8`
* `deallocate(ptr: *mut u8, size: usize)`
* `get_metadata() -> u64`
* `get_subscriptions() -> u64`
* `get_routes() -> u64`
* `handle_http(ptr: u32, len: u32) -> u64`
* `handle_event(ptr: u32, len: u32) -> u64`

### HTTP Routing & Responses

#### `HttpRequest`
Provides zero-copy decoding of path, method, headers, query parameters, and body bytes:
```rust
let query_val = req.query("user_id");
let auth_header = req.header("authorization");
let body: MyPayload = req.json()?;
```

#### `HttpResponse`
Fluent builders for common status envelopes:
```rust
HttpResponse::ok();
HttpResponse::json(&my_data);
HttpResponse::json_with_status(201, &created_resource);
HttpResponse::bad_request("Invalid UUIDv7 identifier");
HttpResponse::not_found("Entity does not exist");
HttpResponse::error("Internal execution failed");
```

### Event Stream Handlers & Verdicts

#### `EventContext`
Encapsulates an incoming broker message:
```rust
let event_id: &str = &event.event_id;
let topic: &str = &event.topic;
let hlc: &str = &event.hlc;
let payload: MyEvent = event.json()?;
```

#### `EventVerdict`
Standardized outcomes returned to the chassis to govern message acknowledgments:
* `EventVerdict::Ack`: Event successfully processed. Message acknowledged (`ACK`).
* `EventVerdict::Nack(err)`: Temporary failure. Retried according to broker policy (`NACK`).
* `EventVerdict::DeadLetter(err)`: Permanent fatal failure. Routed directly to DLQ.
* `EventVerdict::IgnoredStaleHlc`: Discarded due to causal staleness; silently acknowledged.

```rust
EventVerdict::Ack.to_json()
```

---

### RAII Host Database Client (`Database` & `Transaction`)

SpectraFlux provides pooled host database connections to guest cells via the `host_db` import interface. All operations require an isolated transaction.

```rust
use fluxcell_sdk::prelude::*;

let db = Database::default(); // or Database::open("analytics")
let mut tx = db.begin_tx().map_err(|e| HttpResponse::error(e))?;

// Execute mutating SQL with parameterized queries
let affected = tx.execute(
    "UPDATE accounts SET balance = balance - $1 WHERE id = $2",
    &serde_json::json!([500, "acct-1234"]),
)?;

// Query typed rows
#[derive(serde::Deserialize)]
struct Account { id: String, balance: i64 }

let accounts: Vec<Account> = tx.query_as(
    "SELECT id, balance FROM accounts WHERE id = $1",
    &serde_json::json!(["acct-1234"]),
)?;

// Explicit commit
tx.commit()?;
```

#### RAII Auto-Rollback Guarantee
If `tx` goes out of scope without calling `.commit()`, the `Drop` implementation automatically issues an asynchronous `ROLLBACK` on the host, ensuring no table locks are leaked even during unhandled exceptions or early returns (`?`).

---

### 80/20 Durable Step Checkpoints (`step`)

Prevents re-executing external non-idempotent side effects during event retries by persisting intermediate results to the host storage:

```rust
use fluxcell_sdk::prelude::*;

// Executes closure ONLY ONCE per command UUIDv7; returns cached result on retries
let charge_json = step(&event.event_id, "stripe_charge", || {
    // Perform external HTTP call or mutation
    Ok(serde_json::json!({ "charge_id": "ch_987", "status": "succeeded" }).to_string())
})?;
```

---

### Causal Monotonicity & Watermark Engine (`CausalGuard`)

When downstream databases lack native HLC support and only maintain low-precision timestamps (like PostgreSQL `updated_at TIMESTAMPTZ`), distributed out-of-order message delivery can cause state overwrites.

#### In-Memory Guard
```rust
use fluxcell_sdk::prelude::*;

static GUARD: std::sync::LazyLock<CausalGuard<String>> = 
    std::sync::LazyLock::new(|| CausalGuard::new(10_000));

let verdict = GUARD.evaluate_and_advance(&user_id, &event.hlc);
if verdict.should_discard() {
    return EventVerdict::IgnoredStaleHlc.to_json();
}
```

#### Database Monotonic High-Water Mark
```rust
// Atomically checks and updates high-water mark inside host transaction
let verdict = advance_db_watermark(&mut tx, "users", &user_id, &event.hlc)?;
if verdict.should_discard() {
    return Ok(EventVerdict::IgnoredStaleHlc);
}
```

---

### In-Memory Deduplication Buffer (`DeduplicationBuffer`)

Thread-safe, bounded LRU-style deduplication gate:

```rust
use fluxcell_sdk::prelude::*;

static DEDUP: std::sync::LazyLock<DeduplicationBuffer<String, u64>> = 
    std::sync::LazyLock::new(|| DeduplicationBuffer::new(5_000));

if !DEDUP.check_and_update(&event.event_id, &event.hlc, current_timestamp) {
    // Duplicate or stale delivery detected; discard
    return EventVerdict::IgnoredStaleHlc.to_json();
}
```

---

### Direct Telemetry Buffer (`TelemetryBuffer`)

Maintains an in-memory ring buffer of guest diagnostic logs and exports batched reports without spamming the event broker:

```rust
use fluxcell_sdk::prelude::*;

static TELEMETRY: std::sync::LazyLock<TelemetryBuffer> = 
    std::sync::LazyLock::new(|| TelemetryBuffer::new(200));

TELEMETRY.info("Order processed successfully");
TELEMETRY.error("Payment gateway timeout");

let report_json = TELEMETRY.drain_report_json();
```

---

## Compilation & Toolchain Setup

Fluxcells target the WebAssembly System Interface (`wasm32-wasip1`).

### 1. Add WASM Target
```bash
rustup target add wasm32-wasip1
```

### 2. Build Release Artifact
```bash
cargo build --target wasm32-wasip1 --release
```

The resulting binary will be located at:
```
target/wasm32-wasip1/release/<crate_name>.wasm
```

---

## Example Fluxcell Implementations

### Full Transactional Event Consumer
```rust
use fluxcell_sdk::prelude::*;
use serde::Deserialize;

#[derive(Deserialize)]
struct OrderCreated {
    order_id: String,
    customer_id: String,
    amount_cents: i64,
}

pub struct OrderProcessor;

impl Fluxcell for OrderProcessor {
    fn metadata() -> FluxcellMetadata {
        FluxcellMetadata::new("1.0.0", "Monotonic Order Processor")
    }

    fn subscriptions() -> Vec<String> {
        vec!["orders.created".to_string()]
    }

    fn handle_event(event: EventContext) -> serde_json::Value {
        let order: OrderCreated = match event.json() {
            Ok(o) => o,
            Err(e) => return EventVerdict::DeadLetter(e.to_string()).to_json(),
        };

        let db = Database::default();
        let mut tx = match db.begin_tx() {
            Ok(tx) => tx,
            Err(e) => return EventVerdict::Nack(e).to_json(),
        };

        // Enforce causal ordering
        match advance_db_watermark(&mut tx, "orders", &order.order_id, &event.hlc) {
            Ok(CausalVerdict::Fresh) => {}
            Ok(_) => return EventVerdict::IgnoredStaleHlc.to_json(),
            Err(e) => return EventVerdict::Nack(e).to_json(),
        }

        // Apply mutation
        let res = tx.execute(
            "INSERT INTO orders (id, customer_id, amount_cents) VALUES ($1, $2, $3)",
            &serde_json::json!([order.order_id, order.customer_id, order.amount_cents]),
        );

        if let Err(e) = res {
            return EventVerdict::Nack(e).to_json();
        }

        if let Err(e) = tx.commit() {
            return EventVerdict::Nack(e).to_json();
        }

        EventVerdict::Ack.to_json()
    }
}

export_fluxcell!(OrderProcessor);
```

---

## Security & Invariants

* **Hermetic Sandbox:** Fluxcells cannot access arbitrary host files, network sockets, or environment variables directly.
* **Deterministic Fuel & Epochs:** Infinite loops and blocking operations are terminated safely by the host engine's epoch watchdog.
* **Immutable Checksums:** Production deployments enforce strict SHA-256 cryptographic verification before staging or activation.
