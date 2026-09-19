use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const MIN_TIMEOUT_MS: u64 = 10;
pub const MAX_TIMEOUT_MS: u64 = 300_000;

#[derive(Debug, Clone, Deserialize)]
pub struct FluxConfig {
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default)]
    pub broker: BrokerConfig,
    #[serde(default, alias = "internal-storage", alias = "internal_storage")]
    pub internal_storage: InternalStorageConfig,
    #[serde(default, alias = "fluxcell-storage", alias = "fluxcell_storage")]
    pub fluxcell_storage: FluxcellStorageConfig,
    #[serde(default)]
    pub storage: StorageConfig,
    #[serde(default)]
    pub databases: HashMap<String, DatabaseInstanceConfig>,
    #[serde(default)]
    pub database: DatabaseConfig,
    #[serde(default)]
    pub gateway_admin_url: Option<String>,
    #[serde(default = "default_profiles")]
    pub profiles: HashMap<String, ExecutionProfileConfig>,
    #[serde(default)]
    pub fluxcells: HashMap<String, FluxcellConfig>,
    #[serde(default)]
    pub deployer: DeployerConfig,
    #[serde(default)]
    pub resilience: ResilienceConfig,
    #[serde(default)]
    pub mailer: MailerSectionConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedExecutionConfig {
    pub profile: String,
    pub timeout_ms: u64,
    pub max_instances: usize,
    pub offload: OffloadStrategy,
    pub max_memory_bytes: usize,
}

impl Default for FluxConfig {
    fn default() -> Self {
        Self {
            port: default_port(),
            host: default_host(),
            broker: BrokerConfig::default(),
            internal_storage: InternalStorageConfig::default(),
            fluxcell_storage: FluxcellStorageConfig::default(),
            storage: StorageConfig::default(),
            databases: HashMap::new(),
            database: DatabaseConfig::default(),
            gateway_admin_url: Some("http://127.0.0.1:8000".to_string()),
            profiles: default_profiles(),
            fluxcells: HashMap::new(),
            deployer: DeployerConfig::default(),
            resilience: ResilienceConfig::default(),
            mailer: MailerSectionConfig::default(),
        }
    }
}

fn default_fluxcells() -> HashMap<String, FluxcellConfig> {
    let mut map = HashMap::new();
    map.insert(
        "magic_link".to_string(),
        FluxcellConfig {
            wasm_module: "fluxcells/magic_link.wasm".to_string(),
            mount_path: "/auth".to_string(),
            enabled: true,
            subscriptions: Some(vec![
                "auth.magic_link".to_string(),
                "mutation.requestmagiclink".to_string(),
            ]),
            profile: Some("extended".to_string()),
            timeout_ms: None,
            max_memory_mb: None,
            max_instances: None,
        },
    );
    map.insert(
        "webhook".to_string(),
        FluxcellConfig {
            wasm_module: "fluxcells/webhook.wasm".to_string(),
            mount_path: "/api/webhooks".to_string(),
            enabled: true,
            subscriptions: Some(vec![
                "webhook.dispatch".to_string(),
                "mutation.*".to_string(),
            ]),
            profile: Some("extended".to_string()),
            timeout_ms: None,
            max_memory_mb: None,
            max_instances: None,
        },
    );
    map
}

impl FluxConfig {
    pub fn resolve_config_path(path: &str) -> Option<String> {
        let mut base_paths = vec![path.to_string()];
        if path.contains("spectral-") {
            base_paths.push(path.replace("spectral-", "spectra-"));
        }
        let mut candidates = Vec::new();
        for p in &base_paths {
            candidates.push(p.clone());
            candidates.push(format!("{}.local", p));
            if p.ends_with(".toml") {
                candidates.push(p.replace(".toml", "-local.toml"));
                candidates.push(p.replace(".toml", ".local.toml"));
            }
            if !p.starts_with("runtime/") {
                candidates.push(format!("runtime/{}", p));
                candidates.push(format!("runtime/{}.local", p));
                if p.ends_with(".toml") {
                    candidates.push(format!("runtime/{}", p.replace(".toml", "-local.toml")));
                    candidates.push(format!("runtime/{}", p.replace(".toml", ".local.toml")));
                }
            }
            candidates.push(format!("../{}", p));
            candidates.push(format!("../runtime/{}", p));
        }
        for candidate in candidates {
            if std::path::Path::new(&candidate).exists() {
                return Some(candidate);
            }
        }
        None
    }

