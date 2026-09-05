use std::{collections::HashSet, sync::Arc, time::Duration};

use anyhow::{Context, Result, anyhow, bail};
use tokio::sync::Mutex;
use tracing::warn;
use uuid::Uuid;

use crate::{
    agent_conversations::{
        model::{
            AgentConversationRow, AgentConversationToolPolicyRow, CreateAgentConversation,
            TOOL_GROUP_MEMORY_WRITES, TOOL_GROUP_NOTIFICATIONS, TOOL_GROUP_ORDERS,
            TOOL_POLICY_ALLOW, TOOL_POLICY_CONFIRM, TOOL_POLICY_DENY,
        },
        store,
    },
    agents::store::get_agent,
    db::DbPool,
    harness::{in_flight::InFlightTracker, model::CAPABILITY_SCHEMA_VERSION},
    opencode::{
        client::{
            DeleteSessionResult, OpenCodeClient, OpenCodePermissionReply, OpenCodePermissionRule,
            SessionStatusKind,
        },
        workspace_control_client::WorkspaceController,
    },
};

pub const CONVERSATION_PROFILE: &str = "agent-conversations";
pub const CONVERSATION_MESSAGE_MAX_CHARS: usize = 12_000;
pub const CONVERSATION_TITLE_MAX_CHARS: usize = 100;
const ACTIVE_TURN_POLL_INTERVAL: Duration = Duration::from_secs(2);
const ACTIVE_TURN_TIMEOUT: Duration = Duration::from_secs(30 * 60);

#[derive(Clone, Default)]
pub struct ConversationTurnTracker {
    active: Arc<Mutex<HashSet<Uuid>>>,
}

impl ConversationTurnTracker {
    pub async fn try_acquire(&self, conversation_id: Uuid) -> Option<ConversationTurnGuard> {
        let mut active = self.active.lock().await;
        active
            .insert(conversation_id)
            .then_some(ConversationTurnGuard {
                active: Arc::clone(&self.active),
                conversation_id,
            })
    }

    pub async fn is_active(&self, conversation_id: Uuid) -> bool {
        self.active.lock().await.contains(&conversation_id)
    }
}

pub struct ConversationTurnGuard {
    active: Arc<Mutex<HashSet<Uuid>>>,
    conversation_id: Uuid,
}

impl Drop for ConversationTurnGuard {
    fn drop(&mut self) {
        let active = Arc::clone(&self.active);
        let conversation_id = self.conversation_id;
        tokio::spawn(async move {
            active.lock().await.remove(&conversation_id);
        });
    }
}

pub struct ConversationService<'a> {
    pub pool: &'a DbPool,
    pub client: &'a OpenCodeClient,
    pub base_url: &'a str,
    pub agent_api_base_url: &'a str,
    pub workspace_controller: &'a dyn WorkspaceController,
    pub in_flight: &'a InFlightTracker,
    pub turn_tracker: &'a ConversationTurnTracker,
    pub shutdown_rx: tokio::sync::watch::Receiver<bool>,
}

impl<'a> ConversationService<'a> {
    pub async fn create_web_conversation(
        &self,
        agent_key: &str,
        provider_id: &str,
        model_id: &str,
        model_variant: Option<&str>,
    ) -> Result<AgentConversationRow> {
        self.create_gateway_conversation(
            agent_key,
            crate::agent_conversations::model::CONVERSATION_CHANNEL_WEB,
            "",
            provider_id,
            model_id,
            model_variant,
        )
        .await
        .map(|row| row.with_external_conversation_key(None))
    }

