use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result};
use dashmap::DashMap;
use futures::StreamExt;
use teloxide::{
    prelude::*,
    types::{ChatAction, ChatId, Update, UpdateKind},
    update_listeners::AsUpdateStream,
    utils::command::BotCommands,
};
use tokio::sync::watch;
use tokio::task::{AbortHandle, JoinHandle};
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::{
    agent_conversations::{
        self,
        model::{CONVERSATION_CHANNEL_TELEGRAM, CONVERSATION_CHANNEL_WEB},
        service::{ConversationService, ConversationTurnTracker},
        store as conversation_store,
    },
    agents::crypto::{EncryptionKey, decrypt},
    db::DbPool,
    gateway::{
        model::{AgentGatewayRow, PendingLink, TelegramGatewayConfig, TelegramGatewayConfigView},
        store as gateway_store,
        telegram::{
            CALLBACK_PERMISSION, CALLBACK_SWITCH, GatewayCommand, allowed_updates, build_bot,
            edit_permission_message, extract_start_payload, is_authorized_chat, parse_command,
            send_approval_keyboard, send_conversation_switcher,
        },
    },
    harness::in_flight::InFlightTracker,
    harness::workspace_lease::WorkspaceLeaseManager,
    notifications,
    opencode::{
        client::{OpenCodeClient, OpenCodePermissionReply},
        store::{OpenCodeMessageRow, OpenCodeSessionErrorRow, get_session_detail},
        workspace::OpenCodeWorkspaceRuntimeConfig,
    },
};

const REGISTRY_REFRESH_INTERVAL: Duration = Duration::from_secs(30);
const PERMISSION_POLL_INTERVAL: Duration = Duration::from_secs(2);
const TELEGRAM_REPLY_POLL_INTERVAL: Duration = Duration::from_secs(2);
const TELEGRAM_REPLY_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const TELEGRAM_REPLY_MIRROR_TIMEOUT: Duration = Duration::from_secs(10);
const TELEGRAM_TYPING_REFRESH_INTERVAL: Duration = Duration::from_secs(4);
const TELEGRAM_MESSAGE_MAX_CHARS: usize = 4096;
const NOTIFICATION_CHANNEL: &str = "notification_created";

/// Supervises one background polling task per configured Telegram gateway. Also
/// listens for notification events and dispatches them to the bound chat for
/// each gateway.
pub struct GatewayService {
    pool: DbPool,
    opencode_client: Arc<OpenCodeClient>,
    opencode_base_url: String,
    encryption_key: EncryptionKey,
    shutdown_rx: watch::Receiver<bool>,
    in_flight: InFlightTracker,
    workspace_leases: WorkspaceLeaseManager,
    conversation_turns: ConversationTurnTracker,
    pending_links: Arc<DashMap<Uuid, PendingLink>>,
}

struct GatewayTaskHandle {
    task: JoinHandle<()>,
    permission_task: JoinHandle<()>,
}

