use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use sha2::{Digest, Sha256};

#[derive(Parser)]
#[command(name = "fluxcell", author = "SpectraGQL Team", version = "0.1.0", about = "Developer CLI for scaffolding, building, and deploying SpectraGQL Fluxcells")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Scaffold a new Fluxcell project from a template
    New {
        /// Name of the new Fluxcell (e.g. order-auditor, invoice-mailer)
        name: String,
        /// Target directory path (defaults to ./<name>)
        #[arg(short, long)]
        path: Option<PathBuf>,
        /// Starter template: 'minimal' (default) or 'mailer'
        #[arg(short, long, default_value = "minimal")]
        template: String,
    },
    /// Initialize the current directory as a new Fluxcell
    Init {
        /// Name of the fluxcell (defaults to current directory name)
        #[arg(short, long)]
        name: Option<String>,
        /// Starter template: 'minimal' (default) or 'mailer'
        #[arg(short, long, default_value = "minimal")]
        template: String,
    },
    /// Compile the Fluxcell to wasm32-wasip1 and compute its cryptographic SHA-256 digest
    Build {
        /// Project directory path (defaults to current directory)
        #[arg(short, long)]
        path: Option<PathBuf>,
        /// Build in release mode
        #[arg(long, default_value_t = true)]
        release: bool,
    },
    /// Deploy the Fluxcell to a SpectraGQL gateway or Spectral Flux appliance
    Deploy {
        /// Gateway GraphQL endpoint
        #[arg(short, long, default_value = "http://127.0.0.1:8000", env = "SPECTRA_GATEWAY_URL")]
        gateway: String,
        /// Downstream chassis endpoint (used for direct dev uploads)
        #[arg(short, long, default_value = "http://127.0.0.1:8081", env = "SPECTRA_FLUX_URL")]
        flux_url: String,
        /// Deployment authentication token (or env SPECTRA_DEPLOY_TOKEN)
        #[arg(short, long, env = "SPECTRA_DEPLOY_TOKEN")]
        token: Option<String>,
        /// Target HTTP mount path in the gateway (e.g. /api/invoices)
        #[arg(short, long)]
        mount: Option<String>,
        /// Name of the fluxcell (inferred from Cargo.toml if omitted)
        #[arg(short, long)]
        name: Option<String>,
        /// Remote HTTPS artifact URL (e.g. GitHub Releases, S3, R2)
        #[arg(long)]
        artifact_url: Option<String>,
        /// Expected SHA-256 hash (computed automatically if local build exists)
        #[arg(long)]
        sha256: Option<String>,
        /// Directly upload local compiled .wasm file to the downstream chassis (Dev mode)
        #[arg(long)]
        dev_upload: bool,
        /// Automatically activate the fluxcell after staging
        #[arg(long, default_value_t = false)]
        activate: bool,
    },
    /// Inspect running and staged fluxcells on the gateway
    Status {
        /// Downstream chassis or gateway status endpoint
        #[arg(short, long, default_value = "http://127.0.0.1:8081", env = "SPECTRA_FLUX_URL")]
        endpoint: String,
    },
    /// Trigger emergency lockdown, disabling all deployment ingress cluster-wide
    Lockdown {
        /// Downstream chassis endpoint
        #[arg(short, long, default_value = "http://127.0.0.1:8081", env = "SPECTRA_FLUX_URL")]
        endpoint: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::New { name, path, template } => {
            let target_dir = path.unwrap_or_else(|| PathBuf::from(&name));
            scaffold_fluxcell(&name, &target_dir, &template)?;
        }
        Commands::Init { name, template } => {
            let current_dir = std::env::current_dir()?;
            let resolved_name = name.unwrap_or_else(|| {
                current_dir
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("my-fluxcell")
                    .to_string()
            });
            scaffold_fluxcell(&resolved_name, &current_dir, &template)?;
        }
        Commands::Build { path, release } => {
            run_build(path.as_deref(), release)?;
        }
        Commands::Deploy {
            gateway,
            flux_url,
            token,
            mount,
            name,
            artifact_url,
            sha256,
            dev_upload,
            activate,
        } => {
            run_deploy(
                &gateway,
                &flux_url,
                token.as_deref(),
                mount.as_deref(),
                name.as_deref(),
                artifact_url.as_deref(),
                sha256.as_deref(),
                dev_upload,
                activate,
            )
            .await?;
        }
        Commands::Status { endpoint } => {
            query_status(&endpoint).await?;
        }
        Commands::Lockdown { endpoint } => {
            trigger_lockdown(&endpoint).await?;
        }
    }

    Ok(())
}

