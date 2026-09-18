# SpectraFlux Embedded Admin Console & Observability

SpectraFlux includes an embedded, high-performance administrative console and observability dashboard accessible directly at `http://localhost:8081/admin`.

---

## 1. Features Overview

The console runs out-of-band and introduces zero overhead into the WebAssembly execution hot path:

* **Execution Profiles Dashboard:** Real-time percentile latency graphs (p50, p95, p99), memory allocations, and execution fuel metrics.
* **1GB Profile Allotment:** High-capacity ring buffer for capturing high-volume trace points without disk exhaustion.
* **Sanitized Configuration Viewer:** Inspect active runtime settings, broker subjects, and database connection pools with sensitive credentials automatically redacted.
* **Live Log Stream:** Direct out-of-band streaming of chassis host logs and guest telemetry rollups.
* **Mounted Routes Matrix:** Real-time visibility into all staged and active Fluxcells and their mounted radix tree endpoints.

---

## 2. Execution Profiles & Metric Tracking

The dashboard continuously tracks guest execution metrics:

| Metric | Description | Unit |
| :--- | :--- | :--- |
| **p50 Latency** | Median guest execution duration | Microseconds ($\mu\text{s}$) |
| **p95 Latency** | 95th percentile execution tail latency | Milliseconds ($\text{ms}$) |
| **p99 Latency** | 99th percentile worst-case execution spike | Milliseconds ($\text{ms}$) |
| **Fuel Consumed** | Computational instructions consumed per invocation | Instructions |
| **Memory Footprint** | Active linear memory pages utilized by Wasmtime instance | Megabytes ($\text{MB}$) |
| **Pool Saturation** | Percentage of pre-allocated instance slots currently occupied | Percentage ($\%$) |

---

## 3. Sanitized Configuration Inspector (`/admin/api/config`)

To simplify debugging across staging and production environments, the console provides a live configuration inspector.

### Security Redaction Invariant
All sensitive connection parameters are cryptographically masked before leaving the host:
- PostgreSQL passwords: `postgres://user:***@host:5432/db`
- Deployer bearer tokens: `sk_deploy_***`
- Broker access keys: `token: ***`

---

## 4. REST Observability Endpoints

In addition to the visual web UI, all metrics and profiles are exposed via zero-allocation JSON REST APIs:

| Endpoint | Method | Description |
| :--- | :--- | :--- |
| `/admin` | `GET` | Single-page embedded admin console (HTML/JS) |
| `/admin/api/status` | `GET` | Chassis status, active cells, memory pools, and uptime |
| `/admin/api/config` | `GET` | Sanitized runtime configuration |
| `/admin/api/profiles` | `GET` | Aggregate execution profiles (p50/p95/p99) and fuel stats |
| `/admin/api/traces` | `GET` | Recent domain and HTTP trace events |
| `/admin/api/logs` | `GET` | Tail of internal ring buffer logs |
| `/healthz` | `GET` | Instant HTTP 200 liveness probe |
| `/livez` | `GET` | Database and broker readiness verification |

---

## 5. Keyboard Navigation & Shortcuts

The console supports rapid keyboard shortcuts:
- `1` - `5`: Switch between Dashboard tabs (Overview, Cells, Profiles, Config, Logs).
- `r`: Force immediate refresh of metrics and route tables.
- `?`: Toggle help drawer and shortcut reference.