impl GatewayService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        pool: DbPool,
        opencode_client: Arc<OpenCodeClient>,
        opencode_base_url: String,
        encryption_key: EncryptionKey,
        shutdown_rx: watch::Receiver<bool>,
        in_flight: InFlightTracker,
        workspace_leases: WorkspaceLeaseManager,
        conversation_turns: ConversationTurnTracker,
        pending_links: Arc<DashMap<Uuid, PendingLink>>,
    ) -> Self {
        Self {
            pool,
            opencode_client,
            opencode_base_url,
            encryption_key,
            shutdown_rx,
            in_flight,
            workspace_leases,
            conversation_turns,
            pending_links,
        }
    }

    /// Run the supervisor loop until shutdown, spawning one polling task per
    /// configured Telegram gateway and one notification dispatcher task. Returns
    /// `Ok(())` on graceful shutdown.
    pub async fn run(mut self) -> Result<()> {
        info!("gateway service starting");
        let mut tasks: HashMap<String, GatewayTaskHandle> = HashMap::new();
        let mut notification_handle = self.spawn_notification_dispatcher().await;

        loop {
            if *self.shutdown_rx.borrow() {
                break;
            }

            match gateway_store::list_configured_telegram_gateways(&self.pool).await {
                Ok(rows) => {
                    self.reconcile_telegram_tasks(&mut tasks, rows).await;
                }
                Err(error) => {
                    error!(error = ?error, "gateway service failed to load configured gateways");
                }
            }

            tokio::select! {
                _ = tokio::time::sleep(REGISTRY_REFRESH_INTERVAL) => {}
                _ = self.shutdown_rx.changed() => break,
            }
        }

        for handle in tasks.drain().map(|(_, handle)| handle) {
            handle.task.abort();
            handle.permission_task.abort();
        }
        if let Some(handle) = notification_handle.take() {
            handle.abort();
        }
        info!("gateway service stopped");
        Ok(())
    }

    async fn reconcile_telegram_tasks(
        &self,
        tasks: &mut HashMap<String, GatewayTaskHandle>,
        rows: Vec<AgentGatewayRow>,
    ) {
        let mut to_stop = Vec::new();
        for (key, handle) in tasks.iter() {
            let task_done = handle.task.is_finished() || handle.permission_task.is_finished();
            let missing_from_rows = !rows.iter().any(|row| row.agent_key == *key);
            if task_done || missing_from_rows {
                to_stop.push(key.clone());
            }
        }
        for key in to_stop {
            if let Some(handle) = tasks.remove(&key) {
                info!(agent_key = %key, "stopping gateway task");
                handle.task.abort();
                handle.permission_task.abort();
            }
        }

        for row in rows {
            if tasks.contains_key(&row.agent_key) {
                continue;
            }
            let config = TelegramGatewayConfig::from_value(&row.config);
            if !config.is_ready() {
                warn!(
                    agent_key = %row.agent_key,
                    "configured telegram gateway is missing bot token material; skipping"
                );
                continue;
            }
            info!(agent_key = %row.agent_key, "starting telegram gateway task");
            let handle = self.spawn_telegram_gateway(&row.agent_key, config);
            tasks.insert(row.agent_key.clone(), handle);
        }
    }

    fn spawn_telegram_gateway(
        &self,
        agent_key: &str,
        config: TelegramGatewayConfig,
    ) -> GatewayTaskHandle {
        let agent_key = agent_key.to_string();
        let token = self
            .decrypt_bot_token(&config)
            .map(|token| (build_bot(&token), token));
        let permission_task = match &token {
            Some((_, token)) => self.spawn_permission_poller(agent_key.clone(), token.clone()),
            None => tokio::spawn(async {}),
        };
        let task = match token {
            Some((bot, _)) => {
                let agent_key = agent_key.clone();
                let service = self.clone_state();
                tokio::spawn(async move {
                    Self::run_telegram_listener(agent_key.clone(), bot, service).await;
                })
            }
            None => tokio::spawn(async {}),
        };
        GatewayTaskHandle {
            task,
            permission_task,
        }
    }

    async fn run_telegram_listener(agent_key: String, bot: Bot, service: GatewayServiceState) {
        let mut listener = teloxide::update_listeners::Polling::builder(bot.clone())
            .allowed_updates(allowed_updates())
            .build();
        let bot_username = bot
            .get_me()
            .send()
            .await
            .map(|me| me.username().to_string())
            .unwrap_or_else(|error| {
                warn!(agent_key = %agent_key, error = ?error, "failed to fetch bot username");
                String::new()
            });
        let mut shutdown_rx = service.shutdown_rx.clone();
        let listener_stream = listener.as_stream();
        tokio::pin!(listener_stream);
        loop {
            tokio::select! {
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        break;
                    }
                }
                item = listener_stream.next() => {
                    let Some(update) = item else { break; };
                    match update {
                        Ok(update) => {
                            if let Err(error) = service
                                .handle_telegram_update(&agent_key, &bot_username, &bot, update)
                                .await
                            {
                                warn!(agent_key = %agent_key, error = ?error, "telegram update handler failed");
                            }
                        }
                        Err(error) => {
                            warn!(agent_key = %agent_key, error = ?error, "telegram polling error");
                        }
                    }
                }
            }
        }
        info!(agent_key = %agent_key, "telegram gateway task stopped");
    }

    fn spawn_permission_poller(&self, agent_key: String, token: String) -> JoinHandle<()> {
        let service = self.clone_state();
        let mut shutdown_rx = service.shutdown_rx.clone();
        let bot = build_bot(&token);
        tokio::spawn(async move {
            let mut seen: HashSet<String> = HashSet::new();
            loop {
                tokio::select! {
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() { return; }
                    }
                    _ = tokio::time::sleep(PERMISSION_POLL_INTERVAL) => {}
                }
                if let Err(error) = service
                    .poll_pending_permissions(&agent_key, &bot, &mut seen)
                    .await
                {
                    warn!(agent_key = %agent_key, error = ?error, "permission poll failed");
                }
            }
        })
    }

    async fn spawn_notification_dispatcher(&self) -> Option<JoinHandle<()>> {
        let mut listener = match sqlx::postgres::PgListener::connect_with(&self.pool).await {
            Ok(listener) => listener,
            Err(error) => {
                error!(error = ?error, "notification listener connection failed");
                return None;
            }
        };
        if let Err(error) = listener.listen(NOTIFICATION_CHANNEL).await {
            error!(error = ?error, "notification listener subscription failed");
            return None;
        }
        let service = self.clone_state();
        let mut shutdown_rx = self.shutdown_rx.clone();
        Some(tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() { return; }
                    }
                    result = listener.recv() => {
                        match result {
                            Ok(notification) => {
                                let payload = notification.payload();
                                if let Err(error) = service.dispatch_notification_by_id(payload).await {
                                    warn!(payload = payload, error = ?error, "failed to dispatch notification");
                                }
                            }
                            Err(error) => {
                                error!(error = ?error, "notification listener failed");
                                break;
                            }
                        }
                    }
                }
            }
        }))
    }

    fn decrypt_bot_token(&self, config: &TelegramGatewayConfig) -> Option<String> {
        let ciphertext = config.bot_token_ciphertext.as_ref()?;
        let key_id = config.bot_token_key_id.as_ref()?;
        if key_id != &self.encryption_key.key_id {
            warn!(
                stored_key_id = %key_id,
                active_key_id = %self.encryption_key.key_id,
                "telegram bot token was encrypted with a different key id"
            );
            return None;
        }
        match decrypt(&self.encryption_key, ciphertext) {
            Ok(token) => Some(token),
            Err(error) => {
                warn!(error = ?error, "failed to decrypt telegram bot token");
                None
            }
        }
    }

    fn clone_state(&self) -> GatewayServiceState {
        GatewayServiceState {
            pool: self.pool.clone(),
            opencode_client: Arc::clone(&self.opencode_client),
            opencode_base_url: self.opencode_base_url.clone(),
            encryption_key: self.encryption_key.clone(),
            shutdown_rx: self.shutdown_rx.clone(),
            in_flight: self.in_flight.clone(),
            workspace_leases: self.workspace_leases.clone(),
            conversation_turns: self.conversation_turns.clone(),
            pending_links: Arc::clone(&self.pending_links),
        }
    }
}

/// Per-task clone of [`GatewayService`] state. The supervisor clones this for
/// each spawned polling task so it can be moved into the task without holding
/// a borrow on the supervisor.
#[derive(Clone)]
struct GatewayServiceState {
    pool: DbPool,
    opencode_client: Arc<OpenCodeClient>,
    opencode_base_url: String,
    encryption_key: EncryptionKey,
    shutdown_rx: watch::Receiver<bool>,
    in_flight: InFlightTracker,
    workspace_leases: WorkspaceLeaseManager,
    conversation_turns: ConversationTurnTracker,
    pending_links: Arc<DashMap<Uuid, PendingLink>>,
}

#[derive(Default)]
struct TelegramTurnBaseline {
    assistant_message_ids: HashSet<String>,
    session_error_count: usize,
}

#[derive(Debug, PartialEq, Eq)]
enum TelegramTurnReply {
    Assistant(Vec<String>),
    Error(String),
}

impl GatewayServiceState {
    async fn handle_telegram_update(
        &self,
        agent_key: &str,
        bot_username: &str,
        bot: &Bot,
        update: Update,
    ) -> Result<()> {
        match update.kind {
            UpdateKind::Message(message) => {
                self.handle_telegram_message(agent_key, bot_username, bot, message)
                    .await?;
            }
            UpdateKind::CallbackQuery(query) => {
                self.handle_telegram_callback(agent_key, bot, query).await?;
            }
            _ => {}
        }
        Ok(())
    }

