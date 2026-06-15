use chrono::{DateTime, Utc};
use serde::Deserialize;

/// Row shape returned by the registry list query.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AgentListRow {
    pub display_name: String,
    pub agent_key: String,
    pub enabled: bool,
    pub wallet_address: String,
    pub api_key: String,
    pub api_key_last_used_at: Option<DateTime<Utc>>,
}

/// Row shape returned by the single-agent detail query.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AgentDetailRow {
    pub display_name: String,
    pub agent_key: String,
    pub enabled: bool,
    pub prompt: String,
    pub wallet_address: String,
    pub api_key: String,
    pub api_key_last_used_at: Option<DateTime<Utc>>,
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
    pub prompt: String,
    pub wallet_address: String,
    pub api_key: String,
    pub api_key_last_used_at: Option<DateTime<Utc>>,
    pub hyperliquid_private_key_ciphertext: Vec<u8>,
    pub hyperliquid_private_key_key_id: String,
}

/// Operator input when creating an agent.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CreateAgentForm {
    pub display_name: String,
    pub hyperliquid_private_key: String,
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
    use super::slugify_agent_key;

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
}