fn scaffold_fluxcell(name: &str, target_dir: &Path, template: &str) -> Result<()> {
    println!("⚡ Scaffolding new Fluxcell '{name}' at {}...", target_dir.display());

    fs::create_dir_all(target_dir.join("src"))?;
    fs::create_dir_all(target_dir.join("wit"))?;
    fs::create_dir_all(target_dir.join(".github/workflows"))?;

    // Cargo.toml
    let cargo_toml = format!(
r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2024"
description = "{name} Fluxcell for SpectraGQL"
license = "MIT"

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
serde = {{ version = "1.0", features = ["derive"] }}
serde_json = "1.0"

[build-dependencies]
chrono = {{ version = "0.4", default-features = false, features = ["clock"] }}
"#);
    fs::write(target_dir.join("Cargo.toml"), cargo_toml)?;

    // build.rs
    let build_rs = r#"use std::process::Command;

fn main() {
    let git_hash = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=GIT_HASH={git_hash}");
    println!("cargo:rustc-env=BUILD_TIMESTAMP={}", chrono::Utc::now().to_rfc3339());
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=wit/fluxcell.wit");
}
"#;
    fs::write(target_dir.join("build.rs"), build_rs)?;

    // build.sh
    let build_sh = r#"#!/usr/bin/env bash
set -euo pipefail

echo "==> Verifying wasm32-wasip1 target..."
rustup target add wasm32-wasip1 2>/dev/null || true

echo "==> Compiling Fluxcell..."
cargo build --target wasm32-wasip1 --release

WASM_PATH=$(find target/wasm32-wasip1/release -maxdepth 1 -name "*.wasm" ! -name "*.*.wasm" | head -n 1)

if [ -n "$WASM_PATH" ]; then
    SIZE=$(du -h "$WASM_PATH" | cut -f1)
    SHA256=$(shasum -a 256 "$WASM_PATH" | cut -d ' ' -f 1)
    echo "✓ Built: $WASM_PATH ($SIZE)"
    echo "✓ SHA-256: $SHA256"
fi
"#;
    fs::write(target_dir.join("build.sh"), build_sh)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(target_dir.join("build.sh"), fs::Permissions::from_mode(0o755));
    }

    // wit/fluxcell.wit
    let wit_content = r#"package spectra:fluxcell@0.1.0;

interface types {
    record http-request {
        method: string,
        uri: string,
        headers: list<tuple<string, string>>,
        body: list<u8>,
    }

    record http-response {
        status: u16,
        headers: list<tuple<string, string>>,
        body: list<u8>,
    }

    record event-packet {
        topic: string,
        id: string,
        timestamp: u64,
        payload: list<u8>,
    }

    record route-def {
        method: string,
        path: string,
    }
}

world fluxcell {
    use types.{http-request, http-response, event-packet, route-def};

    import kv-store: interface {
        get: func(key: string) -> option<list<u8>>;
        set: func(key: string, value: list<u8>, ttl-seconds: option<u64>) -> result<_, string>;
        delete: func(key: string) -> result<bool, string>;
    };

    export get-routes: func() -> list<route-def>;
    export get-subscriptions: func() -> list<string>;
    export handle-event: func(event: event-packet) -> result<_, string>;
    export handle-http: func(request: http-request) -> http-response;
}
"#;
    fs::write(target_dir.join("wit/fluxcell.wit"), wit_content)?;

    // .gitignore
    let gitignore = "/target\n*.wasm\n.DS_Store\n";
    fs::write(target_dir.join(".gitignore"), gitignore)?;

    // .github/workflows/release.yml
    let workflow = format!(
r#"name: Release Fluxcell Wasm

on:
  push:
    tags:
      - 'v*'

jobs:
  build-release:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: wasm32-wasip1
      - name: Build WASM
        run: |
          cargo build --target wasm32-wasip1 --release
          WASM_FILE=$(find target/wasm32-wasip1/release -maxdepth 1 -name "*.wasm" ! -name "*.*.wasm" | head -n 1)
          cp "$WASM_FILE" "{name}.wasm"
          sha256sum "{name}.wasm" > "{name}.wasm.sha256"
      - name: Create GitHub Release
        uses: softprops/action-gh-release@v2
        with:
          files: |
            {name}.wasm
            {name}.wasm.sha256
"#);
    fs::write(target_dir.join(".github/workflows/release.yml"), workflow)?;

    // src/lib.rs
    let lib_rs = if template == "mailer" || template == "invoice-mailer" {
        include_str!("../../../examples/rust/mailer/src/lib.rs")
    } else {
        r#"use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct RouteDefinition {
    pub method: String,
    pub path: String,
}

#[derive(Serialize, Deserialize)]
pub struct FluxcellMetadata {
    pub name: String,
    pub version: String,
    pub git_hash: String,
    pub build_timestamp: String,
}

#[derive(Serialize, Deserialize)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

pub fn routes() -> Vec<RouteDefinition> {
    vec![
        RouteDefinition { method: "GET".into(), path: "/health".into() },
        RouteDefinition { method: "GET".into(), path: "/status".into() },
        RouteDefinition { method: "POST".into(), path: "/echo".into() },
    ]
}

pub fn subscriptions() -> Vec<String> {
    vec!["events.incoming".to_string(), "mutation.*".to_string()]
}

#[unsafe(no_mangle)]
pub extern "C" fn get_routes() -> u64 {
    let json = serde_json::to_string(&routes()).unwrap_or_else(|_| "[]".into());
    pack_string(json)
}

#[unsafe(no_mangle)]
pub extern "C" fn get_subscriptions() -> u64 {
    let json = serde_json::to_string(&subscriptions()).unwrap_or_else(|_| "[]".into());
    pack_string(json)
}

#[unsafe(no_mangle)]
pub extern "C" fn get_metadata() -> u64 {
    let meta = FluxcellMetadata {
        name: env!("CARGO_PKG_NAME").to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        git_hash: option_env!("GIT_HASH").unwrap_or("unknown").to_string(),
        build_timestamp: option_env!("BUILD_TIMESTAMP").unwrap_or("").to_string(),
    };
    let json = serde_json::to_string(&meta).unwrap_or_else(|_| "{}".into());
    pack_string(json)
}

#[unsafe(no_mangle)]
pub extern "C" fn handle_http(ptr: u32, len: u32) -> u64 {
    let _input = unpack_string(ptr, len);
    let resp = HttpResponse {
        status: 200,
        headers: vec![("content-type".into(), "application/json".into())],
        body: serde_json::json!({
            "status": "ok",
            "cell": env!("CARGO_PKG_NAME"),
            "version": env!("CARGO_PKG_VERSION"),
        }).to_string().into_bytes(),
    };
    let json = serde_json::to_string(&resp).unwrap_or_default();
    pack_string(json)
}

#[unsafe(no_mangle)]
pub extern "C" fn handle_event(ptr: u32, len: u32) -> u64 {
    let _input = unpack_string(ptr, len);
    pack_string("{\"status\":\"processed\"}".to_string())
}

#[unsafe(no_mangle)]
pub extern "C" fn allocate(size: usize) -> *mut u8 {
    let mut buf = Vec::with_capacity(size);
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr
}

#[unsafe(no_mangle)]
pub extern "C" fn deallocate(ptr: *mut u8, size: usize) {
    if !ptr.is_null() {
        unsafe { drop(Vec::from_raw_parts(ptr, 0, size)) };
    }
}

fn pack_string(s: String) -> u64 {
    let bytes = s.into_bytes();
    let len = bytes.len() as u64;
    let ptr = bytes.as_ptr() as u64;
    std::mem::forget(bytes);
    (len << 32) | (ptr & 0xFFFF_FFFF)
}

fn unpack_string(ptr: u32, len: u32) -> String {
    unsafe {
        let slice = std::slice::from_raw_parts(ptr as *const u8, len as usize);
        String::from_utf8_lossy(slice).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_routes_validity() {
        let r = routes();
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].path, "/health");
    }

    #[test]
    fn test_subscriptions_validity() {
        let s = subscriptions();
        assert_eq!(s.len(), 2);
    }
}
"#
    };
    fs::write(target_dir.join("src/lib.rs"), lib_rs)?;

    println!("✓ Successfully created Fluxcell '{name}'!");
    println!("\nNext steps:");
    println!("  cd {}", target_dir.display());
    println!("  fluxcell build");
    println!("  fluxcell deploy --dev-upload");

    Ok(())
}

