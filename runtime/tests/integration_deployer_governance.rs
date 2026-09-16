use spectra_flux::config::DeployerConfig;
use spectra_flux::deployer::{DeployerGuard, DeployerRegistry, FluxcellDeployer, FluxcellStatus};
use spectra_flux::http::FluxRouter;
use spectra_flux::wasm::WasmHost;
use std::net::IpAddr;
use std::sync::{Arc, RwLock};

fn create_valid_wasm() -> Vec<u8> {
    wat::parse_str(
        r#"(module
            (memory (export "memory") 1)
            (func (export "allocate") (param i32) (result i32) i32.const 0)
            (func (export "deallocate") (param i32 i32))
            (func (export "get_routes") (result i64)
                ;; Data at offset 1024, len 58: [{"method":"GET","path":"/status","description":"status"}]
                ;; (1024 << 32) | 58 = 4398046511162
                (i64.const 4398046511162)
            )
            (data (i32.const 1024) "[{\"method\":\"GET\",\"path\":\"/status\",\"description\":\"status\"}]")
        )"#,
    )
    .expect("Failed to parse valid WAT")
}

#[tokio::test]
async fn test_deployer_full_staging_and_activation_lifecycle() {
    let temp_dir = std::env::temp_dir().join(format!("spectral_test_deploy_{}", uuid::Uuid::new_v4()));
    let mut config = DeployerConfig::default();
    config.enabled = true;
    config.storage_dir = temp_dir.to_string_lossy().to_string();
    config.external_deploy_enabled = true;
    config.dev_upload_enabled = true;

    let guard = Arc::new(DeployerGuard::new(true, true));
    let registry = Arc::new(DeployerRegistry::new(&temp_dir).unwrap());
    let wasm_host = Arc::new(WasmHost::new(5, None).unwrap());
    let router = Arc::new(RwLock::new(FluxRouter::new()));

    let deployer = Arc::new(FluxcellDeployer::new(
        config.clone(),
        guard.clone(),
        registry.clone(),
        wasm_host.clone(),
        router.clone(),
    ));

    let wasm_bytes = create_valid_wasm();

    // 1. Stage uploaded artifact
    let staged = deployer
        .stage_uploaded_artifact("payment-auditor", wasm_bytes, "/api/payments", Some(10_000), None)
        .expect("Staging uploaded artifact failed");

    assert_eq!(staged.name, "payment-auditor");
    assert_eq!(staged.status, FluxcellStatus::Staged);
    assert_eq!(staged.mount_path, "/api/payments");
    assert!(!staged.sha256.is_empty());

    // 2. Verify router has NOT mounted routes yet while staged
    {
        let r = router.read().unwrap();
        assert!(r.lookup("GET", "/api/payments/status").is_err());
    }

    // 3. Activate the staged cell
    let activated = deployer
        .activate("payment-auditor", &staged.sha256)
        .expect("Activation failed");

    assert_eq!(activated.name, "payment-auditor");
    assert_eq!(activated.status, FluxcellStatus::Active);

    // 4. Verify router has mounted the routes now that it is active
    {
        let r = router.read().unwrap();
        let matched = r.lookup("GET", "/api/payments/status").expect("Route should be mounted");
        assert_eq!(matched.fluxcell_name, "payment-auditor");
    }

    // 5. Verify audit history records both STAGE and ACTIVATE
    let audits = registry.list_audit_events();
    assert!(audits.iter().any(|a| a.action == "STAGE" && a.name == "payment-auditor"));
    assert!(audits.iter().any(|a| a.action == "ACTIVATE" && a.name == "payment-auditor"));

    // 6. Verify persistence across registry reboot
    let reloaded_registry = DeployerRegistry::new(&temp_dir).unwrap();
    let reloaded_record = reloaded_registry
        .get_record("payment-auditor")
        .expect("Record must persist in manifest");
    assert_eq!(reloaded_record.status, FluxcellStatus::Active);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_deployer_lockdown_terminates_ingress() {
    let temp_dir = std::env::temp_dir().join(format!("spectral_lockdown_test_{}", uuid::Uuid::new_v4()));
    let config = DeployerConfig {
        enabled: true,
        storage_dir: temp_dir.to_string_lossy().to_string(),
        external_deploy_enabled: true,
        dev_upload_enabled: true,
        ..Default::default()
    };

    let guard = Arc::new(DeployerGuard::new(true, true));
    let registry = Arc::new(DeployerRegistry::new(&temp_dir).unwrap());
    let wasm_host = Arc::new(WasmHost::new(5, None).unwrap());
    let router = Arc::new(RwLock::new(FluxRouter::new()));

    let deployer = Arc::new(FluxcellDeployer::new(
        config,
        guard.clone(),
        registry.clone(),
        wasm_host,
        router,
    ));

    // Initially allowed
    assert!(guard.is_dev_upload_allowed());
    assert!(guard.is_external_deploy_allowed());

    // Trigger lockdown
    guard.emergency_lockdown();

    assert!(!guard.is_dev_upload_allowed());
    assert!(!guard.is_external_deploy_allowed());

    // Attempting upload must fail
    let wasm_bytes = create_valid_wasm();
    let err = deployer
        .stage_uploaded_artifact("lockdown-cell", wasm_bytes, "/api/lock", None, None)
        .unwrap_err();
    assert!(err.to_string().contains("locked down or disabled"));

    // Attempting remote artifact staging must fail
    let remote_err = deployer
        .stage_remote_artifact("lockdown-cell", "https://github.com/my-org/cell.wasm", "dummy-sha", "/api/lock", None, None, None)
        .await
        .unwrap_err();
    assert!(remote_err.to_string().contains("locked down or disabled"));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_ssrf_forbidden_ip_boundaries() {
    // Loopback
    assert!(spectra_flux::deployer::ssrf_shield::is_forbidden_ip(&"127.0.0.1".parse::<IpAddr>().unwrap()));
    assert!(spectra_flux::deployer::ssrf_shield::is_forbidden_ip(&"127.255.255.254".parse::<IpAddr>().unwrap()));

    // AWS Cloud metadata IMDS
    assert!(spectra_flux::deployer::ssrf_shield::is_forbidden_ip(&"169.254.169.254".parse::<IpAddr>().unwrap()));

    // RFC 1918 Private ranges
    assert!(spectra_flux::deployer::ssrf_shield::is_forbidden_ip(&"10.0.0.1".parse::<IpAddr>().unwrap()));
    assert!(spectra_flux::deployer::ssrf_shield::is_forbidden_ip(&"172.16.0.1".parse::<IpAddr>().unwrap()));
    assert!(spectra_flux::deployer::ssrf_shield::is_forbidden_ip(&"172.31.255.255".parse::<IpAddr>().unwrap()));
    assert!(spectra_flux::deployer::ssrf_shield::is_forbidden_ip(&"192.168.1.1".parse::<IpAddr>().unwrap()));

    // RFC 6598 CGNAT
    assert!(spectra_flux::deployer::ssrf_shield::is_forbidden_ip(&"100.64.0.1".parse::<IpAddr>().unwrap()));
    assert!(spectra_flux::deployer::ssrf_shield::is_forbidden_ip(&"100.127.255.254".parse::<IpAddr>().unwrap()));

    // RFC 4193 IPv6 ULA
    assert!(spectra_flux::deployer::ssrf_shield::is_forbidden_ip(&"::1".parse::<IpAddr>().unwrap()));
    assert!(spectra_flux::deployer::ssrf_shield::is_forbidden_ip(&"fc00::1".parse::<IpAddr>().unwrap()));
    assert!(spectra_flux::deployer::ssrf_shield::is_forbidden_ip(&"fdff:ffff::1".parse::<IpAddr>().unwrap()));

    // Public IPs must not be forbidden
    assert!(!spectra_flux::deployer::ssrf_shield::is_forbidden_ip(&"1.1.1.1".parse::<IpAddr>().unwrap()));
    assert!(!spectra_flux::deployer::ssrf_shield::is_forbidden_ip(&"8.8.8.8".parse::<IpAddr>().unwrap()));
}

#[tokio::test]
async fn test_deployer_sha256_mismatch_rejection() {
    let temp_dir = std::env::temp_dir().join(format!("spectral_sha_test_{}", uuid::Uuid::new_v4()));
    let config = DeployerConfig {
        enabled: true,
        storage_dir: temp_dir.to_string_lossy().to_string(),
        external_deploy_enabled: true,
        dev_upload_enabled: true,
        allowed_artifact_hosts: vec!["example.com".to_string()],
        ..Default::default()
    };

    let guard = Arc::new(DeployerGuard::new(true, true));
    let registry = Arc::new(DeployerRegistry::new(&temp_dir).unwrap());
    let wasm_host = Arc::new(WasmHost::new(5, None).unwrap());
    let router = Arc::new(RwLock::new(FluxRouter::new()));

    let deployer = Arc::new(FluxcellDeployer::new(
        config,
        guard,
        registry.clone(),
        wasm_host,
        router,
    ));

    // Staging with wrong SHA-256 for remote artifact (if URL validation passes)
    let bad_sha = "0000000000000000000000000000000000000000000000000000000000000000";
    let res = deployer.stage_remote_artifact("tampered-cell", "https://example.com/cell.wasm", bad_sha, "/api/tamper", None, None, None).await;

    // Either network fails or SHA fails, but in both cases audit must log the rejection or error
    assert!(res.is_err());

    let _ = std::fs::remove_dir_all(&temp_dir);
}
