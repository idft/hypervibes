use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
};
use std::sync::Arc;

use super::shared::urlencode;
use super::show::{AgentSettingsQuery, AgentShowQueries, render_agent_show_page};
use crate::{
    agent_conversations::{
        model::AgentChatPolicyDefaultRow,
        store::{replace_agent_chat_policy_defaults, validate_agent_chat_policy_defaults},
    },
    agents::store::{
        get_agent, replace_agent_analysis_instruments, replace_agent_trading_instruments,
    },
    memory::delete_memories_for_agent,
    web::{AppState, auth::AuthenticatedUser, error::AppError, templates::AgentShowTab},
};
pub(in crate::web::routes) async fn agents_show_settings(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path(agent_key): Path<String>,
    Query(query): Query<AgentSettingsQuery>,
) -> Result<Response, AppError> {
    render_agent_show_page(
        &state,
        &user,
        &agent_key,
        AgentShowTab::Settings,
        AgentShowQueries {
            settings: Some(query),
            ..Default::default()
        },
    )
    .await
}
pub(in crate::web::routes) async fn agents_reset_memories(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let deleted = delete_memories_for_agent(&state.db_pool, &agent.agent_key).await?;
    let notice = if deleted == 0 {
        "No memories were stored for this agent."
    } else {
        "All memories stored for this agent were deleted."
    };
    Ok(Redirect::to(&format!(
        "/agents/{agent_key}/settings?notice={}",
        urlencode(notice)
    ))
    .into_response())
}
pub(in crate::web::routes) async fn agents_update_trading_instruments(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
    axum::Form(form_pairs): axum::Form<Vec<(String, String)>>,
) -> Result<Response, AppError> {
    let instrument_ids: Vec<String> = form_pairs
        .into_iter()
        .filter_map(|(key, value)| (key == "trading_instrument_id").then_some(value))
        .collect();
    let updated =
        replace_agent_trading_instruments(&state.db_pool, &agent_key, &instrument_ids).await?;

    if !updated {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    }

    Ok(Redirect::to(&format!("/agents/{agent_key}/settings")).into_response())
}

pub(in crate::web::routes) async fn agents_update_analysis_instruments(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
    axum::Form(form_pairs): axum::Form<Vec<(String, String)>>,
) -> Result<Response, AppError> {
    let instrument_ids = form_pairs
        .into_iter()
        .filter_map(|(key, value)| (key == "analysis_instrument_id").then_some(value))
        .collect::<Vec<_>>();
    if !replace_agent_analysis_instruments(&state.db_pool, &agent_key, &instrument_ids).await? {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    }
    Ok(Redirect::to(&format!("/agents/{agent_key}/settings")).into_response())
}

pub(in crate::web::routes) async fn agents_update_chat_permission_defaults(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
    axum::Form(form_pairs): axum::Form<Vec<(String, String)>>,
) -> Result<Response, AppError> {
    if get_agent(&state.db_pool, &agent_key).await?.is_none() {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    }
    let mut policies = Vec::with_capacity(form_pairs.len());
    for (name, policy) in form_pairs {
        if name == "csrf_token" {
            continue;
        }
        let Some(tool_group) = name.strip_suffix("_policy") else {
            return Ok((StatusCode::BAD_REQUEST, "unknown permission field").into_response());
        };
        policies.push(AgentChatPolicyDefaultRow {
            tool_group: tool_group.to_string(),
            policy,
        });
    }
    if let Err(error) = validate_agent_chat_policy_defaults(&policies) {
        return Ok((StatusCode::BAD_REQUEST, error.to_string()).into_response());
    }
    if !replace_agent_chat_policy_defaults(&state.db_pool, &agent_key, &policies).await? {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    }
    Ok(Redirect::to(&format!(
        "/agents/{agent_key}/settings?notice={}#chat-permission-defaults",
        urlencode("Chat permission defaults saved. New conversations will use these permissions.")
    ))
    .into_response())
}