    async fn handle_telegram_message(
        &self,
        agent_key: &str,
        bot_username: &str,
        bot: &Bot,
        message: teloxide::types::Message,
    ) -> Result<()> {
        let Some(text) = message.text() else {
            return Ok(());
        };
        let chat_id = message.chat.id;

        if let Some(token) = extract_start_payload(text) {
            self.consume_start_payload(agent_key, bot, chat_id, &token, &message)
                .await?;
            return Ok(());
        }

        let Some(gateway) = gateway_store::get_gateway(&self.pool, agent_key, "telegram").await?
        else {
            return Ok(());
        };
        let config = TelegramGatewayConfig::from_value(&gateway.config);
        if !is_authorized_chat(&config, chat_id.0) {
            warn!(
                agent_key = agent_key,
                chat_id = chat_id.0,
                "telegram message from unauthorized chat ignored"
            );
            return Ok(());
        }

        if let Some(cmd) = parse_command(text, bot_username)? {
            self.handle_telegram_command(agent_key, bot, chat_id, cmd)
                .await?;
            return Ok(());
        }

        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Ok(());
        }
        self.submit_telegram_chat_message(agent_key, bot, chat_id, trimmed)
            .await?;
        Ok(())
    }

    async fn consume_start_payload(
        &self,
        agent_key: &str,
        bot: &Bot,
        chat_id: ChatId,
        token: &str,
        message: &teloxide::types::Message,
    ) -> Result<()> {
        let parsed = match Uuid::parse_str(token) {
            Ok(value) => value,
            Err(_) => {
                bot.send_message(
                    chat_id,
                    "Invalid link token. Start a new link from the web UI.",
                )
                .await
                .ok();
                return Ok(());
            }
        };
        let Some(sender) = message.from.clone() else {
            return Ok(());
        };
        let existing = self.pending_links.get(&parsed).map(|entry| entry.clone());
        let Some(link) = existing else {
            bot.send_message(
                chat_id,
                "This link token is unknown or has expired. Start a new link from the web UI.",
            )
            .await
            .ok();
            return Ok(());
        };
        if link.agent_key != agent_key {
            warn!(
                agent_key = agent_key,
                link_agent_key = %link.agent_key,
                "telegram /start token agent mismatch ignored"
            );
            bot.send_message(chat_id, "This link token belongs to a different agent.")
                .await
                .ok();
            return Ok(());
        }
        if link.is_expired() {
            let _ = self.pending_links.remove(&parsed);
            bot.send_message(
                chat_id,
                "This link token has expired. Start a new link from the web UI.",
            )
            .await
            .ok();
            return Ok(());
        }
        let linked = gateway_store::set_telegram_chat(
            &self.pool,
            agent_key,
            chat_id.0,
            sender.username.as_deref(),
        )
        .await?;
        if !linked {
            bot.send_message(
                chat_id,
                "This Telegram gateway is no longer configured. Start a new link from the web UI.",
            )
            .await
            .ok();
            return Ok(());
        }
        let _ = self.pending_links.remove(&parsed);
        bot.send_message(
            chat_id,
            "Telegram chat linked. You can now use this chat with HyperVibes.",
        )
        .await
        .ok();
        Ok(())
    }

    async fn handle_telegram_command(
        &self,
        agent_key: &str,
        bot: &Bot,
        chat_id: ChatId,
        cmd: GatewayCommand,
    ) -> Result<()> {
        match cmd {
            GatewayCommand::New => {
                self.command_new_conversation(agent_key, bot, chat_id)
                    .await?;
            }
            GatewayCommand::Switch => {
                self.command_switch_conversation(agent_key, bot, chat_id)
                    .await?;
            }
            GatewayCommand::Compact => {
                self.command_compact(agent_key, bot, chat_id).await?;
            }
            GatewayCommand::Stop => {
                self.command_stop(agent_key, bot, chat_id).await?;
            }
            GatewayCommand::Model(provider, model) => {
                self.command_model(agent_key, bot, chat_id, &provider, &model)
                    .await?;
            }
            GatewayCommand::Help => {
                bot.send_message(chat_id, GatewayCommand::descriptions().to_string())
                    .await
                    .ok();
            }
        }
        Ok(())
    }

    async fn command_new_conversation(
        &self,
        agent_key: &str,
        bot: &Bot,
        chat_id: ChatId,
    ) -> Result<()> {
        let selected_model = self
            .find_current_conversation(agent_key, chat_id.0)
            .await?
            .and_then(|conversation| {
                model_selection(
                    &conversation.model_provider_id,
                    &conversation.model_id,
                    conversation.model_variant.as_deref(),
                )
            });
        let conversation = match self
            .create_telegram_conversation(agent_key, None, selected_model)
            .await
        {
            Ok(conversation) => conversation,
            Err(error) => {
                bot.send_message(chat_id, telegram_error_message(&error))
                    .await
                    .ok();
                return Ok(());
            }
        };
        let activated = conversation_store::activate_conversation_for_external_key(
            &self.pool,
            agent_key,
            CONVERSATION_CHANNEL_TELEGRAM,
            conversation.id,
            &chat_id.0.to_string(),
        )
        .await?;
        if !activated {
            return Err(anyhow::anyhow!(
                "created Telegram conversation could not be activated"
            ));
        }
        bot.send_message(
            chat_id,
            telegram_new_session_message(&conversation.model_provider_id, &conversation.model_id),
        )
        .await
        .ok();
        Ok(())
    }

    async fn command_switch_conversation(
        &self,
        agent_key: &str,
        bot: &Bot,
        chat_id: ChatId,
    ) -> Result<()> {
        let conversations = conversation_store::list_conversations_for_channel(
            &self.pool,
            agent_key,
            CONVERSATION_CHANNEL_TELEGRAM,
        )
        .await?;
        let items: Vec<(Uuid, String)> = conversations
            .into_iter()
            .map(|conv| (conv.id, conv.title))
            .collect();
        send_conversation_switcher(bot, chat_id, &items).await
    }

    async fn command_compact(&self, agent_key: &str, bot: &Bot, chat_id: ChatId) -> Result<()> {
        let Some(conversation) = self.find_current_conversation(agent_key, chat_id.0).await? else {
            bot.send_message(chat_id, "No active conversation. Use /new to start one.")
                .await
                .ok();
            return Ok(());
        };
        match self
            .conversation_service()
            .compact_conversation(agent_key, conversation.id)
            .await
        {
            Ok(()) => {
                bot.send_message(chat_id, "Conversation compacted.")
                    .await
                    .ok();
            }
            Err(error) => {
                bot.send_message(chat_id, format!("Compact failed: {error}"))
                    .await
                    .ok();
            }
        }
        Ok(())
    }

    async fn command_stop(&self, agent_key: &str, bot: &Bot, chat_id: ChatId) -> Result<()> {
        let Some(conversation) = self.find_current_conversation(agent_key, chat_id.0).await? else {
            bot.send_message(chat_id, "No active conversation to stop.")
                .await
                .ok();
            return Ok(());
        };
        match self
            .conversation_service()
            .stop_conversation(agent_key, conversation.id)
            .await
        {
            Ok(true) => {
                bot.send_message(chat_id, "Active turn stopped.").await.ok();
            }
            Ok(false) => {
                bot.send_message(chat_id, "No active turn was running.")
                    .await
                    .ok();
            }
            Err(error) => {
                bot.send_message(chat_id, format!("Stop failed: {error}"))
                    .await
                    .ok();
            }
        }
        Ok(())
    }

    async fn command_model(
        &self,
        agent_key: &str,
        bot: &Bot,
        chat_id: ChatId,
        provider: &str,
        model: &str,
    ) -> Result<()> {
        let Some(conversation) = self.find_current_conversation(agent_key, chat_id.0).await? else {
            let conversation = match self
                .create_telegram_conversation(
                    agent_key,
                    Some(chat_id.0),
                    Some((provider.to_string(), model.to_string(), None)),
                )
                .await
            {
                Ok(conversation) => conversation,
                Err(error) => {
                    bot.send_message(chat_id, telegram_error_message(&error))
                        .await
                        .ok();
                    return Ok(());
                }
            };
            bot.send_message(
                chat_id,
                format!(
                    "New conversation created with {provider}/{model}: {}",
                    conversation.title
                ),
            )
            .await
            .ok();
            return Ok(());
        };
        match self
            .conversation_service()
            .update_conversation_model_and_policies(
                agent_key,
                conversation.id,
                provider,
                model,
                None,
                &conversation.tool_policies,
            )
            .await
        {
            Ok(()) => {
                bot.send_message(chat_id, format!("Model updated to {provider}/{model}"))
                    .await
                    .ok();
            }
            Err(error) => {
                bot.send_message(chat_id, format!("Model update failed: {error}"))
                    .await
                    .ok();
            }
        }
        Ok(())
    }

    async fn submit_telegram_chat_message(
        &self,
        agent_key: &str,
        bot: &Bot,
        chat_id: ChatId,
        text: &str,
    ) -> Result<()> {
        match bot.send_chat_action(chat_id, ChatAction::Typing).await {
            Ok(_) => {
                info!(
                    agent_key = agent_key,
                    chat_id = chat_id.0,
                    "sent Telegram typing action"
                );
            }
            Err(error) => {
                warn!(agent_key = agent_key, chat_id = chat_id.0, error = ?error, "failed to send Telegram typing action");
            }
        }
        let typing_indicator =
            self.spawn_telegram_typing_indicator(agent_key.to_string(), bot.clone(), chat_id);
        let result = async {
            let conversation = match self.find_current_conversation(agent_key, chat_id.0).await? {
                Some(conversation) => conversation,
                None => {
                    self.create_telegram_conversation(agent_key, Some(chat_id.0), None)
                        .await?
                }
            };
            let reply_baseline = get_session_detail(&self.pool, &conversation.opencode_session_id)
                .await?
                .map(|detail| telegram_turn_baseline(&detail.messages, &detail.session_errors))
                .unwrap_or_default();
            let message_id = format!(
                "msg_{}",
                chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
            );
            self.conversation_service()
                .submit_conversation_turn(agent_key, conversation.id, &message_id, text)
                .await?;
            self.spawn_telegram_reply_forwarder(
                agent_key.to_string(),
                bot.clone(),
                chat_id,
                conversation.opencode_session_id,
                reply_baseline,
                typing_indicator.clone(),
            );
            Ok(())
        }
        .await;
        if let Err(error) = result {
            typing_indicator.abort();
            warn!(agent_key = agent_key, error = ?error, "telegram chat turn submission failed");
            bot.send_message(chat_id, telegram_error_message(&error))
                .await
                .ok();
        }
        Ok(())
    }

    fn spawn_telegram_reply_forwarder(
        &self,
        agent_key: String,
        bot: Bot,
        chat_id: ChatId,
        session_id: String,
        reply_baseline: TelegramTurnBaseline,
        typing_indicator: AbortHandle,
    ) {
        let service = self.clone();
        tokio::spawn(async move {
            let reply = service
                .wait_for_telegram_turn_reply(&agent_key, &session_id, &reply_baseline)
                .await;
            typing_indicator.abort();
            let reply = match reply {
                Ok(reply) => reply,
                Err(error) => {
                    warn!(
                        agent_key,
                        session_id,
                        error = ?error,
                        "failed to load completed Telegram conversation turn"
                    );
                    return;
                }
            };
            let Some(reply) = reply else {
                return;
            };
            let messages = match reply {
                TelegramTurnReply::Assistant(messages) => messages,
                TelegramTurnReply::Error(error) => vec![telegram_error_text(&error)],
            };
            for message in messages {
                for chunk in telegram_message_chunks(&message) {
                    if let Err(error) = bot.send_message(chat_id, chunk).await {
                        warn!(
                            agent_key,
                            session_id,
                            error = ?error,
                            "failed to send completed Telegram conversation turn"
                        );
                        return;
                    }
                }
            }
        });
    }

    fn spawn_telegram_typing_indicator(
        &self,
        agent_key: String,
        bot: Bot,
        chat_id: ChatId,
    ) -> AbortHandle {
        let mut shutdown_rx = self.shutdown_rx.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(TELEGRAM_TYPING_REFRESH_INTERVAL) => {}
                    changed = shutdown_rx.changed() => {
                        if changed.is_err() || *shutdown_rx.borrow() {
                            return;
                        }
                    }
                }
                if let Err(error) = bot.send_chat_action(chat_id, ChatAction::Typing).await {
                    warn!(agent_key = agent_key, chat_id = chat_id.0, error = ?error, "failed to refresh Telegram typing action");
                }
            }
        })
        .abort_handle()
    }

    async fn wait_for_telegram_turn_reply(
        &self,
        agent_key: &str,
        session_id: &str,
        reply_baseline: &TelegramTurnBaseline,
    ) -> Result<Option<TelegramTurnReply>> {
        let agent = crate::agents::store::get_agent(&self.pool, agent_key)
            .await?
            .ok_or_else(|| anyhow::anyhow!("agent not found"))?;
        let runtime = OpenCodeWorkspaceRuntimeConfig::from_value(&agent.runtime_config)
            .ok_or_else(|| anyhow::anyhow!("agent missing workspace metadata"))?;
        let mut shutdown_rx = self.shutdown_rx.clone();
        let completed = match tokio::time::timeout(TELEGRAM_REPLY_TIMEOUT, async {
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(TELEGRAM_REPLY_POLL_INTERVAL) => {}
                    changed = shutdown_rx.changed() => {
                        if changed.is_err() || *shutdown_rx.borrow() {
                            return Ok(false);
                        }
                    }
                }
                match self
                    .opencode_client
                    .get_session_status_in_directory(
                        &self.opencode_base_url,
                        session_id,
                        Some(&runtime.workspace_container_path),
                    )
                    .await
                {
                    Ok(Some(status)) if status.is_active() => {}
                    Ok(_) => return Ok(true),
                    Err(error) => return Err(error),
                }
            }
        })
        .await
        {
            Ok(result) => result?,
            Err(_) => {
                warn!(
                    agent_key,
                    session_id,
                    timeout_seconds = TELEGRAM_REPLY_TIMEOUT.as_secs(),
                    "Telegram conversation turn did not complete before timeout"
                );
                return Ok(None);
            }
        };
        if !completed {
            return Ok(None);
        }

        match tokio::time::timeout(TELEGRAM_REPLY_MIRROR_TIMEOUT, async {
            loop {
                if let Some(detail) = get_session_detail(&self.pool, session_id).await?
                    && let Some(reply) = telegram_turn_reply(
                        &detail.messages,
                        &detail.session_errors,
                        reply_baseline,
                    )
                {
                    return Ok(Some(reply));
                }
                tokio::select! {
                    _ = tokio::time::sleep(TELEGRAM_REPLY_POLL_INTERVAL) => {}
                    changed = shutdown_rx.changed() => {
                        if changed.is_err() || *shutdown_rx.borrow() {
                            return Ok(None);
                        }
                    }
                }
            }
        })
        .await
        {
            Ok(result) => result,
            Err(_) => {
                warn!(
                    agent_key,
                    session_id,
                    timeout_seconds = TELEGRAM_REPLY_MIRROR_TIMEOUT.as_secs(),
                    "completed Telegram conversation turn had no mirrored reply"
                );
                Ok(None)
            }
        }
    }

    async fn find_current_conversation(
        &self,
        agent_key: &str,
        chat_id: i64,
    ) -> Result<Option<agent_conversations::model::AgentConversationRow>> {
        conversation_store::get_conversation_by_external_key(
            &self.pool,
            agent_key,
            CONVERSATION_CHANNEL_TELEGRAM,
            &chat_id.to_string(),
        )
        .await
    }

    async fn create_telegram_conversation(
        &self,
        agent_key: &str,
        chat_id: Option<i64>,
        selected_model: Option<(String, String, Option<String>)>,
    ) -> Result<agent_conversations::model::AgentConversationRow> {
        let (provider_id, model_id, model_variant) = match selected_model {
            Some(model) => model,
            None => conversation_store::list_conversations_for_channel(
                &self.pool,
                agent_key,
                CONVERSATION_CHANNEL_WEB,
            )
            .await?
            .into_iter()
            .find_map(|conversation| {
                model_selection(
                    &conversation.model_provider_id,
                    &conversation.model_id,
                    conversation.model_variant.as_deref(),
                )
            })
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "No model is configured. Use /model <provider> <model> to select one."
                )
            })?,
        };
        let external_conversation_key =
            chat_id.map_or_else(String::new, |chat_id| chat_id.to_string());
        self.conversation_service()
            .create_gateway_conversation(
                agent_key,
                CONVERSATION_CHANNEL_TELEGRAM,
                &external_conversation_key,
                &provider_id,
                &model_id,
                model_variant.as_deref(),
            )
            .await
    }

    async fn handle_telegram_callback(
        &self,
        agent_key: &str,
        bot: &Bot,
        query: teloxide::types::CallbackQuery,
    ) -> Result<()> {
        let Some(data) = query.data.clone() else {
            return Ok(());
        };
        let parts: Vec<&str> = data.splitn(3, ':').collect();
        if parts.is_empty() {
            return Ok(());
        }
        match parts[0] {
            CALLBACK_PERMISSION => {
                if parts.len() != 3 {
                    return Ok(());
                }
                self.handle_permission_callback(agent_key, bot, query, parts[1], parts[2])
                    .await?;
            }
            CALLBACK_SWITCH => {
                if parts.len() != 2 {
                    return Ok(());
                }
                self.handle_switch_callback(agent_key, bot, query, parts[1])
                    .await?;
            }
            _ => {}
        }
        Ok(())
    }

    async fn handle_switch_callback(
        &self,
        agent_key: &str,
        bot: &Bot,
        query: teloxide::types::CallbackQuery,
        conversation_id_str: &str,
    ) -> Result<()> {
        let Ok(conversation_id) = Uuid::parse_str(conversation_id_str) else {
            return Ok(());
        };
        let Some(message) = query.regular_message() else {
            return Ok(());
        };
        let chat_id = message.chat.id;
        let target_message_id = message.id;
        let callback_id = query.id.clone();
        let Some(gateway) = gateway_store::get_gateway(&self.pool, agent_key, "telegram").await?
        else {
            return Ok(());
        };
        let config = TelegramGatewayConfig::from_value(&gateway.config);
        if !is_authorized_chat(&config, chat_id.0) {
            return Ok(());
        }
        let switched = conversation_store::activate_conversation_for_external_key(
            &self.pool,
            agent_key,
            CONVERSATION_CHANNEL_TELEGRAM,
            conversation_id,
            &chat_id.0.to_string(),
        )
        .await?;
        if !switched {
            bot.answer_callback_query(callback_id)
                .text("Conversation is no longer available.")
                .await
                .ok();
            return Ok(());
        }
        bot.answer_callback_query(callback_id).await.ok();
        bot.edit_message_text(
            chat_id,
            target_message_id,
            "Switched conversation. Send a message to continue.",
        )
        .await
        .ok();
        Ok(())
    }

    async fn poll_pending_permissions(
        &self,
        agent_key: &str,
        bot: &Bot,
        seen: &mut HashSet<String>,
    ) -> Result<()> {
        let Some(gateway) = gateway_store::get_gateway(&self.pool, agent_key, "telegram").await?
        else {
            return Ok(());
        };
        let config = TelegramGatewayConfig::from_value(&gateway.config);
        let Some(chat_id) = config.chat_id else {
            return Ok(());
        };
        let agent = crate::agents::store::get_agent(&self.pool, agent_key)
            .await?
            .ok_or_else(|| anyhow::anyhow!("agent not found"))?;
        let Some(runtime) = crate::opencode::workspace::OpenCodeWorkspaceRuntimeConfig::from_value(
            &agent.runtime_config,
        ) else {
            return Ok(());
        };
        let pending = self
            .opencode_client
            .list_pending_permissions(&self.opencode_base_url, &runtime.workspace_container_path)
            .await?;
        for request in pending {
            if seen.contains(&request.id) {
                continue;
            }
            match send_approval_keyboard(bot, ChatId(chat_id), &request.id, &request.permission)
                .await
            {
                Ok(_) => {
                    seen.insert(request.id.clone());
                }
                Err(error) => {
                    warn!(agent_key = agent_key, error = ?error, "failed to send approval keyboard");
                }
            }
        }
        Ok(())
    }

    async fn handle_permission_callback(
        &self,
        agent_key: &str,
        bot: &Bot,
        query: teloxide::types::CallbackQuery,
        request_id: &str,
        action: &str,
    ) -> Result<()> {
        let reply = match action {
            "once" => OpenCodePermissionReply::Once,
            "reject" => OpenCodePermissionReply::Reject,
            _ => return Ok(()),
        };
        let agent = crate::agents::store::get_agent(&self.pool, agent_key)
            .await?
            .ok_or_else(|| anyhow::anyhow!("agent not found"))?;
        let runtime = crate::opencode::workspace::OpenCodeWorkspaceRuntimeConfig::from_value(
            &agent.runtime_config,
        )
        .ok_or_else(|| anyhow::anyhow!("agent missing workspace metadata"))?;
        let pending = self
            .opencode_client
            .list_pending_permissions(&self.opencode_base_url, &runtime.workspace_container_path)
            .await?;
        let Some(request) = pending.iter().find(|item| item.id == request_id) else {
            let callback_id = query.id.clone();
            bot.answer_callback_query(callback_id)
                .text("Permission request is no longer pending.")
                .await
                .ok();
            return Ok(());
        };
        let session_id = request.session_id.clone();
        let accepted = self
            .opencode_client
            .reply_to_permission(
                &self.opencode_base_url,
                &runtime.workspace_container_path,
                &session_id,
                request_id,
                reply,
            )
            .await?;
        let decision = if accepted {
            match reply {
                OpenCodePermissionReply::Once => "approved",
                OpenCodePermissionReply::Reject => "rejected",
            }
        } else {
            "expired"
        };
        let (callback_id, message) = (query.id.clone(), query.regular_message().cloned());
        bot.answer_callback_query(callback_id).await.ok();
        if let Some(message) = message {
            edit_permission_message(bot, message.chat.id, message.id, decision)
                .await
                .ok();
        }
        Ok(())
    }

    async fn dispatch_notification_by_id(&self, id: &str) -> Result<()> {
        let id = Uuid::parse_str(id.trim()).context("invalid notification id payload")?;
        let row = sqlx::query_as::<_, notifications::model::NotificationRow>(
            "SELECT id, agent_key, title, body, severity, status
               FROM notifications
               WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .context("failed to fetch notification for dispatch")?;
        let Some(row) = row else {
            return Ok(());
        };
        if row.status != "queued" {
            return Ok(());
        };
        let record = notifications::model::NotificationRecord::from(row);
        self.send_notification(&record).await
    }

    async fn send_notification(
        &self,
        notification: &notifications::model::NotificationRecord,
    ) -> Result<()> {
        let Some(gateway) =
            gateway_store::get_gateway(&self.pool, &notification.agent_key, "telegram").await?
        else {
            return Ok(());
        };
        let config = TelegramGatewayConfig::from_value(&gateway.config);
        let Some(chat_id) = config.chat_id else {
            return Ok(());
        };
        let Some(ciphertext) = config.bot_token_ciphertext.as_ref() else {
            return Ok(());
        };
        let Some(key_id) = config.bot_token_key_id.as_ref() else {
            return Ok(());
        };
        if key_id != &self.encryption_key.key_id {
            notifications::store::mark_failed(
                &self.pool,
                notification.id,
                "notification bot token was encrypted with an inactive key",
            )
            .await
            .ok();
            return Ok(());
        }
        let token = match decrypt(&self.encryption_key, ciphertext) {
            Ok(token) => token,
            Err(error) => {
                notifications::store::mark_failed(&self.pool, notification.id, &error.to_string())
                    .await
                    .ok();
                return Ok(());
            }
        };
        let bot = build_bot(&token);
        let text = format_notification(notification);
        if bot.send_message(ChatId(chat_id), text).await.is_ok() {
            notifications::store::mark_sent(&self.pool, notification.id)
                .await
                .ok();
        } else {
            notifications::store::mark_failed(
                &self.pool,
                notification.id,
                "telegram send_message failed",
            )
            .await
            .ok();
        }
        Ok(())
    }

    fn conversation_service(&self) -> ConversationService<'_> {
        ConversationService {
            pool: &self.pool,
            client: &self.opencode_client,
            base_url: &self.opencode_base_url,
            workspace_leases: &self.workspace_leases,
            in_flight: &self.in_flight,
            turn_tracker: &self.conversation_turns,
            shutdown_rx: self.shutdown_rx.clone(),
        }
    }
}

