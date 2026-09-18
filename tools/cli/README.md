# `fluxcell`: Developer CLI for SpectraFlux & SpectraGQL

[![Crates.io](https://img.shields.io/badge/crates.io-v0.1.0-blue.svg)](Cargo.toml)
[![License: MIT](https://img.shields.io/badge/License-MIT-black.svg)](LICENSE)
[![Binary](https://img.shields.io/badge/binary-fluxcell-cyan.svg)](#installation)

`fluxcell` is the canonical command-line tool for scaffolding, compiling, testing, inspecting, and deploying WebAssembly **Fluxcells** into **SpectraFlux** micro-VM runtime chassis and **SpectraGQL** API gateways.

---

## Table of Contents

- [Key Capabilities](#key-capabilities)
- [Installation](#installation)
- [Command Reference](#command-reference)
  - [`fluxcell new`](#fluxcell-new)
  - [`fluxcell init`](#fluxcell-init)
  - [`fluxcell build`](#fluxcell-build)
  - [`fluxcell deploy`](#fluxcell-deploy)
  - [`fluxcell status`](#fluxcell-status)
  - [`fluxcell lockdown`](#fluxcell-lockdown)
- [Environment Variables](#environment-variables)
- [Deployment Governance & Workflows](#deployment-governance--workflows)
  - [Rapid Local Development (`--dev-upload`)](#rapid-local-development---dev-upload)
  - [Production Mode B Deployment](#production-mode-b-deployment)
  - [Emergency Security Lockdown](#emergency-security-lockdown)

---

## Key Capabilities

* **Polyglot Scaffolding:** Generate turn-key Rust or TypeScript/AssemblyScript Fluxcell starter projects adhering to strict WIT interface specifications.
* **Deterministic Compilations:** Native integration with `cargo` and `wasm32-wasip1`, automatically calculating cryptographic SHA-256 digests.
* **Dual-Path Deployments:**
  * Fast dev-loop direct upload to local runtime chassis (`--dev-upload`).
  * Production Mode B GraphQL mutation dispatch (`deployFluxcell`) with UUIDv7 receipts, HLC stamps, and SSRF boundary validation.
* **Cluster Governance & Killswitches:** Real-time cluster status checks and instantaneous atomic ingress lockdown.

---

## Installation

### From Source (SpectraFlux Workspace)

```bash
cargo install --path tools/cli
```

### Direct Build

```bash
cargo build --release -p fluxcell-cli
# The compiled executable will be in target/release/fluxcell
```

Verify installation:

```bash
fluxcell --help
```

---

## Command Reference

### `fluxcell new`
Scaffolds a new Fluxcell project in a dedicated directory using canonical templates.

```bash
fluxcell new <NAME> [OPTIONS]
```

#### Arguments & Options
* `<NAME>`: Name of the new Fluxcell (e.g. `order-auditor`, `invoice-mailer`).
* `-p, --path <PATH>`: Destination directory (defaults to `./<NAME>`).
* `-l, --lang <LANG>`: Target language: `rust` (default) or `typescript` / `ts`.
* `-t, --template <TEMPLATE>`: Starter template: `minimal` (default) or `mailer`.

#### Examples
```bash
# Create a minimal Rust Fluxcell
fluxcell new my-auditor

# Create an emailer Fluxcell in TypeScript
fluxcell new invoice-mailer --lang ts

# Create in a specific directory
fluxcell new auth-guard --path ./services/auth-guard
```

---

### `fluxcell init`
Initializes the current working directory as a new Fluxcell project.

```bash
fluxcell init [OPTIONS]
```

#### Options
* `-n, --name <NAME>`: Project name (defaults to current directory name).
* `-l, --lang <LANG>`: Target language: `rust` or `ts`.
* `-t, --template <TEMPLATE>`: Starter template: `minimal` or `mailer`.

---

### `fluxcell build`
Compiles the Fluxcell to the `wasm32-wasip1` WebAssembly target and computes its cryptographic SHA-256 hash.

```bash
fluxcell build [OPTIONS]
```

#### Options
* `-p, --path <PATH>`: Project directory containing `Cargo.toml` (defaults to current directory).
* `--release`: Build in release mode (default: `true`).

#### Output Example
```
⚡ Compiling Fluxcell at . to wasm32-wasip1...
✓ Built: target/wasm32-wasip1/release/my_auditor.wasm (18.42 KB)
✓ SHA-256: 4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945
```

---

### `fluxcell deploy`
Deploys the Fluxcell to a SpectraGQL gateway or directly to a downstream SpectraFlux runtime chassis.

```bash
fluxcell deploy [OPTIONS]
```

#### Options
* `-g, --gateway <URL>`: Gateway GraphQL endpoint (default: `http://127.0.0.1:8000`, env: `SPECTRA_GATEWAY_URL`).
* `-f, --flux-url <URL>`: Downstream chassis endpoint (default: `http://127.0.0.1:8081`, env: `SPECTRA_FLUX_URL`).
* `-t, --token <TOKEN>`: Deployment authorization bearer token (env: `SPECTRA_DEPLOY_TOKEN`).
* `-m, --mount, --mount-path <PATH>`: HTTP mount path in gateway/chassis radix tree (e.g. `/api/v1/auditor`).
* `-n, --name <NAME>`: Cell name (inferred from `Cargo.toml` if omitted).
* `--artifact-url <URL>`: Remote HTTPS URL for production deployments (GitHub Releases, AWS S3, Cloudflare R2).
* `--sha256 <HASH>`: Expected SHA-256 checksum (computed automatically if built locally).
* `--dev-upload`: Uploads raw local `.wasm` file directly to the downstream chassis (Dev mode).
* `--activate`: Automatically activates the cell upon successful staging (default: `false`).

---

### `fluxcell status`
Inspects active and staged Fluxcells running in the runtime chassis, along with deployment killswitch states.

```bash
fluxcell status [OPTIONS]
```

#### Options
* `-e, --endpoint <URL>`: Downstream chassis or gateway status endpoint (default: `http://127.0.0.1:8081`, env: `SPECTRA_FLUX_URL`).

#### Example Output
```
─────────────────────────────────────────────────────────────
  SPECTRAFLUX STATUS & GOVERNANCE
─────────────────────────────────────────────────────────────
  Killswitch (External Deploy): true
  Killswitch (Dev Upload):      true

  ACTIVE FLUXCELLS:
    • invoice-mailer v1.0.0 (Mount: /api/invoices)
    • order-auditor v0.2.1 (Mount: /api/orders)

  STAGED PENDING APPROVAL:
    • auth-guard v1.1.0 (SHA: a1b2c3d4e5f6...)
─────────────────────────────────────────────────────────────
```

---

### `fluxcell lockdown`
Triggers an emergency lockdown, immediately flipping lock-free atomic killswitches cluster-wide to terminate all deployment ingress.

```bash
fluxcell lockdown [OPTIONS]
```

#### Options
* `-e, --endpoint <URL>`: Downstream chassis or gateway status endpoint (default: `http://127.0.0.1:8081`, env: `SPECTRA_FLUX_URL`).

#### Example Output
```
🚨 TRIGGERING EMERGENCY LOCKDOWN on http://127.0.0.1:8081...
🛑 LOCKDOWN SUCCESSFUL: All deployment ingress has been instantly terminated.
{
  "status": "LOCKED_DOWN",
  "external_deploy_enabled": false,
  "dev_upload_enabled": false,
  "message": "All deployment ingress disabled immediately"
}
```

---

## Environment Variables

| Variable | Description | Default |
| :--- | :--- | :--- |
| `SPECTRA_GATEWAY_URL` | SpectraGQL gateway GraphQL endpoint | `http://127.0.0.1:8000` |
| `SPECTRA_FLUX_URL` | SpectraFlux downstream runtime chassis | `http://127.0.0.1:8081` |
| `SPECTRA_DEPLOY_TOKEN` | Bearer token for deployment authorization | *(None - required)* |

---

## Deployment Governance & Workflows

### Rapid Local Development (`--dev-upload`)

During local feature development, you can upload raw compiled `.wasm` binaries directly to the local SpectraFlux chassis without publishing to external HTTPS artifact stores:

```bash
# 1. Build and upload directly with auto-activation
fluxcell deploy --dev-upload --mount /api/auditor --activate
```

* Bypasses external network fetching.
* Direct upload endpoint: `POST /_flux/deployer/upload`.
* Default timeout: 10,000 ms.

---

### Production Mode B Deployment

In production environments, all deployments must follow strict governance and zero-compiler invariants:

1. **Build & Release in CI/CD:**
   External CI compiles the `.wasm` file, computes its SHA-256 hash, and publishes both as release assets (e.g. GitHub Releases).
2. **Issue Mode B Mutation:**
   ```bash
   fluxcell deploy \
     --artifact-url https://github.com/my-org/cells/releases/download/v1.0.0/order_cell.wasm \
     --sha256 4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945 \
     --mount /api/v1/orders \
     --activate
   ```
3. **Gateway Verification:**
   * **Constant-Time Token Auth:** Validates `SPECTRA_DEPLOY_TOKEN`.
   * **SSRF Shield:** Blocks loopback (`127.0.0.1`), RFC 1918 private IPs, cloud metadata (`169.254.169.254`), and redirects.
   * **Cryptographic Verification:** Validates the streamed artifact's SHA-256 against the expected hash.
   * **Two-Phase Activation:** Staged cells do not route traffic until explicitly activated.

---

### Emergency Security Lockdown

If malicious activity or unexpected behavior is detected, operators can immediately isolate the cluster:

```bash
fluxcell lockdown
```

This hits `/admin/api/v1/security/lockdown`, instantly zeroing atomic deployment flags. All subsequent upload and remote fetch requests are immediately rejected with HTTP 403 / 503 errors.
