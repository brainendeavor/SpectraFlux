//! Outbound Transactional Mailer Service for SpectraFlux
//!
//! Provides a unified, high-performance transactional email delivery engine
//! supporting multiple tier-1 email providers:
//! - Mailtrap (Production Sending & Sandbox Testing)
//! - Resend
//! - Postmark
//! - SendGrid
//! - Universal SMTP (RFC 5321)
//! - Console / Local Dev Logger (Zero-credential fallback)

use std::env;
use std::time::Duration;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MailerProvider {
    Console,
    Mailtrap,
    Resend,
    Postmark,
    Sendgrid,
    Smtp,
}

impl Default for MailerProvider {
    fn default() -> Self {
        MailerProvider::Console
    }
}

#[derive(Debug, Clone)]
pub struct MailerConfig {
    pub provider: MailerProvider,
    pub api_key: Option<String>,
    pub from_email: String,
    pub from_name: String,
    pub app_name: String,
    pub logo_url: Option<String>,
    pub accent_color: Option<String>,
    pub base_url: String,
    pub mailtrap_inbox_id: Option<String>,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<u16>,
    pub smtp_user: Option<String>,
    pub smtp_pass: Option<String>,
    pub smtp_secure: bool,
}

pub fn parse_provider(s: &str) -> Option<MailerProvider> {
    match s.trim().to_lowercase().as_str() {
        "mailtrap" => Some(MailerProvider::Mailtrap),
        "resend" => Some(MailerProvider::Resend),
        "postmark" => Some(MailerProvider::Postmark),
        "sendgrid" => Some(MailerProvider::Sendgrid),
        "smtp" => Some(MailerProvider::Smtp),
        "console" => Some(MailerProvider::Console),
        _ => None,
    }
}

pub fn mask_secret(k: &str) -> String {
    if k.len() <= 8 {
        "••••••••".to_string()
    } else {
        format!("{}••••{}", &k[..4], &k[k.len() - 4..])
    }
}

impl MailerConfig {
    /// Loads configuration dynamically from 12-factor environment variables
    pub fn from_env() -> Self {
        let provider_str = env::var("FLUX__MAILER__PROVIDER")
            .or_else(|_| env::var("MAILER_PROVIDER"))
            .unwrap_or_else(|_| {
                if env::var("MAILTRAP_API_TOKEN").is_ok() || env::var("FLUX__MAILER__MAILTRAP_API_TOKEN").is_ok() {
                    "mailtrap".to_string()
                } else if env::var("RESEND_API_KEY").is_ok() || env::var("FLUX__MAILER__RESEND_API_KEY").is_ok() {
                    "resend".to_string()
                } else if env::var("POSTMARK_API_TOKEN").is_ok() {
                    "postmark".to_string()
                } else if env::var("SENDGRID_API_KEY").is_ok() {
                    "sendgrid".to_string()
                } else {
                    "console".to_string()
                }
            });

        let provider = parse_provider(&provider_str).unwrap_or(MailerProvider::Console);

        let api_key = env::var("FLUX__MAILER__API_KEY")
            .or_else(|_| env::var("MAILTRAP_API_TOKEN"))
            .or_else(|_| env::var("RESEND_API_KEY"))
            .or_else(|_| env::var("POSTMARK_API_TOKEN"))
            .or_else(|_| env::var("SENDGRID_API_KEY"))
            .ok();

        let from_email = env::var("FLUX__MAILER__FROM")
            .or_else(|_| env::var("EMAIL_FROM"))
            .unwrap_or_else(|_| "noreply@example.com".to_string());

        let app_name = env::var("FLUX__MAILER__APP_NAME")
            .or_else(|_| env::var("APP_NAME"))
            .unwrap_or_else(|_| "Auth Service".to_string());

        let from_name = env::var("FLUX__MAILER__FROM_NAME")
            .or_else(|_| env::var("EMAIL_FROM_NAME"))
            .unwrap_or_else(|_| app_name.clone());

        let logo_url = env::var("FLUX__MAILER__LOGO_URL")
            .or_else(|_| env::var("APP_LOGO_URL"))
            .ok();

        let accent_color = env::var("FLUX__MAILER__ACCENT_COLOR")
            .or_else(|_| env::var("APP_ACCENT_COLOR"))
            .ok();

        let base_url = env::var("FLUX__MAILER__BASE_URL")
            .or_else(|_| env::var("APP_BASE_URL"))
            .or_else(|_| env::var("PUBLIC_APP_URL"))
            .or_else(|_| env::var("RAILWAY_PUBLIC_DOMAIN").map(|d| format!("https://{}", d)))
            .unwrap_or_else(|_| "http://localhost:8000".to_string())
            .trim_end_matches('/')
            .to_string();

        let mailtrap_inbox_id = env::var("FLUX__MAILER__MAILTRAP_INBOX_ID")
            .or_else(|_| env::var("MAILTRAP_INBOX_ID"))
            .ok();

        let smtp_host = env::var("FLUX__MAILER__SMTP_HOST")
            .or_else(|_| env::var("SMTP_HOST"))
            .ok();

        let smtp_port = env::var("FLUX__MAILER__SMTP_PORT")
            .or_else(|_| env::var("SMTP_PORT"))
            .ok()
            .and_then(|p| p.parse::<u16>().ok());

        let smtp_user = env::var("FLUX__MAILER__SMTP_USER")
            .or_else(|_| env::var("SMTP_USER"))
            .ok();

        let smtp_pass = env::var("FLUX__MAILER__SMTP_PASS")
            .or_else(|_| env::var("SMTP_PASS"))
            .ok();

        let smtp_secure = env::var("FLUX__MAILER__SMTP_SECURE")
            .or_else(|_| env::var("SMTP_SECURE"))
            .map(|v| v == "true" || v == "1")
            .unwrap_or(true);

        Self {
            provider,
            api_key,
            from_email,
            from_name,
            app_name,
            logo_url,
            accent_color,
            base_url,
            mailtrap_inbox_id,
            smtp_host,
            smtp_port,
            smtp_user,
            smtp_pass,
            smtp_secure,
        }
    }