fn run_build(project_dir: Option<&Path>, release: bool) -> Result<PathBuf> {
    let base_dir = project_dir.unwrap_or_else(|| Path::new("."));
    println!("⚡ Compiling Fluxcell at {} to wasm32-wasip1...", base_dir.display());

    let mut args = vec!["build", "--target", "wasm32-wasip1"];
    if release {
        args.push("--release");
    }

    let manifest_path = base_dir.join("Cargo.toml");
    let manifest_str = manifest_path.to_string_lossy().to_string();
    if manifest_path.exists() {
        args.push("--manifest-path");
        args.push(&manifest_str);
    }

    let status = Command::new("cargo")
        .args(&args)
        .status()
        .context("Failed to invoke cargo build. Ensure Rust and cargo are installed.")?;

    if !status.success() {
        bail!("Compilation failed.");
    }

    let profile = if release { "release" } else { "debug" };
    let base_target = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| base_dir.join("target"));
    let target_dir = base_target.join("wasm32-wasip1").join(profile);

    let mut wasm_file = None;
    if let Ok(entries) = fs::read_dir(&target_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) == Some("wasm") {
                wasm_file = Some(p);
                break;
            }
        }
    }

    let wasm_path = wasm_file.ok_or_else(|| {
        anyhow::anyhow!("No .wasm file found in {}", target_dir.display())
    })?;

    let bytes = fs::read(&wasm_path)?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let hash = hex::encode(hasher.finalize());

    let size_kb = bytes.len() as f64 / 1024.0;
    println!("✓ Built: {} ({:.2} KB)", wasm_path.display(), size_kb);
    println!("✓ SHA-256: {}", hash);

    Ok(wasm_path)
}

