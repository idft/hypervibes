use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use uuid::Uuid;

pub const CONVERSATION_CHANNEL_WEB: &str = "web";
pub const CONVERSATION_CHANNEL_TELEGRAM: &str = "telegram";
pub const TOOL_GROUP_ORDERS: &str = "orders";
pub const TOOL_GROUP_MEMORY_WRITES: &str = "memory_writes";
pub const TOOL_POLICY_DENY: &str = "deny";
pub const TOOL_POLICY_CONFIRM: &str = "confirm";
pub const TOOL_POLICY_ALLOW: &str = "allow";

#[derive(Debug, Clone)]
pub struct AgentConversationRow {
    pub id: Uuid,
    pub agent_key: String,
    pub opencode_session_id: String,
    pub channel: String,
    pub external_conversation_key: Option<String>,
    pub title: String,
    pub model_provider_id: String,
    pub model_id: String,
    pub model_variant: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub tool_policies: Vec<AgentConversationToolPolicyRow>,
}

impl AgentConversationRow {
    /// Builder-style helper that returns a clone with the supplied external
    /// conversation key. Used by [`crate::agent_conversations::service::ConversationService::create_web_conversation`]
    /// to erase the empty external key passed through the generic
    /// `create_gateway_conversation` entry point.
    pub fn with_external_conversation_key(mut self, value: Option<String>) -> Self {
        self.external_conversation_key = value;
        self
    }
}

#[derive(Debug, Clone)]
pub struct AgentConversationListRow {
    pub id: Uuid,
    pub agent_key: String,
    pub opencode_session_id: String,
    pub channel: String,
    pub external_conversation_key: Option<String>,
    pub title: String,
    pub model_provider_id: String,
    pub model_id: String,
    pub model_variant: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub tool_policies: Vec<AgentConversationToolPolicyRow>,
    pub opencode_status: Option<String>,
    pub opencode_updated_at: Option<DateTime<Utc>>,
    pub input_tokens: Option<i32>,
    pub output_tokens: Option<i32>,
    pub cache_read_tokens: Option<i32>,
    pub cache_write_tokens: Option<i32>,
    pub reasoning_tokens: Option<i32>,
    pub context_tokens: Option<i32>,
    pub peak_context_tokens: Option<i32>,
    pub estimated_cost: Option<Decimal>,
    pub compaction_count: Option<i32>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AgentConversationToolPolicyRow {
    pub conversation_id: Uuid,
    pub tool_group: String,
    pub policy: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct CreateAgentConversation {
    pub agent_key: String,
    pub opencode_session_id: String,
    pub channel: String,
    pub external_conversation_key: Option<String>,
    pub title: String,
    pub model_provider_id: String,
    pub model_id: String,
    pub model_variant: Option<String>,
}

impl CreateAgentConversation {
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();
        if self.agent_key.trim().is_empty() {
            errors.push("agent_key is required.".to_string());
        }
        if self.opencode_session_id.trim().is_empty() {
            errors.push("opencode_session_id is required.".to_string());
        }
        if self.channel.trim().is_empty() {
            errors.push("channel is required.".to_string());
        }
        if self
            .external_conversation_key
            .as_deref()
            .is_some_and(|key| key.trim().is_empty())
        {
            errors.push("external_conversation_key must not be empty if provided.".to_string());
        }
        if self.title.trim().is_empty() {
            errors.push("title is required.".to_string());
        }
        if self.model_provider_id.trim().is_empty() {
            errors.push("model_provider_id is required.".to_string());
        }
        if self.model_id.trim().is_empty() {
            errors.push("model_id is required.".to_string());
        }
        if self
            .model_variant
            .as_deref()
            .is_some_and(|variant| variant.trim().is_empty())
        {
            errors.push("model_variant must not be empty if provided.".to_string());
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct UpdateAgentConversationModel {
    pub model_provider_id: String,
    pub model_id: String,
    pub model_variant: Option<String>,
}

impl UpdateAgentConversationModel {
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();
        if self.model_provider_id.trim().is_empty() {
            errors.push("model_provider_id is required.".to_string());
        }
        if self.model_id.trim().is_empty() {
            errors.push("model_id is required.".to_string());
        }
        if self
            .model_variant
            .as_deref()
            .is_some_and(|variant| variant.trim().is_empty())
        {
            errors.push("model_variant must not be empty if provided.".to_string());
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversation_model_rejects_blank_variant() {
        let update = UpdateAgentConversationModel {
            model_provider_id: "anthropic".to_string(),
            model_id: "claude-sonnet-4".to_string(),
            model_variant: Some("  ".to_string()),
        };

        assert!(update.validate().is_err());
    }
}
