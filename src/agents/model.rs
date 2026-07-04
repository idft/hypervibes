use chrono::{DateTime, Utc};
use serde::Deserialize;

pub const BACKEND_KIND_OPENCODE: &str = "opencode";

pub fn is_valid_backend_kind(value: &str) -> bool {
    matches!(value, BACKEND_KIND_OPENCODE)
}

pub fn is_valid_runtime_id(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return false;
    }

    let mut last = first;
    for ch in chars {
        if !ch.is_ascii_lowercase() && !ch.is_ascii_digit() && ch != '-' {
            return false;
        }
        last = ch;
    }

    last.is_ascii_lowercase() || last.is_ascii_digit()
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AgentRuntimeRow {
    pub id: String,
    pub created_at: DateTime<Utc>,
    #[allow(dead_code)]
    pub updated_at: DateTime<Utc>,
    pub name: String,
    pub backend_kind: String,
    pub enabled: bool,
    pub base_url: Option<String>,
    #[allow(dead_code)]
    pub runtime_config: serde_json::Value,
}

/// Row shape returned by the registry list query.
#[derive(Debug, Clone, sqlx::FromRow)]
#[allow(dead_code)]
pub struct AgentListRow {
    pub display_name: String,
    pub agent_key: String,
    pub enabled: bool,
    pub wallet_address: String,
    pub environment: String,
    pub api_key: String,
    pub api_key_last_used_at: Option<DateTime<Utc>>,
    pub backend_kind: String,
    pub runtime_id: String,
    pub runtime_name: String,
    pub runtime_base_url: Option<String>,
}

/// Row shape returned by the single-agent detail query.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AgentDetailRow {
    pub display_name: String,
    pub agent_key: String,
    pub enabled: bool,
    pub analysis_prompt: String,
    pub trading_prompt: String,
    pub wallet_address: String,
    pub environment: String,
    pub api_key: String,
    pub api_key_last_used_at: Option<DateTime<Utc>>,
    pub backend_kind: String,
    pub runtime_id: String,
    pub runtime_name: String,
    pub runtime_base_url: Option<String>,
    pub runtime_config: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Full registry row as stored in Postgres.
#[derive(Debug, Clone)]
pub struct AgentRegistryRow {
    pub agent_key: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub enabled: bool,
    pub display_name: String,
    pub analysis_prompt: String,
    pub trading_prompt: String,
    pub wallet_address: String,
    pub environment: String,
    pub api_key: String,
    pub api_key_last_used_at: Option<DateTime<Utc>>,
    pub backend_kind: String,
    pub runtime_id: String,
    pub runtime_config: serde_json::Value,
    pub hyperliquid_private_key_ciphertext: Vec<u8>,
    pub hyperliquid_private_key_key_id: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct CreateAgentRuntimeForm {
    pub id: String,
    pub name: String,
    pub backend_kind: String,
    pub base_url: String,
    pub enabled: Option<String>,
}

impl CreateAgentRuntimeForm {
    pub fn enabled(&self) -> bool {
        self.enabled.is_some()
    }

    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();

        let id = self.id.trim();
        if id.is_empty() {
            errors.push("Runtime ID is required.".to_string());
        } else if !is_valid_runtime_id(id) {
            errors.push(
                "Runtime ID must use lowercase letters, digits, or hyphens, and start/end with a letter or digit."
                    .to_string(),
            );
        }

        if self.name.trim().is_empty() {
            errors.push("Name is required.".to_string());
        }

        let backend_kind = self.backend_kind.trim();
        if !is_valid_backend_kind(backend_kind) {
            errors.push("Backend kind must be opencode.".to_string());
        }

        let base_url = self.base_url.trim();
        if backend_kind == BACKEND_KIND_OPENCODE && base_url.is_empty() {
            errors.push("Base URL is required for OpenCode runtimes.".to_string());
        }
        if !base_url.is_empty() && reqwest::Url::parse(base_url).is_err() {
            errors.push("Base URL must be a valid URL.".to_string());
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// Operator input when creating an agent.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CreateAgentForm {
    pub display_name: String,
    pub hyperliquid_private_key: String,
    pub runtime_id: String,
    /// HTML checkboxes only send a value when checked, so this is optional.
    pub enabled: Option<String>,
}

impl CreateAgentForm {
    /// Whether the operator left the enabled checkbox checked.
    pub fn enabled(&self) -> bool {
        self.enabled.is_some()
    }

    /// Validate the form and return a list of user-facing errors.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();