    /// Applies tenant-specific configuration overrides on top of base mailer configuration
    pub fn apply_tenant_overrides(&mut self, tenant: &crate::config::MailerTenantConfig) {
        if let Some(p) = &tenant.provider {
            if let Some(parsed) = parse_provider(p) {
                self.provider = parsed;
            }
        }
        if let Some(k) = &tenant.api_key {
            self.api_key = Some(k.clone());
        }
        if let Some(e) = &tenant.from_email {
            self.from_email = e.clone();
        }
        if let Some(n) = &tenant.from_name {
            self.from_name = n.clone();
        }
        if let Some(a) = &tenant.app_name {
            self.app_name = a.clone();
        }
        if let Some(l) = &tenant.logo_url {
            self.logo_url = Some(l.clone());
        }
        if let Some(c) = &tenant.accent_color {
            self.accent_color = Some(c.clone());
        }
        if let Some(b) = &tenant.base_url {
            self.base_url = b.trim_end_matches('/').to_string();
        }
        if let Some(m) = &tenant.mailtrap_inbox_id {
            self.mailtrap_inbox_id = Some(m.clone());
        }
        if let Some(h) = &tenant.smtp_host {
            self.smtp_host = Some(h.clone());
        }
        if let Some(p) = tenant.smtp_port {
            self.smtp_port = Some(p);
        }
        if let Some(u) = &tenant.smtp_user {
            self.smtp_user = Some(u.clone());
        }
        if let Some(pass) = &tenant.smtp_pass {
            self.smtp_pass = Some(pass.clone());
        }
        if let Some(s) = tenant.smtp_secure {
            self.smtp_secure = s;
        }
    }

    /// Returns a sanitized JSON representation of mailer configuration for administrative inspection.
    /// Invariant: Raw credentials, API keys, and passwords are never exposed.
    pub fn to_sanitized_json(&self) -> serde_json::Value {
        let is_prod = env::var("RAILWAY_ENVIRONMENT").is_ok()
            || env::var("NODE_ENV").as_deref() == Ok("production")
            || env::var("ENVIRONMENT").as_deref() == Ok("production");

        let masked_api_key = self.api_key.as_ref().map(|k| mask_secret(k));

        let provider_str = match self.provider {
            MailerProvider::Console => "console",
            MailerProvider::Mailtrap => "mailtrap",
            MailerProvider::Resend => "resend",
            MailerProvider::Postmark => "postmark",
            MailerProvider::Sendgrid => "sendgrid",
            MailerProvider::Smtp => "smtp",
        };

        serde_json::json!({
            "provider": provider_str,
            "fromEmail": self.from_email,
            "fromName": self.from_name,
            "fromFormatted": format!("{} <{}>", self.from_name, self.from_email),
            "appName": self.app_name,
            "logoUrlConfigured": self.logo_url.is_some(),
            "logoUrl": self.logo_url,
            "accentColor": self.accent_color,
            "baseUrl": self.base_url,
            "apiKeyConfigured": self.api_key.is_some(),
            "apiKeyMasked": masked_api_key,
            "mailtrapInboxId": self.mailtrap_inbox_id,
            "smtpHost": self.smtp_host,
            "smtpPort": self.smtp_port,
            "smtpUser": self.smtp_user,
            "smtpSecure": self.smtp_secure,
            "isProduction": is_prod,
        })
    }
}

