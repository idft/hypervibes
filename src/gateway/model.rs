use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const GATEWAY_TYPE_TELEGRAM: &str = "telegram";

const PENDING_LINK_TTL: Duration = Duration::from_secs(600);

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AgentGatewayRow {
    pub agent_key: String,
    pub config: serde_json::Value,
}

/// Type-specific configuration stored in `agent_gateways.config` for Telegram.
///
/// `bot_token_ciphertext` is a sealed blob produced by
/// [`crate::agents::crypto::encrypt`] (12-byte nonce + AEAD ciphertext +
/// tag). It is never rendered in templates, logs, or SSE events.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TelegramGatewayConfig {
    #[serde(default)]
    pub bot_token_ciphertext: Option<Vec<u8>>,
    #[serde(default)]
    pub bot_token_key_id: Option<String>,
    #[serde(default)]
    pub chat_id: Option<i64>,
    #[serde(default)]
    pub chat_username: Option<String>,
    #[serde(default)]
    pub bot_username: Option<String>,
}

impl TelegramGatewayConfig {
    /// Decode the `config` JSON column into the Telegram shape. Returns the
    /// default (empty) configuration when the column is missing or malformed
    /// so gateway tasks never panic on bad data.
    pub fn from_value(value: &serde_json::Value) -> Self {
        serde_json::from_value(value.clone()).unwrap_or_default()
    }

    /// Whether the gateway has the encrypted token material needed to poll
    /// Telegram. A chat binding is established by the first polled `/start`.
    pub fn is_ready(&self) -> bool {
        self.bot_token_ciphertext.is_some() && self.bot_token_key_id.is_some()
    }

    /// Render a JSON-safe view of the config for templates. Secret fields
    /// (ciphertext, key id) are omitted entirely.
    pub fn to_view(&self) -> TelegramGatewayConfigView {
        TelegramGatewayConfigView {
            bot_username: self.bot_username.clone(),
            chat_id: self.chat_id,
            chat_username: self.chat_username.clone(),
        }
    }
}

/// Render-safe projection of [`TelegramGatewayConfig`] for templates and API
/// responses. Never contains the encrypted token bytes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TelegramGatewayConfigView {
    pub bot_username: Option<String>,
    pub chat_id: Option<i64>,
    pub chat_username: Option<String>,
}

/// In-memory pending link entry created when an operator starts the Telegram
/// chat-binding flow. The token is returned to the operator as a deep link and
/// is consumed when Telegram receives the `/start` message.
#[derive(Debug, Clone)]
pub struct PendingLink {
    pub token: Uuid,
    pub agent_key: String,
    pub user_id: Uuid,
    pub chat_id: Option<i64>,
    pub chat_username: Option<String>,
    pub created_at: Instant,
}

impl PendingLink {
    pub fn new(agent_key: String, user_id: Uuid) -> Self {
        Self {
            token: Uuid::new_v4(),
            agent_key,
            user_id,
            chat_id: None,
            chat_username: None,
            created_at: Instant::now(),
        }
    }

    pub fn is_expired(&self) -> bool {
        self.created_at.elapsed() > PENDING_LINK_TTL
    }
}

/// Severity levels for notifications. Mirrors the `notifications.severity`
/// CHECK constraint from migration `0016`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NotificationSeverity {
    Info,
    Warning,
    Error,
}

impl NotificationSeverity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "info" => Some(Self::Info),
            "warning" => Some(Self::Warning),
            "error" => Some(Self::Error),
            _ => None,
        }
    }

    pub fn emoji(self) -> &'static str {
        match self {
            Self::Info => "\u{2139}\u{fe0f} ",
            Self::Warning => "\u{26a0}\u{fe0f} ",
            Self::Error => "\u{274c} ",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_link_expires_after_ttl() {
        let mut link = PendingLink::new("agent".to_string(), Uuid::new_v4());
        link.created_at = Instant::now() - Duration::from_secs(601);
        assert!(link.is_expired());
    }

    #[test]
    fn pending_link_not_expired_within_ttl() {
        let link = PendingLink::new("agent".to_string(), Uuid::new_v4());
        assert!(!link.is_expired());
    }

    #[test]
    fn telegram_config_from_empty_value_returns_default() {
        let config = TelegramGatewayConfig::from_value(&serde_json::Value::Null);
        assert!(!config.is_ready());
    }

    #[test]
    fn telegram_config_is_ready_when_token_material_is_present_without_chat() {
        let config = TelegramGatewayConfig {
            bot_token_ciphertext: Some(vec![1, 2, 3]),
            bot_token_key_id: Some("test".to_string()),
            ..Default::default()
        };
        assert!(config.is_ready());
    }

    #[test]
    fn telegram_config_is_not_ready_without_token_key_id() {
        let config = TelegramGatewayConfig {
            bot_token_ciphertext: Some(vec![1, 2, 3]),
            ..Default::default()
        };
        assert!(!config.is_ready());
    }

    #[test]
    fn telegram_config_view_omits_secrets() {
        let config = TelegramGatewayConfig {
            bot_token_ciphertext: Some(vec![1, 2, 3]),
            bot_token_key_id: Some("test".to_string()),
            bot_username: Some("bot".to_string()),
            chat_id: Some(42),
            chat_username: Some("user".to_string()),
        };
        let view = config.to_view();
        let json = serde_json::to_string(&view).expect("serialize view");
        assert!(!json.contains("ciphertext"));
        assert!(!json.contains("key_id"));
        assert!(json.contains("bot_username"));
        assert!(json.contains("42"));
    }

    #[test]
    fn notification_severity_parses_known_values() {
        assert_eq!(
            NotificationSeverity::parse("info"),
            Some(NotificationSeverity::Info)
        );
        assert_eq!(
            NotificationSeverity::parse("warning"),
            Some(NotificationSeverity::Warning)
        );
        assert_eq!(
            NotificationSeverity::parse("error"),
            Some(NotificationSeverity::Error)
        );
        assert_eq!(NotificationSeverity::parse("critical"), None);
    }

    #[test]
    fn notification_severity_emoji_distinct_per_level() {
        assert_eq!(NotificationSeverity::Info.emoji(), "\u{2139}\u{fe0f} ");
        assert_eq!(NotificationSeverity::Warning.emoji(), "\u{26a0}\u{fe0f} ");
        assert_eq!(NotificationSeverity::Error.emoji(), "\u{274c} ");
    }
}
