# AGENTS.md: Developer & Agent Architectural Guide for SpectraFlux

This document outlines the core architectural tenets, invariants, and guidelines for engineers and autonomous AI agents working in the `SpectraFlux` repository.

---

## 1. Core Architectural Tenets

`SpectraFlux` is a high-velocity WebAssembly (WASM) execution chassis and micro-worker appliance designed to run downstream write-model handlers, event-driven projections, and reactive workflows.

---

## 2. Public Open-Source Repository Invariant (Zero Application Pollution)

`SpectraFlux` is a public, generic infrastructure appliance and open-source engine framework.

* **Strict Invariant:** Application-specific domain names, application IDs, business schemas, proprietary fluxcell `.wasm` binaries, or application-specific event subscriptions must **NEVER** be hardcoded or committed into `SpectraFlux` repository files (including `runtime/spectra-flux.toml`, Dockerfiles, or default source code).
* **Zero-Compiler Container Invariant:** `spectra-flux` runtime containers must NEVER bundle `rustc`, `cargo`, or `git`. Container images must remain ultra-lean (< 25 MB).
* **Dynamic Deployment Pattern:** Fluxcells are compiled in external CI/CD pipelines or application repositories (such as `OpenCoEval`) and deployed dynamically to the running chassis via the Deployer API:
  `POST /_flux/deployer/upload?name=<name>&mount_path=<path>&auto_activate=true`
* **Configuration Mechanism:** All environment-specific connection strings (NATS/Kafka broker, PostgreSQL database, gateway admin URL) must be supplied dynamically at deployment runtime via 12-factor environment variables (`FLUX__BROKER__METHOD`, `FLUX__BROKER__ADDR`, `FLUX__DATABASE__URL`, etc.) or volume mounts.

---

## 3. Decoupled Storage Tiers Invariant

Storage inside `SpectraFlux` is strictly decoupled into two isolated tiers:
* **`internal-storage`**: Reserved strictly for chassis framework operations (Saga checkpoints, magic-link OTP tokens, session cache, domain traces). Defaults to `kevy://embedded`.
* **`fluxcell-storage`**: Dedicated exclusively to guest fluxcell application state, KV operations (`kv_store`), and Redis protocol commands (`host_redis`). Defaults to `kevy:///data/fluxcell-storage.kevy` with persistent volume mount support and fail-safe local dev fallback to `./data/`.
* **Linker Boundary:** Host ABI bindings must NEVER cross storage boundaries: `bind_host_checkpoint` must use `internal_storage`; `bind_host_kv` and `bind_host_redis` must use `fluxcell_storage`.

---

## 4. Relational Database Standard ("Postgres or Bust") & Embedded Migrations

* **PostgreSQL Relational Exclusivity:** SpectraFlux standardizes exclusively on PostgreSQL for persistent relational storage to eliminate dialect fragmentation and avoid container bloat.
* **Embedded Compile-Time Migrations:** Database migrations are defined in standard `dbmate` plain SQL files under `db/migrations/` and embedded into the binary at compile time (`include_str!`). The runtime container must NEVER require external Go or Node `dbmate` CLI binaries. Versions are tracked in the standard `schema_migrations` table.