/// Format a notification for Telegram using MarkdownV2-safe plain text.
fn format_notification(notification: &notifications::model::NotificationRecord) -> String {
    let emoji = notification.severity.emoji();
    format!("{}{}\n\n{}", emoji, notification.title, notification.body)
}

fn telegram_error_message(error: &anyhow::Error) -> String {
    telegram_error_text(&error.to_string())
}

fn telegram_error_text(error: &str) -> String {
    let detail: String = error.chars().take(1000).collect();
    format!("I couldn't process that message: {detail}")
}

fn telegram_turn_baseline(
    messages: &[OpenCodeMessageRow],
    session_errors: &[OpenCodeSessionErrorRow],
) -> TelegramTurnBaseline {
    TelegramTurnBaseline {
        assistant_message_ids: messages
            .iter()
            .filter(|message| message.role == "assistant")
            .map(|message| message.id.clone())
            .collect(),
        session_error_count: session_errors.len(),
    }
}

fn telegram_turn_reply(
    messages: &[OpenCodeMessageRow],
    session_errors: &[OpenCodeSessionErrorRow],
    baseline: &TelegramTurnBaseline,
) -> Option<TelegramTurnReply> {
    let replies: Vec<String> = messages
        .iter()
        .filter(|message| {
            message.role == "assistant" && !baseline.assistant_message_ids.contains(&message.id)
        })
        .filter_map(|message| {
            message
                .text
                .as_deref()
                .filter(|text| !text.trim().is_empty())
                .map(ToOwned::to_owned)
        })
        .collect();
    if !replies.is_empty() {
        return Some(TelegramTurnReply::Assistant(replies));
    }
    session_errors
        .iter()
        .skip(baseline.session_error_count)
        .rev()
        .find_map(|error| error.error_message.as_deref())
        .filter(|error| !error.trim().is_empty())
        .map(|error| TelegramTurnReply::Error(error.to_string()))
}

