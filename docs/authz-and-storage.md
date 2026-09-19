# Authorization, Postgres Migrations & Decoupled Storage Tiers

SpectraFlux provides a high-performance execution runtime for WebAssembly fluxcells, downstream write-model handlers, and reactive CQRS workflows.

To prevent memory starvation, cache collisions, and operational coupling, SpectraFlux decouples internal framework caching from guest application storage, standardizes on PostgreSQL for durable relational role storage, and exposes host-mediated Redis capabilities.

> [!TIP]
> For the end-to-end authorization architecture, caller identity consumption from SpectraGQL, and domain permission patterns in fluxcells, see the **[Downstream Authorization Guide](authz.md)**.

---

## 1. Decoupled Storage Tiers Architecture

Storage in `SpectraFlux` is bifurcated into two distinct, isolated tiers configured via standardized URLs:

```
                              SpectraFlux Chassis
                                      │
                 ┌────────────────────┴────────────────────┐
                 │                                         │
                 ▼                                         ▼
       [internal-storage]                        [fluxcell-storage]
       • Saga Step Checkpoints                   • Guest Fluxcell KV Store (`kv_store`)
       • Magic-Link OTPs & Tokens                • Guest Redis Operations (`host_redis`)
       • Framework Session Cache                 • Guest State & Application Caching
       • Domain Operation Traces                 • Default: `kevy:///data/fluxcell-storage.kevy`
       • Default: `kevy://embedded`              • Fallback: External Redis/Valkey Cluster
```

### A. Internal Storage (`internal-storage`)
* **Role:** Internal framework persistence only.
* **Workloads:**
  * **Saga Checkpointing:** Memoizing step results (`ctx.step`) during multi-phase distributed workflows to enable 80/20 durable retries without duplicating side effects.
  * **Authentication & Magic Links:** Storing short-lived OTP tokens (`magic_token:<clean_token>`) and caching authenticated user session claims.
  * **Domain Traces:** Storing recent end-to-end operation execution traces for admin inspection.
* **Default:** `kevy://embedded` (fast, in-process, zero external dependencies).

### B. Fluxcell Storage (`fluxcell-storage`)
* **Role:** Dedicated guest application storage and state.
* **Workloads:**
  * Application-level caching.
  * In-process or external Redis protocol commands (`host_redis::execute`).
  * Guest KV store (`kv_store::get`, `kv_store::set`, `kv_store::delete`).
* **Default:** `kevy:///data/fluxcell-storage.kevy` (file-backed Kevy store writing to `/data/` volume).

---

## 2. Unified Storage URL Scheme

Both storage tiers are configured using a unified URL scheme:

| Scheme | Description | Example |
| :--- | :--- | :--- |
| `kevy://embedded` | In-memory Kevy key-value store (ephemeral). | `url = "kevy://embedded"` |
| `kevy://memory` | Explicit in-memory Kevy store (alias for embedded). | `url = "kevy://memory"` |
| `kevy:///path` | Persistent, file-backed Kevy store on disk. | `url = "kevy:///data/fluxcell-storage.kevy"` |
| `redis://...` | External Redis cluster or standalone instance. | `url = "redis://redis.internal:6379"` |
| `rediss://...` | TLS-encrypted external Redis instance. | `url = "rediss://user:pass@host:6379"` |
| `valkey://...` | Normalized to Redis protocol for Valkey clusters. | `url = "valkey://10.0.0.5:6379"` |

### Container Volume Persistence & Local Dev Fallback
In production and Docker containers, `fluxcell-storage` writes to a mounted `/data/` volume.

In local development environments (such as macOS or non-root Linux containers where `/data/` is not writeable), `KevyStorage::new_with_persist` automatically logs a warning and falls back to `./data/fluxcell-storage.kevy` without failing chassis startup.

---

## 3. Configuration Reference (`spectra-flux.toml`)

