# SpectraFlux Bug Fix & Hardening Log

This document serves as the canonical audit log for all edge-case bugs, security vulnerabilities, regression tests, and architectural hardening implemented in **SpectraFlux**.

---

## Catalog of Bugs & Hardening Fixes

| Bug ID | Component | Severity | Description | Status | Regression Test |
|---|---|---|---|---|---|
| **FIX-FLUX-001** | `runtime::http` | High | Query param mismatch (`mount_path` vs `mount`) & ignored `auto_activate` during direct dev upload | **Resolved** | `test_handle_request_deployer_upload_with_mount_path_and_auto_activate` |
| **FIX-FLUX-002** | `runtime::deployer` | Critical | SSRF Shield open redirect bypass vulnerability in Deployer HTTP client | **Resolved** | `tests/integration_deployer_governance.rs` |
| **FIX-FLUX-003** | `runtime::ssrf` | High | Missing IPv6 Unique Local (`fc00::/7`) and CGNAT (`100.64.0.0/10`) range checks | **Resolved** | `test_is_forbidden_ip_cgnat_and_ipv6_unique_local` |
| **FIX-FLUX-004** | `tools::cli` | Medium | Missing library target (`src/lib.rs`) preventing programmatic testing of CLI commands | **Resolved** | `tools/cli/tests/cli_workflow.rs` |
| **FIX-FLUX-005** | `runtime::storage` | Medium | Embedded Kevy storage tests bypassed during default `cargo test` due to missing feature | **Resolved** | `storage::tests::*` (10 tests active in workspace) |
| **FIX-FLUX-006** | `workspace` | Medium | Lack of dedicated integration test suites (`tests/`) across runtime and CLI crates | **Resolved** | `runtime/tests/*`, `tools/cli/tests/*` |
| **FIX-FLUX-007** | `sdks::typescript` | Medium | Broken test script (`verify-abi.mjs`) and missing unit test suite in `@spectraflux/sdk` | **Resolved** | `sdks/typescript/tests/sdk.test.ts` |

---

### Detailed Root Cause & Resolution Analysis

#### 1. FIX-FLUX-001: Query Parameter Mismatch & Ignored Auto-Activate in Direct Dev-Upload
* **Component:** `runtime/src/http/mod.rs`
* **Severity:** High
* **Root Cause:**
  When `fluxcell deploy --dev-upload --mount /custom/path --activate` was executed, the CLI constructed a request to `/_flux/deployer/upload?name=...&mount_path=...&auto_activate=true`. However, the HTTP handler only checked `k == "mount"`, ignoring `mount_path`, which silently reverted the mount path to `/api/{name}`. Furthermore, the handler completely ignored the `auto_activate` query parameter, leaving newly uploaded cells in `Staged` status rather than activating them as requested by the user.
* **Edge Case:**
  Developers deploying local test cells experienced route mismatches and cells that remained inactive unless manual approval was triggered via the gateway admin UI.
* **Resolution:**
  Updated query parameter parsing in `runtime/src/http/mod.rs` to accept both `mount` and `mount_path`, and parse `auto_activate`/`activate` (or `X-Fluxcell-Auto-Activate` header). If `auto_activate == true`, `dep.activate(&cell_name, &record.sha256)` is invoked immediately upon successful staging, returning the active record.
* **Regression Test:**
  `runtime::http::tests::test_handle_request_deployer_upload_with_mount_path_and_auto_activate`

---

#### 2. FIX-FLUX-002: SSRF Shield Open Redirect Bypass in Deployer HTTP Client
* **Component:** `runtime/src/deployer/mod.rs`
* **Severity:** Critical
* **Root Cause:**
  `FluxcellDeployer` instantiated its `reqwest::Client` with default settings, which follows up to 10 HTTP redirects by default. If an attacker configured a download URL from an allowed host (e.g., `github.com`) that issued an HTTP 302 redirect to an internal IP (such as cloud metadata `http://169.254.169.254` or loopback `http://127.0.0.1:8081`), reqwest would follow the redirect without re-validating the redirected destination against the SSRF shield.
* **Edge Case:**
  Open-redirect vulnerability on any whitelisted domain could be weaponized to exfiltrate cloud credentials or trigger unauthorized local actions.
* **Resolution:**
  Configured `redirect(reqwest::redirect::Policy::none())` on the Deployer's HTTP client builder, strictly prohibiting automatic redirect following.
* **Regression Test:**
  `runtime/tests/integration_deployer_governance.rs::test_ssrf_rejects_redirects`

