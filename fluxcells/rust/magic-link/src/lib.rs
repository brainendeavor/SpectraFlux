//! Magic Link Authentication Fluxcell for SpectraGQL
//!
//! Provides passwordless magic link authentication saga:
//! - Consumes `mutation.requestmagiclink` events
//! - Mints 256-bit single-use authentication tokens
//! - Renders responsive HTML/text email templates
//! - Exposes `/verify` endpoint for atomic token redemption (`GETDEL`)

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MagicLinkRoute {
    pub method: String,
    pub path: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MagicLinkRequest {
    pub email: String,
    #[serde(default)]
    pub redirect_uri: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MagicLinkTokenData {
    pub email: String,
    pub created_at: String,
    pub redirect_uri: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyResponse {
    pub status: String,
    pub email: String,
    pub session_id: String,
    pub redirect_uri: Option<String>,
}

/// Mints a cryptographically secure 256-bit token (64 hex characters)
pub fn mint_magic_token(email: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(Uuid::new_v4().as_bytes());
    hasher.update(email.as_bytes());
    hasher.update(Uuid::now_v7().as_bytes());
    let digest = hasher.finalize();
    hex_encode(&digest)
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

/// Escapes HTML metacharacters to prevent XSS and attribute injection in email clients
pub fn escape_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            _ => out.push(c),
        }
    }
    out
}

/// Sanitizes verification URL, guaranteeing http(s) scheme and percent-encoding quote/whitespace breakouts
pub fn sanitize_url(url: &str) -> String {
    let trimmed = url.trim();
    if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
        return "#".to_string();
    }
    let mut safe = String::with_capacity(trimmed.len());
    for c in trimmed.chars() {
        match c {
            '"' => safe.push_str("%22"),
            '\'' => safe.push_str("%27"),
            '<' => safe.push_str("%3C"),
            '>' => safe.push_str("%3E"),
            ' ' => safe.push_str("%20"),
            '`' => safe.push_str("%60"),
            _ => safe.push(c),
        }
    }
    safe
}

/// Configuration for email visual branding and white-labeling
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmailBranding {
    pub app_name: String,
    pub logo_url: Option<String>,
    pub accent_color: Option<String>,
    pub support_email: Option<String>,
}

impl Default for EmailBranding {
    fn default() -> Self {
        Self {
            app_name: "Auth Service".to_string(),
            logo_url: None,
            accent_color: Some("#38bdf8".to_string()),
            support_email: None,
        }
    }
}