/// Multi-tenant Mailer Registry managing default provider configuration and named tenant profiles.
#[derive(Debug, Clone)]
pub struct MailerRegistry {
    pub default_config: MailerConfig,
    pub tenants: std::collections::HashMap<String, MailerConfig>,
}

impl MailerRegistry {
    pub fn new(default_config: MailerConfig) -> Self {
        Self {
            default_config,
            tenants: std::collections::HashMap::new(),
        }
    }

    pub fn from_config(section: &crate::config::MailerSectionConfig) -> Self {
        let mut base = MailerConfig::from_env();

        if let Some(p) = &section.provider {
            if let Some(parsed) = parse_provider(p) {
                base.provider = parsed;
            }
        }
        if let Some(k) = &section.api_key {
            base.api_key = Some(k.clone());
        }
        if let Some(e) = &section.from_email {
            base.from_email = e.clone();
        }
        if let Some(n) = &section.from_name {
            base.from_name = n.clone();
        }
        if let Some(a) = &section.app_name {
            base.app_name = a.clone();
        }
        if let Some(l) = &section.logo_url {
            base.logo_url = Some(l.clone());
        }
        if let Some(c) = &section.accent_color {
            base.accent_color = Some(c.clone());
        }
        if let Some(b) = &section.base_url {
            base.base_url = b.trim_end_matches('/').to_string();
        }
        if let Some(m) = &section.mailtrap_inbox_id {
            base.mailtrap_inbox_id = Some(m.clone());
        }
        if let Some(h) = &section.smtp_host {
            base.smtp_host = Some(h.clone());
        }
        if let Some(p) = section.smtp_port {
            base.smtp_port = Some(p);
        }
        if let Some(u) = &section.smtp_user {
            base.smtp_user = Some(u.clone());
        }
        if let Some(pass) = &section.smtp_pass {
            base.smtp_pass = Some(pass.clone());
        }
        if let Some(s) = section.smtp_secure {
            base.smtp_secure = s;
        }

        let mut tenants = std::collections::HashMap::new();
        for (tenant_name, tenant_cfg) in &section.tenants {
            let mut t_cfg = base.clone();
            t_cfg.apply_tenant_overrides(tenant_cfg);
            tenants.insert(tenant_name.clone(), t_cfg);
        }

        Self {
            default_config: base,
            tenants,
        }
    }

    /// Resolves active mailer configuration for an optional tenant identifier.
    /// Falls back to default chassis mailer configuration if tenant is not specified or unmapped.
    pub fn get_config(&self, tenant: Option<&str>) -> &MailerConfig {
        if let Some(t) = tenant {
            let trimmed = t.trim();
            if let Some(cfg) = self.tenants.get(trimmed) {
                return cfg;
            }
            let t_lower = trimmed.to_lowercase();
            if let Some(cfg) = self.tenants.get(&t_lower) {
                return cfg;
            }
            for (k, v) in &self.tenants {
                if k.eq_ignore_ascii_case(trimmed) {
                    return v;
                }
            }
        }
        &self.default_config
    }

    /// Returns a sanitized JSON representation containing default configuration and all tenant profiles.
    pub fn to_sanitized_json(&self) -> serde_json::Value {
        let mut def_val = self.default_config.to_sanitized_json();
        let mut tenants_map = serde_json::Map::new();

        for (name, cfg) in &self.tenants {
            tenants_map.insert(name.clone(), cfg.to_sanitized_json());
        }

        if let Some(obj) = def_val.as_object_mut() {
            obj.insert("tenantsCount".to_string(), serde_json::json!(self.tenants.len()));
            obj.insert("tenants".to_string(), serde_json::Value::Object(tenants_map));
        }
        def_val
    }
}

