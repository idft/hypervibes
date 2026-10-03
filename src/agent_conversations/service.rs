use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use futures::FutureExt;
use tokio::sync::Mutex;
use tracing::{info, warn};
use uuid::Uuid;

use crate::{
    agent_conversations::{
        model::{
            AgentConversationRow, AgentConversationToolPolicyRow, CreateAgentConversation,
            TOOL_GROUP_INDICATOR_WRITES, TOOL_GROUP_JOURNAL_WRITES, TOOL_GROUP_MEMORY_WRITES,
            TOOL_GROUP_NOTIFICATIONS, TOOL_GROUP_ORDERS, TOOL_GROUP_STRATEGY_PROMPT_WRITES,
            TOOL_POLICY_ALLOW, TOOL_POLICY_CONFIRM, TOOL_POLICY_DENY,
        },
        store,
    },
    agents::store::get_agent,
    db::DbPool,
    harness::{in_flight::InFlightTracker, model::CAPABILITY_SCHEMA_VERSION},
    opencode::{
        client::{
            DeleteSessionResult, OpenCodeClient, OpenCodeConversationPrompt,
            OpenCodePermissionReply, OpenCodePermissionRule, SessionStatusKind,
        },
        workspace_control_client::WorkspaceController,
    },
};

pub const CONVERSATION_PROFILE: &str = "agent-conversations";
pub const CONVERSATION_MESSAGE_MAX_CHARS: usize = 100_000;
pub const CONVERSATION_TITLE_MAX_CHARS: usize = 100;
const ACTIVE_TURN_POLL_INTERVAL: Duration = Duration::from_secs(2);
const ACTIVE_TURN_TIMEOUT: Duration = Duration::from_secs(30 * 60);
pub const CONVERSATION_MESSAGE_QUEUE_LIMIT: usize = 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueuedConversationMessage {
    pub message_id: String,
    pub text: String,
}

/// Whether a submitted turn was sent to OpenCode immediately or queued behind
/// the turn that is still running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationTurnOutcome {
    Sent,
    Queued,
}

#[derive(Clone, Default)]
pub struct ConversationTurnTracker {
    state: Arc<Mutex<ConversationTurnState>>,
}

#[derive(Default)]
struct ConversationTurnState {
    active: HashMap<Uuid, Arc<Mutex<()>>>,
    queue: HashMap<Uuid, VecDeque<QueuedConversationMessage>>,
}

impl ConversationTurnTracker {
    async fn admit(
        &self,
        conversation_id: Uuid,
        message: QueuedConversationMessage,
    ) -> Result<Option<ConversationTurnGuard>> {
        let mut state = self.state.lock().await;
        let entries = state.queue.entry(conversation_id).or_default();
        if entries.len() >= CONVERSATION_MESSAGE_QUEUE_LIMIT {
            bail!(
                "Too many messages are already queued for this conversation. Wait for the current turn to finish."
            );
        }
        let had_pending = !entries.is_empty();
        entries.push_back(message);
        if state.active.contains_key(&conversation_id) {
            return Ok(None);
        }
        let dispatch = Arc::new(Mutex::new(()));
        state.active.insert(conversation_id, Arc::clone(&dispatch));
        Ok(Some(ConversationTurnGuard {
            state: Arc::clone(&self.state),
            conversation_id,
            released: false,
            dispatch,
            had_pending,
        }))
    }

    pub async fn is_active(&self, conversation_id: Uuid) -> bool {
        self.state
            .lock()
            .await
            .active
            .contains_key(&conversation_id)
    }

    async fn dispatch_lock(&self, conversation_id: Uuid) -> Option<Arc<Mutex<()>>> {
        self.state
            .lock()
            .await
            .active
            .get(&conversation_id)
            .cloned()
    }

    /// Return the next queued message to submit, removing it from the queue.
    pub async fn dequeue(&self, conversation_id: Uuid) -> Option<QueuedConversationMessage> {
        self.state
            .lock()
            .await
            .queue
            .get_mut(&conversation_id)
            .and_then(VecDeque::pop_front)
    }

    /// Return a failed submission to the front of the queue so a later turn
    /// watcher retries it before newer messages.
    pub async fn requeue_front(&self, conversation_id: Uuid, message: QueuedConversationMessage) {
        self.state
            .lock()
            .await
            .queue
            .entry(conversation_id)
            .or_default()
            .push_front(message);
    }