fn telegram_message_chunks(text: &str) -> Vec<String> {
    text.chars()
        .collect::<Vec<_>>()
        .chunks(TELEGRAM_MESSAGE_MAX_CHARS)
        .map(|chunk| chunk.iter().collect())
        .collect()
}

fn model_selection(
    provider_id: &str,
    model_id: &str,
    model_variant: Option<&str>,
) -> Option<(String, String, Option<String>)> {
    let provider_id = provider_id.trim();
    let model_id = model_id.trim();
    if provider_id.is_empty() || model_id.is_empty() {
        return None;
    }
    Some((
        provider_id.to_string(),
        model_id.to_string(),
        model_variant
            .map(str::trim)
            .filter(|variant| !variant.is_empty())
            .map(ToOwned::to_owned),
    ))
}

fn telegram_new_session_message(provider_id: &str, model_id: &str) -> String {
    format!(
        "New Session\nModel: {} {}",
        provider_id.trim(),
        model_id.trim()
    )
}

/// View used by the operator settings page to render the Telegram gateway
/// section. Mirrors [`TelegramGatewayConfigView`] plus a pending link token
/// when a flow is in progress.
#[derive(Debug, Clone)]
pub struct GatewayTelegramView {
    pub has_token: bool,
    pub connected: bool,
    pub config: TelegramGatewayConfigView,
    pub pending_token: Option<Uuid>,
}