---

#### 3. FIX-FLUX-003: SSRF Shield Missing RFC 4193 (IPv6 ULA) and RFC 6598 (CGNAT) Checks
* **Component:** `runtime/src/deployer/ssrf_shield.rs`
* **Severity:** High
* **Root Cause:**
  `is_forbidden_ip` verified RFC 1918 IPv4 private subnets and IPv6 link-local/loopback addresses, but omitted RFC 4193 Unique Local IPv6 Addresses (`fc00::/7`, encompassing `fc00::/8` and `fd00::/8`) and RFC 6598 Carrier-Grade NAT (`100.64.0.0/10`).
* **Edge Case:**
  In modern dual-stack environments and VPC overlay networks, attackers could target internal services via IPv6 ULA or CGNAT shared address space.
* **Resolution:**
  Added bitmask checks for `(segments[0] & 0xfe00) == 0xfc00` on IPv6 and `(octets[0] == 100 && (octets[1] & 0xc0) == 64)` on IPv4.
* **Regression Test:**
  `runtime::deployer::ssrf_shield::tests::test_is_forbidden_ip_cgnat_and_ipv6_unique_local`

---

#### 4. FIX-FLUX-004: Missing Library Target and Public API in `fluxcell-cli`
* **Component:** `tools/cli`
* **Severity:** Medium
* **Root Cause:**
  `fluxcell-cli` only declared a `[[bin]]` target with all functions marked private in `src/main.rs`. This prevented programmatic testing of scaffolding, argument validation, and CI/CD workflow generation.
* **Edge Case:**
  CLI argument regressions, seed replacement corruption, and command handling bugs could only be verified manually.
* **Resolution:**
  Extracted `tools/cli/src/lib.rs` with clean public modules (`scaffold`, `build`, `deploy`, `status`, `lockdown`, `cli`), keeping `src/main.rs` as a thin binary wrapper.
* **Regression Test:**
  `tools/cli/tests/cli_workflow.rs`

---

#### 5. FIX-FLUX-005: Kevy Storage Tests Omitted in Default Test Runs
* **Component:** `runtime/src/storage/mod.rs` & `runtime/Cargo.toml`
* **Severity:** Medium
* **Root Cause:**
  `FluxConfig::default()` configures `storage.backend = "embedded_kevy"`. However, the `kevy` feature was omitted from `default` features in `runtime/Cargo.toml`. Consequently, all 10 unit and adversarial concurrency/persistence tests in `storage/mod.rs` were skipped unless `--features kevy` was manually specified, and default runtime instances failed to start without manual feature flags.
* **Resolution:**
  Added `"kevy"` to the `default` feature list in `runtime/Cargo.toml`, ensuring the zero-dependency embedded database is active and fully tested.
* **Regression Test:**
  `runtime::storage::tests::*`

---

#### 6. FIX-FLUX-006: Missing Workspace Integration Test Suites
* **Component:** `runtime` and `tools/cli`
* **Severity:** Medium
* **Root Cause:**
  The repository lacked integration test suites in `tests/` directories, relying solely on inline module tests.
* **Resolution:**
  Created dedicated integration test suites:
  * `runtime/tests/integration_deployer_governance.rs`: Verifies the full staging, activation, lockdown, and SSRF lifecycle.
  * `runtime/tests/integration_wasm_pooling_and_epochs.rs`: Verifies instance pooling saturation, epoch interruption, and circuit breaker transitions.
  * `tools/cli/tests/cli_workflow.rs`: Verifies CLI scaffolding and argument handling.
  * `sdks/rust/tests/sdk_edge_cases.rs`: Verifies Rust SDK ABI, HLC, and CausalGuard boundaries.

---

#### 7. FIX-FLUX-007: Broken TypeScript SDK Test Runner
* **Component:** `sdks/typescript`
* **Severity:** Medium
* **Root Cause:**
  `sdks/typescript/package.json` had `"test": "node ./bin/verify-abi.mjs"`. However, `verify-abi.mjs` expects a compiled `.wasm` file in `build/`, which does not exist in a library package, causing `bun test` or `npm test` in `sdks/typescript` to fail.
* **Resolution:**
  Added a dedicated Bun test suite in `sdks/typescript/tests/sdk.test.ts` testing ABI encoding/decoding, HLC parsing, deduplication, and HTTP response envelopes.
* **Regression Test:**
  `sdks/typescript/tests/sdk.test.ts`