        let display_name = self.display_name.trim();
        if display_name.is_empty() {
            errors.push("Display name is required.".to_string());
        } else {
            let key = slugify_agent_key(display_name);
            if key.is_empty() {
                errors.push("Display name must contain some letters or digits.".to_string());
            }
        }

        let private_key = self.hyperliquid_private_key.trim();
        if private_key.is_empty() {
            errors.push("Hyperliquid private key is required.".to_string());
        } else if crate::agents::keys::derive_wallet_address(private_key).is_err() {
            errors.push("Hyperliquid private key is invalid.".to_string());
        }

        if self.runtime_id.trim().is_empty() {
            errors.push("Runtime is required.".to_string());
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// Derive a URL-safe agent key from a display name.
///
/// The result is lowercased and any run of characters that are not ASCII
/// letters or digits is collapsed to a single hyphen. Leading/trailing
/// hyphens are removed.
pub fn slugify_agent_key(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut prev_was_hyphen = true; // treat leading non-alphanum as hyphen to trim

    for c in name.trim().to_ascii_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            prev_was_hyphen = false;
        } else if !prev_was_hyphen {
            out.push('-');
            prev_was_hyphen = true;
        }
    }

    // trim trailing hyphen if present
    if out.ends_with('-') {
        out.pop();
    }

    out
}

#[cfg(test)]
mod tests {
    use super::{
        BACKEND_KIND_OPENCODE, CreateAgentForm, CreateAgentRuntimeForm, is_valid_backend_kind,
        is_valid_runtime_id, slugify_agent_key,
    };

    #[test]
    fn slugifies_display_name() {
        assert_eq!(slugify_agent_key("BTC Momentum"), "btc-momentum");
        assert_eq!(
            slugify_agent_key("ETH Mean-Reversion!!"),
            "eth-mean-reversion"
        );
        assert_eq!(slugify_agent_key("  SOL  Breakout  "), "sol-breakout");
        assert_eq!(slugify_agent_key("Macro Basket v2"), "macro-basket-v2");
    }

    #[test]
    fn slugify_rejects_only_special_chars() {
        assert!(slugify_agent_key("!!!").is_empty());
    }

    #[test]
    fn backend_kind_validation_accepts_known_values() {
        assert!(is_valid_backend_kind(BACKEND_KIND_OPENCODE));
        assert!(!is_valid_backend_kind("OpenCode"));
        assert!(!is_valid_backend_kind("other"));
    }

    #[test]
    fn runtime_id_validation_rejects_invalid_shapes() {
        assert!(is_valid_runtime_id("opencode-local"));
        assert!(is_valid_runtime_id("a1"));
        assert!(!is_valid_runtime_id(""));
        assert!(!is_valid_runtime_id("-bad"));
        assert!(!is_valid_runtime_id("bad-"));
        assert!(!is_valid_runtime_id("Bad"));
        assert!(!is_valid_runtime_id("bad_id"));
    }

    #[test]
    fn create_agent_runtime_form_rejects_invalid_backend_and_runtime_id() {
        let form = CreateAgentRuntimeForm {
            id: "Bad_Runtime".to_string(),
            name: "".to_string(),
            backend_kind: "other".to_string(),
            base_url: "not-a-url".to_string(),
            enabled: Some("on".to_string()),
        };

        let errors = form.validate().expect_err("validation should fail");
        assert!(errors.iter().any(|error| error.contains("Runtime ID")));
        assert!(
            errors
                .iter()
                .any(|error| error.contains("Name is required"))
        );
        assert!(
            errors
                .iter()
                .any(|error| error.contains("Backend kind must be opencode"))
        );
        assert!(
            errors
                .iter()
                .any(|error| error.contains("Base URL must be a valid URL"))
        );
    }

    #[test]
    fn create_agent_runtime_form_requires_base_url_for_opencode() {
        let form = CreateAgentRuntimeForm {
            id: "opencode-local".to_string(),
            name: "OpenCode local".to_string(),
            backend_kind: BACKEND_KIND_OPENCODE.to_string(),
            base_url: String::new(),
            enabled: Some("on".to_string()),
        };

        let errors = form.validate().expect_err("validation should fail");
        assert!(
            errors
                .iter()
                .any(|error| error.contains("Base URL is required for OpenCode runtimes"))
        );
    }

    #[test]
    fn create_agent_form_requires_runtime() {
        let form = CreateAgentForm {
            display_name: "Test Agent".to_string(),
            hyperliquid_private_key:
                "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d".to_string(),
            runtime_id: String::new(),
            enabled: Some("on".to_string()),
        };

        let errors = form.validate().expect_err("validation should fail");
        assert!(
            errors
                .iter()
                .any(|error| error.contains("Runtime is required"))
        );
    }
}