    pub async fn queued_messages(&self, conversation_id: Uuid) -> Vec<QueuedConversationMessage> {
        self.state
            .lock()
            .await
            .queue
            .get(&conversation_id)
            .map(|entries| entries.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub async fn clear_queued(&self, conversation_id: Uuid) {
        self.state.lock().await.queue.remove(&conversation_id);
    }

    async fn discard_queued(&self, conversation_id: Uuid, message_id: &str) {
        if let Some(entries) = self.state.lock().await.queue.get_mut(&conversation_id) {
            entries.retain(|message| message.message_id != message_id);
        }
    }
}

pub struct ConversationTurnGuard {
    state: Arc<Mutex<ConversationTurnState>>,
    conversation_id: Uuid,
    released: bool,
    dispatch: Arc<Mutex<()>>,
    had_pending: bool,
}

impl ConversationTurnGuard {
    async fn next_queued_or_release(&mut self) -> Option<QueuedConversationMessage> {
        let mut state = self.state.lock().await;
        let next = state
            .queue
            .get_mut(&self.conversation_id)
            .and_then(VecDeque::pop_front);
        if next.is_none() {
            state.queue.remove(&self.conversation_id);
            state.active.remove(&self.conversation_id);
            self.released = true;
        }
        next
    }
}

impl Drop for ConversationTurnGuard {
    fn drop(&mut self) {
        if self.released {
            return;
        }
        let state = Arc::clone(&self.state);
        let conversation_id = self.conversation_id;
        tokio::spawn(async move {
            state.lock().await.active.remove(&conversation_id);
        });
    }
}

pub struct ConversationService<'a> {
    pub pool: &'a DbPool,
    pub client: &'a OpenCodeClient,
    pub base_url: &'a str,
    pub agent_api_base_url: &'a str,
    pub workspace_controller: Arc<dyn WorkspaceController>,
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
        // Admission and spawning contain no await: cancellation cannot leave a
        // reservation behind before the tracked task owns the entire workflow.
        let in_flight = self.in_flight.track();
        if *self.shutdown_rx.borrow() {
            bail!("Conversation creation is unavailable while the server is shutting down.");
        }
        let pool = self.pool.clone();
        let client = self.client.clone();
        let base_url = self.base_url.to_string();
        let agent_api_base_url = self.agent_api_base_url.to_string();
        let workspace_controller = Arc::clone(&self.workspace_controller);
        let tracker = self.in_flight.clone();
        let turn_tracker = self.turn_tracker.clone();
        let shutdown_rx = self.shutdown_rx.clone();
        let input = CreateAgentConversation {
            agent_key: agent_key.to_string(),
            opencode_session_id: format!("pending_{}", Uuid::new_v4()),
            channel: channel.to_string(),
            external_conversation_key: (!external_conversation_key.trim().is_empty())
                .then(|| external_conversation_key.trim().to_string()),
            title: "New conversation".to_string(),
            model_provider_id: provider_id.to_string(),
            model_id: model_id.to_string(),
            model_variant: model_variant.map(ToOwned::to_owned),
        };
        let task = tokio::spawn(async move {
            let _in_flight = in_flight;
            let started = Instant::now();
            let service = ConversationService {
                pool: &pool,
                client: &client,
                base_url: &base_url,
                agent_api_base_url: &agent_api_base_url,
                workspace_controller,
                in_flight: &tracker,
                turn_tracker: &turn_tracker,
                shutdown_rx,
            };
            let mut stage = "validation";
            let mut conversation_id = None;
            let result = std::panic::AssertUnwindSafe(service.initialize_conversation(
                &input,
                &mut stage,
                &mut conversation_id,
            ))
            .catch_unwind()
            .await;
            match result {
                Ok(result) => {
                    match &result {
                        Ok(row) => info!(agent_key = %input.agent_key, conversation_id = %row.id,
                            stage, elapsed_ms = started.elapsed().as_millis(), "conversation creation completed"),
                        Err(error) => warn!(agent_key = %input.agent_key, ?conversation_id,
                            stage, elapsed_ms = started.elapsed().as_millis(), error = ?error,
                            "conversation creation failed"),
                    }
                    result
                }
                Err(panic) => {
                    warn!(agent_key = %input.agent_key, ?conversation_id, stage,
                        elapsed_ms = started.elapsed().as_millis(), "conversation creation task panicked");
                    std::panic::resume_unwind(panic)
                }
            }
        });
        // A dropped caller drops only this handle; Tokio detaches the task.
        task.await
            .context("conversation creation task failed to join")?
    }