    /// Create a new conversation bound to an external gateway channel
    /// (e.g. `telegram`) and the external chat id (e.g. the Telegram chat
    /// id as a string). Reuses the same OpenCode session creation flow as
    /// `create_web_conversation` and is the entry point used by the
    /// gateway service when an operator sends `/new` or first messages
    /// the bot.
    pub async fn create_gateway_conversation(
        &self,
        agent_key: &str,
        channel: &str,
        external_conversation_key: &str,
        provider_id: &str,
        model_id: &str,
        model_variant: Option<&str>,
    ) -> Result<AgentConversationRow> {
        let agent = self.load_agent(agent_key).await?;
        let external_key = if external_conversation_key.trim().is_empty() {
            None
        } else {
            Some(external_conversation_key.trim().to_string())
        };
        let input = CreateAgentConversation {
            agent_key: agent_key.to_string(),
            opencode_session_id: format!("pending_{}", Uuid::new_v4()),
            channel: channel.to_string(),
            external_conversation_key: external_key,
            title: "New conversation".to_string(),
            model_provider_id: provider_id.to_string(),
            model_id: model_id.to_string(),
            model_variant: model_variant.map(ToOwned::to_owned),
        };
        let conversation =
            store::create_conversation_with_default_policies(self.pool, &input).await?;
        let workspace = async {
            crate::agent_conversations::workspace::prepare_conversation_workspace(
                self.pool,
                agent_key,
                conversation.id,
                CAPABILITY_SCHEMA_VERSION,
            )
            .await?;
            let workspace = self
                .workspace_controller
                .materialize_conversation_workspace(
                    agent_key,
                    conversation.id,
                    workspace_store::workspace::ConversationWorkspaceMaterializationInput {
                        display_name: agent.display_name.clone(),
                        api_base_url: self.agent_api_base_url.to_string(),
                        api_key: agent.api_key.clone(),
                    },
                    &format!("conversation-materialize-{}", conversation.id),
                )
                .await?;
            crate::agent_conversations::workspace::mark_conversation_workspace_ready(
                self.pool,
                agent_key,
                conversation.id,
            )
            .await?;
            Result::<_, anyhow::Error>::Ok(workspace)
        }
        .await;
        match workspace {
            Ok(workspace) => match self
                .client
                .create_conversation_session(
                    self.base_url,
                    &workspace.workspace_container_path,
                    provider_id,
                    model_id,
                    model_variant,
                    default_permission_rules(),
                )
                .await
            {
                Ok(session) => {
                    if !store::set_conversation_session_id(
                        self.pool,
                        agent_key,
                        conversation.id,
                        &session.id,
                    )
                    .await?
                    {
                        let _ = self
                            .client
                            .delete_session(
                                self.base_url,
                                &workspace.workspace_container_path,
                                &session.id,
                            )
                            .await;
                        let _ = self
                            .workspace_controller
                            .delete_conversation_workspace(
                                agent_key,
                                conversation.id,
                                &format!("conversation-delete-{}", conversation.id),
                            )
                            .await;
                        bail!("Conversation no longer exists.");
                    }
                    Ok(AgentConversationRow {
                        opencode_session_id: session.id,
                        ..conversation
                    })
                }
                Err(error) => {
                    let _ = self
                        .workspace_controller
                        .delete_conversation_workspace(
                            agent_key,
                            conversation.id,
                            &format!("conversation-delete-{}", conversation.id),
                        )
                        .await;
                    let _ =
                        store::delete_conversation_mapping(self.pool, agent_key, conversation.id)
                            .await;
                    Err(error)
                }
            },
            Err(error) => {
                let _ =
                    store::delete_conversation_mapping(self.pool, agent_key, conversation.id).await;
                Err(error).context("failed to materialize conversation workspace")
            }
        }
    }