    pub fn load_from_file_with_source(path: &str) -> Result<(Self, String)> {
        let apply_env_overrides = |cfg: &mut Self| {
            if let Ok(v) = std::env::var("FLUX_DEPLOYER_ENABLED") {
                if let Ok(b) = v.parse::<bool>() {
                    cfg.deployer.enabled = b;
                }
            }
            // Standard 12-factor DATABASE_URL fallback (e.g. Railway, Supabase, Neon, Render)
            // If FLUX__DATABASE__URL was not explicitly set, honor standard DATABASE_URL or FLUX_DATABASE_URL.
            if std::env::var("FLUX__DATABASE__URL").is_err() {
                if let Ok(db_url) = std::env::var("DATABASE_URL") {
                    if !db_url.trim().is_empty() {
                        cfg.database.url = Some(db_url);
                    }
                } else if let Ok(db_url) = std::env::var("FLUX_DATABASE_URL") {
                    if !db_url.trim().is_empty() {
                        cfg.database.url = Some(db_url);
                    }
                }
            }
            // Storage overrides
            if let Ok(u) = std::env::var("FLUX_INTERNAL_STORAGE_URL").or_else(|_| std::env::var("FLUX__INTERNAL_STORAGE__URL")) {
                if !u.trim().is_empty() {
                    cfg.internal_storage.url = u;
                }
            }
            if let Ok(u) = std::env::var("FLUX_FLUXCELL_STORAGE_URL").or_else(|_| std::env::var("FLUX__FLUXCELL_STORAGE__URL")) {
                if !u.trim().is_empty() {
                    cfg.fluxcell_storage.url = u;
                }
            } else if let Ok(redis_url) = std::env::var("REDIS_URL") {
                if !redis_url.trim().is_empty() {
                    cfg.fluxcell_storage.url = redis_url;
                }
            } else if let Ok(valkey_url) = std::env::var("VALKEY_URL") {
                if !valkey_url.trim().is_empty() {
                    cfg.fluxcell_storage.url = valkey_url;
                }
            }
            cfg.sync_storage_tiers();
        };

        if let Some(resolved_path) = Self::resolve_config_path(path) {
            let settings = config::Config::builder()
                .add_source(config::File::with_name(&resolved_path).format(config::FileFormat::Toml).required(true))
                .add_source(config::Environment::with_prefix("FLUX").separator("__"))
                .build()?;
            let mut cfg: Self = settings.try_deserialize()?;
            apply_env_overrides(&mut cfg);
            for (k, v) in default_profiles() {
                cfg.profiles.entry(k).or_insert(v);
            }
            for (k, v) in default_fluxcells() {
                cfg.fluxcells.entry(k).or_insert(v);
            }
            Ok((cfg, resolved_path))
        } else if path == "spectra-flux.toml" || path == "spectral-flux.toml" || path == "nonexistent.toml" {
            let settings = config::Config::builder()
                .add_source(config::Environment::with_prefix("FLUX").separator("__"))
                .build()?;
            let env_cfg: Result<Self, _> = settings.try_deserialize();
            match env_cfg {
                Ok(mut cfg) => {
                    apply_env_overrides(&mut cfg);
                    for (k, v) in default_profiles() {
                        cfg.profiles.entry(k).or_insert(v);
                    }
                    for (k, v) in default_fluxcells() {
                        cfg.fluxcells.entry(k).or_insert(v);
                    }
                    Ok((cfg, "environment overrides".to_string()))
                }
                Err(_) => {
                    let mut cfg = Self::default_local();
                    apply_env_overrides(&mut cfg);
                    Ok((cfg, "built-in default (in-memory)".to_string()))
                }
            }
        } else {
            Err(anyhow::anyhow!(
                "Configuration file '{}' not found. Checked candidate locations: ['{}', '{}.local', 'runtime/{}', 'runtime/{}.local']",
                path, path, path, path, path
            ))
        }
    }

    pub fn sync_storage_tiers(&mut self) {
        // If legacy [storage] was explicitly specified with redis/valkey or custom addr,
        // synchronize fluxcell_storage.url to match.
        if self.fluxcell_storage.url == default_fluxcell_storage_url() {
            if let Some(addr) = &self.storage.addr {
                self.fluxcell_storage.url = addr.clone();
            } else if self.storage.backend == "redis" || self.storage.backend == "valkey" {
                self.fluxcell_storage.url = "redis://127.0.0.1:6379".to_string();
            }
        }
        // If fluxcell_storage was explicitly configured (not default), reflect it in storage for backwards compatibility
        if self.fluxcell_storage.url != default_fluxcell_storage_url() {
            if self.storage.backend == default_storage_backend() {
                if self.fluxcell_storage.url.starts_with("valkey://") {
                    self.storage.backend = "valkey".to_string();
                } else if self.fluxcell_storage.url.starts_with("redis://") {
                    self.storage.backend = "redis".to_string();
                }
            }
            if self.storage.addr.is_none() {
                self.storage.addr = Some(self.fluxcell_storage.url.clone());
            }
        }
    }

    pub fn load_from_file(path: &str) -> Result<Self> {
        Self::load_from_file_with_source(path).map(|(cfg, _)| cfg)
    }

    pub fn from_toml_str(s: &str) -> Result<Self> {
        let settings = config::Config::builder()
            .add_source(config::File::from_str(s, config::FileFormat::Toml))
            .build()?;
        let mut cfg: Self = settings.try_deserialize()?;
        cfg.sync_storage_tiers();
        for (k, v) in default_profiles() {
            cfg.profiles.entry(k).or_insert(v);
        }
        for (k, v) in default_fluxcells() {
            cfg.fluxcells.entry(k).or_insert(v);
        }
        Ok(cfg)
    }