    async fn initialize_conversation(
        &self,
        input: &CreateAgentConversation,
        stage: &mut &'static str,
        conversation_id: &mut Option<Uuid>,
    ) -> Result<AgentConversationRow> {
        let agent_key = input.agent_key.as_str();
        let agent = self.load_agent(agent_key).await?;
        input
            .validate()
            .map_err(|errors| anyhow!(errors.join(" ")))?;
        *stage = "reservation";
        let conversation =
            store::create_conversation_with_default_policies(self.pool, input).await?;
        *conversation_id = Some(conversation.id);
        let workspace = async {
            *stage = "workspace preparation";
            crate::agent_conversations::workspace::prepare_conversation_workspace(
                self.pool,
                agent_key,
                conversation.id,
                CAPABILITY_SCHEMA_VERSION,
            )
            .await?;
            *stage = "workspace materialization";
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
            *stage = "workspace ready";
            if !crate::agent_conversations::workspace::mark_conversation_workspace_ready(
                self.pool,
                agent_key,
                conversation.id,
            )
            .await?
            {
                bail!("Conversation workspace no longer exists.");
            }
            Result::<_, anyhow::Error>::Ok(workspace)
        }
        .await;
        if workspace.is_ok() {
            *stage = "session creation";
        }
        match workspace {
            Ok(workspace) => match self
                .client
                .create_conversation_session(
                    self.base_url,
                    &workspace.workspace_container_path,
                    &input.model_provider_id,
                    &input.model_id,
                    input.model_variant.as_deref(),
                    permission_rules(&conversation.tool_policies)?,
                )
                .await
            {
                Ok(session) => {
                    *stage = "session persistence";
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
    ) -> Result<ConversationTurnOutcome> {
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
        if *self.shutdown_rx.borrow() {
            bail!("Server is shutting down. Please try again after it restarts.");
        }
        self.load_agent(agent_key).await?;
        let conversation = self.load_conversation(agent_key, conversation_id).await?;
        let workspace = self.workspace_path(agent_key, conversation_id).await?;
        let Some(turn_guard) = self
            .turn_tracker
            .admit(
                conversation_id,
                QueuedConversationMessage {
                    message_id: message_id.to_string(),
                    text: text.to_string(),
                },
            )
            .await?
        else {
            return Ok(ConversationTurnOutcome::Queued);
        };
        let dispatch_guard = Arc::clone(&turn_guard.dispatch).lock_owned().await;
        let status = self
            .client
            .get_session_status_in_directory(
                self.base_url,
                &conversation.opencode_session_id,
                Some(&workspace),
            )
            .await;
        let status = match status {
            Ok(status) => status,
            Err(error) => {
                self.turn_tracker
                    .discard_queued(conversation_id, message_id)
                    .await;
                drop(dispatch_guard);
                self.spawn_turn_watcher(turn_guard, conversation, workspace, None);
                return Err(error);
            }
        };
        if status.as_ref().is_some_and(SessionStatusKind::is_active) || turn_guard.had_pending {
            // A turn is running outside our tracker (for example after a
            // server restart). Keep the acquired guard and queue the message;
            // the watcher below submits it once the session returns to idle.
            drop(dispatch_guard);
            self.spawn_turn_watcher(turn_guard, conversation, workspace, None);
            return Ok(ConversationTurnOutcome::Queued);
        }
        let Some(first) = self.turn_tracker.dequeue(conversation_id).await else {
            bail!("Message was cancelled when the conversation was stopped.");
        };
        let sent = send_conversation_turn_prompt(
            self.client,
            self.base_url,
            &workspace,
            &conversation,
            &first.message_id,
            &first.text,
        )
        .await;
        drop(dispatch_guard);
        let initial_title = sent
            .as_ref()
            .ok()
            .and_then(|()| first_turn_title(&conversation, &first.text));
        self.spawn_turn_watcher(turn_guard, conversation, workspace, initial_title);
        sent?;
        Ok(ConversationTurnOutcome::Sent)
    }

    /// Watch the submitted turn until it completes, then submit queued
    /// messages one at a time. The guard is held for the whole queue so a
    /// queued message never races a direct submission.
    fn spawn_turn_watcher(
        &self,
        turn_guard: ConversationTurnGuard,
        conversation: AgentConversationRow,
        workspace: String,
        initial_title: Option<String>,
    ) {
        let client = self.client.clone();
        let base_url = self.base_url.to_string();
        let conversation_agent_key = conversation.agent_key.clone();
        let conversation_id = conversation.id;
        let session_id = conversation.opencode_session_id.clone();
        let pool = self.pool.clone();
        let turn_tracker = self.turn_tracker.clone();
        let mut shutdown_rx = self.shutdown_rx.clone();
        let in_flight = self.in_flight.track();
        tokio::spawn(async move {
            let mut turn_guard = turn_guard;
            let _in_flight = in_flight;
            if let Some(title) = initial_title {
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
            let mut session_id = session_id;
            loop {
                if *shutdown_rx.borrow() {
                    return;
                }
                let completed = tokio::time::timeout(ACTIVE_TURN_TIMEOUT, async {
                    loop {
                        tokio::select! {
                            _ = tokio::time::sleep(ACTIVE_TURN_POLL_INTERVAL) => {},
                            changed = shutdown_rx.changed() => {
                                if changed.is_err() || *shutdown_rx.borrow() { return false; }
                            }
                        }
                        match client.get_session_status_in_directory(&base_url, &session_id, Some(&workspace)).await {
                            Ok(Some(status)) if status.is_active() => continue,
                            Ok(_) => return true,
                            Err(error) => {
                                warn!(session_id, error = ?error, "failed to observe active conversation turn");
                                continue;
                            }
                        }
                    }
                })
                .await;
                match completed {
                    Ok(true) => {}
                    Ok(false) => return,
                    Err(_) => {
                        warn!(
                            session_id,
                            timeout_seconds = ACTIVE_TURN_TIMEOUT.as_secs(),
                            "conversation turn observation timed out"
                        );
                        return;
                    }
                }
                // Serialize queued dispatch with Stop. Stop either cancels a
                // turn already dispatched, or clears its queue before another
                // prompt can be sent.
                let _dispatch_guard = Arc::clone(&turn_guard.dispatch).lock_owned().await;
                if *shutdown_rx.borrow() {
                    return;
                }
                let Some(queued) = turn_guard.next_queued_or_release().await else {
                    return;
                };
                match store::get_agent_conversation(&pool, &conversation_agent_key, conversation_id)
                    .await
                {
                    Ok(Some(next)) if !next.is_initializing() => {
                        if let Err(error) = send_conversation_turn_prompt(
                            &client,
                            &base_url,
                            &workspace,
                            &next,
                            &queued.message_id,
                            &queued.text,
                        )
                        .await
                        {
                            warn!(
                                conversation_id = %conversation_id,
                                error = ?error,
                                text = %queued.text,
                                "failed to submit queued conversation message; it will retry on the next turn"
                            );
                            turn_tracker.requeue_front(conversation_id, queued).await;
                            return;
                        }
                        session_id = next.opencode_session_id;
                    }
                    Ok(_) => {
                        // The conversation was deleted or reset; drop the
                        // remaining queued messages with it.
                        turn_tracker.clear_queued(conversation_id).await;
                        return;
                    }
                    Err(error) => {
                        warn!(conversation_id = %conversation_id, error = ?error, "failed to reload conversation for queued message");
                        turn_tracker.requeue_front(conversation_id, queued).await;
                        return;
                    }
                }
            }
        });
    }

    pub async fn stop_conversation(&self, agent_key: &str, conversation_id: Uuid) -> Result<bool> {
        self.load_agent(agent_key).await?;
        let conversation = self.load_conversation(agent_key, conversation_id).await?;
        let workspace = self.workspace_path(agent_key, conversation_id).await?;
        let dispatch = self.turn_tracker.dispatch_lock(conversation_id).await;
        let _dispatch_guard = match dispatch {
            Some(lock) => Some(lock.lock_owned().await),
            None => None,
        };
        // Clear before aborting, so observing the abort's idle state cannot
        // cause the watcher to start another queued turn.
        self.turn_tracker.clear_queued(conversation_id).await;
        let aborted = self
            .client
            .abort_session_in_directory(
                self.base_url,
                &conversation.opencode_session_id,
                Some(&workspace),
            )
            .await?;
        Ok(aborted)
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
        self.turn_tracker.clear_queued(conversation_id).await;
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
            .and_then(|conversation| {
                conversation.require_initialized()?;
                Ok(conversation)
            })
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
        if self.turn_tracker.is_active(conversation.id).await {
            bail!("Stop the active conversation before changing it.");
        }
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

async fn send_conversation_turn_prompt(
    client: &OpenCodeClient,
    base_url: &str,
    workspace: &str,
    conversation: &AgentConversationRow,
    message_id: &str,
    text: &str,
) -> Result<()> {
    client
        .send_conversation_prompt_async(
            base_url,
            workspace,
            &conversation.opencode_session_id,
            &OpenCodeConversationPrompt {
                message_id: message_id.to_string(),
                provider_id: conversation.model_provider_id.clone(),
                model_id: conversation.model_id.clone(),
                variant: conversation.model_variant.clone(),
                text: text.to_string(),
            },
        )
        .await
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
    let journal_writes = policies
        .iter()
        .find(|policy| policy.tool_group == TOOL_GROUP_JOURNAL_WRITES)
        .map(|policy| policy.policy.as_str())
        .ok_or_else(|| anyhow!("Missing Journal writes policy."))?;
    let indicator_writes = policies
        .iter()
        .find(|policy| policy.tool_group == TOOL_GROUP_INDICATOR_WRITES)
        .map(|policy| policy.policy.as_str())
        .ok_or_else(|| anyhow!("Missing Indicators policy."))?;
    let strategy_prompt_writes = policies
        .iter()
        .find(|policy| policy.tool_group == TOOL_GROUP_STRATEGY_PROMPT_WRITES)
        .map(|policy| policy.policy.as_str())
        .ok_or_else(|| anyhow!("Missing Strategy prompts policy."))?;
    if policies.len() != 6 {
        bail!("Conversation policies must contain each tool group exactly once.");
    }
    permission_rules_for(
        orders,
        memory_writes,
        notifications,
        journal_writes,
        indicator_writes,
        strategy_prompt_writes,
    )
}

fn permission_rules_for(
    orders: &str,
    memory_writes: &str,
    notifications: &str,
    journal_writes: &str,
    indicator_writes: &str,
    strategy_prompt_writes: &str,
) -> Result<Vec<OpenCodePermissionRule>> {
    let orders = action_for_policy(orders)?;
    let memory_writes = action_for_policy(memory_writes)?;
    let notifications = action_for_policy(notifications)?;
    let journal_writes = action_for_policy(journal_writes)?;
    let indicator_writes = action_for_policy(indicator_writes)?;
    let strategy_prompt_writes = action_for_policy(strategy_prompt_writes)?;
    let mut rules = [
        "hypervibes_get_account",
        "hypervibes_list_strategy_prompts",
        "hypervibes_get_strategy_prompt",
        "hypervibes_list_analysis_instruments",
        "hypervibes_list_indicators",
        "hypervibes_get_indicator",
        "hypervibes_get_indicator_results",
        "hypervibes_get_memory_detail",
        "hypervibes_list_memories",
        "hypervibes_list_orders",
        "hypervibes_list_account_transactions",
        "hypervibes_list_account_trades",
        "hypervibes_get_account_trade",
        "hypervibes_list_journal_notes",
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
        permission: "hypervibes_add_journal_note".to_string(),
        pattern: "*".to_string(),
        action: journal_writes.to_string(),
    });
    rules.push(OpenCodePermissionRule {
        permission: "hypervibes_update_strategy_prompt".to_string(),
        pattern: "*".to_string(),
        action: strategy_prompt_writes.to_string(),
    });
    for permission in ["hypervibes_create_indicator", "hypervibes_update_indicator"] {
        rules.push(OpenCodePermissionRule {
            permission: permission.to_string(),
            pattern: "*".to_string(),
            action: indicator_writes.to_string(),
        });
    }
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
            .admit(
                conversation_id,
                QueuedConversationMessage {
                    message_id: "msg_test".into(),
                    text: "test".into(),
                },
            )
            .await
            .expect("admit conversation turn")
            .expect("acquire conversation turn");
        assert!(tracker.is_active(conversation_id).await);
        drop(guard);
        tokio::task::yield_now().await;
        assert!(!tracker.is_active(conversation_id).await);
    }

    #[tokio::test]
    async fn turn_tracker_keeps_queued_messages_in_submission_order() {
        let tracker = ConversationTurnTracker::default();
        let conversation_id = Uuid::new_v4();
        let queued = |i: usize| QueuedConversationMessage {
            message_id: format!("msg_{i}"),
            text: format!("text {i}"),
        };

        let _guard = tracker
            .admit(conversation_id, queued(0))
            .await
            .expect("admit first")
            .expect("own watcher");
        for i in 1..CONVERSATION_MESSAGE_QUEUE_LIMIT {
            assert!(
                tracker
                    .admit(conversation_id, queued(i))
                    .await
                    .expect("admit queued")
                    .is_none()
            );
        }
        assert!(
            tracker
                .admit(
                    conversation_id,
                    QueuedConversationMessage {
                        message_id: "msg_overflow".to_string(),
                        text: "overflow".to_string(),
                    }
                )
                .await
                .is_err(),
            "queue refuses messages beyond the limit"
        );
        assert_eq!(
            tracker
                .queued_messages(conversation_id)
                .await
                .iter()
                .map(|message| message.text.as_str())
                .collect::<Vec<_>>(),
            (0..CONVERSATION_MESSAGE_QUEUE_LIMIT)
                .map(|i| format!("text {i}"))
                .collect::<Vec<_>>()
        );

        assert_eq!(
            tracker
                .dequeue(conversation_id)
                .await
                .map(|message| message.text),
            Some("text 0".to_string())
        );
        tracker
            .requeue_front(
                conversation_id,
                QueuedConversationMessage {
                    message_id: "msg_retry".to_string(),
                    text: "retry first".to_string(),
                },
            )
            .await;
        assert_eq!(
            tracker
                .dequeue(conversation_id)
                .await
                .map(|message| message.text),
            Some("retry first".to_string()),
            "requeued messages drain before newer ones"
        );

        tracker.clear_queued(conversation_id).await;
        assert!(tracker.queued_messages(conversation_id).await.is_empty());
        assert!(tracker.dequeue(conversation_id).await.is_none());
    }

    #[tokio::test]
    async fn queue_completion_and_new_admission_are_atomic() {
        let tracker = ConversationTurnTracker::default();
        let id = Uuid::new_v4();
        let message = |text: &str| QueuedConversationMessage {
            message_id: format!("msg_{text}"),
            text: text.to_string(),
        };
        let mut first = tracker
            .admit(id, message("first"))
            .await
            .expect("admit first")
            .expect("first owns watcher");
        assert_eq!(
            tracker.dequeue(id).await.map(|queued| queued.text),
            Some("first".into())
        );
        assert!(
            tracker
                .admit(id, message("second"))
                .await
                .expect("admit second")
                .is_none()
        );
        assert_eq!(
            first
                .next_queued_or_release()
                .await
                .map(|queued| queued.text),
            Some("second".into())
        );
        assert!(first.next_queued_or_release().await.is_none());
        let mut next = tracker
            .admit(id, message("third"))
            .await
            .expect("admit third")
            .expect("new message owns a watcher after queue completion");
        drop(first);
        tokio::task::yield_now().await;
        assert!(
            tracker.is_active(id).await,
            "old guard cannot release a newer watcher"
        );
        assert_eq!(
            next.next_queued_or_release()
                .await
                .map(|queued| queued.text),
            Some("third".into())
        );
        assert!(next.next_queued_or_release().await.is_none());
        assert!(!tracker.is_active(id).await);
    }

    #[test]
    fn chat_allows_strategy_prompt_reads_without_notification_access() {
        let rules = permission_rules_for(
            TOOL_POLICY_CONFIRM,
            TOOL_POLICY_CONFIRM,
            TOOL_POLICY_DENY,
            TOOL_POLICY_CONFIRM,
            TOOL_POLICY_CONFIRM,
            TOOL_POLICY_CONFIRM,
        )
        .expect("standard defaults are valid");
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
        assert_eq!(
            action_for("hypervibes_list_analysis_instruments"),
            Some("allow")
        );
        assert_eq!(action_for("hypervibes_send_notification"), Some("deny"));
        assert_eq!(action_for("hypervibes_add_journal_note"), Some("ask"));
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
            (TOOL_GROUP_JOURNAL_WRITES, TOOL_POLICY_ALLOW),
            (TOOL_GROUP_INDICATOR_WRITES, TOOL_POLICY_CONFIRM),
            (TOOL_GROUP_STRATEGY_PROMPT_WRITES, TOOL_POLICY_CONFIRM),
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