    pub async fn submit_conversation_turn(
        &self,
        agent_key: &str,
        conversation_id: Uuid,
        message_id: &str,
        message: &str,
    ) -> Result<()> {
        let text = message.trim();
        if text.is_empty() {
            bail!("Message cannot be blank.");
        }
        if text.chars().count() > CONVERSATION_MESSAGE_MAX_CHARS {
            bail!("Message must be at most {CONVERSATION_MESSAGE_MAX_CHARS} characters.");
        }
        if !message_id.starts_with("msg_") {
            bail!("Invalid message id.");
        }
        self.load_agent(agent_key).await?;
        let conversation = self.load_conversation(agent_key, conversation_id).await?;
        let Some(turn_guard) = self.turn_tracker.try_acquire(conversation_id).await else {
            bail!("This conversation already has a turn in progress.");
        };
        let workspace = self.workspace_path(agent_key, conversation_id).await?;
        let status = self
            .client
            .get_session_status_in_directory(
                self.base_url,
                &conversation.opencode_session_id,
                Some(&workspace),
            )
            .await?;
        if status.as_ref().is_some_and(SessionStatusKind::is_active) {
            bail!("The current turn is still running. Wait for it to finish or stop it first.");
        }
        self.client
            .send_conversation_prompt_async(
                self.base_url,
                &workspace,
                &conversation.opencode_session_id,
                &crate::opencode::client::OpenCodeConversationPrompt {
                    message_id: message_id.to_string(),
                    provider_id: conversation.model_provider_id.clone(),
                    model_id: conversation.model_id.clone(),
                    variant: conversation.model_variant.clone(),
                    text: text.to_string(),
                },
            )
            .await?;
        let client = self.client.clone();
        let base_url = self.base_url.to_string();
        let conversation_title = first_turn_title(&conversation, text);
        let conversation_agent_key = conversation.agent_key;
        let conversation_id = conversation.id;
        let session_id = conversation.opencode_session_id;
        let pool = self.pool.clone();
        let mut shutdown_rx = self.shutdown_rx.clone();
        let in_flight = self.in_flight.track();
        tokio::spawn(async move {
            let _turn_guard = turn_guard;
            let _in_flight = in_flight;
            if let Some(title) = conversation_title {
                if let Err(error) = client
                    .update_session_title(&base_url, &workspace, &session_id, &title)
                    .await
                {
                    warn!(conversation_id = %conversation_id, error = ?error, "failed to set conversation title in OpenCode");
                } else if let Err(error) = store::update_conversation_title(
                    &pool,
                    &conversation_agent_key,
                    conversation_id,
                    &title,
                )
                .await
                {
                    warn!(conversation_id = %conversation_id, error = ?error, "failed to persist conversation title");
                }
            }
            let waited = tokio::time::timeout(ACTIVE_TURN_TIMEOUT, async {
                loop {
                    tokio::select! {
                        _ = tokio::time::sleep(ACTIVE_TURN_POLL_INTERVAL) => {},
                        changed = shutdown_rx.changed() => {
                            if changed.is_err() || *shutdown_rx.borrow() {
                                return;
                            }
                        }
                    }
                    match client.get_session_status_in_directory(&base_url, &session_id, Some(&workspace)).await {
                        Ok(Some(status)) if status.is_active() => continue,
                        Ok(_) => return,
                        Err(error) => {
                            warn!(session_id, error = ?error, "failed to observe active conversation turn");
                            return;
                        }
                    }
                }
            })
            .await;
            if waited.is_err() {
                warn!(
                    session_id,
                    timeout_seconds = ACTIVE_TURN_TIMEOUT.as_secs(),
                    "conversation turn observation timed out"
                );
            }
        });
        Ok(())
    }

    pub async fn stop_conversation(&self, agent_key: &str, conversation_id: Uuid) -> Result<bool> {
        self.load_agent(agent_key).await?;
        let conversation = self.load_conversation(agent_key, conversation_id).await?;
        let workspace = self.workspace_path(agent_key, conversation_id).await?;
        self.client
            .abort_session_in_directory(
                self.base_url,
                &conversation.opencode_session_id,
                Some(&workspace),
            )
            .await
    }

    pub async fn compact_conversation(&self, agent_key: &str, conversation_id: Uuid) -> Result<()> {
        self.load_agent(agent_key).await?;
        let conversation = self.load_conversation(agent_key, conversation_id).await?;
        let workspace = self.workspace_path(agent_key, conversation_id).await?;
        self.require_idle(&conversation, &workspace).await?;
        self.client
            .compact_session(
                self.base_url,
                &workspace,
                &conversation.opencode_session_id,
                &conversation.model_provider_id,
                &conversation.model_id,
            )
            .await
    }

    pub async fn delete_conversation(&self, agent_key: &str, conversation_id: Uuid) -> Result<()> {
        self.load_agent(agent_key).await?;
        let conversation = self.load_conversation(agent_key, conversation_id).await?;
        let workspace = self.workspace_path(agent_key, conversation_id).await?;
        self.require_idle(&conversation, &workspace).await?;
        match self
            .client
            .delete_session(self.base_url, &workspace, &conversation.opencode_session_id)
            .await?
        {
            DeleteSessionResult::Deleted | DeleteSessionResult::NotFound => {}
        }
        self.workspace_controller
            .delete_conversation_workspace(
                agent_key,
                conversation_id,
                &format!("conversation-delete-{conversation_id}"),
            )
            .await?;
        if !store::delete_conversation_mapping(self.pool, agent_key, conversation_id).await? {
            bail!("Conversation no longer exists.");
        }
        Ok(())
    }

    pub async fn update_conversation_model_and_policies(
        &self,
        agent_key: &str,
        conversation_id: Uuid,
        provider_id: &str,
        model_id: &str,
        model_variant: Option<&str>,
        policies: &[AgentConversationToolPolicyRow],
    ) -> Result<()> {
        self.load_agent(agent_key).await?;
        let conversation = self.load_conversation(agent_key, conversation_id).await?;
        let workspace = self.workspace_path(agent_key, conversation_id).await?;
        self.require_idle(&conversation, &workspace).await?;
        let rules = permission_rules(policies)?;
        self.client
            .update_session_permissions(
                self.base_url,
                &workspace,
                &conversation.opencode_session_id,
                rules,
            )
            .await?;
        if !store::update_conversation_model_and_policies(
            self.pool,
            agent_key,
            conversation_id,
            provider_id,
            model_id,
            model_variant,
            policies,
        )
        .await?
        {
            bail!("Conversation no longer exists.");
        }
        Ok(())
    }