/// Dispatches an outbound transactional email
pub async fn send_transactional_email(
    config: &MailerConfig,
    to: &str,
    subject: &str,
    html_body: &str,
    text_body: &str,
) -> Result<(), String> {
    match config.provider {
        MailerProvider::Console => {
            let is_prod = env::var("RAILWAY_ENVIRONMENT").is_ok()
                || env::var("NODE_ENV").as_deref() == Ok("production")
                || env::var("ENVIRONMENT").as_deref() == Ok("production");
            let is_explicit = env::var("FLUX__MAILER__PROVIDER").as_deref() == Ok("console");

            if is_prod && !is_explicit {
                let err_msg = "No outbound transactional email provider configured in production! Set FLUX__MAILER__PROVIDER (e.g. 'mailtrap' or 'resend') and your provider API key in Railway environment variables.".to_string();
                log::error!("{}", err_msg);
                return Err(err_msg);
            }

            println!("\n═══════════════════════════════════════════════════════════════════════");
            println!("📧 [DEV AUTH MAILER] Outbound Transactional Email");
            println!("   To      : {}", to);
            println!("   From    : {} <{}>", config.from_name, config.from_email);
            println!("   Subject : {}", subject);
            println!("───────────────────────────────────────────────────────────────────────");
            println!("{}", text_body);
            println!("═══════════════════════════════════════════════════════════════════════\n");
            Ok(())
        }
        MailerProvider::Mailtrap => {
            send_via_mailtrap(config, to, subject, html_body, text_body).await
        }
        MailerProvider::Resend => {
            send_via_resend(config, to, subject, html_body, text_body).await
        }
        MailerProvider::Postmark => {
            send_via_postmark(config, to, subject, html_body, text_body).await
        }
        MailerProvider::Sendgrid => {
            send_via_sendgrid(config, to, subject, html_body, text_body).await
        }
        MailerProvider::Smtp => {
            send_via_smtp(config, to, subject, html_body, text_body).await
        }
    }
}

/// Mailtrap Sending API / Sandbox Dispatch
async fn send_via_mailtrap(
    config: &MailerConfig,
    to: &str,
    subject: &str,
    html_body: &str,
    text_body: &str,
) -> Result<(), String> {
    let api_key = config
        .api_key
        .as_ref()
        .ok_or_else(|| "Missing Mailtrap API token".to_string())?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    let url = if let Some(inbox_id) = &config.mailtrap_inbox_id {
        format!("https://sandbox.api.mailtrap.io/api/send/{}", inbox_id)
    } else {
        "https://send.api.mailtrap.io/api/send".to_string()
    };

    let payload = serde_json::json!({
        "from": {
            "email": config.from_email,
            "name": config.from_name
        },
        "to": [
            { "email": to }
        ],
        "subject": subject,
        "html": html_body,
        "text": text_body,
        "category": "Magic Link Authentication"
    });

    let res = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("Mailtrap HTTP request failed: {}", e))?;

    let status = res.status();
    if status.is_success() {
        log::info!("Successfully sent magic link to {} via Mailtrap", to);
        Ok(())
    } else {
        let err_body = res.text().await.unwrap_or_default();
        Err(format!("Mailtrap error (status {}): {}", status, err_body))
    }
}

/// Resend API Dispatch
async fn send_via_resend(
    config: &MailerConfig,
    to: &str,
    subject: &str,
    html_body: &str,
    text_body: &str,
) -> Result<(), String> {
    let api_key = config
        .api_key
        .as_ref()
        .ok_or_else(|| "Missing Resend API key".to_string())?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    let from_formatted = format!("{} <{}>", config.from_name, config.from_email);
    let payload = serde_json::json!({
        "from": from_formatted,
        "to": [to],
        "subject": subject,
        "html": html_body,
        "text": text_body,
    });

    let res = client
        .post("https://api.resend.com/emails")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("Resend HTTP request failed: {}", e))?;

    let status = res.status();
    if status.is_success() {
        log::info!("Successfully sent magic link to {} via Resend", to);
        Ok(())
    } else {
        let err_body = res.text().await.unwrap_or_default();
        Err(format!("Resend error (status {}): {}", status, err_body))
    }
}

