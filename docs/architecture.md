# SpectraFlux Runtime Architecture

**SpectraFlux** is an ultra-high-velocity WebAssembly (WASM) execution chassis and micro-worker appliance designed to run downstream write-model handlers, event-driven projections, and reactive workflows.

---

## 1. Engine Core & Wasmtime Integration

SpectraFlux uses Bytecode Alliance's **Wasmtime** as its core execution engine:

```
┌─────────────────────────────────────────────────────────────────────────────┐
│ SpectraFlux Runtime Chassis (:8081)                                         │
│                                                                             │
│  ┌───────────────────────────────────────────────────────────────────────┐  │
│  │ Wasmtime Execution Core                                               │  │
│  │   • Instance Pooling: Pre-allocated memory slots (< 50µs acquisition) │  │
│  │   • Epoch Interruption: Preemptive execution watchdog                 │  │
│  │   • Hardware Isolation: Zero-memory-bleed between fluxcell instances  │  │
│  └───────────────────────────────────┬───────────────────────────────────┘  │
│                                      │                                      │
│                                      ▼                                      │
│  ┌───────────────────────────────────────────────────────────────────────┐  │
│  │ Host Capabilities Provider (WIT Imports)                              │  │
│  │   • host_db: Pooled PostgreSQL transactions (deadpool-postgres)       │  │
│  │   • kv_store: In-memory Kevy / Redis-compatible KV                    │  │
│  │   • host_broker: Outbound topic dispatching (NATS / Kafka)            │  │
│  └───────────────────────────────────┬───────────────────────────────────┘  │
│                                      │                                      │
│                                      ▼                                      │
│  ┌───────────────────────────────────────────────────────────────────────┐  │
│  │ Dynamic Radix Router & Broker Multi-Subject Consumer                  │  │
│  │   • HTTP Ingress: Matchit radix tree (:8081/<mount_path>/*)           │  │
│  │   • Broker Ingress: Multi-subject wildcard multiplexing (orders.*)    │  │
│  └───────────────────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Instance Pooling Allocator
Rather than repeatedly compiling or allocating new linear memory on each invocation, SpectraFlux configures Wasmtime's **Pooling Allocator**:
- Pre-allocates fixed memory chunks for guest instances.
- Re-zeros memory in microseconds upon release.
- Yields cold-start latencies below 1 millisecond.

### Epoch Interruption Watchdog
To prevent misbehaved or malicious guest code from running infinite loops or hanging threads:
- The chassis runs a background epoch tick timer.
- Guest execution is granted an epoch budget (default: 5,000 ms).
- When a guest exhausts its epoch budget, Wasmtime triggers a preemptive trap, releasing host resources cleanly.

---

## 2. WebAssembly Interface (WIT) & Memory Contract

Fluxcells interact with the host via canonical WebAssembly Interface Type (`wit/fluxcell.wit`) specifications and C-ABI export conventions.

### Memory Marshalling
All cross-boundary strings and JSON payloads follow the 64-bit packed ABI representation:
- **Upper 32 bits:** Pointer address in guest WebAssembly linear memory (`ptr`).
- **Lower 32 bits:** Byte length of the buffer (`len`).

```rust
pub fn pack_ptr_len(ptr: u32, len: u32) -> u64 {
    ((ptr as u64) << 32) | (len as u64)
}

pub fn unpack_ptr_len(packed: u64) -> (u32, u32) {
    ((packed >> 32) as u32, packed as u32)
}
```

### Guest Exports
Every Fluxcell exports the following C-ABI entrypoints:
1. `allocate(size: usize) -> *mut u8`: Allocates linear guest memory for host writes.
2. `deallocate(ptr: *mut u8, size: usize)`: Frees previously allocated memory.
3. `get_metadata() -> u64`: Returns JSON metadata (name, version, description).
4. `get_subscriptions() -> u64`: Returns array of broker event subjects/topics.
5. `get_routes() -> u64`: Returns mounted HTTP routes and supported HTTP methods.
6. `handle_http(ptr: u32, len: u32) -> u64`: Dispatches synchronous HTTP request.
7. `handle_event(ptr: u32, len: u32) -> u64`: Dispatches asynchronous event payload.

---

## 3. Host Capabilities

Fluxcells execute in a strict sandbox without direct access to sockets, filesystem, or host threads. All I/O occurs via imported host capabilities:

### 1. `host_db` (Database Access)
- **Engine:** `deadpool-postgres` connection pooling.
- **Transactional Atomicity:** All database operations require an explicit transaction (`begin_tx`).
- **RAII Auto-Rollback:** If a transaction goes out of scope or errors without an explicit `.commit()`, the host immediately issues an asynchronous `ROLLBACK`, guaranteeing no table locks are held.
- **12-Factor Configuration:** Dynamically resolved from `DATABASE_URL` or `FLUX__DATABASE__URL`.

### 2. `kv_store` (Key-Value Storage & Checkpoints)
- **Engine:** In-process embedded **Kevy** (Redis-compatible storage engine) or external Redis/Valkey.
- **Usage:** Durable step checkpoints (`chk:<cmd_id>:<step_name>`), deduplication rings, and high-water mark caches.

### 3. `host_broker` (Event Dispatch)
- Enables guests to publish new events or domain notifications to outbound broker topics (e.g. `orders.completed`, `notifications.email`).

---

## 4. Multi-Subject Broker Multiplexing

SpectraFlux multiplexes event processing across multiple subscribed topics:
- Guests declare wildcard subscriptions (e.g., `spectra.events.orders.>`, `payments.*`).
- The chassis broker runner provisions consumer streams and dynamically routes incoming messages to the designated guest cells based on matching topic filters.
- Supports both NATS JetStream and Apache Kafka / Redpanda.
