# Downstream Authorization (`authz`) in SpectraFlux

SpectraFlux is a high-velocity WebAssembly (WASM) execution chassis designed to process downstream write-model handlers, event-driven projections, and reactive workflows.

In the Spectra architecture, **authorization is a cooperative dual-pillar pipeline**: [SpectraGQL](https://github.com/brainendeavor/SpectraGQL) acts as the high-velocity edge gatekeeper, while **SpectraFlux** acts as the downstream execution engine enforcing fine-grained domain authorization, multi-tenant boundaries, and state-machine transitions.

---

## 1. The Spectra Dual-Pillar Authorization Model

```
                                  Client Request / Mutation
                                              │
                                              ▼
                               ┌─────────────────────────────┐
                               │   SpectraGQL (Edge Gateway) │
                               │                             │
                               │  • Line-Rate JWT/OIDC Auth  │
                               │  • Bounded L1 Cache (<50µs) │
                               │  • Operation-Level RBAC     │
                               │  • Declarative ABAC (CEL)   │
                               └──────────────┬──────────────┘
                                              │ Verified Event Payload + Metadata
                                              │ (NATS, Kafka, Redis Streams)
                                              ▼
 ┌────────────────────────────────────────────────────────────────────────────────────────┐
 │ SpectraFlux Chassis (Downstream Execution Engine)                                      │
 │                                                                                        │
 │   ┌──────────────────────────────────────────────────────────────────────────────────┐ │
 │   │ Zero Cryptographic Overhead                                                      │ │
 │   │   • Verified Identity Claims injected: userId, tenantId, roles, metadata         │ │
 │   │   • No JWKS fetching, no RSA/ECDSA signature verification in WASM guest          │ │
 │   └────────────────────────────────────────┬─────────────────────────────────────────┘ │
 │                                            ▼                                           │
 │   ┌──────────────────────────────────────────────────────────────────────────────────┐ │
 │   │ Sandboxed Fluxcell Execution (Wasmtime)                                          │ │
 │   │   • Fine-Grained Domain Authorization (object-level ownership, state machine)    │ │
 │   │   • Multi-Tenant Boundary Enforcement                                            │ │
 │   │   • Reads & Updates Persistent Roles in PostgreSQL via `host_db`                 │ │
 │   └────────────────────────────────────────┬─────────────────────────────────────────┘ │
 │                                            ▼                                           │
 │   ┌──────────────────────────────────────────────────────────────────────────────────┐ │
 │   │ Decoupled Storage Tier Isolation                                                 │ │
 │   │   • `internal-storage`: Saga checkpoints & OTP tokens (kevy://embedded)          │ │
 │   │   • `fluxcell-storage`: Guest app KV & Redis (kevy:///data/fluxcell-storage.kevy)  │ │
 │   └──────────────────────────────────────────────────────────────────────────────────┘ │
 └────────────────────────────────────────────────────────────────────────────────────────┘
```

### Coarse-Grained Edge Auth vs. Fine-Grained Domain Auth
The architecture strictly delineates perimeter security from business logic:
* **Edge Gatekeeper (`SpectraGQL`):** Enforces **coarse-grained access control**. It verifies JWT signatures, ensures the token is not expired, evaluates declarative operation-level permissions (e.g. "Can user with role `editor` call `updateProject`?"), evaluates perimeter CEL rules, and rejects invalid requests before they reach the broker or upstream services.
* **Downstream Chassis (`SpectraFlux`):** Enforces **fine-grained domain access control**. It answers contextual, state-dependent questions:
  * "Does this user own project `#42`?"
  * "Does the user's `tenantId` match the project record in PostgreSQL?"
  * "Is the entity in a valid state transition (e.g. `DRAFT` $\to$ `SUBMITTED`) for the user's role?"

### The Zero-Cryptographic Overhead Invariant
WASM guests running in SpectraFlux do **not** verify JWT cryptographic signatures, fetch remote JWKS certificates, or parse X.509 keys. 

Because SpectraGQL has already authenticated the client and validated signatures at the edge, it dispatches an event payload containing verified, tamper-proof identity claims. Guest fluxcells read these claims directly from the event envelope, preserving sub-millisecond execution speeds and avoiding bloated cryptographic libraries inside WASM binaries.

---

## 2. Consuming Verified Identity in Fluxcells

### Rust Guest SDK (`fluxcell-sdk`)

The `fluxcell-sdk` provides ergonomic helpers on `EventContext` and `HttpRequest` to inspect caller identity without boilerplate:

```rust
use fluxcell_sdk::prelude::*;

struct ProjectManagerCell;

impl Fluxcell for ProjectManagerCell {
    fn metadata() -> FluxcellMetadata {
        FluxcellMetadata::new("0.1.0", "Project Manager Service")
    }

    fn subscriptions() -> Vec<String> {
        vec!["mutation.updateproject".to_string()]
    }

    fn handle_event(ctx: EventContext) -> EventVerdict {
        // 1. Inspect verified caller identity injected by SpectraGQL
        let user_id = match ctx.user_id() {
            Some(id) => id,
            None => return EventVerdict::DeadLetter("Missing authenticated user ID".to_string()),
        };
        let tenant_id = ctx.tenant_id().unwrap_or_else(|| "default_tenant".to_string());

        // 2. Enforce role-based preconditions
        if !ctx.has_role("editor") && !ctx.has_role("admin") {
            return EventVerdict::DeadLetter(format!("User {} lacks editor or admin role", user_id));
        }

        // 3. Parse command payload
        #[derive(serde::Deserialize)]
        struct UpdateProjectCmd {
            project_id: i64,
            title: String,
        }
        let cmd: UpdateProjectCmd = match ctx.json() {
            Ok(c) => c,
            Err(e) => return EventVerdict::DeadLetter(format!("Invalid command payload: {}", e)),
        };

        // 4. Fine-grained domain authorization against PostgreSQL via host_db
        let mut tx = match Database::default().begin_tx() {
            Ok(t) => t,
            Err(e) => return EventVerdict::Nack(format!("Database error: {}", e)),
        };

        // Verify entity ownership and tenant isolation
        let rows = match tx.query(
            "SELECT owner_id, tenant_id FROM projects WHERE id = $1",
            &serde_json::json!([cmd.project_id]),
        ) {
            Ok(r) => r,
            Err(e) => return EventVerdict::Nack(e),
        };

        if rows.is_empty() {
            return EventVerdict::DeadLetter(format!("Project {} not found", cmd.project_id));
        }

        let project_tenant = rows[0].get("tenant_id").and_then(|v| v.as_str()).unwrap_or("");
        let project_owner = rows[0].get("owner_id").and_then(|v| v.as_str()).unwrap_or("");

        // Multi-tenant barrier
        if project_tenant != tenant_id {
            return EventVerdict::DeadLetter("Cross-tenant access prohibited".to_string());
        }

        // Ownership or Admin check
        if project_owner != user_id && !ctx.has_role("admin") {
            return EventVerdict::DeadLetter("Only project owners or admins may update this record".to_string());
        }

        // 5. Execute state mutation
        if let Err(e) = tx.execute(
            "UPDATE projects SET title = $1, updated_at = NOW() WHERE id = $2",
            &serde_json::json!([cmd.title, cmd.project_id]),
        ) {
            return EventVerdict::Nack(e);
        }

        if let Err(e) = tx.commit() {
            return EventVerdict::Nack(e);
        }

        EventVerdict::Ack
    }
}

export_fluxcell!(ProjectManagerCell);
```

### HTTP Mode Caller Identity
When fluxcells expose synchronous HTTP endpoints (`handle_http`), identity headers injected by SpectraGQL (`x-user-id`, `x-tenant-id`, `x-user-roles`) are accessible via `req.header(...)`:

```rust
fn handle_http(req: HttpRequest) -> HttpResponse {
    let user_id = req.header("x-user-id").unwrap_or("anonymous");
    let roles = req.header("x-user-roles").unwrap_or("");
    
    if !roles.split(',').any(|r| r.trim() == "admin") {
        return HttpResponse::error("Forbidden: admin role required");
    }

    HttpResponse::json(&serde_json::json!({
        "status": "ok",
        "caller": user_id
    }))
}
```

### TypeScript Guest SDK (`@spectraflux/sdk`)

In AssemblyScript / TypeScript fluxcells:

```typescript
import {
  Fluxcell,
  FluxcellMetadata,
  EventContext,
  EventVerdict,
  registerFluxcell,
} from "@spectraflux/sdk/assembly/index";

export class ProjectWorkerCell extends Fluxcell {
  metadata(): FluxcellMetadata {
    return new FluxcellMetadata("0.1.0", "TypeScript Project Worker");
  }

  subscriptions(): string[] {
    return ["mutation.updateproject"];
  }

  handleEvent(ctx: EventContext): EventVerdict {
    // Payload contains verified claims dispatched by SpectraGQL
    const payload = ctx.payloadJson;
    if (payload.indexOf('"userId"') == -1) {
      return EventVerdict.DeadLetter;
    }

    // Process domain logic...
    return EventVerdict.Ack;
  }
}

registerFluxcell(new ProjectWorkerCell());
```

---

## 3. Persistent Role Storage in PostgreSQL

SpectraFlux adheres strictly to the **"Postgres or Bust"** relational standard. Relational role and user permissions are stored centrally in PostgreSQL, managed via embedded compile-time migrations.

### Baseline Schema (`auth_user_roles`)
The baseline migration [`db/migrations/20260919000001_create_auth_user_roles.sql`](../db/migrations/20260919000001_create_auth_user_roles.sql) establishes:

```sql
CREATE TABLE IF NOT EXISTS auth_user_roles (
    user_id VARCHAR(255) PRIMARY KEY,
    email VARCHAR(255) UNIQUE NOT NULL,
    roles JSONB NOT NULL DEFAULT '["viewer"]'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- GIN index for high-velocity JSONB role containment queries (`@>`)
CREATE INDEX IF NOT EXISTS idx_auth_user_roles_roles_gin ON auth_user_roles USING gin (roles);
```

### Zero-Compiler / Zero-CLI `dbmate` Migrations
* **Zero Container Bloat:** Migrations are embedded into the `spectra-flux` chassis binary at compile time via `include_str!`. The chassis container does not require external migration CLIs or runtimes, preserving the $< 25\text{ MB}$ image invariant.
* **Automatic Version Tracking:** When `auto_migrate = true` is set in `spectra-flux.toml` (or via `DATABASE_URL` runtime configuration), pending migrations execute during chassis startup and are recorded in `schema_migrations`.

### Querying Roles from Fluxcells
Fluxcells can query or modify roles transactionally using `host_db`:

```rust
// Check if user has admin privileges via JSONB containment
let is_admin_rows = tx.query(
    "SELECT user_id FROM auth_user_roles WHERE user_id = $1 AND roles @> '[\"admin\"]'::jsonb",
    &serde_json::json!([user_id]),
)?;

let is_admin = !is_admin_rows.is_empty();
```

---

## 4. Session Management & Decoupled Storage Tiers

SpectraFlux bifurcates storage into two isolated tiers to guarantee security and operational boundaries:

```
                                  SpectraFlux Storage
                                           │
                     ┌─────────────────────┴─────────────────────┐
                     │                                           │
                     ▼                                           ▼
           [internal-storage]                          [fluxcell-storage]
           • Saga Step Checkpoints                     • Guest Fluxcell KV (`kv_store`)
           • Magic-Link OTP Tokens                     • Guest Redis (`host_redis`)
           • Authenticated Session Cache               • Guest Application Data
           • Default: `kevy://embedded`                • Default: `kevy:///data/...`
```

### Storage Tier Separation Rules
1. **`internal-storage` Isolation:** Chassis operational data (Saga step checkpoints, magic-link OTP tokens, domain execution traces) is stored in `internal-storage` (default `kevy://embedded`).
2. **`fluxcell-storage` Isolation:** Guest fluxcells store application state, cache entries, and key-value pairs in `fluxcell-storage` (default `kevy:///data/fluxcell-storage.kevy` or external Redis/Valkey).
3. **Linker Boundary:** Host ABI bindings strictly prevent cross-tier pollution:
   * `bind_host_checkpoint` exclusively reads and writes to `internal_storage`.
   * `bind_host_kv` and `bind_host_redis` exclusively read and write to `fluxcell_storage`.
   * **Security Guarantee:** A guest fluxcell cannot accidentally read, overwrite, or evict active session tokens or Saga checkpoints.

---

## 5. Subscribing to Edge Rejection Audit Events

When SpectraGQL rejects an unauthorized mutation at the network perimeter (e.g. unauthenticated actor attempting a privileged mutation, or a CEL tenant mismatch), it publishes a structured audit event under:
`interceptors.rejected.<operation_name>` (e.g., `interceptors.rejected.updateproject`).

SpectraFlux fluxcells can subscribe to these topics to power:
* **Security Information & Event Management (SIEM):** Aggregating and indexing failed security attempts.
* **Automated Account Lockout:** Tracking IP or credential stuffing patterns across multiple failed requests.
* **Real-Time Admin Alerts:** Emitting Slack/PagerDuty webhooks when anomalous rejection volumes occur.

```rust
impl Fluxcell for SecurityMonitorCell {
    fn metadata() -> FluxcellMetadata {
        FluxcellMetadata::new("0.1.0", "Security Audit Monitor")
    }

    fn subscriptions() -> Vec<String> {
        vec!["interceptors.rejected.*".to_string()]
    }

    fn handle_event(ctx: EventContext) -> EventVerdict {
        // Log or trigger SIEM alerting for rejected edge requests
        EventVerdict::Ack
    }
}
```

---

## 6. Zero Application Pollution Invariant

SpectraFlux is a generic, open-source infrastructure appliance.
* Application domain names, tenant IDs, and business authorization schemas must **never** be hardcoded into the `SpectraFlux` repository.
* All role hierarchies, entity permissions, and business rules are encapsulated in guest fluxcell WASM modules compiled and deployed dynamically via the Deployer API.