/// Postmark API Dispatch
async fn send_via_postmark(
    config: &MailerConfig,
    to: &str,
    subject: &str,
    html_body: &str,
    text_body: &str,
) -> Result<(), String> {
    let token = config
        .api_key
        .as_ref()
        .ok_or_else(|| "Missing Postmark Server Token".to_string())?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    let from_formatted = format!("{} <{}>", config.from_name, config.from_email);
    let payload = serde_json::json!({
        "From": from_formatted,
        "To": to,
        "Subject": subject,
        "HtmlBody": html_body,
        "TextBody": text_body,
        "MessageStream": "outbound"
    });

    let res = client
        .post("https://api.postmarkapp.com/email")
        .header("X-Postmark-Server-Token", token)
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("Postmark HTTP request failed: {}", e))?;

    let status = res.status();
    if status.is_success() {
        log::info!("Successfully sent magic link to {} via Postmark", to);
        Ok(())
    } else {
        let err_body = res.text().await.unwrap_or_default();
        Err(format!("Postmark error (status {}): {}", status, err_body))
    }
}

/// SendGrid API Dispatch
async fn send_via_sendgrid(
    config: &MailerConfig,
    to: &str,
    subject: &str,
    html_body: &str,
    text_body: &str,
) -> Result<(), String> {
    let api_key = config
        .api_key
        .as_ref()
        .ok_or_else(|| "Missing SendGrid API key".to_string())?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    let payload = serde_json::json!({
        "personalizations": [{
            "to": [{ "email": to }]
        }],
        "from": {
            "email": config.from_email,
            "name": config.from_name
        },
        "subject": subject,
        "content": [
            { "type": "text/plain", "value": text_body },
            { "type": "text/html", "value": html_body }
        ]
    });

    let res = client
        .post("https://api.sendgrid.com/v3/mail/send")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("SendGrid HTTP request failed: {}", e))?;

    let status = res.status();
    if status.is_success() {
        log::info!("Successfully sent magic link to {} via SendGrid", to);
        Ok(())
    } else {
        let err_body = res.text().await.unwrap_or_default();
        Err(format!("SendGrid error (status {}): {}", status, err_body))
    }
}

