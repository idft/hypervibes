use std::sync::Arc;

use axum::{
    Form,
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
};
use serde::Deserialize;
use tracing::warn;
use uuid::Uuid;

use crate::{
    agents::crypto::encrypt,
    agents::store::get_agent,
    gateway::{
        self, GATEWAY_TYPE_TELEGRAM,
        model::TelegramGatewayConfig,
        store as gateway_store,
        telegram::{build_bot, fetch_bot_username},
    },
    web::{
        AppState, auth::AuthenticatedUser, error::AppError,
        templates::TelegramGatewayPartialTemplate,
    },
};

#[derive(Debug, Default, Deserialize)]
pub(in crate::web::routes) struct AddTelegramTokenForm {
    #[serde(default)]
    pub bot_token: String,
}

/// `POST /agents/{agent_key}/settings/gateway/telegram/token`
pub(in crate::web::routes) async fn add_telegram_token(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path(agent_key): Path<String>,
    Form(form): Form<AddTelegramTokenForm>,
) -> Result<Response, AppError> {
    let _ = user;
    let token = form.bot_token.trim();
    if token.is_empty() {
        return Ok((StatusCode::BAD_REQUEST, "bot token must not be empty").into_response());
    }
    if let Some(row) =
        gateway_store::get_gateway(&state.db_pool, &agent_key, GATEWAY_TYPE_TELEGRAM).await?
    {
        let config = TelegramGatewayConfig::from_value(&row.config);
        if config.is_ready() {
            return Ok((
                StatusCode::CONFLICT,
                "disconnect Telegram before adding a new bot token",
            )
                .into_response());
        }
    }
    // Validate before changing the stored config. Persisting a rejected token
    // would leave the previous bot username visible while polling fails.
    let bot_username = match fetch_bot_username(&build_bot(token)).await {
        Ok(bot_username) => bot_username,
        Err(error) => {
            warn!(agent_key = %agent_key, error = ?error, "telegram getMe failed during token add");
            return Ok((StatusCode::BAD_REQUEST, "Telegram rejected the bot token").into_response());
        }
    };

    let ciphertext = encrypt(&state.encryption_key, token)?;
    let mut config = match gateway_store::get_gateway(
        &state.db_pool,
        &agent_key,
        GATEWAY_TYPE_TELEGRAM,
    )
    .await?
    {
        Some(row) => TelegramGatewayConfig::from_value(&row.config),
        None => TelegramGatewayConfig::default(),
    };
    config.bot_token_ciphertext = Some(ciphertext);
    config.bot_token_key_id = Some(state.encryption_key.key_id.clone());

    config.bot_username = Some(bot_username);

    gateway_store::upsert_telegram_config(&state.db_pool, &agent_key, &config).await?;
    Ok(Redirect::to(&format!("/agents/{agent_key}/settings")).into_response())
}

/// `GET /agents/{agent_key}/settings/gateway/telegram/status`
pub(in crate::web::routes) async fn telegram_gateway_status(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let gateway_telegram =
        load_telegram_gateway_view(&state, &agent.agent_key, user.id, None).await;
    let html = TelegramGatewayPartialTemplate::render_view(agent, gateway_telegram)?;
    Ok(Html(html).into_response())
}

/// `POST /agents/{agent_key}/settings/gateway/telegram/link`
pub(in crate::web::routes) async fn start_telegram_link(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    let gateway =
        gateway_store::get_gateway(&state.db_pool, &agent_key, GATEWAY_TYPE_TELEGRAM).await?;
    let Some(gateway) = gateway else {
        return Ok((
            StatusCode::BAD_REQUEST,
            "telegram gateway is not configured",
        )
            .into_response());
    };
    let config = TelegramGatewayConfig::from_value(&gateway.config);
    if !config.is_ready() || config.bot_username.is_none() {
        return Ok((
            StatusCode::BAD_REQUEST,
            "telegram bot token is not configured",
        )
            .into_response());
    }
    let Some(pending_links) = state.gateway_pending_links.clone() else {
        return Ok((
            StatusCode::SERVICE_UNAVAILABLE,
            "gateway service is not running",
        )
            .into_response());
    };
    let token = pending_links
        .iter()
        .find(|entry| {
            entry.agent_key == agent_key && entry.user_id == user.id && !entry.is_expired()
        })
        .map(|entry| *entry.key());
    let token = match token {
        Some(token) => token,
        None => {
            let prior_tokens: Vec<Uuid> = pending_links
                .iter()
                .filter(|entry| entry.agent_key == agent_key && entry.user_id == user.id)
                .map(|entry| *entry.key())
                .collect();
            for token in prior_tokens {
                pending_links.remove(&token);
            }
            let link = gateway::model::PendingLink::new(agent_key.clone(), user.id);
            let token = link.token;
            pending_links.insert(token, link);
            token
        }
    };
    let bot_username = config
        .bot_username
        .as_deref()
        .expect("validated Telegram bot username");
    Ok(Redirect::to(&format!("https://t.me/{bot_username}?start={token}")).into_response())
}

