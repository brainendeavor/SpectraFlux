<p align="center">
  <img src="assets/spectraflux_logo.svg" alt="SpectraFlux Logo" width="260" />
</p>

<p align="center">
  <strong>The High-Velocity Streaming Execution Chassis &amp; WebAssembly Runtime</strong><br>
  <em>Executing sandboxed event-driven Fluxcells with sub-millisecond dispatch and hardware isolation.</em>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-blue.svg" alt="License: MIT" /></a>
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/rust-2024%20edition-orange.svg" alt="Rust Edition" /></a>
  <a href="https://bytecodealliance.org/"><img src="https://img.shields.io/badge/wasm-wasmtime%20engine-black.svg" alt="Wasmtime" /></a>
</p>

---

## 1. Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────────────┐
│ SpectraGQL (API Gateway Appliance)                                          │
│   ├── Mode A: Reverse-Proxies Reads                                         │
│   └── Mode B: Emits UUIDv7/HLC Command Receipts & Dispatches to Broker       │
└──────────────────────────────────────┬──────────────────────────────────────┘
                                       │ (NATS / Kafka / Redis Streams)
                                       ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│ SpectraFlux (Downstream Execution Chassis & Worker Appliance)               │
│                                                                             │
│  ┌───────────────────────────────────────────────────────────────────────┐  │
│  │ Wasmtime Execution Engine                                             │  │
│  │   • Epoch Interruption & Preemptive Timeouts                          │  │
│  │   • Circuit Breakers & Adaptive Backoff                               │  │
│  │   • Zero-Copy Multi-Instance Instance Pooling                         │  │
│  └───────────────────────────────────┬───────────────────────────────────┘  │
│                                      ▼                                      │
│  ┌───────────────────────────────────────────────────────────────────────┐  │
│  │ Host Capabilities Bridge (WIT Contract)                               │  │
│  │   • host_db: PostgreSQL Connection Pooling & Embedded dbmate          │  │
│  │   • kv_store / host_redis: Decoupled Storage (internal vs fluxcell)   │  │
│  │   • host_broker: Outbound topic dispatching                           │  │
│  └───────────────────────────────────┬───────────────────────────────────┘  │
│                                      ▼                                      │
│  ┌───────────────────────────────────────────────────────────────────────┐  │
│  │ Dynamic WebAssembly Fluxcells (Sandboxed Micro-Units)                 │  │
│  │   • magic-link, webhook, custom guest cells                           │  │
│  │   • Authoring via fluxcell-sdk (Rust) and @spectraflux/sdk (TS)       │  │
│  └───────────────────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## 2. Repository Layout

* **[`wit/fluxcell.wit`](wit/fluxcell.wit)**: Canonical WebAssembly Interface Type (WIT) specification defining host capabilities and guest entrypoints.
* **[`runtime/`](runtime/)**: The `spectra-flux` host runtime execution engine and HTTP router (:8081).
* **[`sdks/`](sdks/)**: Official Guest SDKs for writing WebAssembly Fluxcells:
  * **[`sdks/rust/`](sdks/rust/)**: Canonical Rust guest SDK (`fluxcell-sdk`).
  * **[`sdks/typescript/`](sdks/typescript/)**: Official TypeScript / JavaScript SDK (`@spectraflux/sdk`).
* **[`tools/`](tools/)**: Developer tooling:
  * **[`tools/cli/`](tools/cli/)**: Developer CLI (`fluxcell-cli`).
* **[`seeds/`](seeds/)**: Canonical barebones starter seeds for cloning or scaffolding:
  * **[`seeds/rust/`](seeds/rust/)**: Barebones Rust starter seed (`fluxcell-seed-rs`).
  * **[`seeds/typescript/`](seeds/typescript/)**: Barebones TypeScript starter seed (`fluxcell-seed-ts`).
* **[`examples/`](examples/)**: Feature-complete sample applications:
  * **[`examples/rust/mailer/`](examples/rust/mailer/)**: Asynchronous invoice mailer and webhook dispatcher.
  * **[`examples/typescript/`](examples/typescript/)**: TypeScript sample applications.
* **[`fluxcells/`](fluxcells/)**: Pre-built runtime cells:
  * **[`fluxcells/rust/`](fluxcells/rust/)**: System reference cells (`magic-link`, `webhook`).
  * **[`fluxcells/typescript/`](fluxcells/typescript/)**: System TypeScript cells.

---

## Core Documentation

- **[Downstream Authorization (`authz`)](docs/authz.md)**: Cooperative dual-pillar security with SpectraGQL, consuming verified identity claims with zero cryptographic overhead, fine-grained domain authorization, and PostgreSQL `auth_user_roles`.
- **[Chassis Runtime Architecture](docs/architecture.md)**: Wasmtime engine pooling, epoch interruption watchdog, and host capability imports (`host_db`, `kv_store`, `host_broker`).
- **[Storage Architecture & Decoupled Tiers](docs/authz-and-storage.md)**: Decoupled storage tiers (`internal-storage` vs `fluxcell-storage`), unified `kevy://` URLs, PostgreSQL schema migrations via embedded `dbmate`, and `host_redis` ABI.
- **[Deployer & Security Governance](docs/deployer-and-governance.md)**: Zero-compiler container invariant (< 25 MB), dynamic staging API, SSRF shield, and emergency lockdown.
- **[Durable Steps & Resilience](docs/durable-steps-and-resilience.md)**: 80/20 durable step memoization (`ctx.step`), exponential backoff, DLQ routing, and monotonic HLC causality guards.
- **[Embedded Admin Console & Observability](docs/admin-dashboard.md)**: Real-time execution profiles (p50/p95/p99 latency, fuel, memory), 1GB allotment, sanitized config inspector, and live logs.

