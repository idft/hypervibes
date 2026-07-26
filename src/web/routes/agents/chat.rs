use std::{
    collections::VecDeque,
    convert::Infallible,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use askama::Template;
use axum::{
    Form,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{
        Html, IntoResponse, Redirect, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use futures::{StreamExt, stream::unfold};
use serde::Deserialize;
use tokio::sync::broadcast;
use tracing::warn;
use uuid::Uuid;

use crate::{
    agent_conversations::{
        model::{AgentConversationRow, TOOL_GROUP_MEMORY_WRITES, TOOL_GROUP_ORDERS},
        service::{CONVERSATION_MESSAGE_MAX_CHARS, ConversationService},
    },
    agents::store::get_agent,
    model_catalog::options::parse_model_selection,
    opencode::{client::OpenCodePermissionReply, store::get_session_detail},
    web::{
        AppState,
        auth::AuthenticatedUser,
        error::AppError,
        run_detail_events::RunDetailDbEvent,
        templates::{
            AgentConversationComposerPartialTemplate, AgentConversationEmptyPageTemplate,
            AgentConversationPageTemplate, AgentConversationPermissionRequestView,
            AgentConversationPermissionsPartialTemplate, AgentConversationSettingsView,
            AgentConversationSidebarPartialTemplate, AgentConversationSummaryPartialTemplate,
            AgentConversationTranscriptPartialTemplate, OpenCodeSessionView, conversation_items,
            load_navbar,
        },
    },
};

use super::shared::{
    build_model_picker_view, is_htmx_request, load_model_picker_context,
    validate_model_selection_for_agent,
};

#[derive(Default, Deserialize)]
pub(in crate::web::routes) struct NewConversationForm {
    #[serde(default)]
    model_selection: String,
}
#[derive(Default, Deserialize)]
pub(in crate::web::routes) struct ConversationMessageForm {
    #[serde(default)]
    message: String,
    #[serde(default)]
    message_id: String,
}
#[derive(Default, Deserialize)]
pub(in crate::web::routes) struct ConversationSettingsForm {
    #[serde(default)]
    model_selection: String,
    #[serde(default)]
    orders_policy: String,
    #[serde(default)]
    memory_writes_policy: String,
}
#[derive(Default, Deserialize)]
pub(in crate::web::routes) struct PermissionReplyForm {
    #[serde(default)]
    reply: String,
}

struct ConversationSnapshot {
    agent: crate::agents::model::AgentDetailRow,
    conversation: AgentConversationRow,
    conversations: Vec<crate::agent_conversations::model::AgentConversationListRow>,
    session: Option<OpenCodeSessionView>,
    busy: bool,
    settings: AgentConversationSettingsView,
    permissions: Vec<AgentConversationPermissionRequestView>,
}

fn service(state: &AppState) -> ConversationService<'_> {
    ConversationService {
        pool: &state.db_pool,
        client: &state.opencode_client,
        base_url: &state.opencode_base_url,
        workspace_leases: &state.workspace_leases,
        in_flight: &state.in_flight,
        turn_tracker: &state.conversation_turns,
        shutdown_rx: state.shutdown_rx.clone(),
    }
}

async fn load_snapshot(
    state: &Arc<AppState>,
    agent_key: &str,
    conversation_id: Uuid,
) -> Result<Option<ConversationSnapshot>, AppError> {
    let Some(agent) = get_agent(&state.db_pool, agent_key).await? else {
        return Ok(None);
    };
    let Some(conversation) = crate::agent_conversations::store::get_agent_conversation(
        &state.db_pool,
        agent_key,
        conversation_id,
    )
    .await?
    else {
        return Ok(None);
    };
    let conversations =
        crate::agent_conversations::store::list_agent_conversations(&state.db_pool, agent_key)
            .await?;
    let session = get_session_detail(&state.db_pool, &conversation.opencode_session_id)
        .await?
        .as_ref()
        .map(OpenCodeSessionView::from_detail);
    let status_text = session_status(&state.db_pool, &conversation.opencode_session_id)
        .await
        .unwrap_or_else(|| "syncing".to_string());
    let busy = state.conversation_turns.is_active(conversation_id).await
        || matches!(status_text.as_str(), "busy" | "retry");
    let picker = build_model_picker_view(
        "conversation-model-selection",
        &format!(
            "{}/{}",
            conversation.model_provider_id, conversation.model_id
        ),
        load_model_picker_context(state, &agent).await,
    );
    let orders_policy = conversation
        .tool_policies
        .iter()
        .find(|item| item.tool_group == TOOL_GROUP_ORDERS)
        .map(|item| item.policy.clone())
        .unwrap_or_else(|| "confirm".to_string());
    let memory_writes_policy = conversation
        .tool_policies
        .iter()
        .find(|item| item.tool_group == TOOL_GROUP_MEMORY_WRITES)
        .map(|item| item.policy.clone())
        .unwrap_or_else(|| "confirm".to_string());
    let permissions = match crate::opencode::workspace::OpenCodeWorkspaceRuntimeConfig::from_value(
        &agent.runtime_config,
    ) {
        Some(runtime) => state
            .opencode_client
            .list_pending_permissions(&state.opencode_base_url, &runtime.workspace_container_path)
            .await
            .unwrap_or_default()
            .iter()
            .filter(|request| request.session_id == conversation.opencode_session_id)
            .map(AgentConversationPermissionRequestView::from)
            .collect(),
        None => Vec::new(),
    };
    Ok(Some(ConversationSnapshot {
        agent,
        conversation,
        conversations,
        session,
        busy,
        settings: AgentConversationSettingsView {
            model_picker: picker,
            orders_policy,
            memory_writes_policy,
            disabled: busy,
        },
        permissions,
    }))
}

async fn session_status(pool: &crate::db::DbPool, session_id: &str) -> Option<String> {
    get_session_detail(pool, session_id)
        .await
        .ok()
        .flatten()
        .and_then(|detail| detail.session.status)
}

struct RenderedSnapshot {
    sidebar: String,
    summary: String,
    transcript: String,
    composer: String,
    permissions: String,
}
impl RenderedSnapshot {
    fn events(self) -> Vec<Event> {
        vec![
            Event::default()
                .event("conversation-sidebar")
                .data(self.sidebar),
            Event::default()
                .event("conversation-summary")
                .data(self.summary),
            Event::default()
                .event("conversation-transcript")
                .data(self.transcript),
            Event::default()
                .event("conversation-composer")
                .data(self.composer),
            Event::default()
                .event("conversation-permissions")
                .data(self.permissions),
        ]
    }
}
fn render_snapshot(
    snapshot: &ConversationSnapshot,
    message: String,
    error: Option<String>,
) -> Result<RenderedSnapshot, AppError> {
    let sidebar = AgentConversationSidebarPartialTemplate {
        agent_key: snapshot.agent.agent_key.clone(),
        new_conversation_model_selection: format!(
            "{}/{}",
            snapshot.conversation.model_provider_id, snapshot.conversation.model_id
        ),
        conversations: conversation_items(&snapshot.conversations, Some(snapshot.conversation.id)),
    }
    .render()?;
    let summary = AgentConversationSummaryPartialTemplate {
        agent_key: snapshot.agent.agent_key.clone(),
        conversation_id: snapshot.conversation.id,
        title: snapshot.conversation.title.clone(),
        model_text: format!(
            "{}/{}",
            snapshot.conversation.model_provider_id, snapshot.conversation.model_id
        ),
        busy: snapshot.busy,
        session: snapshot.session.clone(),
        settings: snapshot.settings.clone(),
    }
    .render()?;
    let transcript = AgentConversationTranscriptPartialTemplate {
        session: snapshot.session.clone(),
        busy: snapshot.busy,
    }
    .render()?;
    let composer = AgentConversationComposerPartialTemplate {
        agent_key: snapshot.agent.agent_key.clone(),
        conversation_id: snapshot.conversation.id,
        message_id: opencode_message_id(),
        busy: snapshot.busy,
        message,
        error,
    }
    .render()?;
    let permissions = AgentConversationPermissionsPartialTemplate {
        agent_key: snapshot.agent.agent_key.clone(),
        conversation_id: snapshot.conversation.id,
        requests: snapshot.permissions.clone(),
    }
    .render()?;
    Ok(RenderedSnapshot {
        sidebar,
        summary,
        transcript,
        composer,
        permissions,
    })
}

fn opencode_message_id() -> String {
    let now_millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is before the Unix epoch")
        .as_millis() as u64;
    opencode_message_id_at(now_millis, Uuid::new_v4())
}

pub(super) fn opencode_message_id_at(timestamp_millis: u64, random: Uuid) -> String {
    // OpenCode compares message IDs lexically to determine the latest turn.
    // Its IDs encode the current millisecond in the first 12 hexadecimal digits.
    let timestamp = (timestamp_millis << 12) & 0x0000_ffff_ffff_ffff;
    let random = random.simple().to_string();
    format!("msg_{timestamp:012x}{}", &random[..14])
}

pub(in crate::web::routes) async fn agents_show_chat(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
    user: AuthenticatedUser,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let conversations =
        crate::agent_conversations::store::list_agent_conversations(&state.db_pool, &agent_key)
            .await?;
    if let Some(conversation) = conversations.first() {
        return Ok(
            Redirect::to(&format!("/agents/{agent_key}/chat/{}", conversation.id)).into_response(),
        );
    }
    let picker = build_model_picker_view(
        "conversation-model-selection",
        "",
        load_model_picker_context(&state, &agent).await,
    );
    Ok(Html(AgentConversationEmptyPageTemplate::render_view(
        agent,
        picker,
        Vec::new(),
        load_navbar(&state.db_pool, user.id).await?,
    )?)
    .into_response())
}

pub(in crate::web::routes) async fn agents_new_chat(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
    user: AuthenticatedUser,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let conversations =
        crate::agent_conversations::store::list_agent_conversations(&state.db_pool, &agent_key)
            .await?;
    if let Some(conversation) = conversations.first() {
        return Ok(
            Redirect::to(&format!("/agents/{agent_key}/chat/{}", conversation.id)).into_response(),
        );
    }
    let picker = build_model_picker_view(
        "conversation-model-selection",
        "",
        load_model_picker_context(&state, &agent).await,
    );
    Ok(Html(AgentConversationEmptyPageTemplate::render_view(
        agent,
        picker,
        Vec::new(),
        load_navbar(&state.db_pool, user.id).await?,
    )?)
    .into_response())
}

pub(in crate::web::routes) async fn agents_show_chat_detail(
    State(state): State<Arc<AppState>>,
    Path((agent_key, conversation_id)): Path<(String, Uuid)>,
    user: AuthenticatedUser,
) -> Result<Response, AppError> {
    let Some(snapshot) = load_snapshot(&state, &agent_key, conversation_id).await? else {
        return Ok((StatusCode::NOT_FOUND, "conversation not found").into_response());
    };
    let rendered = render_snapshot(&snapshot, String::new(), None)?;
    let html = AgentConversationPageTemplate::render_view(
        crate::web::templates::AgentConversationPageInput {
            agent: snapshot.agent,
            conversation_id,
            sidebar_html: rendered.sidebar,
            summary_html: rendered.summary,
            transcript_html: rendered.transcript,
            composer_html: rendered.composer,
            permissions_html: rendered.permissions,
            navbar: load_navbar(&state.db_pool, user.id).await?,
        },
    )?;
    Ok(Html(html).into_response())
}

pub(in crate::web::routes) async fn agents_create_conversation(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
    user: AuthenticatedUser,
    Form(form): Form<NewConversationForm>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let conversations =
        crate::agent_conversations::store::list_agent_conversations(&state.db_pool, &agent_key)
            .await?;
    let model_selection = if form.model_selection.trim().is_empty() {
        conversations
            .first()
            .map(|conversation| {
                format!(
                    "{}/{}",
                    conversation.model_provider_id, conversation.model_id
                )
            })
            .unwrap_or_default()
    } else {
        form.model_selection
    };
    let selection = match parse_model_selection(&model_selection)
        .and_then(|selection| selection.ok_or_else(|| "Select a model.".to_string()))
    {
        Ok(selection) => selection,
        Err(error) => {
            return render_empty_error(&state, agent, model_selection, error, user.id).await;
        }
    };
    let selection = match validate_model_selection_for_agent(&state, &agent, Some(selection)).await
    {
        Ok(Some(selection)) => selection,
        Ok(None) | Err(_) => {
            return render_empty_error(
                &state,
                agent,
                model_selection,
                "Select a valid model.".to_string(),
                user.id,
            )
            .await;
        }
    };
    let conversation = service(&state)
        .create_web_conversation(&agent_key, &selection.0, &selection.1)
        .await?;
    Ok(Redirect::to(&format!("/agents/{agent_key}/chat/{}", conversation.id)).into_response())
}

async fn render_empty_error(
    state: &Arc<AppState>,
    agent: crate::agents::model::AgentDetailRow,
    selection: String,
    error: String,
    user_id: Uuid,
) -> Result<Response, AppError> {
    let picker = build_model_picker_view(
        "conversation-model-selection",
        &selection,
        load_model_picker_context(state, &agent).await,
    );
    Ok(Html(AgentConversationEmptyPageTemplate::render_view(
        agent,
        picker,
        vec![error],
        load_navbar(&state.db_pool, user_id).await?,
    )?)
    .into_response())
}

pub(in crate::web::routes) async fn agents_send_conversation_message(
    State(state): State<Arc<AppState>>,
    Path((agent_key, conversation_id)): Path<(String, Uuid)>,
    headers: HeaderMap,
    Form(form): Form<ConversationMessageForm>,
) -> Result<Response, AppError> {
    if form.message.trim().is_empty()
        || form.message.chars().count() > CONVERSATION_MESSAGE_MAX_CHARS
    {
        let error = if form.message.trim().is_empty() {
            "Message cannot be blank."
        } else {
            "Message is too long."
        };
        return render_message_error(&state, &agent_key, conversation_id, form.message, error)
            .await;
    }
    match service(&state)
        .submit_conversation_turn(&agent_key, conversation_id, &form.message_id, &form.message)
        .await
    {
        Ok(()) => chat_message_redirect(&agent_key, conversation_id, is_htmx_request(&headers)),
        Err(error) => {
            render_message_error(
                &state,
                &agent_key,
                conversation_id,
                form.message,
                &error.to_string(),
            )
            .await
        }
    }
}

pub(super) fn chat_message_redirect(
    agent_key: &str,
    conversation_id: Uuid,
    htmx: bool,
) -> Result<Response, AppError> {
    let location = format!("/agents/{agent_key}/chat/{conversation_id}");
    if !htmx {
        return Ok(Redirect::to(&location).into_response());
    }
    let mut response = StatusCode::OK.into_response();
    response.headers_mut().insert(
        "HX-Redirect",
        HeaderValue::try_from(location).map_err(|error| {
            AppError(anyhow::anyhow!("invalid chat redirect location: {error}"))
        })?,
    );
    Ok(response)
}

async fn render_message_error(
    state: &Arc<AppState>,
    agent_key: &str,
    conversation_id: Uuid,
    message: String,
    error: &str,
) -> Result<Response, AppError> {
    let Some(snapshot) = load_snapshot(state, agent_key, conversation_id).await? else {
        return Ok((StatusCode::NOT_FOUND, "conversation not found").into_response());
    };
    let rendered = render_snapshot(&snapshot, message, Some(error.to_string()))?;
    Ok(Html(rendered.composer).into_response())
}

pub(in crate::web::routes) async fn agents_stop_conversation(
    State(state): State<Arc<AppState>>,
    Path((agent_key, conversation_id)): Path<(String, Uuid)>,
) -> Result<Response, AppError> {
    service(&state)
        .stop_conversation(&agent_key, conversation_id)
        .await?;
    Ok(Redirect::to(&format!("/agents/{agent_key}/chat/{conversation_id}")).into_response())
}
pub(in crate::web::routes) async fn agents_compact_conversation(
    State(state): State<Arc<AppState>>,
    Path((agent_key, conversation_id)): Path<(String, Uuid)>,
) -> Result<Response, AppError> {
    service(&state)
        .compact_conversation(&agent_key, conversation_id)
        .await?;
    Ok(Redirect::to(&format!("/agents/{agent_key}/chat/{conversation_id}")).into_response())
}
pub(in crate::web::routes) async fn agents_delete_conversation(
    State(state): State<Arc<AppState>>,
    Path((agent_key, conversation_id)): Path<(String, Uuid)>,
) -> Result<Response, AppError> {
    service(&state)
        .delete_conversation(&agent_key, conversation_id)
        .await?;
    Ok(Redirect::to(&format!("/agents/{agent_key}/chat")).into_response())
}

pub(in crate::web::routes) async fn agents_update_conversation_settings(
    State(state): State<Arc<AppState>>,
    Path((agent_key, conversation_id)): Path<(String, Uuid)>,
    Form(form): Form<ConversationSettingsForm>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let selection = parse_model_selection(&form.model_selection)
        .map_err(anyhow::Error::msg)?
        .ok_or_else(|| AppError(anyhow::anyhow!("Select a model.")))?;
    let selection = validate_model_selection_for_agent(&state, &agent, Some(selection))
        .await
        .map_err(|error| AppError(anyhow::anyhow!(error)))?
        .ok_or_else(|| AppError(anyhow::anyhow!("Select a model.")))?;
    let current = crate::agent_conversations::store::get_agent_conversation(
        &state.db_pool,
        &agent_key,
        conversation_id,
    )
    .await?
    .ok_or_else(|| AppError(anyhow::anyhow!("conversation not found")))?;
    let mut policies = current.tool_policies;
    for policy in &mut policies {
        if policy.tool_group == TOOL_GROUP_ORDERS {
            policy.policy = form.orders_policy.clone();
        } else if policy.tool_group == TOOL_GROUP_MEMORY_WRITES {
            policy.policy = form.memory_writes_policy.clone();
        }
    }
    service(&state)
        .update_conversation_model_and_policies(
            &agent_key,
            conversation_id,
            &selection.0,
            &selection.1,
            &policies,
        )
        .await?;
    Ok(Redirect::to(&format!("/agents/{agent_key}/chat/{conversation_id}")).into_response())
}

pub(in crate::web::routes) async fn agents_reply_to_conversation_permission(
    State(state): State<Arc<AppState>>,
    Path((agent_key, conversation_id, request_id)): Path<(String, Uuid, String)>,
    Form(form): Form<PermissionReplyForm>,
) -> Result<Response, AppError> {
    let reply = match form.reply.as_str() {
        "once" => OpenCodePermissionReply::Once,
        "reject" => OpenCodePermissionReply::Reject,
        _ => return Ok((StatusCode::BAD_REQUEST, "invalid permission reply").into_response()),
    };
    if !service(&state)
        .reply_to_permission(&agent_key, conversation_id, &request_id, reply)
        .await?
    {
        return Ok((
            StatusCode::NOT_FOUND,
            "permission request not found for conversation",
        )
            .into_response());
    }
    Ok(Redirect::to(&format!("/agents/{agent_key}/chat/{conversation_id}")).into_response())
}

pub(in crate::web::routes) async fn agent_conversation_stream(
    State(state): State<Arc<AppState>>,
    Path((agent_key, conversation_id)): Path<(String, Uuid)>,
) -> Result<Response, AppError> {
    let receiver = state.run_detail_events.subscribe();
    let Some(snapshot) = load_snapshot(&state, &agent_key, conversation_id).await? else {
        return Ok((StatusCode::NOT_FOUND, "conversation not found").into_response());
    };
    let initial = render_snapshot(&snapshot, String::new(), None)?.events();
    let shutdown_rx = state.shutdown_rx.clone();
    let stream =
        tokio_stream::iter(initial.into_iter().map(Ok::<Event, Infallible>)).chain(unfold(
            ConversationStreamState {
                state,
                agent_key,
                conversation_id,
                session_id: snapshot.conversation.opencode_session_id,
                receiver,
                shutdown_rx,
                pending: VecDeque::new(),
                interval: tokio::time::interval(Duration::from_secs(3)),
                busy: snapshot.busy,
                last_permissions: signature(&snapshot.permissions),
            },
            next_conversation_event,
        ));
    Ok(Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
        .into_response())
}

struct ConversationStreamState {
    state: Arc<AppState>,
    agent_key: String,
    conversation_id: Uuid,
    session_id: String,
    receiver: broadcast::Receiver<RunDetailDbEvent>,
    shutdown_rx: tokio::sync::watch::Receiver<bool>,
    pending: VecDeque<Event>,
    interval: tokio::time::Interval,
    busy: bool,
    last_permissions: String,
}
async fn next_conversation_event(
    mut stream: ConversationStreamState,
) -> Option<(Result<Event, Infallible>, ConversationStreamState)> {
    loop {
        if let Some(event) = stream.pending.pop_front() {
            return Some((Ok(event), stream));
        }
        tokio::select! {
            changed = stream.shutdown_rx.changed() => {
                if changed.is_err() || *stream.shutdown_rx.borrow() { return None; }
            },
            _ = stream.interval.tick(), if stream.busy => {
                if let Ok(Some(snapshot)) = load_snapshot(&stream.state, &stream.agent_key, stream.conversation_id).await {
                    let next = signature(&snapshot.permissions);
                    let busy_changed = snapshot.busy != stream.busy;
                    let permissions_changed = next != stream.last_permissions;
                    stream.busy = snapshot.busy;
                    stream.last_permissions = next;
                    if busy_changed {
                        if let Ok(rendered) = render_snapshot(&snapshot, String::new(), None) {
                            stream.pending.extend(rendered.events());
                        }
                    } else if permissions_changed
                        && let Ok(rendered) = render_snapshot(&snapshot, String::new(), None)
                    {
                        stream.pending.push_back(Event::default().event("conversation-permissions").data(rendered.permissions));
                    }
                }
            },
            notification = stream.receiver.recv() => {
                let event = match notification {
                    Ok(event) => event,
                    Err(broadcast::error::RecvError::Lagged(_)) => RunDetailDbEvent::Resync,
                    Err(broadcast::error::RecvError::Closed) => return None,
                };
                let matches = match &event {
                    RunDetailDbEvent::Resync => true,
                    RunDetailDbEvent::ConversationChanged { conversation_id } => *conversation_id == stream.conversation_id,
                    RunDetailDbEvent::SessionChanged { session_id } => session_id == &stream.session_id,
                    RunDetailDbEvent::RunChanged { .. } => false,
                };
                if !matches { continue; }
                match load_snapshot(&stream.state, &stream.agent_key, stream.conversation_id).await {
                    Ok(Some(snapshot)) => {
                        stream.session_id = snapshot.conversation.opencode_session_id.clone();
                        stream.busy = snapshot.busy;
                        stream.last_permissions = signature(&snapshot.permissions);
                        match render_snapshot(&snapshot, String::new(), None) {
                            Ok(rendered) => stream.pending.extend(rendered.events()),
                            Err(error) => warn!(conversation_id = %stream.conversation_id, error = ?error, "failed to render conversation SSE snapshot"),
                        }
                    }
                    Ok(None) => return None,
                    Err(error) => warn!(conversation_id = %stream.conversation_id, error = ?error, "failed to refresh conversation SSE snapshot"),
                }
            },
        }
    }
}
fn signature(requests: &[AgentConversationPermissionRequestView]) -> String {
    requests
        .iter()
        .map(|request| format!("{}:{}:{}", request.id, request.permission, request.patterns))
        .collect::<Vec<_>>()
        .join("|")
}