async fn run_deploy(
    gateway: &str,
    flux_url: &str,
    token: Option<&str>,
    mount: Option<&str>,
    name: Option<&str>,
    artifact_url: Option<&str>,
    sha256: Option<&str>,
    dev_upload: bool,
    activate: bool,
) -> Result<()> {
    let client = reqwest::Client::new();

    // Infer crate name from Cargo.toml if not passed
    let cell_name = if let Some(n) = name {
        n.to_string()
    } else if Path::new("Cargo.toml").exists() {
        let content = fs::read_to_string("Cargo.toml")?;
        content
            .lines()
            .find(|line| line.starts_with("name ="))
            .and_then(|l| l.split('=').nth(1))
            .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string())
            .unwrap_or_else(|| "fluxcell-app".to_string())
    } else {
        "fluxcell-app".to_string()
    };

    let mount_path = mount.unwrap_or("/");

    if dev_upload {
        println!("⚡ Direct Dev Upload: Uploading local .wasm to {flux_url}...");
        let wasm_path = run_build(None, true)?;
        let bytes = fs::read(&wasm_path)?;

        let url = format!(
            "{}/_flux/deployer/upload?name={}&mount_path={}&auto_activate={}",
            flux_url.trim_end_matches('/'),
            cell_name,
            mount_path,
            activate
        );

        let mut req = client.post(&url).body(bytes);
        if let Some(t) = token {
            req = req.header("Authorization", format!("Bearer {t}"));
        }

        let resp = req.send().await.context("Failed to connect to Spectral Flux chassis")?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();

        if status.is_success() {
            println!("✓ Successfully uploaded '{cell_name}'! Response: {text}");
        } else {
            bail!("Dev upload rejected (HTTP {status}): {text}");
        }
        return Ok(());
    }

    let remote_url = artifact_url.ok_or_else(|| {
        anyhow::anyhow!("Missing --artifact-url (or pass --dev-upload for local development). Example: --artifact-url https://github.com/my-org/cell/releases/download/v1.0.0/cell.wasm")
    })?;

    let hash = if let Some(h) = sha256 {
        h.to_string()
    } else if let Ok(wasm_path) = run_build(None, true) {
        let bytes = fs::read(wasm_path)?;
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        hex::encode(hasher.finalize())
    } else {
        bail!("Missing --sha256 checksum for remote artifact.");
    };

    println!("⚡ Submitting Mode B deployFluxcell mutation to {gateway}/graphql...");
    let mutation = format!(
        r#"mutation {{ deployFluxcell(input: {{ name: "{}", artifactUrl: "{}", sha256: "{}", mountPath: "{}", autoActivate: {} }}) {{ commandId status hlc }} }}"#,
        cell_name, remote_url, hash, mount_path, activate
    );

    let mut req = client
        .post(format!("{}/graphql", gateway.trim_end_matches('/')))
        .json(&serde_json::json!({ "query": mutation }));

    if let Some(t) = token {
        req = req.header("Authorization", format!("Bearer {t}"));
    }

    let resp = req.send().await.context("Failed to connect to SpectraGQL gateway")?;
    let status = resp.status();
    let body: serde_json::Value = resp.json().await.context("Failed to parse GraphQL response")?;

    if !status.is_success() || body.get("errors").is_some() {
        bail!("Deployment mutation rejected (HTTP {status}): {body:#}");
    }

    println!("✓ Receipt Received:");
    println!("{:#}", body["data"]["deployFluxcell"]);

    if !activate {
        println!("\nCell is STAGED. To activate, approve in SpectraHub Admin UI or run:");
        println!("  fluxcell deploy --name {cell_name} --artifact-url {remote_url} --activate");
    }

    Ok(())
}

