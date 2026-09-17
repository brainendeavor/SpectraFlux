use anyhow::{Context, Result};
use hyper_util::rt::TokioIo;
use hyper_util::server::conn::auto::Builder as ServerBuilder;
use spectra_flux::broker::create_broker;
use spectra_flux::config::FluxConfig;
use spectra_flux::db::{DatabaseRegistry, PostgresDb};
use spectra_flux::deployer::{DeployerGuard, DeployerRegistry, FluxcellDeployer, FluxcellStatus};
use spectra_flux::http::{FluxRouter, RouteDefinition};
use spectra_flux::storage::create_storage;
use spectra_flux::telemetry::TelemetryClient;
use spectra_flux::wasm::{CircuitBreakerConfig, FluxcellWasmConfig, WasmHost};
use std::net::SocketAddr;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    log::info!("🚀 Initializing Spectral Flux Engine...");

    // 1. Load Configuration
    let config_path = std::env::var("FLUX_CONFIG").unwrap_or_else(|_| "spectral-flux.toml".to_string());
    let (config, resolved_config_path) = match FluxConfig::load_from_file_with_source(&config_path) {
        Ok(res) => {
            log::info!("Loaded configuration from '{}'", res.1);
            res
        }
        Err(e) => {
            log::warn!("Could not load '{}' ({}). Falling back to default configuration.", config_path, e);
            (FluxConfig::default_local(), "built-in default (in-memory)".to_string())
        }
    };

    let config_summary = Arc::new(config.to_sanitized_json(&resolved_config_path));

    let worker_id = format!("spectral-flux-{}", uuid::Uuid::now_v7());
    log::info!("Instance Worker ID: {}", worker_id);

    // 2. Initialize Telemetry Client
    let telemetry = Arc::new(TelemetryClient::new(
        worker_id.clone(),
        config.broker.method.clone(),
        config.broker.stream.clone(),
        200,
    ));

    telemetry.record_log("INFO", "Spectral Flux engine initializing", None);

    // Start heartbeat reporter if gateway admin URL configured
    let stop_signal = Arc::new(AtomicBool::new(false));
    if let Some(admin_url) = &config.gateway_admin_url {
        log::info!("Configuring telemetry heartbeat to Gateway at '{}'...", admin_url);
        telemetry.clone().start_heartbeat_task(
            admin_url.clone(),
            Duration::from_secs(3),
            stop_signal.clone(),
        );
    }

    // 3. Initialize Unified Storage
    log::info!("Initializing storage backend '{}'...", config.storage.backend);
    let storage = create_storage(&config.storage.backend, config.storage.addr.as_deref())
        .await
        .context("Failed to initialize storage engine")?;
    telemetry.record_log("INFO", &format!("Storage backend '{}' ready", config.storage.backend), None);
    let trace_storage = Arc::new(spectra_flux::telemetry::DomainTraceStorage::new(storage.clone()));

    // 3.5. Initialize Database Registry
    let mut db_registry = DatabaseRegistry::new();
    for (name, db_cfg) in &config.databases {
        log::info!("Connecting to database '{}'...", name);
        match PostgresDb::new(&db_cfg.url, db_cfg.max_connections) {
            Ok(db) => {
                let is_default = name == "default";
                db_registry.register(name, Arc::new(db), is_default);
                log::info!("Database '{}' registered", name);
            }
            Err(e) => {
                log::warn!("Failed to initialize database '{}': {:#}", name, e);
            }
        }
    }
    if db_registry.is_empty() {
        if let Some(db_url) = &config.database.url {
            match PostgresDb::new(db_url, config.database.max_connections) {
                Ok(db) => {
                    db_registry.register("default", Arc::new(db), true);
                    log::info!("Registered default database from config.database");
                }
                Err(e) => {
                    log::warn!("Failed to initialize default database: {:#}", e);
                }
            }
        }
    }

    // 4. Initialize WASM Host
    log::info!("Initializing Wasmtime Host Engine with epoch interruption...");
    let wasm_host = Arc::new(
        WasmHost::with_db(5, Some(storage.clone()), Some(Arc::new(db_registry)))
            .context("Failed to initialize WASM host engine")?,
    );

    // 5. Initialize Router with Collision Detection
    let mut router = FluxRouter::new();

    for (name, cell_cfg) in &config.fluxcells {
        if !cell_cfg.enabled {
            continue;
        }

        log::info!("Loading fluxcell '{}' mounted at '{}'...", name, cell_cfg.mount_path);

        // Check if wasm module file exists
        if std::path::Path::new(&cell_cfg.wasm_module).exists() {
            let bytes = std::fs::read(&cell_cfg.wasm_module)
                .with_context(|| format!("Failed to read WASM module '{}'", cell_cfg.wasm_module))?;

            let resolved = config.resolve_cell_execution(name, cell_cfg, None, None, None)?;
            let wasm_cfg = FluxcellWasmConfig {
                profile: resolved.profile,
                timeout_ms: resolved.timeout_ms,
                max_memory_bytes: resolved.max_memory_bytes,
                max_instances: resolved.max_instances,
                offload: resolved.offload,
                circuit_breaker: CircuitBreakerConfig::default(),
            };

            wasm_host
                .register_wasm_bytes_with_subs(
                    name,
                    &bytes,
                    wasm_cfg,
                    cell_cfg.subscriptions.clone(),
                )
                .with_context(|| format!("Failed to register WASM module '{}'", cell_cfg.wasm_module))?;

            if let Some(routes) = wasm_host.get_fluxcell_routes(name) {
                let route_count = routes.len();
                router
                    .register_fluxcell_routes(name, &cell_cfg.mount_path, &routes)
                    .with_context(|| format!("Route collision detected for fluxcell '{}'", name))?;
                log::info!("Registered {} WASM routes for '{}' at '{}'", route_count, name, cell_cfg.mount_path);
                telemetry.record_log(
                    "INFO",
                    &format!("✨ Fluxcell '{}' is ALIVE: mounted at '{}' with {} WASM route(s)", name, cell_cfg.mount_path, route_count),
                    None,
                );
            } else {
                log::warn!("No routes exported by WASM fluxcell '{}'", name);
                telemetry.record_log(
                    "WARN",
                    &format!("Fluxcell '{}' loaded but exported 0 routes", name),
                    None,
                );
            }
        } else {
            // Built-in fallback routes for out-of-the-box fluxcells
            let routes = match name.as_str() {
                "magic_link" | "magic-link" => vec![
                    RouteDefinition::new("GET", "/verify", "Verify magic link token"),
                    RouteDefinition::new("POST", "/verify", "Redeem magic link token"),
                    RouteDefinition::new("GET", "/status", "Auth service status"),
                ],
                "webhook" => vec![
                    RouteDefinition::new("GET", "/health", "Webhook service health"),
                    RouteDefinition::new("GET", "/dlq", "Dead-letter queue status"),
                    RouteDefinition::new("POST", "/test", "Test webhook delivery"),
                ],
                _ => Vec::new(),
            };

            if !routes.is_empty() {
                router
                    .register_fluxcell_routes(name, &cell_cfg.mount_path, &routes)
                    .with_context(|| format!("Route collision detected for fluxcell '{}'", name))?;
                log::info!("Registered {} built-in routes for fluxcell '{}'", routes.len(), name);
                telemetry.record_log(
                    "INFO",
                    &format!("✨ Fluxcell '{}' is ALIVE: mounted at '{}' with {} built-in route(s)", name, cell_cfg.mount_path, routes.len()),
                    None,
                );
            }
        }
    }

    let shared_router = Arc::new(RwLock::new(router));

    // 6. Initialize Deployer Subsystem (if enabled)
    let deployer: Option<Arc<FluxcellDeployer>> = if config.deployer.enabled {
        log::info!("Initializing Fluxcell Deployer subsystem (storage: {:?})...", config.deployer.storage_dir);
        let guard = Arc::new(DeployerGuard::new(
            config.deployer.external_deploy_enabled,
            config.deployer.dev_upload_enabled,
        ));
        let registry = Arc::new(DeployerRegistry::new(&config.deployer.storage_dir)?);
        let dep = Arc::new(FluxcellDeployer::new(
            config.deployer.clone(),
            guard,
            registry.clone(),
            wasm_host.clone(),
            shared_router.clone(),
        ));

        // Boot-load persisted active cells from fluxcells.json
        let persisted = registry.list_records();
        for record in persisted {
            if record.status == FluxcellStatus::Active {
                let wasm_file = registry.storage_dir().join(&record.wasm_file);
                if wasm_file.exists() {
                    match std::fs::read(&wasm_file) {
                        Ok(bytes) => {
                            let wasm_cfg = FluxcellWasmConfig {
                                profile: record.profile.clone(),
                                timeout_ms: record.timeout_ms,
                                max_memory_bytes: record.max_memory_bytes,
                                max_instances: record.max_instances,
                                offload: record.offload,
                                circuit_breaker: CircuitBreakerConfig::default(),
                            };
                            if let Err(e) = wasm_host.register_wasm_bytes(&record.name, &bytes, wasm_cfg) {
                                log::error!("Failed to register persisted fluxcell '{}': {}", record.name, e);
                            } else {
                                let mut router_lock = shared_router.write().unwrap_or_else(|e| e.into_inner());
                                if let Err(e) = router_lock.register_fluxcell_routes(&record.name, &record.mount_path, &record.routes) {
                                    log::error!("Failed to mount routes for persisted fluxcell '{}': {}", record.name, e);
                                } else {
                                    log::info!("Restored active fluxcell '{}' (v{}) mounted at '{}'", record.name, record.version, record.mount_path);
                                }
                            }
                        }
                        Err(e) => log::error!("Failed to read persisted wasm file {:?}: {}", wasm_file, e),
                    }
                }
            }
        }

        Some(dep)
    } else {
        log::info!("Fluxcell Deployer subsystem is disabled in configuration.");
        None
    };

    // 7. Connect Broker Consumer Loop
    log::info!("Connecting to broker '{}' at '{}'...", config.broker.method, config.broker.addr);
    match create_broker(&config.broker).await {
        Ok(broker) => {
            let subjects = vec![
                "mutation.>".to_string(),
                "webhook.>".to_string(),
                "auth.>".to_string(),
                "deployer.>".to_string(),
            ];
            let group = config.broker.consumer_group.clone();
            let processor = Arc::new(spectra_flux::broker::EventProcessor::new(
                broker,
                wasm_host.clone(),
                telemetry.clone(),
                storage.clone(),
                deployer.clone(),
                Some(trace_storage.clone()),
                config.resilience.clone(),
                worker_id.clone(),
            ));
            processor.spawn_consumer_loop(subjects, group);
        }
        Err(e) => {
            log::warn!("Broker connection failed: {}. Continuing in standalone API mode.", e);
        }
    }

    // 8. Start HTTP Server (:8081)
    let addr: SocketAddr = format!("{}:{}", config.host, config.port).parse()?;
    let listener = TcpListener::bind(addr).await?;
    log::info!("🚀 Spectral Flux HTTP Support Server listening on http://{}", addr);
    telemetry.record_log("INFO", &format!("🚀 Spectral Flux HTTP server listening on http://{}", addr), None);

    let router_arc = shared_router.clone();
    let telemetry_arc = telemetry.clone();
    let wasm_dispatcher = wasm_host.clone();
    let deployer_arc = deployer.clone();
    let trace_storage_arc = trace_storage.clone();
    let config_summary_arc = config_summary.clone();

    tokio::select! {
        _ = async {
            loop {
                let (stream, _) = match listener.accept().await {
                    Ok(res) => res,
                    Err(e) => {
                        log::error!("TCP accept error: {}", e);
                        continue;
                    }
                };

                let io = TokioIo::new(stream);
                let router_clone = router_arc.clone();
                let tele_clone = telemetry_arc.clone();
                let dispatcher_clone = wasm_dispatcher.clone();
                let deployer_clone = deployer_arc.clone();
                let trace_storage_clone = trace_storage_arc.clone();
                let config_summary_clone = config_summary_arc.clone();

                tokio::spawn(async move {
                    let service = hyper::service::service_fn(move |req| {
                        spectra_flux::http::handle_request_with_config(
                            req,
                            router_clone.clone(),
                            tele_clone.clone(),
                            dispatcher_clone.clone(),
                            deployer_clone.clone(),
                            Some(trace_storage_clone.clone()),
                            Some(config_summary_clone.clone()),
                        )
                    });

                    if let Err(err) = ServerBuilder::new(hyper_util::rt::TokioExecutor::new())
                        .serve_connection(io, service)
                        .await
                    {
                        log::debug!("HTTP connection closed: {:?}", err);
                    }
                });
            }
        } => {}
        _ = tokio::signal::ctrl_c() => {
            log::info!("Shutting down Spectral Flux Engine gracefully...");
            stop_signal.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    log::info!("Spectral Flux Engine shutdown complete.");
    Ok(())
}
