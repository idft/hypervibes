use std::{collections::HashSet, sync::Arc, time::Duration};

use anyhow::{Context, Result, anyhow, bail};
use tokio::sync::Mutex;
use tracing::warn;
use uuid::Uuid;

use crate::{
    agent_conversations::{
        model::{
            AgentConversationRow, AgentConversationToolPolicyRow, CreateAgentConversation,
            TOOL_GROUP_MEMORY_WRITES, TOOL_GROUP_ORDERS, TOOL_POLICY_ALLOW, TOOL_POLICY_CONFIRM,
            TOOL_POLICY_DENY,
        },
        store,
    },
    agents::{model::AgentDetailRow, store::get_agent},
    db::DbPool,
    harness::{in_flight::InFlightTracker, workspace_lease::WorkspaceLeaseManager},
    opencode::{
        client::{
            DeleteSessionResult, OpenCodeClient, OpenCodePermissionReply, OpenCodePermissionRule,
            SessionStatusKind,
        },
        workspace::OpenCodeWorkspaceRuntimeConfig,
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
    pub workspace_leases: &'a WorkspaceLeaseManager,
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
        let agent = self.load_agent_with_workspace(agent_key).await?;
        let runtime = workspace_runtime(&agent)?;
        let _lease = self.workspace_leases.acquire_live_read(agent_key).await;
        let session = self
            .client
            .create_conversation_session(
                self.base_url,
                &runtime.workspace_container_path,
                provider_id,
                model_id,
                model_variant,
                default_permission_rules(),
            )
            .await?;
        let input = CreateAgentConversation {
            agent_key: agent_key.to_string(),
            opencode_session_id: session.id.clone(),
            channel: crate::agent_conversations::model::CONVERSATION_CHANNEL_WEB.to_string(),
            external_conversation_key: None,
            title: "New conversation".to_string(),
            model_provider_id: provider_id.to_string(),
            model_id: model_id.to_string(),
            model_variant: model_variant.map(ToOwned::to_owned),
        };
        match store::create_conversation_with_default_policies(self.pool, &input).await {
            Ok(conversation) => Ok(conversation),
            Err(error) => {
                if let Err(cleanup_error) = self
                    .client
                    .delete_session(
                        self.base_url,
                        &runtime.workspace_container_path,
                        &session.id,
                    )
                    .await
                {
                    warn!(agent_key, session_id = %session.id, error = ?cleanup_error, "failed to clean up unmapped OpenCode conversation session");
                }
                Err(error).context("failed to persist OpenCode conversation mapping")
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
        let agent = self.load_agent_with_workspace(agent_key).await?;
        let runtime = workspace_runtime(&agent)?;
        if crate::harness::store::agent_has_blocking_workspace_maintenance(self.pool, agent_key)
            .await?
        {
            bail!("Workspace maintenance is in progress. Send a message after it finishes.");
        }
        let conversation = self.load_conversation(agent_key, conversation_id).await?;
        let Some(turn_guard) = self.turn_tracker.try_acquire(conversation_id).await else {
            bail!("This conversation already has a turn in progress.");
        };
        let lease = self.workspace_leases.acquire_live_read(agent_key).await;
        let status = self
            .client
            .get_session_status_in_directory(
                self.base_url,
                &conversation.opencode_session_id,
                Some(&runtime.workspace_container_path),
            )
            .await?;
        if status.as_ref().is_some_and(SessionStatusKind::is_active) {
            bail!("The current turn is still running. Wait for it to finish or stop it first.");
        }
        self.client
            .send_conversation_prompt_async(
                self.base_url,
                &runtime.workspace_container_path,
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
        let workspace = runtime.workspace_container_path;
        let conversation_title = first_turn_title(&conversation, text);
        let conversation_agent_key = conversation.agent_key;
        let conversation_id = conversation.id;
        let session_id = conversation.opencode_session_id;
        let pool = self.pool.clone();
        let mut shutdown_rx = self.shutdown_rx.clone();
        let in_flight = self.in_flight.track();
        tokio::spawn(async move {
            let _lease = lease;
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
        let agent = self.load_agent_with_workspace(agent_key).await?;
        let _runtime = workspace_runtime(&agent)?;
        let conversation = self.load_conversation(agent_key, conversation_id).await?;
        let _lease = self.workspace_leases.acquire_live_read(agent_key).await;
        self.client
            .abort_session_in_directory(
                self.base_url,
                &conversation.opencode_session_id,
                Some(&_runtime.workspace_container_path),
            )
            .await
    }

    pub async fn compact_conversation(&self, agent_key: &str, conversation_id: Uuid) -> Result<()> {
        let agent = self.load_agent_with_workspace(agent_key).await?;
        let runtime = workspace_runtime(&agent)?;
        let conversation = self.load_conversation(agent_key, conversation_id).await?;
        let _lease = self.workspace_leases.acquire_live_read(agent_key).await;
        self.require_idle(&conversation, &runtime.workspace_container_path)
            .await?;
        self.client
            .compact_session(
                self.base_url,
                &runtime.workspace_container_path,
                &conversation.opencode_session_id,
                &conversation.model_provider_id,
                &conversation.model_id,
            )
            .await
    }

    pub async fn delete_conversation(&self, agent_key: &str, conversation_id: Uuid) -> Result<()> {
        let agent = self.load_agent_with_workspace(agent_key).await?;
        let runtime = workspace_runtime(&agent)?;
        let conversation = self.load_conversation(agent_key, conversation_id).await?;
        let _lease = self.workspace_leases.acquire_live_read(agent_key).await;
        self.require_idle(&conversation, &runtime.workspace_container_path)
            .await?;
        match self
            .client
            .delete_session(
                self.base_url,
                &runtime.workspace_container_path,
                &conversation.opencode_session_id,
            )
            .await?
        {
            DeleteSessionResult::Deleted | DeleteSessionResult::NotFound => {}
        }
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
        let agent = self.load_agent_with_workspace(agent_key).await?;
        let runtime = workspace_runtime(&agent)?;
        let conversation = self.load_conversation(agent_key, conversation_id).await?;
        let _lease = self.workspace_leases.acquire_live_read(agent_key).await;
        self.require_idle(&conversation, &runtime.workspace_container_path)
            .await?;
        let rules = permission_rules(policies)?;
        self.client
            .update_session_permissions(
                self.base_url,
                &runtime.workspace_container_path,
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
        let agent = self.load_agent_with_workspace(agent_key).await?;
        let runtime = workspace_runtime(&agent)?;
        let conversation = self.load_conversation(agent_key, conversation_id).await?;
        self.client
            .reply_to_permission(
                self.base_url,
                &runtime.workspace_container_path,
                &conversation.opencode_session_id,
                request_id,
                reply,
            )
            .await
    }

    async fn load_agent_with_workspace(&self, agent_key: &str) -> Result<AgentDetailRow> {
        let agent = get_agent(self.pool, agent_key)
            .await?
            .ok_or_else(|| anyhow!("Agent not found."))?;
        workspace_runtime(&agent)?;
        Ok(agent)
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
    permission_rules_for(TOOL_POLICY_CONFIRM, TOOL_POLICY_CONFIRM)
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
    if policies.len() != 2 {
        bail!("Conversation policies must contain Orders and Memory writes exactly once.");
    }
    permission_rules_for(orders, memory_writes)
}

fn permission_rules_for(orders: &str, memory_writes: &str) -> Result<Vec<OpenCodePermissionRule>> {
    let action = |policy: &str| match policy {
        TOOL_POLICY_DENY => Ok("deny"),
        TOOL_POLICY_CONFIRM => Ok("ask"),
        TOOL_POLICY_ALLOW => Ok("allow"),
        _ => Err(anyhow!("Invalid conversation tool policy.")),
    };
    let orders = action(orders)?;
    let memory_writes = action(memory_writes)?;
    let mut rules = [
        "vibetrading_get_account",
        "vibetrading_get_latest_analysis",
        "vibetrading_get_market_analysis",
        "vibetrading_get_memory_detail",
        "vibetrading_list_memories",
        "vibetrading_list_orders",
        "vibetrading_list_account_transactions",
        "vibetrading_get_order",
    ]
    .into_iter()
    .map(|permission| OpenCodePermissionRule {
        permission: permission.to_string(),
        pattern: "*".to_string(),
        action: "allow".to_string(),
    })
    .collect::<Vec<_>>();
    for permission in [
        "vibetrading_submit_orders",
        "vibetrading_cancel_orders",
        "vibetrading_cancel_all_orders",
    ] {
        rules.push(OpenCodePermissionRule {
            permission: permission.to_string(),
            pattern: "*".to_string(),
            action: orders.to_string(),
        });
    }
    rules.push(OpenCodePermissionRule {
        permission: "vibetrading_write_memory".to_string(),
        pattern: "*".to_string(),
        action: memory_writes.to_string(),
    });
    Ok(rules)
}

fn workspace_runtime(agent: &AgentDetailRow) -> Result<OpenCodeWorkspaceRuntimeConfig> {
    OpenCodeWorkspaceRuntimeConfig::from_value(&agent.runtime_config)
        .ok_or_else(|| anyhow!("Agent is missing OpenCode workspace metadata."))
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
}