/// `POST /agents/{agent_key}/settings/gateway/telegram/link/{token}/confirm`
pub(in crate::web::routes) async fn confirm_telegram_link(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path((agent_key, token)): Path<(String, Uuid)>,
) -> Result<Response, AppError> {
    let Some(pending_links) = state.gateway_pending_links.clone() else {
        return Ok((
            StatusCode::SERVICE_UNAVAILABLE,
            "gateway service is not running",
        )
            .into_response());
    };
    let entry = pending_links.get(&token);
    let Some(link) = entry.as_ref() else {
        return Ok((StatusCode::NOT_FOUND, "link token not found or has expired").into_response());
    };
    if link.agent_key != agent_key {
        return Ok((StatusCode::NOT_FOUND, "link token not found or has expired").into_response());
    }
    if link.user_id != user.id {
        return Ok((StatusCode::FORBIDDEN, "link token belongs to another user").into_response());
    }
    if link.is_expired() {
        let _ = pending_links.remove(&token);
        return Ok((StatusCode::GONE, "link token has expired").into_response());
    }
    let chat_id = match link.chat_id {
        Some(chat_id) => chat_id,
        None => {
            return Ok((
                StatusCode::CONFLICT,
                "telegram chat has not yet sent the /start command",
            )
                .into_response());
        }
    };
    let chat_username = link.chat_username.clone();
    drop(entry);
    gateway_store::set_telegram_chat(
        &state.db_pool,
        &agent_key,
        chat_id,
        chat_username.as_deref(),
    )
    .await?;
    let _ = pending_links.remove(&token);
    Ok(Redirect::to(&format!("/agents/{agent_key}/settings")).into_response())
}

/// `POST /agents/{agent_key}/settings/gateway/telegram/link/{token}/reject`
pub(in crate::web::routes) async fn reject_telegram_link(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path((agent_key, token)): Path<(String, Uuid)>,
) -> Result<Response, AppError> {
    let _ = user;
    if let Some(pending_links) = state.gateway_pending_links.clone() {
        let _ = pending_links.remove(&token);
    }
    Ok(Redirect::to(&format!("/agents/{agent_key}/settings")).into_response())
}

/// `POST /agents/{agent_key}/settings/gateway/telegram/disconnect`
pub(in crate::web::routes) async fn disconnect_telegram_gateway(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    let _ = user;
    if let Some(pending_links) = state.gateway_pending_links.clone() {
        let tokens: Vec<Uuid> = pending_links
            .iter()
            .filter(|entry| entry.agent_key == agent_key)
            .map(|entry| *entry.key())
            .collect();
        for token in tokens {
            pending_links.remove(&token);
        }
    }
    gateway_store::disconnect_telegram_gateway(&state.db_pool, &agent_key).await?;
    Ok(Redirect::to(&format!("/agents/{agent_key}/settings")).into_response())
}

/// Render the Telegram gateway section for the agent settings page. Called
/// from `render_agent_show_page` when the Settings tab is active.
pub(in crate::web::routes) async fn load_telegram_gateway_view(
    state: &Arc<AppState>,
    agent_key: &str,
    user_id: Uuid,
    requested_token: Option<Uuid>,
) -> gateway::GatewayTelegramView {
    let row =
        match gateway_store::get_gateway(&state.db_pool, agent_key, GATEWAY_TYPE_TELEGRAM).await {
            Ok(row) => row,
            Err(error) => {
                warn!(agent_key = agent_key, error = ?error, "failed to load telegram gateway row");
                return gateway::GatewayTelegramView::empty();
            }
        };
    let pending_token = state.gateway_pending_links.as_ref().and_then(|links| {
        requested_token
            .filter(|token| {
                links.get(token).is_some_and(|entry| {
                    entry.agent_key == agent_key && entry.user_id == user_id && !entry.is_expired()
                })
            })
            .or_else(|| {
                links
                    .iter()
                    .find(|entry| {
                        entry.agent_key == agent_key
                            && entry.user_id == user_id
                            && !entry.is_expired()
                    })
                    .map(|entry| *entry.key())
            })
    });
    match row {
        Some(row) => gateway::GatewayTelegramView::from_row(&row, pending_token),
        None => gateway::GatewayTelegramView::empty(),
    }
}