    pub fn default_local() -> Self {
        Self {
            port: 8081,
            host: "0.0.0.0".to_string(),
            broker: BrokerConfig::default(),
            internal_storage: InternalStorageConfig::default(),
            fluxcell_storage: FluxcellStorageConfig::default(),
            storage: StorageConfig::default(),
            databases: HashMap::new(),
            database: DatabaseConfig::default(),
            gateway_admin_url: Some("http://127.0.0.1:8000".to_string()),
            profiles: default_profiles(),
            fluxcells: default_fluxcells(),
            deployer: DeployerConfig {
                enabled: true,
                external_deploy_enabled: true,
                dev_upload_enabled: true,
                auto_activate: true,
                allowed_artifact_hosts: Vec::new(),
                require_https: false,
                block_private_networks: false,
                max_wasm_size_bytes: default_max_wasm_size(),
                storage_dir: default_storage_dir(),
            },
            resilience: ResilienceConfig::default(),
            mailer: MailerSectionConfig::default(),
        }
    }

    pub fn to_sanitized_json(&self, config_source: &str) -> serde_json::Value {
        let sanitize_db_url = |url: &str| -> String {
            if let Some((prefix, rest)) = url.split_once('@') {
                if let Some((scheme, user)) = prefix.split_once("://") {
                    if user.contains(':') {
                        let user_only = user.split_once(':').map(|(u, _)| u).unwrap_or(user);
                        return format!("{}://{}:***@{}", scheme, user_only, rest);
                    }
                }
                format!("***@{}", rest)
            } else {
                url.to_string()
            }
        };

        let mut dbs = serde_json::Map::new();
        for (name, db) in &self.databases {
            dbs.insert(
                name.clone(),
                serde_json::json!({
                    "url": sanitize_db_url(&db.url),
                    "maxConnections": db.max_connections,
                }),
            );
        }

        let default_db = self.database.url.as_ref().map(|u| {
            serde_json::json!({
                "url": sanitize_db_url(u),
                "maxConnections": self.database.max_connections,
            })
        });

        let mut fcells = serde_json::Map::new();
        for (name, cell) in &self.fluxcells {
            fcells.insert(
                name.clone(),
                serde_json::json!({
                    "mountPath": cell.mount_path,
                    "profile": cell.profile.as_deref().unwrap_or("standard"),
                    "enabled": cell.enabled,
                    "timeoutMs": cell.timeout_ms,
                    "maxMemoryMb": cell.max_memory_mb,
                    "maxInstances": cell.max_instances,
                }),
            );
        }

        serde_json::json!({
            "configSource": config_source,
            "host": self.host,
            "port": self.port,
            "gatewayAdminUrl": self.gateway_admin_url,
            "broker": {
                "method": self.broker.method,
                "addr": self.broker.addr,
                "stream": self.broker.stream,
                "consumerGroup": self.broker.consumer_group,
            },
            "storage": {
                "backend": self.storage.backend,
                "addr": self.storage.addr,
            },
            "internalStorage": {
                "url": self.internal_storage.url,
            },
            "fluxcellStorage": {
                "url": self.fluxcell_storage.url,
            },
            "database": default_db,
            "databases": dbs,
            "profiles": self.profiles,
            "fluxcells": fcells,
            "deployer": {
                "enabled": self.deployer.enabled,
                "storageDir": self.deployer.storage_dir,
                "externalDeployEnabled": self.deployer.external_deploy_enabled,
                "devUploadEnabled": self.deployer.dev_upload_enabled,
            },
            "resilience": {
                "maxRetries": self.resilience.max_retries,
                "backoffInitialMs": self.resilience.backoff_initial_ms,
                "backoffMaxMs": self.resilience.backoff_max_ms,
                "dlqEnabled": self.resilience.dlq_enabled,
                "dlqTopicPrefix": self.resilience.dlq_topic_prefix,
            },
            "mailer": crate::mailer::MailerRegistry::from_config(&self.mailer).to_sanitized_json(),
        })
    }

    pub fn resolve_cell_execution(
        &self,
        cell_name: &str,
        cell_cfg: &FluxcellConfig,
        guest_profile: Option<&str>,
        guest_timeout_ms: Option<u64>,
        guest_max_memory_mb: Option<usize>,
    ) -> Result<ResolvedExecutionConfig> {
        // 1. Determine profile name: explicit cell config profile > guest self-declared profile
        let profile_name = cell_cfg.profile.as_deref().or(guest_profile);

        // 2. Base profile lookup
        let base_profile = match profile_name {
            Some(p) => self.profiles.get(p).cloned().ok_or_else(|| {
                anyhow::anyhow!(
                    "Fluxcell '{}' specifies unknown execution profile '{}'",
                    cell_name,
                    p
                )
            })?,
            None => {
                // If neither cell config nor guest declares a profile, check if explicit timeout_ms was given
                if cell_cfg.timeout_ms.is_none() && guest_timeout_ms.is_none() {
                    return Err(anyhow::anyhow!(
                        "Fluxcell '{}' rejected: execution profile must be explicitly configured (e.g. profile = 'standard', 'extended', 'batch') or explicit timeout_ms provided",
                        cell_name
                    ));
                }
                // When explicit timeout_ms is given without a named profile, use standard profile as template
                self.profiles.get("standard").cloned().unwrap_or_else(|| ExecutionProfileConfig {
                    timeout_ms: 10_000,
                    max_instances: 16,
                    offload: OffloadStrategy::BlockingPool,
                    max_memory_mb: Some(16),
                })
            }
        };

        // 3. Apply overrides: cell_cfg > guest declaration > base profile
        let final_timeout = cell_cfg
            .timeout_ms
            .or(guest_timeout_ms)
            .unwrap_or(base_profile.timeout_ms);

        if final_timeout < MIN_TIMEOUT_MS || final_timeout > MAX_TIMEOUT_MS {
            return Err(anyhow::anyhow!(
                "Fluxcell '{}' timeout_ms ({}) out of bounds: must be between {}ms and {}ms",
                cell_name,
                final_timeout,
                MIN_TIMEOUT_MS,
                MAX_TIMEOUT_MS
            ));
        }

        let final_instances = cell_cfg
            .max_instances
            .unwrap_or(base_profile.max_instances);
        if final_instances == 0 {
            return Err(anyhow::anyhow!(
                "Fluxcell '{}' max_instances must be greater than 0",
                cell_name
            ));
        }

        let final_memory_mb = cell_cfg
            .max_memory_mb
            .or(guest_max_memory_mb)
            .or(base_profile.max_memory_mb)
            .unwrap_or(16);

        Ok(ResolvedExecutionConfig {
            profile: profile_name.unwrap_or("custom").to_string(),
            timeout_ms: final_timeout,
            max_instances: final_instances,
            offload: base_profile.offload,
            max_memory_bytes: final_memory_mb * 1024 * 1024,
        })
    }
}

