use clap::Parser;
use fluxcell_cli::{scaffold_fluxcell, Cli, Commands};
use std::fs;
use tempfile::tempdir;

#[test]
fn test_cli_argument_parsing() {
    // 1. New command - Rust
    let cli = Cli::try_parse_from([
        "fluxcell",
        "new",
        "order-auditor",
        "--lang",
        "rust",
        "--template",
        "minimal",
    ])
    .expect("Failed to parse new command");

    match cli.command {
        Commands::New { name, lang, template, .. } => {
            assert_eq!(name, "order-auditor");
            assert_eq!(lang, "rust");
            assert_eq!(template, "minimal");
        }
        _ => panic!("Expected Commands::New"),
    }

    // 2. New command - TypeScript
    let cli_ts = Cli::try_parse_from([
        "fluxcell",
        "new",
        "audit-ts",
        "--lang",
        "ts",
    ])
    .expect("Failed to parse TypeScript new command");

    match cli_ts.command {
        Commands::New { name, lang, .. } => {
            assert_eq!(name, "audit-ts");
            assert_eq!(lang, "ts");
        }
        _ => panic!("Expected Commands::New"),
    }

    // 3. Build command
    let cli_build = Cli::try_parse_from(["fluxcell", "build", "--release"])
        .expect("Failed to parse build command");
    match cli_build.command {
        Commands::Build { release, path } => {
            assert!(release);
            assert!(path.is_none());
        }
        _ => panic!("Expected Commands::Build"),
    }

    // 4. Deploy command
    let cli_deploy = Cli::try_parse_from([
        "fluxcell",
        "deploy",
        "--gateway",
        "http://127.0.0.1:9000",
        "--artifact-url",
        "https://github.com/my-org/cell/releases/v1.0.0/cell.wasm",
        "--sha256",
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        "--mount",
        "/api/audits",
        "--activate",
    ])
    .expect("Failed to parse deploy command");

    match cli_deploy.command {
        Commands::Deploy {
            gateway,
            artifact_url,
            sha256,
            mount,
            activate,
            ..
        } => {
            assert_eq!(gateway, "http://127.0.0.1:9000");
            assert_eq!(
                artifact_url.as_deref(),
                Some("https://github.com/my-org/cell/releases/v1.0.0/cell.wasm")
            );
            assert_eq!(
                sha256.as_deref(),
                Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
            );
            assert_eq!(mount.as_deref(), Some("/api/audits"));
            assert!(activate);
        }
        _ => panic!("Expected Commands::Deploy"),
    }

    // 5. Status command
    let cli_status = Cli::try_parse_from(["fluxcell", "status"])
        .expect("Failed to parse status command");
    match cli_status.command {
        Commands::Status { endpoint } => {
            assert_eq!(endpoint, "http://127.0.0.1:8081");
        }
        _ => panic!("Expected Commands::Status"),
    }

    // 6. Lockdown command
    let cli_lockdown = Cli::try_parse_from(["fluxcell", "lockdown"])
        .expect("Failed to parse lockdown command");
    match cli_lockdown.command {
        Commands::Lockdown { endpoint } => {
            assert_eq!(endpoint, "http://127.0.0.1:8081");
        }
        _ => panic!("Expected Commands::Lockdown"),
    }
}

#[test]
fn test_scaffold_rust_minimal() {
    let tmp = tempdir().expect("Failed to create tempdir");
    let target = tmp.path().join("order-auditor");

    scaffold_fluxcell("order-auditor", &target, "minimal", "rust")
        .expect("Scaffolding minimal rust fluxcell failed");

    // Verify key files created
    assert!(target.join("Cargo.toml").exists(), "Cargo.toml must exist");
    assert!(target.join("build.rs").exists(), "build.rs must exist");
    assert!(target.join("build.sh").exists(), "build.sh must exist");
    assert!(target.join("wit/fluxcell.wit").exists(), "wit/fluxcell.wit must exist");
    assert!(target.join("src/lib.rs").exists(), "src/lib.rs must exist");
    assert!(target.join(".gitignore").exists(), ".gitignore must exist");
    assert!(
        target.join(".github/workflows/release.yml").exists(),
        "GitHub release workflow must exist"
    );

    // Verify Cargo.toml substitutions
    let cargo_toml = fs::read_to_string(target.join("Cargo.toml")).unwrap();
    assert!(cargo_toml.contains("name = \"order-auditor\""));
    assert!(cargo_toml.contains("order-auditor WebAssembly Fluxcell"));

    // Verify lib.rs contains export_fluxcell!
    let lib_rs = fs::read_to_string(target.join("src/lib.rs")).unwrap();
    assert!(lib_rs.contains("export_fluxcell!"));
    assert!(lib_rs.contains("impl Fluxcell for"));

    // Verify workflow substitutions
    let workflow = fs::read_to_string(target.join(".github/workflows/release.yml")).unwrap();
    assert!(workflow.contains("order-auditor.wasm"));
    assert!(workflow.contains("order-auditor.wasm.sha256"));
}

#[test]
fn test_scaffold_rust_mailer() {
    let tmp = tempdir().expect("Failed to create tempdir");
    let target = tmp.path().join("invoice-mailer");

    scaffold_fluxcell("invoice-mailer", &target, "mailer", "rust")
        .expect("Scaffolding mailer rust fluxcell failed");

    assert!(target.join("src/lib.rs").exists());
    let lib_rs = fs::read_to_string(target.join("src/lib.rs")).unwrap();
    assert!(
        lib_rs.contains("InvoicePayload") || lib_rs.contains("render_invoice_email") || lib_rs.contains("mailer"),
        "Mailer template must contain mailer-specific definitions"
    );
}

#[test]
fn test_scaffold_typescript() {
    let tmp = tempdir().expect("Failed to create tempdir");
    let target = tmp.path().join("my-ts-cell");

    scaffold_fluxcell("my-ts-cell", &target, "minimal", "typescript")
        .expect("Scaffolding typescript fluxcell failed");

    // Verify key files created
    assert!(target.join("package.json").exists(), "package.json must exist");
    assert!(target.join("asconfig.json").exists(), "asconfig.json must exist");
    assert!(target.join("tsconfig.json").exists(), "tsconfig.json must exist");
    assert!(target.join("build.sh").exists(), "build.sh must exist");
    assert!(target.join("wit/fluxcell.wit").exists(), "wit/fluxcell.wit must exist");
    assert!(target.join("assembly/index.ts").exists(), "assembly/index.ts must exist");
    assert!(target.join("README.md").exists(), "README.md must exist");
    assert!(target.join(".gitignore").exists(), ".gitignore must exist");

    // Verify package.json substitutions
    let pkg_json = fs::read_to_string(target.join("package.json")).unwrap();
    assert!(pkg_json.contains("\"name\": \"my-ts-cell\""));

    // Verify assembly/index.ts contains registerFluxcell
    let index_ts = fs::read_to_string(target.join("assembly/index.ts")).unwrap();
    assert!(index_ts.contains("registerFluxcell"));
    assert!(index_ts.contains("export class"));
}