    pub async fn reply_to_permission(
        &self,
        agent_key: &str,
        conversation_id: Uuid,
        request_id: &str,
        reply: OpenCodePermissionReply,
    ) -> Result<bool> {
        self.load_agent(agent_key).await?;
        let conversation = self.load_conversation(agent_key, conversation_id).await?;
        let workspace = self.workspace_path(agent_key, conversation_id).await?;
        self.client
            .reply_to_permission(
                self.base_url,
                &workspace,
                &conversation.opencode_session_id,
                request_id,
                reply,
            )
            .await
    }

    async fn load_agent(&self, agent_key: &str) -> Result<crate::agents::model::AgentDetailRow> {
        get_agent(self.pool, agent_key)
            .await?
            .ok_or_else(|| anyhow!("Agent not found."))
    }

    async fn load_conversation(
        &self,
        agent_key: &str,
        conversation_id: Uuid,
    ) -> Result<AgentConversationRow> {
        store::get_agent_conversation(self.pool, agent_key, conversation_id)
            .await?
            .ok_or_else(|| anyhow!("Conversation not found."))
    }

    async fn workspace_path(&self, agent_key: &str, conversation_id: Uuid) -> Result<String> {
        let workspace = crate::agent_conversations::workspace::get_conversation_workspace(
            self.pool,
            agent_key,
            conversation_id,
        )
        .await?
        .ok_or_else(|| anyhow!("Conversation workspace not found."))?;
        if workspace.workspace_status != "ready" {
            bail!("Conversation workspace is not ready.");
        }
        let inspection = self
            .workspace_controller
            .inspect_conversation_workspace(agent_key, conversation_id)
            .await?;
        if !inspection.workspace_exists {
            bail!("Conversation workspace is missing.");
        }
        Ok(format!(
            "/workspaces/conversations/{agent_key}/{conversation_id}/workspace"
        ))
    }

    async fn require_idle(
        &self,
        conversation: &AgentConversationRow,
        workspace_container_path: &str,
    ) -> Result<()> {
        let status = self
            .client
            .get_session_status_in_directory(
                self.base_url,
                &conversation.opencode_session_id,
                Some(workspace_container_path),
            )
            .await?;
        if status.as_ref().is_some_and(SessionStatusKind::is_active) {
            bail!("Stop the active conversation before changing it.");
        }
        Ok(())
    }
}

fn first_turn_title(conversation: &AgentConversationRow, text: &str) -> Option<String> {
    (conversation.title == "New conversation").then(|| {
        text.lines()
            .find_map(|line| (!line.trim().is_empty()).then_some(line.trim()))
            .unwrap_or("New conversation")
            .chars()
            .take(CONVERSATION_TITLE_MAX_CHARS)
            .collect()
    })
}

pub fn default_permission_rules() -> Vec<OpenCodePermissionRule> {
    permission_rules_for(TOOL_POLICY_CONFIRM, TOOL_POLICY_CONFIRM, TOOL_POLICY_DENY)
        .expect("confirm is a valid conversation tool policy")
}

pub fn permission_rules(
    policies: &[AgentConversationToolPolicyRow],
) -> Result<Vec<OpenCodePermissionRule>> {
    let orders = policies
        .iter()
        .find(|policy| policy.tool_group == TOOL_GROUP_ORDERS)
        .map(|policy| policy.policy.as_str())
        .ok_or_else(|| anyhow!("Missing Orders policy."))?;
    let memory_writes = policies
        .iter()
        .find(|policy| policy.tool_group == TOOL_GROUP_MEMORY_WRITES)
        .map(|policy| policy.policy.as_str())
        .ok_or_else(|| anyhow!("Missing Memory writes policy."))?;
    let notifications = policies
        .iter()
        .find(|policy| policy.tool_group == TOOL_GROUP_NOTIFICATIONS)
        .map(|policy| policy.policy.as_str())
        .ok_or_else(|| anyhow!("Missing Notifications policy."))?;
    if policies.len() != 3 {
        bail!(
            "Conversation policies must contain Orders, Memory writes, and Notifications exactly once."
        );
    }
    permission_rules_for(orders, memory_writes, notifications)
}