fn default_port() -> u16 {
    8081
}

fn default_host() -> String {
    "0.0.0.0".to_string()
}

#[derive(Debug, Clone, Deserialize)]
pub struct BrokerConfig {
    #[serde(default = "default_broker_method")]
    pub method: String,
    #[serde(default = "default_broker_addr")]
    pub addr: String,
    #[serde(default)]
    pub stream: Option<String>,
    #[serde(default = "default_consumer_group")]
    pub consumer_group: String,
}

fn default_broker_method() -> String {
    "in_memory".to_string()
}

fn default_broker_addr() -> String {
    "localhost".to_string()
}

fn default_consumer_group() -> String {
    "spectra-flux-workers".to_string()
}

impl Default for BrokerConfig {
    fn default() -> Self {
        Self {
            method: default_broker_method(),
            addr: default_broker_addr(),
            stream: None,
            consumer_group: default_consumer_group(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InternalStorageConfig {
    #[serde(default = "default_internal_storage_url")]
    pub url: String,
}

pub fn default_internal_storage_url() -> String {
    "kevy://embedded".to_string()
}

impl Default for InternalStorageConfig {
    fn default() -> Self {
        Self {
            url: default_internal_storage_url(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FluxcellStorageConfig {
    #[serde(default = "default_fluxcell_storage_url")]
    pub url: String,
}

pub fn default_fluxcell_storage_url() -> String {
    "kevy:///data/fluxcell-storage.kevy".to_string()
}

impl Default for FluxcellStorageConfig {
    fn default() -> Self {
        Self {
            url: default_fluxcell_storage_url(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct StorageConfig {
    #[serde(default = "default_storage_backend")]
    pub backend: String,
    #[serde(default)]
    pub addr: Option<String>,
}

fn default_storage_backend() -> String {
    "embedded_kevy".to_string()
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            backend: default_storage_backend(),
            addr: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseInstanceConfig {
    pub url: String,
    #[serde(default = "default_db_max_connections")]
    pub max_connections: usize,
    #[serde(default)]
    pub driver: Option<String>,
    #[serde(default = "default_auto_migrate")]
    pub auto_migrate: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseConfig {
    pub url: Option<String>,
    #[serde(default = "default_db_max_connections")]
    pub max_connections: usize,
    #[serde(default = "default_auto_migrate")]
    pub auto_migrate: bool,
}

fn default_db_max_connections() -> usize {
    16
}

fn default_auto_migrate() -> bool {
    true
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            url: std::env::var("FLUX__DATABASE__URL")
                .or_else(|_| std::env::var("DATABASE_URL"))
                .or_else(|_| std::env::var("FLUX_DATABASE_URL"))
                .ok(),
            max_connections: default_db_max_connections(),
            auto_migrate: std::env::var("FLUX__DATABASE__AUTO_MIGRATE")
                .map(|v| v != "false" && v != "0")
                .unwrap_or(true),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OffloadStrategy {
    BlockingPool,
    DedicatedWorker,
    Inline,
}

impl Default for OffloadStrategy {
    fn default() -> Self {
        Self::BlockingPool
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ExecutionProfileConfig {
    pub timeout_ms: u64,
    #[serde(default = "default_max_instances")]
    pub max_instances: usize,
    #[serde(default)]
    pub offload: OffloadStrategy,
    #[serde(default)]
    pub max_memory_mb: Option<usize>,
}

fn default_max_instances() -> usize {
    16
}

impl ExecutionProfileConfig {
    pub fn validate(&self, name: &str) -> Result<()> {
        if self.timeout_ms < MIN_TIMEOUT_MS || self.timeout_ms > MAX_TIMEOUT_MS {
            return Err(anyhow::anyhow!(
                "Execution profile '{}' timeout_ms ({}) out of bounds: must be between {}ms and {}ms",
                name,
                self.timeout_ms,
                MIN_TIMEOUT_MS,
                MAX_TIMEOUT_MS
            ));
        }
        if self.max_instances == 0 {
            return Err(anyhow::anyhow!(
                "Execution profile '{}' max_instances must be greater than 0",
                name
            ));
        }
        Ok(())
    }
}

pub fn default_profiles() -> HashMap<String, ExecutionProfileConfig> {
    let mut m = HashMap::new();
    m.insert(
        "standard".to_string(),
        ExecutionProfileConfig {
            timeout_ms: 10_000,
            max_instances: 12,
            offload: OffloadStrategy::BlockingPool,
            max_memory_mb: Some(32),
        },
    );
    m.insert(
        "extended".to_string(),
        ExecutionProfileConfig {
            timeout_ms: 120_000,
            max_instances: 8,
            offload: OffloadStrategy::BlockingPool,
            max_memory_mb: Some(64),
        },
    );
    m.insert(
        "batch".to_string(),
        ExecutionProfileConfig {
            timeout_ms: 300_000,
            max_instances: 1,
            offload: OffloadStrategy::DedicatedWorker,
            max_memory_mb: Some(128),
        },
    );
    m
}

#[derive(Debug, Clone, Deserialize)]
pub struct FluxcellConfig {
    pub wasm_module: String,
    pub mount_path: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub subscriptions: Option<Vec<String>>,
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub max_memory_mb: Option<usize>,
    #[serde(default)]
    pub max_instances: Option<usize>,
}

fn default_true() -> bool {
    true
}

fn default_false() -> bool {
    false
}

fn default_max_wasm_size() -> usize {
    20 * 1024 * 1024 // 20 MB
}

fn default_storage_dir() -> String {
    "fluxcells".to_string()
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeployerConfig {
    #[serde(default = "default_false")]
    pub enabled: bool,
    #[serde(default = "default_false")]
    pub external_deploy_enabled: bool,
    #[serde(default = "default_false")]
    pub dev_upload_enabled: bool,
    #[serde(default = "default_false")]
    pub auto_activate: bool,
    #[serde(default)]
    pub allowed_artifact_hosts: Vec<String>,
    #[serde(default = "default_true")]
    pub require_https: bool,
    #[serde(default = "default_true")]
    pub block_private_networks: bool,
    #[serde(default = "default_max_wasm_size")]
    pub max_wasm_size_bytes: usize,
    #[serde(default = "default_storage_dir")]
    pub storage_dir: String,
}

impl Default for DeployerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            external_deploy_enabled: false,
            dev_upload_enabled: false,
            auto_activate: false,
            allowed_artifact_hosts: Vec::new(),
            require_https: true,
            block_private_networks: true,
            max_wasm_size_bytes: default_max_wasm_size(),
            storage_dir: default_storage_dir(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResilienceConfig {
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    #[serde(default = "default_backoff_initial_ms")]
    pub backoff_initial_ms: u64,
    #[serde(default = "default_backoff_max_ms")]
    pub backoff_max_ms: u64,
    #[serde(default = "default_dlq_topic_prefix")]
    pub dlq_topic_prefix: String,
    #[serde(default = "default_dlq_enabled")]
    pub dlq_enabled: bool,
}

fn default_max_retries() -> u32 {
    3
}

fn default_backoff_initial_ms() -> u64 {
    200
}

fn default_backoff_max_ms() -> u64 {
    10_000
}

fn default_dlq_topic_prefix() -> String {
    "dlq.".to_string()
}

fn default_dlq_enabled() -> bool {
    true
}

impl Default for ResilienceConfig {
    fn default() -> Self {
        Self {
            max_retries: default_max_retries(),
            backoff_initial_ms: default_backoff_initial_ms(),
            backoff_max_ms: default_backoff_max_ms(),
            dlq_topic_prefix: default_dlq_topic_prefix(),
            dlq_enabled: default_dlq_enabled(),
        }
    }
}

/// Tenant-specific mailer override settings
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MailerTenantConfig {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub from_email: Option<String>,
    #[serde(default)]
    pub from_name: Option<String>,
    #[serde(default)]
    pub app_name: Option<String>,
    #[serde(default)]
    pub logo_url: Option<String>,
    #[serde(default)]
    pub accent_color: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub mailtrap_inbox_id: Option<String>,
    #[serde(default)]
    pub smtp_host: Option<String>,
    #[serde(default)]
    pub smtp_port: Option<u16>,
    #[serde(default)]
    pub smtp_user: Option<String>,
    #[serde(default)]
    pub smtp_pass: Option<String>,
    #[serde(default)]
    pub smtp_secure: Option<bool>,
}

/// Global mailer configuration section with multi-tenant overrides
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MailerSectionConfig {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub from_email: Option<String>,
    #[serde(default)]
    pub from_name: Option<String>,
    #[serde(default)]
    pub app_name: Option<String>,
    #[serde(default)]
    pub logo_url: Option<String>,
    #[serde(default)]
    pub accent_color: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub mailtrap_inbox_id: Option<String>,
    #[serde(default)]
    pub smtp_host: Option<String>,
    #[serde(default)]
    pub smtp_port: Option<u16>,
    #[serde(default)]
    pub smtp_user: Option<String>,
    #[serde(default)]
    pub smtp_pass: Option<String>,
    #[serde(default)]
    pub smtp_secure: Option<bool>,
    #[serde(default)]
    pub tenants: HashMap<String, MailerTenantConfig>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let cfg = FluxConfig::default();
        assert_eq!(cfg.port, 8081);
        assert_eq!(cfg.host, "0.0.0.0");
        assert_eq!(cfg.broker.method, "in_memory");
        assert_eq!(cfg.broker.addr, "localhost");
        assert_eq!(cfg.broker.consumer_group, "spectra-flux-workers");
        assert_eq!(cfg.internal_storage.url, "kevy://embedded");
        assert_eq!(cfg.fluxcell_storage.url, "kevy:///data/fluxcell-storage.kevy");
        assert_eq!(cfg.storage.backend, "embedded_kevy");
        assert_eq!(cfg.storage.addr, None);
        assert_eq!(cfg.gateway_admin_url, Some("http://127.0.0.1:8000".to_string()));
        assert!(cfg.fluxcells.is_empty());
        assert!(!cfg.deployer.enabled);
        assert!(!cfg.deployer.external_deploy_enabled);
        assert!(!cfg.deployer.dev_upload_enabled);
        assert!(!cfg.deployer.auto_activate);
        assert!(cfg.deployer.require_https);
        assert!(cfg.deployer.block_private_networks);
        assert_eq!(cfg.deployer.storage_dir, "fluxcells");
        assert_eq!(cfg.resilience.max_retries, 3);
        assert_eq!(cfg.resilience.backoff_initial_ms, 200);
        assert_eq!(cfg.resilience.backoff_max_ms, 10_000);
        assert_eq!(cfg.resilience.dlq_topic_prefix, "dlq.");
        assert!(cfg.resilience.dlq_enabled);
    }

    #[test]
    fn test_partial_toml_deserialization() {
        let toml_str = r#"
            port = 9090
            host = "127.0.0.1"
        "#;
        let cfg = FluxConfig::from_toml_str(toml_str).unwrap();
        assert_eq!(cfg.port, 9090);
        assert_eq!(cfg.host, "127.0.0.1");
        // Broker and storage should fall back to defaults
        assert_eq!(cfg.broker.method, "in_memory");
        assert_eq!(cfg.storage.backend, "embedded_kevy");
        assert_eq!(cfg.internal_storage.url, "kevy://embedded");
        assert_eq!(cfg.fluxcell_storage.url, "kevy:///data/fluxcell-storage.kevy");
    }

    #[test]
    fn test_full_toml_deserialization() {
        let toml_str = r#"
            port = 8888
            host = "10.0.0.1"
            gateway_admin_url = "http://gateway:8000"

            [broker]
            method = "nats"
            addr = "nats://10.0.0.2:4222"
            stream = "MUTATIONS"
            consumer_group = "flux-cluster"

            [storage]
            backend = "redis"
            addr = "redis://10.0.0.3:6379"

            [database]
            url = "postgres://postgres:secret@localhost:5432/spectraflux"
            max_connections = 32

            [fluxcells.magic_link]
            wasm_module = "cells/magic_link.wasm"
            mount_path = "/auth"
            enabled = true
            subscriptions = ["mutation.auth.login"]

            [fluxcells.webhook]
            wasm_module = "cells/webhook.wasm"
            mount_path = "/api/webhooks"
        "#;
        let cfg = FluxConfig::from_toml_str(toml_str).unwrap();
        assert_eq!(cfg.port, 8888);
        assert_eq!(cfg.host, "10.0.0.1");
        assert_eq!(cfg.gateway_admin_url.as_deref(), Some("http://gateway:8000"));
        assert_eq!(cfg.broker.method, "nats");
        assert_eq!(cfg.broker.addr, "nats://10.0.0.2:4222");
        assert_eq!(cfg.broker.stream.as_deref(), Some("MUTATIONS"));
        assert_eq!(cfg.broker.consumer_group, "flux-cluster");
        assert_eq!(cfg.storage.backend, "redis");
        assert_eq!(cfg.storage.addr.as_deref(), Some("redis://10.0.0.3:6379"));
        assert_eq!(cfg.internal_storage.url, "kevy://embedded");
        assert_eq!(cfg.fluxcell_storage.url, "redis://10.0.0.3:6379");
        assert_eq!(cfg.database.url.as_deref(), Some("postgres://postgres:secret@localhost:5432/spectraflux"));
        assert_eq!(cfg.database.max_connections, 32);

        assert_eq!(cfg.fluxcells.len(), 2);
        let ml = &cfg.fluxcells["magic_link"];
        assert_eq!(ml.mount_path, "/auth");
        assert!(ml.enabled);
        assert_eq!(ml.subscriptions, Some(vec!["mutation.auth.login".to_string()]));

        let wh = &cfg.fluxcells["webhook"];
        assert_eq!(wh.mount_path, "/api/webhooks");
        assert!(wh.enabled); // defaults to true
        assert_eq!(wh.subscriptions, None);
    }

    #[test]
    fn test_invalid_toml_syntax_fails() {
        let bad_toml = "port = not_a_number";
        assert!(FluxConfig::from_toml_str(bad_toml).is_err());
    }

    #[test]
    fn test_invalid_port_range_fails() {
        let bad_port_toml = "port = 70000"; // Exceeds u16::MAX (65535)
        assert!(FluxConfig::from_toml_str(bad_port_toml).is_err());
    }

    #[test]
    fn test_env_var_overrides() {
        // Use a unique env var to avoid race condition across threads
        unsafe {
            std::env::set_var("FLUX__PORT", "7777");
            std::env::set_var("FLUX__HOST", "127.0.0.5");
            std::env::set_var("FLUX__BROKER__METHOD", "redis");
            std::env::set_var("FLUX__BROKER__ADDR", "redis://127.0.0.1:6379");
            std::env::set_var("FLUX__STORAGE__BACKEND", "valkey");
        }

        let cfg = FluxConfig::load_from_file("nonexistent.toml").unwrap();
        assert_eq!(cfg.port, 7777);
        assert_eq!(cfg.host, "127.0.0.5");
        assert_eq!(cfg.broker.method, "redis");
        assert_eq!(cfg.broker.addr, "redis://127.0.0.1:6379");
        assert_eq!(cfg.storage.backend, "valkey");

        unsafe {
            std::env::remove_var("FLUX__PORT");
            std::env::remove_var("FLUX__HOST");
            std::env::remove_var("FLUX__BROKER__METHOD");
            std::env::remove_var("FLUX__BROKER__ADDR");
            std::env::remove_var("FLUX__STORAGE__BACKEND");
        }
    }

    #[test]
    fn test_storage_tiers_explicit_toml() {
        let toml_str = r#"
            [internal-storage]
            url = "kevy://memory"

            [fluxcell-storage]
            url = "redis://redis-cluster:6379"
        "#;
        let cfg = FluxConfig::from_toml_str(toml_str).unwrap();
        assert_eq!(cfg.internal_storage.url, "kevy://memory");
        assert_eq!(cfg.fluxcell_storage.url, "redis://redis-cluster:6379");
        assert_eq!(cfg.storage.backend, "redis");
        assert_eq!(cfg.storage.addr.as_deref(), Some("redis://redis-cluster:6379"));
    }

    #[test]
    fn test_storage_tiers_redis_url_fallback() {
        unsafe {
            std::env::set_var("REDIS_URL", "redis://redis.railway.internal:6379");
        }
        let cfg = FluxConfig::load_from_file("nonexistent.toml").unwrap();
        assert_eq!(cfg.fluxcell_storage.url, "redis://redis.railway.internal:6379");
        assert_eq!(cfg.internal_storage.url, "kevy://embedded");
        unsafe {
            std::env::remove_var("REDIS_URL");
        }
    }

    #[test]
    fn test_default_profiles_exist_and_valid() {
        let cfg = FluxConfig::default();
        assert!(cfg.profiles.contains_key("standard"));
        assert!(cfg.profiles.contains_key("extended"));
        assert!(cfg.profiles.contains_key("batch"));

        let std_prof = &cfg.profiles["standard"];
        assert_eq!(std_prof.timeout_ms, 10_000);
        assert_eq!(std_prof.max_instances, 12);
        assert_eq!(std_prof.offload, OffloadStrategy::BlockingPool);
        assert!(std_prof.validate("standard").is_ok());

        let ext_prof = &cfg.profiles["extended"];
        assert_eq!(ext_prof.timeout_ms, 120_000);
        assert_eq!(ext_prof.max_instances, 8);
        assert_eq!(ext_prof.offload, OffloadStrategy::BlockingPool);
        assert!(ext_prof.validate("extended").is_ok());

        let batch_prof = &cfg.profiles["batch"];
        assert_eq!(batch_prof.timeout_ms, 300_000);
        assert_eq!(batch_prof.max_instances, 1);
        assert_eq!(batch_prof.offload, OffloadStrategy::DedicatedWorker);
        assert!(batch_prof.validate("batch").is_ok());
    }

    #[test]
    fn test_resolve_cell_execution_explicit_profile() {
        let cfg = FluxConfig::default();
        let cell = FluxcellConfig {
            wasm_module: "test.wasm".to_string(),
            mount_path: "/test".to_string(),
            enabled: true,
            subscriptions: None,
            profile: Some("extended".to_string()),
            timeout_ms: None,
            max_memory_mb: None,
            max_instances: None,
        };

        let resolved = cfg.resolve_cell_execution("test_cell", &cell, None, None, None).unwrap();
        assert_eq!(resolved.profile, "extended");
        assert_eq!(resolved.timeout_ms, 120_000);
        assert_eq!(resolved.max_instances, 8);
        assert_eq!(resolved.offload, OffloadStrategy::BlockingPool);
        assert_eq!(resolved.max_memory_bytes, 64 * 1024 * 1024);
    }

    #[test]
    fn test_resolve_cell_execution_guest_declaration() {
        let cfg = FluxConfig::default();
        let cell = FluxcellConfig {
            wasm_module: "test.wasm".to_string(),
            mount_path: "/test".to_string(),
            enabled: true,
            subscriptions: None,
            profile: None,
            timeout_ms: None,
            max_memory_mb: None,
            max_instances: None,
        };

        // Guest declares "batch"
        let resolved = cfg.resolve_cell_execution("test_cell", &cell, Some("batch"), None, None).unwrap();
        assert_eq!(resolved.profile, "batch");
        assert_eq!(resolved.timeout_ms, 300_000);
        assert_eq!(resolved.max_instances, 1);
        assert_eq!(resolved.offload, OffloadStrategy::DedicatedWorker);
    }

    #[test]
    fn test_resolve_cell_execution_missing_profile_and_timeout_fails() {
        let cfg = FluxConfig::default();
        let cell = FluxcellConfig {
            wasm_module: "test.wasm".to_string(),
            mount_path: "/test".to_string(),
            enabled: true,
            subscriptions: None,
            profile: None,
            timeout_ms: None,
            max_memory_mb: None,
            max_instances: None,
        };

        // Neither host config nor guest provides profile or timeout -> MUST FAIL FAST
        let err = cfg.resolve_cell_execution("unconfigured_cell", &cell, None, None, None).unwrap_err();
        assert!(err.to_string().contains("execution profile must be explicitly configured"));
    }

    #[test]
    fn test_resolve_cell_execution_bounds_enforcement() {
        let cfg = FluxConfig::default();

        // 1. Timeout < 10ms rejected
        let cell_low = FluxcellConfig {
            wasm_module: "test.wasm".to_string(),
            mount_path: "/test".to_string(),
            enabled: true,
            subscriptions: None,
            profile: Some("standard".to_string()),
            timeout_ms: Some(5), // below MIN_TIMEOUT_MS
            max_memory_mb: None,
            max_instances: None,
        };
        let err_low = cfg.resolve_cell_execution("low_cell", &cell_low, None, None, None).unwrap_err();
        assert!(err_low.to_string().contains("out of bounds"));

        // 2. Timeout == 0 (unbounded) strictly rejected
        let cell_zero = FluxcellConfig {
            wasm_module: "test.wasm".to_string(),
            mount_path: "/test".to_string(),
            enabled: true,
            subscriptions: None,
            profile: Some("standard".to_string()),
            timeout_ms: Some(0),
            max_memory_mb: None,
            max_instances: None,
        };
        let err_zero = cfg.resolve_cell_execution("zero_cell", &cell_zero, None, None, None).unwrap_err();
        assert!(err_zero.to_string().contains("out of bounds"));

        // 3. Timeout > 300,000ms rejected
        let cell_high = FluxcellConfig {
            wasm_module: "test.wasm".to_string(),
            mount_path: "/test".to_string(),
            enabled: true,
            subscriptions: None,
            profile: Some("standard".to_string()),
            timeout_ms: Some(300_001),
            max_memory_mb: None,
            max_instances: None,
        };
        let err_high = cfg.resolve_cell_execution("high_cell", &cell_high, None, None, None).unwrap_err();
        assert!(err_high.to_string().contains("out of bounds"));
    }

    #[test]
    fn test_custom_profile_in_toml() {
        let toml_str = r#"
            [profiles.heavy_calc]
            timeout_ms = 45000
            max_instances = 8
            offload = "dedicated_worker"
            max_memory_mb = 128

            [fluxcells.calc]
            wasm_module = "cells/calc.wasm"
            mount_path = "/calc"
            profile = "heavy_calc"
        "#;
        let cfg = FluxConfig::from_toml_str(toml_str).unwrap();
        assert!(cfg.profiles.contains_key("heavy_calc"));

        let cell = &cfg.fluxcells["calc"];
        let resolved = cfg.resolve_cell_execution("calc", cell, None, None, None).unwrap();
        assert_eq!(resolved.profile, "heavy_calc");
        assert_eq!(resolved.timeout_ms, 45_000);
        assert_eq!(resolved.max_instances, 8);
        assert_eq!(resolved.offload, OffloadStrategy::DedicatedWorker);
        assert_eq!(resolved.max_memory_bytes, 128 * 1024 * 1024);
    }

    #[test]
    fn test_mailer_and_tenants_toml_deserialization() {
        let toml_str = r#"
            [mailer]
            provider = "console"
            from_email = "base@example.com"
            from_name = "Base System"
            base_url = "https://base.example.com"

            [mailer.tenants.coeval]
            provider = "resend"
            api_key = "re_live_999888777"
            from_email = "auth@coeval.bio"
            from_name = "CoEval Biological"
            app_name = "CoEval"
            base_url = "https://coeval.bio"

            [mailer.tenants.humanshirehumans]
            provider = "mailtrap"
            api_key = "mt_live_111222333"
            from_email = "auth@humanshirehumans.com"
            from_name = "Humans Hire Humans"
        "#;
        let cfg = FluxConfig::from_toml_str(toml_str).unwrap();
        assert_eq!(cfg.mailer.provider.as_deref(), Some("console"));
        assert_eq!(cfg.mailer.from_email.as_deref(), Some("base@example.com"));
        assert_eq!(cfg.mailer.tenants.len(), 2);

        let coeval = &cfg.mailer.tenants["coeval"];
        assert_eq!(coeval.provider.as_deref(), Some("resend"));
        assert_eq!(coeval.api_key.as_deref(), Some("re_live_999888777"));
        assert_eq!(coeval.from_email.as_deref(), Some("auth@coeval.bio"));
        assert_eq!(coeval.base_url.as_deref(), Some("https://coeval.bio"));

        let hhh = &cfg.mailer.tenants["humanshirehumans"];
        assert_eq!(hhh.provider.as_deref(), Some("mailtrap"));
        assert_eq!(hhh.api_key.as_deref(), Some("mt_live_111222333"));
    }
}