async fn query_status(endpoint: &str) -> Result<()> {
    println!("⚡ Querying status from {endpoint}...");
    let client = reqwest::Client::new();
    let url = format!("{}/_flux/deployer/status", endpoint.trim_end_matches('/'));

    let resp = client.get(&url).send().await.context("Failed to connect to status endpoint")?;
    let status = resp.status();
    let body: serde_json::Value = resp.json().await.context("Failed to parse status JSON")?;

    if !status.is_success() {
        bail!("Status request failed (HTTP {status}): {body:#}");
    }

    println!("─────────────────────────────────────────────────────────────");
    println!("  SPECTRAL FLUX STATUS & GOVERNANCE");
    println!("─────────────────────────────────────────────────────────────");
    if let Some(guard) = body.get("guard") {
        println!("  Killswitch (External Deploy): {}", guard["external_deploy_enabled"]);
        println!("  Killswitch (Dev Upload):      {}", guard["dev_upload_enabled"]);
    }

    println!("\n  ACTIVE FLUXCELLS:");
    if let Some(active) = body.get("active_fluxcells").and_then(|a| a.as_array()) {
        if active.is_empty() {
            println!("    (None)");
        } else {
            for cell in active {
                println!(
                    "    • {} v{} (Mount: {})",
                    cell["name"].as_str().unwrap_or(""),
                    cell["version"].as_str().unwrap_or(""),
                    cell["mount_path"].as_str().unwrap_or("")
                );
            }
        }
    }

    println!("\n  STAGED PENDING APPROVAL:");
    if let Some(staged) = body.get("staged_fluxcells").and_then(|s| s.as_array()) {
        if staged.is_empty() {
            println!("    (None)");
        } else {
            for cell in staged {
                println!(
                    "    • {} v{} (SHA: {})",
                    cell["name"].as_str().unwrap_or(""),
                    cell["version"].as_str().unwrap_or(""),
                    cell["sha256"].as_str().unwrap_or("")
                );
            }
        }
    }
    println!("─────────────────────────────────────────────────────────────");

    Ok(())
}

async fn trigger_lockdown(endpoint: &str) -> Result<()> {
    println!("🚨 TRIGGERING EMERGENCY LOCKDOWN on {endpoint}...");
    let client = reqwest::Client::new();
    let url = format!("{}/admin/api/v1/security/lockdown", endpoint.trim_end_matches('/'));

    let resp = client.post(&url).send().await.context("Failed to contact lockdown endpoint")?;
    let status = resp.status();
    let body: serde_json::Value = resp.json().await.context("Failed to parse response")?;

    if status.is_success() {
        println!("🛑 LOCKDOWN SUCCESSFUL: All deployment ingress has been instantly terminated.");
        println!("{:#}", body);
    } else {
        bail!("Lockdown request failed (HTTP {status}): {body:#}");
    }

    Ok(())
}