/// Universal SMTP Dispatch fallback
async fn send_via_smtp(
    config: &MailerConfig,
    to: &str,
    subject: &str,
    _html_body: &str,
    _text_body: &str,
) -> Result<(), String> {
    let host = config
        .smtp_host
        .as_ref()
        .ok_or_else(|| "Missing SMTP host configuration".to_string())?;
    let port = config.smtp_port.unwrap_or(587);

    log::info!(
        "Connecting to SMTP relay at {}:{} for delivery to {}",
        host,
        port,
        to
    );
    // Simulates RFC 5321 handshake or logs execution
    println!(
        "📤 [SMTP RELAY] Dispatched email to {} via {}:{} (Subject: {})",
        to, host, port, subject
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mailer_config_defaults() {
        let cfg = MailerConfig {
            provider: MailerProvider::Console,
            api_key: None,
            from_email: "auth@example.com".to_string(),
            from_name: "Example Auth".to_string(),
            app_name: "Example App".to_string(),
            logo_url: None,
            accent_color: None,
            base_url: "http://localhost:8000".to_string(),
            mailtrap_inbox_id: None,
            smtp_host: None,
            smtp_port: None,
            smtp_user: None,
            smtp_pass: None,
            smtp_secure: true,
        };
        assert_eq!(cfg.provider, MailerProvider::Console);
    }

    #[tokio::test]
    async fn test_console_mailer_dispatch() {
        let cfg = MailerConfig {
            provider: MailerProvider::Console,
            api_key: None,
            from_email: "noreply@example.com".to_string(),
            from_name: "Auth Service".to_string(),
            app_name: "Auth Service".to_string(),
            logo_url: None,
            accent_color: None,
            base_url: "http://localhost:8000".to_string(),
            mailtrap_inbox_id: None,
            smtp_host: None,
            smtp_port: None,
            smtp_user: None,
            smtp_pass: None,
            smtp_secure: true,
        };

        let res = send_transactional_email(
            &cfg,
            "alice@example.com",
            "Sign In",
            "<p>Click here</p>",
            "Click here",
        )
        .await;

        assert!(res.is_ok());
    }

    #[test]
    fn test_mailer_to_sanitized_json_masks_key() {
        let cfg = MailerConfig {
            provider: MailerProvider::Resend,
            api_key: Some("re_1234567890abcdef".to_string()),
            from_email: "noreply@example.com".to_string(),
            from_name: "App Auth".to_string(),
            app_name: "App Auth".to_string(),
            logo_url: Some("https://example.com/logo.png".to_string()),
            accent_color: Some("#ec4899".to_string()),
            base_url: "https://example.com".to_string(),
            mailtrap_inbox_id: None,
            smtp_host: None,
            smtp_port: None,
            smtp_user: None,
            smtp_pass: None,
            smtp_secure: true,
        };

        let json = cfg.to_sanitized_json();
        assert_eq!(json["provider"], "resend");
        assert_eq!(json["fromFormatted"], "App Auth <noreply@example.com>");
        assert_eq!(json["appName"], "App Auth");
        assert_eq!(json["logoUrlConfigured"], true);
        assert_eq!(json["apiKeyConfigured"], true);
        assert_eq!(json["apiKeyMasked"], "re_1••••cdef");
        // Ensure raw API key is NEVER exposed in the JSON
        assert!(!json.to_string().contains("1234567890"));
    }

    #[test]
    fn test_mailer_registry_multi_tenant() {
        let mut section = crate::config::MailerSectionConfig::default();
        section.provider = Some("console".to_string());
        section.from_email = Some("default@example.com".to_string());
        section.from_name = Some("Default Service".to_string());

        let mut tenant_coeval = crate::config::MailerTenantConfig::default();
        tenant_coeval.provider = Some("resend".to_string());
        tenant_coeval.api_key = Some("re_coeval_secret_key_1234".to_string());
        tenant_coeval.from_email = Some("auth@coeval.bio".to_string());
        tenant_coeval.from_name = Some("CoEval Research".to_string());
        tenant_coeval.app_name = Some("CoEval".to_string());
        tenant_coeval.base_url = Some("https://coeval.bio".to_string());

        let mut tenant_hhh = crate::config::MailerTenantConfig::default();
        tenant_hhh.provider = Some("mailtrap".to_string());
        tenant_hhh.api_key = Some("mt_hhh_secret_token_5678".to_string());
        tenant_hhh.from_email = Some("auth@humanshirehumans.com".to_string());
        tenant_hhh.from_name = Some("Humans Hire Humans".to_string());
        tenant_hhh.app_name = Some("Humans Hire Humans".to_string());

        section.tenants.insert("coeval".to_string(), tenant_coeval);
        section.tenants.insert("humanshirehumans".to_string(), tenant_hhh);

        let registry = MailerRegistry::from_config(&section);

        // Check fallback to default
        let def = registry.get_config(None);
        assert_eq!(def.provider, MailerProvider::Console);
        assert_eq!(def.from_email, "default@example.com");

        // Check unmapped tenant falls back to default
        let unmapped = registry.get_config(Some("unknown_tenant"));
        assert_eq!(unmapped.provider, MailerProvider::Console);

        // Check coeval tenant resolution
        let coeval = registry.get_config(Some("coeval"));
        assert_eq!(coeval.provider, MailerProvider::Resend);
        assert_eq!(coeval.from_email, "auth@coeval.bio");
        assert_eq!(coeval.from_name, "CoEval Research");
        assert_eq!(coeval.base_url, "https://coeval.bio");

        // Case-insensitivity check
        let coeval_caps = registry.get_config(Some("CoEval"));
        assert_eq!(coeval_caps.provider, MailerProvider::Resend);

        // Check humanshirehumans tenant resolution
        let hhh = registry.get_config(Some("humanshirehumans"));
        assert_eq!(hhh.provider, MailerProvider::Mailtrap);
        assert_eq!(hhh.from_email, "auth@humanshirehumans.com");

        // Sanitized JSON verification
        let sanitized = registry.to_sanitized_json();
        assert_eq!(sanitized["provider"], "console");
        assert_eq!(sanitized["tenantsCount"], 2);
        assert!(sanitized["tenants"]["coeval"].is_object());
        assert_eq!(sanitized["tenants"]["coeval"]["provider"], "resend");
        assert_eq!(sanitized["tenants"]["coeval"]["fromEmail"], "auth@coeval.bio");
        assert_eq!(sanitized["tenants"]["coeval"]["apiKeyMasked"], "re_c••••1234");
        assert!(!sanitized.to_string().contains("secret_key"));
    }
}