---

## 3. Quick Start: Developing a Fluxcell in Rust

Scaffold instantly via the CLI:
```bash
fluxcell new my-rust-cell --lang rust
```

Or add the SDK to your `Cargo.toml`:

```toml
[dependencies]
fluxcell-sdk = { path = "path/to/SpectraFlux/sdks/rust" }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
```

Write your Fluxcell with zero unsafe code:

```rust
use fluxcell_sdk::prelude::*;

struct MyCell;

impl Fluxcell for MyCell {
    fn metadata() -> FluxcellMetadata {
        FluxcellMetadata::new("0.1.0", "My Custom Fluxcell")
    }

    fn routes() -> Vec<RouteMeta> {
        vec![RouteMeta::post("/mutate", "Execute transactional mutation")]
    }

    fn handle_http(req: HttpRequest) -> HttpResponse {
        if req.path == "/mutate" {
            // 1. Acquire transaction from host database pool
            let mut tx = match Database::default().begin_tx() {
                Ok(t) => t,
                Err(e) => return HttpResponse::error(e),
            };

            // 2. Execute SQL inside transaction
            if let Err(e) = tx.execute("INSERT INTO items (name) VALUES ($1)", &serde_json::json!(["Widget"])) {
                return HttpResponse::error(e);
            }

            // 3. Commit (uncommitted transactions auto-rollback on Drop)
            if let Err(e) = tx.commit() {
                return HttpResponse::error(e);
            }

            return HttpResponse::json(&serde_json::json!({ "status": "COMMITTED" }));
        }

        HttpResponse::not_found("Endpoint not found")
    }
}

export_fluxcell!(MyCell);
```

---

## 4. Quick Start: Developing a Fluxcell in TypeScript

Scaffold instantly via the CLI:
```bash
fluxcell new my-ts-cell --lang ts
```

Or use the TypeScript SDK directly:
```bash
cd seeds/typescript
bun install
bun run build
bun test
```

Write your Fluxcell in AssemblyScript / TypeScript:

```typescript
import {
  Fluxcell,
  FluxcellMetadata,
  HttpRequest,
  HttpResponse,
  RouteMeta,
  registerFluxcell,
} from "@spectraflux/sdk/assembly/index";

export class MyTsCell extends Fluxcell {
  metadata(): FluxcellMetadata {
    return new FluxcellMetadata("0.1.0", "My TypeScript Fluxcell");
  }

  routes(): RouteMeta[] {
    return [
      RouteMeta.get("/health", "Health check endpoint"),
      RouteMeta.post("/mutate", "Transactional mutation"),
    ];
  }

  handleHttp(req: HttpRequest): HttpResponse {
    if (req.path == "/health") {
      return HttpResponse.json('{"status":"healthy"}');
    }
    return HttpResponse.notFound("Endpoint not found");
  }
}

registerFluxcell(new MyTsCell());

// Re-export C-ABI functions for the SpectraFlux Wasmtime host
export {
  allocate,
  deallocate,
  get_metadata,
  get_subscriptions,
  get_routes,
  handle_http,
  handle_event,
} from "@spectraflux/sdk/assembly/index";
```

---

## 5. 80/20 Durable Step Checkpoints (`ctx.step`)

SpectraFlux provides built-in 80/20 durable step memoization. If a worker fails midway through an event and retries, previously completed external side effects (e.g. Stripe payments, SMS delivery, third-party reservation APIs) are skipped by returning the cached output from the chassis host storage:

```typescript
const payment = ctx.step("stripe_charge", (): string => {
  // Executed EXACTLY ONCE per command UUIDv7
  return JSON.stringify({ chargeId: "ch_987", status: "success" });
});
```
* **Host Storage Key:** `chk:<command_uuidv7>:<step_name>` with configurable TTL (default: 86,400s).

---

## 6. Level 1 Resilience: Exponential Backoff & DLQ Routing

Transient failures automatically trigger exponential backoff with randomized jitter. If an event exceeds maximum retry attempts, SpectraFlux routes the poison message to a Dead-Letter Queue (`dlq.<topic_name>`) and cleanly ACKs the broker:

```toml
[resilience]
max_retries = 3
initial_backoff_ms = 100
max_backoff_ms = 5000
backoff_factor = 2.0
jitter = true
dlq_topic_prefix = "dlq."
```

---

## 7. Embedded Admin Console & Execution Profiles (:8081)

SpectraFlux includes an embedded observability and control plane at `http://localhost:8081/admin`:
* **Real-time Latency Profiles:** Continuous rolling percentile latency graphs (p50, p95, p99), memory pages, and execution fuel metrics.
* **1GB Profile Allotment:** High-capacity ring buffer for fine-grained performance sampling without disk exhaustion.
* **Sanitized Configuration Viewer:** Live inspection of runtime settings, broker subjects, and database connection pools with sensitive credentials cryptographically masked.
* **Live Logs & Domain Traces:** Zero-overhead streaming of chassis host logs and guest telemetry rollups.

---

## 8. 12-Factor Database Configuration

PostgreSQL database connections can be configured dynamically at runtime via standard environment variables:
```bash
# Standard 12-factor connection string:
export DATABASE_URL="postgres://user:password@db.host:5432/mydb?sslmode=require"

# Or chassis-specific variable:
export FLUX__DATABASE__URL="postgres://user:password@db.host:5432/mydb"
```

---

## 9. Building and Testing

```bash
# Run Rust tests across entire workspace
cargo test --workspace

# Build the chassis binary
cargo build --package spectra-flux --release

# Build & test TypeScript SDK and Seed
cd seeds/typescript
bun run build
bun test
```