fn permission_rules_for(
    orders: &str,
    memory_writes: &str,
    notifications: &str,
) -> Result<Vec<OpenCodePermissionRule>> {
    let orders = action_for_policy(orders)?;
    let memory_writes = action_for_policy(memory_writes)?;
    let notifications = action_for_policy(notifications)?;
    let mut rules = [
        "hypervibes_get_account",
        "hypervibes_list_strategy_prompts",
        "hypervibes_get_strategy_prompt",
        "hypervibes_get_latest_analysis",
        "hypervibes_get_market_analysis",
        "hypervibes_get_memory_detail",
        "hypervibes_list_memories",
        "hypervibes_list_orders",
        "hypervibes_list_account_transactions",
        "hypervibes_get_order",
    ]
    .into_iter()
    .map(|permission| OpenCodePermissionRule {
        permission: permission.to_string(),
        pattern: "*".to_string(),
        action: "allow".to_string(),
    })
    .collect::<Vec<_>>();
    for permission in [
        "hypervibes_submit_orders",
        "hypervibes_cancel_orders",
        "hypervibes_cancel_all_orders",
    ] {
        rules.push(OpenCodePermissionRule {
            permission: permission.to_string(),
            pattern: "*".to_string(),
            action: orders.to_string(),
        });
    }
    rules.push(OpenCodePermissionRule {
        permission: "hypervibes_send_notification".to_string(),
        pattern: "*".to_string(),
        action: notifications.to_string(),
    });
    rules.push(OpenCodePermissionRule {
        permission: "hypervibes_write_memory".to_string(),
        pattern: "*".to_string(),
        action: memory_writes.to_string(),
    });
    rules.push(OpenCodePermissionRule {
        permission: "hypervibes_update_strategy_prompt".to_string(),
        pattern: "*".to_string(),
        action: "ask".to_string(),
    });
    Ok(rules)
}

fn action_for_policy(policy: &str) -> Result<&'static str> {
    match policy {
        TOOL_POLICY_DENY => Ok("deny"),
        TOOL_POLICY_CONFIRM => Ok("ask"),
        TOOL_POLICY_ALLOW => Ok("allow"),
        _ => Err(anyhow!("Invalid conversation tool policy.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn turn_tracker_reports_active_turns_until_the_guard_drops() {
        let tracker = ConversationTurnTracker::default();
        let conversation_id = Uuid::new_v4();

        assert!(!tracker.is_active(conversation_id).await);
        let guard = tracker
            .try_acquire(conversation_id)
            .await
            .expect("acquire conversation turn");
        assert!(tracker.is_active(conversation_id).await);
        drop(guard);
        tokio::task::yield_now().await;
        assert!(!tracker.is_active(conversation_id).await);
    }

    #[test]
    fn chat_allows_strategy_prompt_reads_without_notification_access() {
        let rules = default_permission_rules();
        let action_for = |permission: &str| {
            rules
                .iter()
                .find(|rule| rule.permission == permission)
                .map(|rule| rule.action.as_str())
        };

        assert_eq!(
            action_for("hypervibes_list_strategy_prompts"),
            Some("allow")
        );
        assert_eq!(action_for("hypervibes_get_strategy_prompt"), Some("allow"));
        assert_eq!(action_for("hypervibes_send_notification"), Some("deny"));
        assert_eq!(action_for("hypervibes_update_strategy_prompt"), Some("ask"));
    }

    #[test]
    fn persisted_notification_policy_is_rendered() {
        let now = chrono::Utc::now();
        let conversation_id = Uuid::new_v4();
        let policies = [
            (TOOL_GROUP_ORDERS, TOOL_POLICY_CONFIRM),
            (TOOL_GROUP_MEMORY_WRITES, TOOL_POLICY_CONFIRM),
            (TOOL_GROUP_NOTIFICATIONS, TOOL_POLICY_ALLOW),
        ]
        .into_iter()
        .map(|(tool_group, policy)| AgentConversationToolPolicyRow {
            conversation_id,
            tool_group: tool_group.to_string(),
            policy: policy.to_string(),
            created_at: now,
            updated_at: now,
        })
        .collect::<Vec<_>>();

        let rules = permission_rules(&policies).expect("validate persisted policies");
        assert_eq!(
            rules
                .iter()
                .find(|rule| rule.permission == "hypervibes_send_notification")
                .map(|rule| rule.action.as_str()),
            Some("allow")
        );
    }
}
