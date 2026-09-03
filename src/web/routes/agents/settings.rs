use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
};
use std::sync::Arc;

use super::shared::urlencode;
use super::show::{AgentSettingsQuery, AgentShowQueries, render_agent_show_page};
use crate::{
    agents::store::{get_agent, replace_agent_instruments},
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
pub(in crate::web::routes) async fn agents_update_instruments(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
    axum::Form(form_pairs): axum::Form<Vec<(String, String)>>,
) -> Result<Response, AppError> {
    let instrument_ids: Vec<String> = form_pairs
        .into_iter()
        .filter_map(|(key, value)| (key == "instrument_id").then_some(value))
        .collect();
    let updated = replace_agent_instruments(&state.db_pool, &agent_key, &instrument_ids).await?;

    if !updated {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    }

    Ok(Redirect::to(&format!("/agents/{agent_key}/settings")).into_response())
}