```toml
# SpectraFlux Chassis Configuration

port = 8081
host = "0.0.0.0"
gateway_admin_url = "http://127.0.0.1:8000"

[broker]
method = "nats"
addr = "nats://127.0.0.1:4222"
consumer_group = "spectra-flux-workers"

# 1. Framework Internal Storage
[internal-storage]
url = "kevy://embedded"

# 2. Guest Fluxcell Application Storage
[fluxcell-storage]
url = "kevy:///data/fluxcell-storage.kevy"
# Or point to an external Redis/Valkey cluster:
# url = "redis://127.0.0.1:6379"

# 3. PostgreSQL Relational Database Pool
[database]
url = "postgresql://postgres:password@localhost:5432/spectraflux"
max_connections = 16
auto_migrate = true
```

### 12-Factor Environment Overrides
Configuration parameters can be overridden at runtime without modifying configuration files:
* `FLUX_INTERNAL_STORAGE_URL` (or `FLUX__INTERNAL_STORAGE__URL`)
* `FLUX_FLUXCELL_STORAGE_URL` (or `FLUX__FLUXCELL_STORAGE__URL`)
* Standard 12-factor cloud fallbacks: If `FLUX_FLUXCELL_STORAGE_URL` is not explicitly set, SpectraFlux automatically binds to `REDIS_URL` or `VALKEY_URL`.
* `DATABASE_URL` (or `FLUX_DATABASE_URL`) automatically overrides `[database].url`.

---

## 4. Relational Authorization & Roles with PostgreSQL

### The "Postgres or Bust" Relational Standard
SpectraFlux standardizes strictly on PostgreSQL for durable relational storage. Supporting multiple relational engines (such as MySQL or SQLite) introduces dialect fragmentation and inflates container sizes, while in-process Kevy Embedded already covers single-node embedded storage needs.

### Zero-Compiler / Zero-CLI `dbmate` Migration Runner
SpectraFlux manages database schemas using standard `dbmate`-compatible SQL migrations located in `db/migrations/`.
* **Zero Container Bloat:** Migrations are embedded at compile time into the binary via `include_str!`. Containers do **not** bundle Go, Node.js, `rustc`, or external CLI binaries, maintaining the $< 25\text{ MB}$ image invariant.
* **Automatic Version Tracking:** When `auto_migrate = true`, the chassis executes pending migrations against the target database during boot, recording versions in the standard `schema_migrations` table.

### Initial Authorization Schema
The baseline migration [`db/migrations/20260919000001_create_auth_user_roles.sql`](file:///Users/bmo/code/SpectraFlux/db/migrations/20260919000001_create_auth_user_roles.sql) establishes:

```sql
CREATE TABLE IF NOT EXISTS auth_user_roles (
    user_id VARCHAR(255) PRIMARY KEY,
    email VARCHAR(255) UNIQUE NOT NULL,
    roles JSONB NOT NULL DEFAULT '["viewer"]'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_auth_user_roles_roles_gin ON auth_user_roles USING gin (roles);
```

---

## 5. Host-Mediated Redis ABI (`host_redis`)

Downstream fluxcells frequently need high-velocity caching, atomic counters, and key-value operations. Rather than compiling complex TCP/TLS networking drivers into WASM binaries, guests invoke the host-mediated Redis ABI:

### Rust Guest SDK (`fluxcell-sdk::redis`)
```rust
use fluxcell_sdk::prelude::*;
use fluxcell_sdk::redis;

#[no_mangle]
pub extern "C" fn handle_event(ptr: u32, len: u32) -> u64 {
    let ctx = EventContext::from_host(ptr, len);

    // 1. Atomic Counter Increment
    let visits = redis::incr("metrics:daily_visits").unwrap_or(0);

    // 2. Setting values with TTL
    redis::set("cache:user:123", r#"{"status":"active"}"#, 3600).unwrap();

    // 3. Set Operations
    redis::sadd("active_tenants", &["tenant_a", "tenant_b"]).unwrap();

    // 4. Raw Redis Command Execution
    let resp = redis::cmd("HGETALL", &["user:profile:123"]).unwrap();

    ctx.response_json(&serde_json::json!({
        "status": "PROCESSED",
        "visits": visits
    }))
}
```

Calls to `host_redis` are intercepted by Wasmtime, executed asynchronously against the configured `fluxcell-storage` (in-process Kevy or external Redis), and telemetry spans (`redis:execute`) are automatically recorded in the domain trace.
