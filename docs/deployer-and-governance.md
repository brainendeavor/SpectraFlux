# Remote Fluxcell Deployment & Security Governance

This document specifies the deployment lifecycle, security governance invariants, SSRF protections, and emergency lockdown mechanisms for **SpectraFlux**.

---

## 1. Core Architectural Tenets

### Zero-Compiler Container Invariant (< 25 MB)
* `spectra-flux` production runtime containers must **NEVER** bundle `rustc`, `cargo`, `bun`, `node`, or `git`.
* The chassis operates strictly as a lean execution engine for pre-compiled, immutable `.wasm` binaries.
* Compilation takes place exclusively in external CI/CD pipelines (GitHub Actions, GitLab CI, Docker multi-stage builds).

### Public Repository Invariant (Zero Application Pollution)
* Application-specific business logic, domain models, proprietary `.wasm` files, or application subscriptions must **NEVER** be committed into the SpectraFlux repository.
* All configuration is provided via 12-factor environment variables (`FLUX__BROKER__METHOD`, `FLUX__DATABASE__URL`) or dynamic Deployer API uploads.

---

## 2. Deployer API Specification

The chassis exposes HTTP deployer endpoints at `/_flux/deployer/*`:

### 1. Dev Direct Upload (`POST /_flux/deployer/upload`)
Allows local development tooling (`fluxcell deploy --dev-upload`) to stage and mount a compiled `.wasm` binary directly into the running chassis:

```http
POST /_flux/deployer/upload?name=<cell_name>&mount_path=<route_path>&auto_activate=<bool>
Content-Type: application/wasm
Authorization: Bearer <SPECTRA_DEPLOY_TOKEN>
```

- **Query Parameters:**
  - `name`: Logical identifier for the fluxcell.
  - `mount_path` (or `mount`): Prefix path where HTTP routes will be mounted in the radix tree.
  - `auto_activate`: If `true`, the cell transitions from `Staged` to `Active` immediately upon successful verification.

### 2. Production Remote Artifact Ingress
In production, deployments are initiated via Mode B GraphQL mutations on SpectraGQL (`deployFluxcell`) or the Deployer API by specifying an HTTPS artifact URL and cryptographic hash:

```json
{
  "artifactUrl": "https://github.com/my-org/cells/releases/download/v1.0.0/order_cell.wasm",
  "sha256": "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
  "mountPath": "/api/v1/orders",
  "autoActivate": false
}
```

---

## 3. Downstream SSRF Shield

When fetching remote `.wasm` artifacts, SpectraFlux enforces multi-layer Server-Side Request Forgery (SSRF) verification before opening any network connections:

1. **HTTPS Enforcement:** Only secure `https://` schemes are accepted.
2. **Strict Redirect Prohibition:** The HTTP client builder explicitly sets `redirect(Policy::none())`. If a remote URL returns an HTTP 301/302 redirect, the request is immediately rejected, preventing open-redirect exploitation.
3. **Pre-Connect DNS Inspection:** Before connecting, hostnames are resolved to IP addresses and verified against forbidden IP bitmasks:
   - **Loopback:** `127.0.0.0/8`, `::1`
   - **Private IPv4 (RFC 1918):** `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`
   - **Carrier-Grade NAT (RFC 6598):** `100.64.0.0/10`
   - **Unique Local IPv6 (RFC 4193):** `fc00::/7` (including `fc00::/8` and `fd00::/8`)
   - **Link-Local & Multicast:** `169.254.0.0/16`, `fe80::/10`, `ff00::/8`
   - **Cloud Metadata Services:** `169.254.169.254` (AWS, GCP, Azure metadata endpoints)

---

## 4. Deployment Authentication Hierarchy

All deployment mutations and API endpoints require authorization verified via constant-time token comparison:

1. **Environment Variable (Top Precedence):**
   `SPECTRA_DEPLOY_TOKEN` (or `FLUX_DEPLOY_TOKEN`).
2. **Configuration File:**
   `[admin]` with `deploy_token = "..."` or `[security]` token.
3. **Fail-Safe Default:**
   If no token is configured, all deployment requests are **unconditionally rejected with HTTP 401 `DEPLOY_UNAUTHORIZED`**.

---

## 5. Two-Phase Staging & Activation

To ensure high-availability and zero accidental routing errors:

1. **Phase 1: Staged**
   - The `.wasm` binary is fetched, its SHA-256 is verified, memory exports and WIT contracts are validated, and the module is saved to local storage (`/tmp/staged` or persistent disk).
   - The cell does **not** mount HTTP routes or subscribe to broker topics.
2. **Phase 2: Active**
   - Upon explicit administrator activation (`fluxcell deploy --activate` or admin console approval), the chassis hot-swaps the cell into the live radix tree.

---

## 6. Emergency Cluster Lockdown

In the event of an incident or detected anomaly, operators can immediately disable all deployment ingress across the cluster:

```bash
fluxcell lockdown
```

This triggers `POST /admin/api/v1/security/lockdown`, which atomically flips lock-free atomic boolean flags:
- `external_deploy_enabled = false`
- `dev_upload_enabled = false`

All subsequent deployment or upload attempts receive `HTTP 403 Forbidden` until administrative unlock.
