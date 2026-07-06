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
    agents::store::{update_agent_analysis_prompt, update_agent_trading_prompt},
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
    pub prompt: String,
}
pub(in crate::web::routes) async fn agents_update_analysis_prompt(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
    Form(form): Form<UpdateAgentPromptForm>,
) -> Result<Response, AppError> {
    let updated =
        update_agent_analysis_prompt(&state.db_pool, &agent_key, form.prompt.trim()).await?;

    if !updated {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    }

    Ok(Redirect::to(&format!("/agents/{agent_key}/prompts")).into_response())
}
pub(in crate::web::routes) async fn agents_update_trading_prompt(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
    Form(form): Form<UpdateAgentPromptForm>,
) -> Result<Response, AppError> {
    let updated =
        update_agent_trading_prompt(&state.db_pool, &agent_key, form.prompt.trim()).await?;

    if !updated {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    }

    Ok(Redirect::to(&format!("/agents/{agent_key}/prompts")).into_response())
}
