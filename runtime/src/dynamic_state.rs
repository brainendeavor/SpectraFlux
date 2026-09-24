use crate::config::FluxConfig;
use crate::mailer::MailerRegistry;
use sha2::Digest;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Lock-free, dynamically swappable chassis state for SpectraFlux.
/// Enables line-rate zero-lock request reads with atomic hot-reloading of configuration,
/// multi-tenant mailer credentials, execution profiles, and topology settings.
#[derive(Clone, Debug)]
pub struct DynamicChassisState {
    pub version: u64,
    pub updated_at_epoch_ms: u64,
    pub config_hash: String,
    pub config_path: String,
    pub raw_config: Arc<String>,
    pub config: Arc<FluxConfig>,
    pub mailer_registry: Arc<MailerRegistry>,
}

impl DynamicChassisState {
    pub fn new(
        config: FluxConfig,
        raw_config: String,
        config_path: String,
        version: u64,
    ) -> Self {
        let hash = format!("{:x}", sha2::Sha256::digest(raw_config.as_bytes()));
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let mailer_registry = Arc::new(MailerRegistry::from_config(&config.mailer));

        Self {
            version,
            updated_at_epoch_ms: now_ms,
            config_hash: hash,
            config_path,
            raw_config: Arc::new(raw_config),
            config: Arc::new(config),
            mailer_registry,
        }
    }

    pub fn new_from_toml(
        toml_str: &str,
        config_path: String,
        version: u64,
    ) -> anyhow::Result<Self> {
        let cfg = FluxConfig::from_toml_str(toml_str)?;
        Ok(Self::new(cfg, toml_str.to_string(), config_path, version))
    }
}

impl Default for DynamicChassisState {
    fn default() -> Self {
        Self::new(
            FluxConfig::default_local(),
            String::new(),
            "spectra-flux.toml".to_string(),
            1,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dynamic_chassis_state_lifecycle() {
        let toml_content = r#"
            port = 8085
            host = "127.0.0.1"

            [mailer]
            provider = "resend"
            api_key = "re_test_key"
            from_email = "root@example.com"

            [mailer.tenants.tenant_alpha]
            provider = "resend"
            api_key = "re_tenant_alpha_key"
            from_email = "auth@tenant-alpha.example.com"
        "#;

        let state = DynamicChassisState::new_from_toml(
            toml_content,
            "test-flux.toml".to_string(),
            1,
        )
        .expect("Valid state parsing");

        assert_eq!(state.version, 1);
        assert_eq!(state.config.port, 8085);
        assert!(!state.config_hash.is_empty());

        let alpha_cfg = state.mailer_registry.get_config(Some("tenant_alpha"));
        assert_eq!(alpha_cfg.from_email, "auth@tenant-alpha.example.com");

        let default_cfg = state.mailer_registry.get_config(None);
        assert_eq!(default_cfg.from_email, "root@example.com");
    }
}
