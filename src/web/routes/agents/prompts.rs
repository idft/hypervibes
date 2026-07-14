use axum::{
    Form,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
};
use serde::Deserialize;
use std::sync::Arc;

use super::show::render_agent_show_page;
use crate::web::error::AppError;
use crate::{
    agents::strategy_prompts::{is_valid_prompt_kind, upsert_agent_strategy_prompt},
    web::{AppState, templates::AgentShowTab},
};
pub(in crate::web::routes) async fn agents_show_prompts(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    render_agent_show_page(
        &state,
        &agent_key,
        AgentShowTab::Prompts,
        None,
        None,
        None,
        None,
    )
    .await
}
#[derive(Debug, Default, Deserialize)]
pub(in crate::web::routes) struct UpdateAgentPromptForm {
    #[serde(default)]
    pub prompt_kind: String,
    #[serde(default)]
    pub prompt: String,
}
pub(in crate::web::routes) async fn agents_update_prompt(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
    Form(form): Form<UpdateAgentPromptForm>,
) -> Result<Response, AppError> {
    let prompt_kind = form.prompt_kind.trim();
    if !is_valid_prompt_kind(prompt_kind) {
        return Ok((StatusCode::BAD_REQUEST, "invalid prompt kind").into_response());
    }

    let updated =
        upsert_agent_strategy_prompt(&state.db_pool, &agent_key, prompt_kind, form.prompt.trim())
            .await?;

    if !updated {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    }

    Ok(Redirect::to(&format!("/agents/{agent_key}/prompts")).into_response())
}
