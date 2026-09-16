# `@spectraflux/sdk`: Canonical AssemblyScript Guest SDK for SpectraFlux

[![npm version](https://img.shields.io/badge/npm-v0.1.0-cb3837.svg)](package.json)
[![License: MIT](https://img.shields.io/badge/License-MIT-black.svg)](LICENSE)
[![WASM Target](https://img.shields.io/badge/target-wasm32-cyan.svg)](https://webassembly.org/)
[![Size](https://img.shields.io/badge/size-%3C25%20KB-brightgreen.svg)](#sub-50-kb-wasm-invariant)

`@spectraflux/sdk` is the canonical AssemblyScript guest SDK and development toolchain for building ultra-lean, low-latency WebAssembly **Fluxcells** on **SpectraFlux**.

It provides strong typing, transaction-first host database integration, monotonic causality guards, deduplication filters, HTTP routing envelopes, and built-in ABI verification tools—all compiling to compact, sub-25 KB standalone WebAssembly binaries.

---

## Table of Contents

- [Architectural Tenets](#architectural-tenets)
- [Sub-50 KB WASM Invariant](#sub-50-kb-wasm-invariant)
- [Quick Start (Bun-First)](#quick-start-bun-first)
- [Project Layout](#project-layout)
- [Core Abstractions](#core-abstractions)
  - [The `Fluxcell` Class & `registerFluxcell`](#the-fluxcell-class--registerfluxcell)
  - [HTTP Routing & Responses (`HttpRequest`, `HttpResponse`)](#http-routing--responses-httprequest-httpresponse)
  - [Event Processing (`EventContext`, `EventVerdict`)](#event-processing-eventcontext-eventverdict)
  - [Host Database Integration (`Database`, `FluxTx`)](#host-database-integration-database-fluxtx)
  - [Causal Monotonicity (`CausalGuard`, `advance_db_watermark`)](#causal-monotonicity-causalguard-advance_db_watermark)
  - [In-Memory Deduplication (`DeduplicationBuffer`)](#in-memory-deduplication-deduplicationbuffer)
  - [Guest Telemetry (`TelemetryBuffer`)](#guest-telemetry-telemetrybuffer)
- [Bundled CLI Tooling](#bundled-cli-tooling)
- [Testing & Verification](#testing--verification)

---

## Architectural Tenets

1. **Ultra-Lean Footprint:** Fluxcells are designed for instantaneous edge cold-starts ($< 1\text{ ms}$) and sub-50 KB payloads.
2. **Zero Broker Bloat:** Direct telemetry rollups and monotonic high-water mark validation avoid chatter over the event broker.
3. **Transaction Safety:** Database operations require an explicit transaction (`FluxTx`), ensuring atomicity across host execution.
4. **Wasmtime ABI Parity:** Strict conformance with the SpectraFlux C-ABI memory conventions (high 32 bits = byte pointer, low 32 bits = byte length).

---

## Sub-50 KB WASM Invariant

In high-throughput distributed architectures, distribution overhead and compilation latency dominate cold-start times. While traditional runtimes bundle heavy runtimes or virtual machines (often 15–30 MB), `@spectraflux/sdk` uses AssemblyScript to produce micro-binaries:

* **Release Size:** Typically **$15 - 22\text{ KB}$** ($< 25\text{ KB}$), strictly within the enforced $< 50\text{ KB}$ boundary.
* **Instant Instantiation:** Near-instantaneous JIT compilation and pooling in Wasmtime.

---

## Quick Start (Bun-First)

### 1. Initialize Project

```bash
# Using the fluxcell CLI or bootstrap tool
bunx @spectraflux/sdk fluxcell-bootstrap my-cell
cd my-cell
```

Or install directly into an existing project:

```bash
bun add @spectraflux/sdk assemblyscript
# Or with npm: npm install @spectraflux/sdk assemblyscript
```

### 2. Write Your Fluxcell (`assembly/index.ts`)

```typescript
import {
  Fluxcell,
  FluxcellMetadata,
  HttpRequest,
  HttpResponse,
  RouteMeta,
  registerFluxcell,
} from "@spectraflux/sdk";

export class OrderStatusCell extends Fluxcell {
  metadata(): FluxcellMetadata {
    return new FluxcellMetadata("1.0.0", "Order Status Inquiry API");
  }

  routes(): RouteMeta[] {
    return [
      RouteMeta.get("/orders/status", "Check order processing status"),
    ];
  }

  handleHttp(req: HttpRequest): HttpResponse {
    if (req.path == "/orders/status") {
      return HttpResponse.json('{"status":"active","healthy":true}');
    }
    return HttpResponse.notFound("Endpoint not found");
  }
}

// Register the cell instance
registerFluxcell(new OrderStatusCell());

// Re-export C-ABI functions for the host
export {
  allocate,
  deallocate,
  get_metadata,
  get_subscriptions,
  get_routes,
  handle_http,
  handle_event,
} from "@spectraflux/sdk";
```

### 3. Build & Test

```bash
# Build release WASM binary
bun run build

# Run unit tests and ABI validation
bun test
```

---

## Project Layout

A typical TypeScript Fluxcell project contains:

```
my-fluxcell/
├── asconfig.json           # AssemblyScript compiler configuration
├── assembly/
│   └── index.ts            # Guest logic & C-ABI exports
├── bin/                    # Optional local build scripts
├── build/
│   ├── release.wasm        # Production compiled binary (< 25 KB)
│   └── release.wasm.sha256 # Cryptographic checksum
├── package.json
└── tsconfig.json
```

---

## Core Abstractions

### The `Fluxcell` Class & `registerFluxcell`

Subclass `Fluxcell` and override methods as needed:

```typescript
export class MyCell extends Fluxcell {
  metadata(): FluxcellMetadata {
    return new FluxcellMetadata("1.0.0", "My Description");
  }

  routes(): RouteMeta[] {
    return [RouteMeta.post("/mutate", "Execute transaction")];
  }

  subscriptions(): string[] {
    return ["orders.events"];
  }

  handleHttp(req: HttpRequest): HttpResponse { ... }
  handleEvent(event: EventContext): string { ... }
}

registerFluxcell(new MyCell());
```

### HTTP Routing & Responses (`HttpRequest`, `HttpResponse`)

#### `RouteMeta`
Declares mounted routes for the chassis radix tree:
```typescript
RouteMeta.get(path: string, description?: string)
RouteMeta.post(path: string, description?: string)
RouteMeta.put(path: string, description?: string)
RouteMeta.delete(path: string, description?: string)
```

#### `HttpRequest`
```typescript
req.path        // Clean path without query strings (e.g. "/orders")
req.method      // HTTP method (GET, POST, etc.)
req.body        // Raw string body
req.getHeader("authorization") // Case-insensitive header lookup
```

#### `HttpResponse`
```typescript
HttpResponse.ok("Operation succeeded");
HttpResponse.json('{"success":true}', 200);
HttpResponse.notFound("Resource not found");
HttpResponse.error("Internal processing failure", 500);
```

---

### Event Processing (`EventContext`, `EventVerdict`)

#### `EventContext`
Parses incoming broker payloads dispatched by the chassis:
```typescript
const event = EventContext.fromJson(rawJson);
const id = event.eventId;
const hlc = event.hlc;
const topic = event.topic;
const payload = event.payloadJson;
```

#### `EventVerdict`
Governs message acknowledgment:
* `EventVerdict.Ack` (0): Completed successfully.
* `EventVerdict.Nack` (1): Transient failure; broker retry.
* `EventVerdict.DeadLetter` (2): Fatal poison message; route to DLQ.

---

### Host Database Integration (`Database`, `FluxTx`)

Fluxcells communicate with host database connection pools using the host-injected `host_db` interface.

```typescript
import { Database, FluxTx } from "@spectraflux/sdk";

const db = Database.default(); // or Database.named("analytics")
const tx: FluxTx = db.beginTx();

try {
  // Execute mutating query
  const affected = tx.execute(
    "UPDATE users SET last_login = NOW() WHERE id = $1",
    '["user-123"]'
  );

  // Query rows as JSON string
  const rowsJson = tx.query(
    "SELECT id, balance FROM accounts WHERE user_id = $1",
    '["user-123"]'
  );

  tx.commit();
} catch (e) {
  tx.rollback();
  throw e;
}
```

---

### Causal Monotonicity (`CausalGuard`, `advance_db_watermark`)

Protects tables lacking high-precision timestamps from out-of-order broker redeliveries.

#### In-Memory Guard
```typescript
import { CausalGuard, CausalVerdict, is_fresh } from "@spectraflux/sdk";

const guard = new CausalGuard(1000);

const verdict = guard.evaluateAndAdvance("entity-1", event.hlc);
if (!is_fresh(verdict)) {
  // Discard duplicate or stale event
  return '{"status":"IGNORED_STALE_HLC"}';
}
```

#### Database Monotonic Watermark Table
```typescript
import { advance_db_watermark, is_fresh } from "@spectraflux/sdk";

const verdict = advance_db_watermark(tx, "orders", orderId, event.hlc);
if (!is_fresh(verdict)) {
  tx.rollback();
  return '{"status":"IGNORED_STALE_HLC"}';
}
```

---

### In-Memory Deduplication (`DeduplicationBuffer`)

Bounded LRU cache for deduplicating incoming events:

```typescript
import { DeduplicationBuffer } from "@spectraflux/sdk";

const dedup = new DeduplicationBuffer<string>(1000);

if (!dedup.checkAndUpdate(event.eventId, event.hlc, "processed")) {
  // Duplicate delivery; skip execution
  return '{"status":"ok"}';
}
```

---

### Guest Telemetry (`TelemetryBuffer`)

Direct out-of-band telemetry logging:

```typescript
import { TelemetryBuffer } from "@spectraflux/sdk";

const logs = new TelemetryBuffer(200);

logs.info("Starting batch computation");
logs.error("Upstream timeout encountered");

const report = logs.drainReportJson();
```

---

## Bundled CLI Tooling

The package ships with four operational CLI utilities in `bin/`:

### `fluxcell-build`
Compiles `assembly/index.ts` to optimized release WebAssembly and emits a companion `.sha256` checksum file.
```bash
bun run build
# or: node ./node_modules/@spectraflux/sdk/bin/build.mjs
```

### `fluxcell-verify`
Validates that the generated `.wasm` binary correctly exports all required C-ABI symbols and memory managers, and confirms binary size satisfies the $< 50\text{ KB}$ invariant.
```bash
bun run verify
```

### `fluxcell-deploy`
Deploys the compiled Fluxcell to a running SpectraGQL gateway or directly to the downstream Spectral Flux chassis:
```bash
# Direct local dev upload
node ./node_modules/@spectraflux/sdk/bin/deploy.mjs --dev-upload --mount /api/v1/orders

# Production remote deployment
node ./node_modules/@spectraflux/sdk/bin/deploy.mjs --artifact-url https://example.com/cell.wasm --mount /api/v1/orders
```

### `fluxcell-bootstrap`
Scaffolds a new AssemblyScript Fluxcell project from scratch.
```bash
bunx @spectraflux/sdk fluxcell-bootstrap my-new-cell
```

---

## Testing & Verification

Run the test suite using Bun:

```bash
bun test
```

The test runner:
1. Compiles AssemblyScript guest modules.
2. Instantiates them in the Wasmtime WebAssembly runtime engine.
3. Tests memory allocation (`allocate`/`deallocate`) and ABI packing.
4. Simulates HTTP dispatch and broker event pipelines.
5. Verifies binary size constraints ($< 50\text{ KB}$).