/// Generates a cryptographically secure 6-digit numeric verification code (100000..=999999)
pub fn mint_magic_code() -> String {
    let bytes = Uuid::new_v4();
    let b = bytes.as_bytes();
    let num = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    let code = (num % 900_000) + 100_000;
    format!("{:06}", code)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedEmail {
    pub subject: String,
    pub html_body: String,
    pub text_body: String,
}

/// Renders both HTML and plain-text email templates with customizable branding and optional verification code
pub fn render_email_templates_branded(
    email: &str,
    verify_url: &str,
    code: Option<&str>,
    branding: Option<&EmailBranding>,
) -> RenderedEmail {
    let default_branding = EmailBranding::default();
    let brand = branding.unwrap_or(&default_branding);
    let app_name = if brand.app_name.trim().is_empty() {
        "Auth Service"
    } else {
        brand.app_name.trim()
    };
    let safe_app_name = escape_html(app_name);
    let safe_email = escape_html(email);
    let safe_url = sanitize_url(verify_url);
    let accent_color = brand.accent_color.as_deref().unwrap_or("#38bdf8");
    let safe_accent_color = if accent_color.starts_with('#') && (accent_color.len() == 7 || accent_color.len() == 4) {
        accent_color
    } else {
        "#38bdf8"
    };

    let subject = match (&code, branding) {
        (Some(c), Some(b)) => format!("{} - Your Sign-In Code is {}", b.app_name, c),
        (Some(c), None) => format!("Your Sign-In Code is {}", c),
        (None, Some(b)) => format!("{} - Your Secure Sign-In Link", b.app_name),
        (None, None) => "Your Secure Sign-In Link".to_string(),
    };

    let logo_html = if let Some(ref logo) = brand.logo_url {
        let safe_logo = sanitize_url(logo);
        if safe_logo != "#" {
            format!(
                r#"<div style="text-align: center; margin-bottom: 24px;"><img src="{}" alt="{}" style="height: 44px; max-width: 220px; object-fit: contain;" /></div>"#,
                safe_logo, safe_app_name
            )
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    let code_box_html = if let Some(c) = code {
        let safe_code = escape_html(c);
        format!(
            r#"<div style="margin: 28px 0; text-align: center; background: #0f172a; padding: 20px; border-radius: 8px; border: 1px solid #334155;">
        <div style="font-size: 11px; font-family: monospace; text-transform: uppercase; letter-spacing: 1.5px; color: #94a3b8; margin-bottom: 8px;">Verification Code</div>
        <div style="font-family: monospace; font-size: 32px; font-weight: 700; letter-spacing: 6px; color: {};">{}</div>
        <div style="font-size: 12px; color: #64748b; margin-top: 6px;">Enter this code in your browser to sign in</div>
      </div>"#,
            safe_accent_color, safe_code
        )
    } else {
        String::new()
    };

    let button_label = if code.is_some() {
        "Or Click Here to Sign In Directly"
    } else {
        "Verify & Sign In"
    };

    let html_body = format!(
        r#"<!DOCTYPE html>
<html>
<head><meta charset="utf-8"><title>{} Sign In</title></head>
<body style="font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; padding: 24px; background: #0f172a; color: #f8fafc;">
  <div style="max-width: 480px; margin: 0 auto; background: #1e293b; padding: 32px; border-radius: 12px; border: 1px solid #334155;">
    {}
    <h2 style="margin-top: 0; color: {}; text-align: center; font-size: 20px; font-weight: 700;">{} Sign In</h2>
    <p style="text-align: center; color: #cbd5e1; font-size: 14px;">We received a sign-in request for <strong>{}</strong>.</p>
    {}
    <div style="margin: 24px 0; text-align: center;">
      <a href="{}" style="background: {}; color: #0f172a; padding: 12px 24px; text-decoration: none; border-radius: 6px; font-weight: 600; display: inline-block;">{}</a>
    </div>
    <p style="color: #94a3b8; font-size: 13px; text-align: center;">This code and link are valid for 15 minutes and can only be used once.</p>
    <p style="color: #64748b; font-size: 12px; margin-top: 24px; text-align: center;">If you did not request this sign-in, you can safely ignore this email.</p>
  </div>
</body>
</html>"#,
        safe_app_name, logo_html, safe_accent_color, safe_app_name, safe_email, code_box_html, safe_url, safe_accent_color, button_label
    );

    let text_code_part = if let Some(c) = code {
        format!("\n\nYour verification code is: {}\n\nEnter this code in your browser, or click the link below to sign in:\n", c)
    } else {
        "\n\nClick the link below to sign in:\n".to_string()
    };

    let text_body = format!(
        "{} Sign In\n\nWe received a sign-in request for {}.{}{}\n\nThis code and link are valid for 15 minutes and can only be used once.\nIf you did not request this sign-in, you can safely ignore this email.",
        app_name, email, text_code_part, verify_url
    );

    RenderedEmail {
        subject,
        html_body,
        text_body,
    }
}

/// Backward-compatible wrapper rendering email templates without code or custom branding
pub fn render_email_templates(email: &str, verify_url: &str) -> RenderedEmail {
    render_email_templates_branded(email, verify_url, None, None)
}

/// Backward-compatible wrapper returning (subject, html_body)
pub fn render_magic_link_email(email: &str, verify_url: &str) -> (String, String) {
    let rendered = render_email_templates(email, verify_url);
    (rendered.subject, rendered.html_body)
}

pub fn get_subscriptions() -> Vec<String> {
    vec![
        "auth.magic_link".to_string(),
        "mutation.requestmagiclink".to_string(),
    ]
}

pub fn get_routes() -> Vec<MagicLinkRoute> {
    vec![
        MagicLinkRoute {
            method: "GET".to_string(),
            path: "/verify".to_string(),
            description: "Verify magic link token and exchange for session".to_string(),
        },
        MagicLinkRoute {
            method: "POST".to_string(),
            path: "/verify".to_string(),
            description: "API redemption of magic link token".to_string(),
        },
        MagicLinkRoute {
            method: "GET".to_string(),
            path: "/status".to_string(),
            description: "Auth service health and status".to_string(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_minting_entropy_and_uniqueness() {
        let mut tokens = std::collections::HashSet::new();
        for _ in 0..100 {
            let token = mint_magic_token("alice@example.com");
            assert_eq!(token.len(), 64);
            assert!(tokens.insert(token)); // Must all be unique
        }
    }

    #[test]
    fn test_render_email() {
        let (subj, body) = render_magic_link_email(
            "test@example.com",
            "https://api.example.com/auth/verify?token=abc123xyz",
        );
        assert_eq!(subj, "Your Secure Sign-In Link");
        assert!(body.contains("test@example.com"));
        assert!(body.contains("https://api.example.com/auth/verify?token=abc123xyz"));
        assert!(body.contains("15 minutes"));
    }

    #[test]
    fn test_email_html_escaping_neutralizes_xss() {
        let malicious_email = "alice<script>alert('xss')</script>@example.com";
        let malicious_url = "https://example.com/verify?token=123\" onmouseover=\"alert(1)";

        let rendered = render_email_templates(malicious_email, malicious_url);

        // Raw malicious script and quote breakout MUST NOT be present
        assert!(!rendered.html_body.contains("<script>"));
        assert!(!rendered.html_body.contains("\" onmouseover="));

        // Sanitized entities and percent encoding MUST be present
        assert!(rendered.html_body.contains("&lt;script&gt;"));
        assert!(rendered.html_body.contains("%22%20onmouseover=%22alert(1)"));

        // Plaintext must preserve readable form without mangling
        assert!(rendered.text_body.contains(malicious_email));
        assert!(rendered.text_body.contains(malicious_url));
    }

    #[test]
    fn test_url_scheme_sanitization() {
        assert_eq!(sanitize_url("javascript:alert(1)"), "#");
        assert_eq!(sanitize_url("data:text/html,<script>alert(1)</script>"), "#");
        assert_eq!(sanitize_url("  https://auth.example.com/verify  "), "https://auth.example.com/verify");
        assert_eq!(sanitize_url("http://localhost:8080/auth"), "http://localhost:8080/auth");
    }

    #[test]
    fn test_routes_and_subscriptions() {
        assert_eq!(get_subscriptions().len(), 2);
        assert_eq!(get_routes().len(), 3);
    }

    #[test]
    fn test_magic_link_adversarial_mass_token_minting() {
        use std::sync::{Arc, Mutex};
        let tokens = Arc::new(Mutex::new(std::collections::HashSet::new()));
        let num_threads = 8;
        let tokens_per_thread = 500;

        let mut handles = Vec::new();
        for t in 0..num_threads {
            let tokens_clone = tokens.clone();
            handles.push(std::thread::spawn(move || {
                for i in 0..tokens_per_thread {
                    let email = format!("user-{}@thread-{}.com", i, t);
                    let tok = mint_magic_token(&email);
                    assert_eq!(tok.len(), 64);
                    let mut set = tokens_clone.lock().unwrap();
                    assert!(set.insert(tok), "Duplicate token detected!");
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        let total = tokens.lock().unwrap().len();
        assert_eq!(total, num_threads * tokens_per_thread);
    }

    #[test]
    fn test_magic_link_adversarial_email_fuzzing() {
        // 1. 10KB oversized email
        let huge_email = format!("{}@example.com", "a".repeat(10_000));
        let rendered = render_email_templates(&huge_email, "https://auth.example.com/verify?token=123");
        assert!(rendered.html_body.contains("example.com"));

        // 2. Unicode bidirectional override attack
        let bidi_email = "victim\u{202E}moc.live@admin.com";
        let rendered_bidi = render_email_templates(bidi_email, "https://auth.example.com/verify?token=123");
        assert!(rendered_bidi.html_body.contains("victim"));

        // 3. Nested tag breakouts
        let nested = "<<<<a href=javascript:steal()>CLICK>>>>";
        let rendered_nested = render_email_templates(nested, "https://auth.example.com/verify?token=123");
        assert!(!rendered_nested.html_body.contains("<a href=javascript:"));
        assert!(rendered_nested.html_body.contains("&lt;&lt;&lt;&lt;a href=javascript:steal()&gt;CLICK&gt;&gt;&gt;&gt;"));
    }

    #[test]
    fn test_mint_magic_code_range_and_entropy() {
        let mut codes = std::collections::HashSet::new();
        for _ in 0..100 {
            let code = mint_magic_code();
            assert_eq!(code.len(), 6);
            let val: u32 = code.parse().expect("Must be valid 6-digit number");
            assert!((100_000..=999_999).contains(&val));
            codes.insert(code);
        }
        // Entropy check: At least 95 unique codes out of 100
        assert!(codes.len() >= 95);
    }

    #[test]
    fn test_render_email_templates_branded_with_code_and_logo() {
        let branding = EmailBranding {
            app_name: "Open CoEval".to_string(),
            logo_url: Some("https://opencoeval.bio/public/images/logo.png".to_string()),
            accent_color: Some("#fa48c5".to_string()),
            support_email: Some("support@opencoeval.bio".to_string()),
        };

        let rendered = render_email_templates_branded(
            "alice@example.com",
            "https://opencoeval.bio/auth/verify?token=tok-123",
            Some("482910"),
            Some(&branding),
        );

        assert_eq!(rendered.subject, "Open CoEval - Your Sign-In Code is 482910");
        assert!(rendered.html_body.contains("Open CoEval Sign In"));
        assert!(rendered.html_body.contains("https://opencoeval.bio/public/images/logo.png"));
        assert!(rendered.html_body.contains("#fa48c5"));
        assert!(rendered.html_body.contains("482910"));
        assert!(rendered.html_body.contains("Or Click Here to Sign In Directly"));
        assert!(rendered.text_body.contains("Your verification code is: 482910"));
        assert!(rendered.text_body.contains("Open CoEval Sign In"));
    }

    #[test]
    fn test_render_email_templates_branded_sanitizes_logo_url() {
        let malicious_branding = EmailBranding {
            app_name: "EvilCorp<script>alert(1)</script>".to_string(),
            logo_url: Some("javascript:alert('xss')".to_string()),
            accent_color: Some("red; font-size: 100px;".to_string()), // Invalid hex
            support_email: None,
        };

        let rendered = render_email_templates_branded(
            "alice@example.com",
            "https://auth.example.com/verify?token=123",
            Some("123456"),
            Some(&malicious_branding),
        );

        assert!(!rendered.html_body.contains("<script>"));
        assert!(!rendered.html_body.contains("javascript:"));
        // Accent color should fall back to safe default
        assert!(rendered.html_body.contains("#38bdf8"));
    }
}

