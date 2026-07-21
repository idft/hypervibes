use chrono::{DateTime, Utc};
use serde::Deserialize;
use uuid::Uuid;

pub const AGENT_LIFECYCLE_ACTIVE: &str = "active";

/// Row shape returned by the registry list query.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AgentListRow {
    pub display_name: String,
    pub agent_key: String,
    pub enabled: bool,
    pub trading_account_address: String,
    pub environment: String,
    pub api_key_last_used_at: Option<DateTime<Utc>>,
}

/// Row shape returned by the single-agent detail query.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AgentDetailRow {
    pub user_id: Uuid,
    pub display_name: String,
    pub agent_key: String,
    pub enabled: bool,
    pub lifecycle: String,
    pub trading_account_address: Option<String>,
    pub environment: String,
    pub api_key: String,
    pub runtime_config: serde_json::Value,
}

impl AgentDetailRow {
    pub fn trading_account_address_display(&self) -> &str {
        self.trading_account_address.as_deref().unwrap_or("Pending")
    }
}

/// Full registry row as stored in Postgres.
#[derive(Debug, Clone)]
pub struct AgentRegistryRow {
    pub agent_key: String,
    pub user_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub enabled: bool,
    pub lifecycle: String,
    pub display_name: String,
    pub trading_account_address: Option<String>,
    pub environment: String,
    pub api_key: String,
    pub api_key_last_used_at: Option<DateTime<Utc>>,
    pub runtime_config: serde_json::Value,
}

/// Operator input when creating an agent.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct CreateAgentForm {
    pub display_name: String,
    pub trading_account_selection: String,
}

impl CreateAgentForm {
    /// Validate the form and return a list of user-facing errors.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();

        let display_name = self.display_name.trim();
        if display_name.is_empty() {
            errors.push("Name is required.".to_string());
        } else {
            let key = slugify_agent_key(display_name);
            if key.is_empty() {
                errors.push("Name must contain some letters or digits.".to_string());
            }
        }

        if self.trading_account_selection.trim().is_empty() {
            errors.push("Select a trading account.".to_string());
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
    use super::{CreateAgentForm, slugify_agent_key};

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
    fn create_agent_form_requires_a_name_and_trading_account_selection() {
        let form = CreateAgentForm {
            display_name: "Test Agent".to_string(),
            trading_account_selection: "main".to_string(),
        };

        assert!(form.validate().is_ok());

        assert!(
            CreateAgentForm {
                display_name: "Test Agent".to_string(),
                trading_account_selection: String::new(),
            }
            .validate()
            .is_err()
        );
    }
}