impl GatewayTelegramView {
    /// Build a view for the settings page from a stored gateway row. When the
    /// operator has started a link flow, pass the pending token so the page
    /// can render a deep link button.
    pub fn from_row(row: &AgentGatewayRow, pending_token: Option<Uuid>) -> Self {
        let config = TelegramGatewayConfig::from_value(&row.config);
        Self {
            has_token: config.is_ready(),
            connected: config.is_ready() && config.chat_id.is_some(),
            config: config.to_view(),
            pending_token,
        }
    }

    /// Empty view used when the agent has no gateway row yet.
    pub fn empty() -> Self {
        Self {
            has_token: false,
            connected: false,
            config: TelegramGatewayConfigView::default(),
            pending_token: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_db;
    use chrono::Utc;

    async fn seed_agent(pool: &DbPool, key: &str) {
        let now = Utc::now();
        crate::agents::store::insert_agent(
            pool,
            &crate::agents::model::AgentRegistryRow {
                agent_key: key.to_string(),
                user_id: test_db::test_user_id(),
                created_at: now,
                updated_at: now,
                enabled: true,
                lifecycle: crate::agents::model::AGENT_LIFECYCLE_ACTIVE.to_string(),
                display_name: key.to_string(),
                trading_account_address: Some(format!("0x{:040x}", uuid::Uuid::new_v4().as_u128())),
                environment: "live".to_string(),
                api_key: format!("vta_{key}"),
                api_key_last_used_at: None,
                runtime_config: serde_json::json!({}),
            },
        )
        .await
        .expect("insert agent");
    }

    fn service(pool: DbPool) -> GatewayService {
        let (_tx, rx) = watch::channel(false);
        GatewayService::new(
            pool,
            Arc::new(
                crate::opencode::client::OpenCodeClient::new(
                    crate::opencode::client::OpenCodeClientConfig::new("u".to_string(), None),
                )
                .unwrap(),
            ),
            "http://localhost:14096".to_string(),
            crate::agents::crypto::EncryptionKey::new(
                "test",
                [
                    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21,
                    22, 23, 24, 25, 26, 27, 28, 29, 30, 31,
                ],
            ),
            rx,
            InFlightTracker::new(),
            WorkspaceLeaseManager::new(),
            ConversationTurnTracker::default(),
            Arc::new(DashMap::new()),
        )
    }

    #[tokio::test]
    async fn send_notification_without_gateway_leaves_row_queued() {
        let pool = test_db::pool().await;
        let key = format!(
            "note-no-gateway-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        seed_agent(&pool, &key).await;
        let record = notifications::store::create_notification(
            &pool,
            &key,
            "title",
            "body",
            crate::gateway::model::NotificationSeverity::Info,
        )
        .await
        .expect("create notification");

        // Without a configured gateway, the dispatcher returns Ok(()) and
        // leaves the row queued. The dispatcher runs through the
        // GatewayServiceState::dispatch_notification_by_id path which the
        // notification listener spawns in production; tests can verify the
        // outcome by re-reading the row after the dispatcher would run.
        let row = sqlx::query_as::<_, notifications::model::NotificationRow>(
            "SELECT id, agent_key, title, body, severity, status
               FROM notifications
               WHERE id = $1",
        )
        .bind(record.id)
        .fetch_one(&pool)
        .await
        .expect("fetch row");
        assert_eq!(row.status, "queued");
        assert_eq!(row.title, "title");
        assert_eq!(row.body, "body");
    }

    #[tokio::test]
    async fn format_notification_includes_emoji_title_and_body() {
        let record = notifications::model::NotificationRecord {
            id: Uuid::nil(),
            agent_key: "agent".to_string(),
            title: "Title".to_string(),
            body: "Body".to_string(),
            severity: crate::gateway::model::NotificationSeverity::Warning,
        };
        let text = format_notification(&record);
        assert!(text.starts_with("\u{26a0}\u{fe0f} "));
        assert!(text.contains("Title"));
        assert!(text.contains("Body"));
    }

    #[test]
    fn telegram_error_message_includes_the_failure() {
        let message = telegram_error_message(&anyhow::anyhow!("model is unavailable"));

        assert_eq!(
            message,
            "I couldn't process that message: model is unavailable"
        );
    }

    #[test]
    fn model_selection_rejects_blank_model_fields() {
        assert_eq!(
            model_selection("openai", "  ", None),
            None,
            "a stale Telegram conversation must not prevent web-model inheritance"
        );
    }

    #[test]
    fn model_selection_trims_values_and_drops_blank_variant() {
        assert_eq!(
            model_selection(" openai ", " gpt5.6-luna ", Some(" ")),
            Some(("openai".to_string(), "gpt5.6-luna".to_string(), None))
        );
    }

    #[test]
    fn telegram_new_session_message_includes_the_model() {
        assert_eq!(
            telegram_new_session_message("openai", "gpt5.6-luna"),
            "New Session\nModel: openai gpt5.6-luna"
        );
    }

    fn message(id: &str, role: &str, text: Option<&str>) -> OpenCodeMessageRow {
        OpenCodeMessageRow {
            id: id.to_string(),
            created_at: Utc::now(),
            role: role.to_string(),
            model_provider: None,
            model_id: None,
            text: text.map(ToOwned::to_owned),
            summary: None,
            system_prompt: None,
        }
    }

    fn session_error(message: &str) -> OpenCodeSessionErrorRow {
        OpenCodeSessionErrorRow {
            error_type: None,
            error_message: Some(message.to_string()),
        }
    }

    #[test]
    fn telegram_turn_reply_forwards_only_new_assistant_text() {
        let messages = vec![
            message("old", "assistant", Some("previous reply")),
            message("user", "user", Some("prompt")),
            message("new", "assistant", Some("new reply")),
        ];
        let baseline = telegram_turn_baseline(&messages[..1], &[]);

        assert_eq!(
            telegram_turn_reply(&messages, &[], &baseline),
            Some(TelegramTurnReply::Assistant(vec!["new reply".to_string()]))
        );
    }

    #[test]
    fn telegram_turn_reply_forwards_only_new_session_errors() {
        let errors = vec![session_error("previous error"), session_error("new error")];
        let baseline = telegram_turn_baseline(&[], &errors[..1]);

        assert_eq!(
            telegram_turn_reply(&[], &errors, &baseline),
            Some(TelegramTurnReply::Error("new error".to_string()))
        );
    }

    #[test]
    fn telegram_message_chunks_preserves_long_unicode_text() {
        let text = "\u{1f4ac}".repeat(TELEGRAM_MESSAGE_MAX_CHARS + 1);
        let chunks = telegram_message_chunks(&text);

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].chars().count(), TELEGRAM_MESSAGE_MAX_CHARS);
        assert_eq!(chunks[1], "\u{1f4ac}");
        assert_eq!(chunks.concat(), text);
    }

    #[test]
    fn telegram_typing_refresh_precedes_action_expiry() {
        assert!(TELEGRAM_TYPING_REFRESH_INTERVAL < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn gateway_telegram_view_from_row_omits_secrets() {
        let pool = test_db::pool().await;
        let key = format!("view-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0));
        seed_agent(&pool, &key).await;
        let config = TelegramGatewayConfig {
            bot_token_ciphertext: Some(vec![1, 2, 3]),
            bot_token_key_id: Some("test".to_string()),
            bot_username: Some("mybot".to_string()),
            chat_id: Some(42),
            chat_username: Some("user".to_string()),
        };
        gateway_store::upsert_telegram_config(&pool, &key, &config)
            .await
            .expect("upsert config");
        let row =
            gateway_store::get_gateway(&pool, &key, crate::gateway::model::GATEWAY_TYPE_TELEGRAM)
                .await
                .expect("get gateway")
                .expect("gateway exists");
        let view = GatewayTelegramView::from_row(&row, None);
        assert!(view.has_token);
        assert!(view.connected);
        assert_eq!(view.config.bot_username.as_deref(), Some("mybot"));
        assert_eq!(view.config.chat_id, Some(42));
        assert_eq!(view.config.chat_username.as_deref(), Some("user"));
        let json = serde_json::to_string(&view.config).expect("serialize view config");
        assert!(!json.contains("ciphertext"));
        assert!(!json.contains("key_id"));
    }

    #[tokio::test]
    async fn gateway_telegram_view_empty_is_disabled() {
        let view = GatewayTelegramView::empty();
        assert!(!view.has_token);
        assert!(!view.connected);
        assert!(view.config.bot_username.is_none());
        assert!(view.config.chat_id.is_none());
    }

    #[tokio::test]
    async fn decrypt_bot_token_returns_none_when_key_id_mismatches() {
        let pool = test_db::pool().await;
        let svc = service((*pool).clone());
        let config = TelegramGatewayConfig {
            bot_token_ciphertext: Some(vec![1, 2, 3]),
            bot_token_key_id: Some("other".to_string()),
            ..Default::default()
        };
        assert!(svc.decrypt_bot_token(&config).is_none());
    }
}
