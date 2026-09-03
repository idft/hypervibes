use axum::{
    Form,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
};
use serde::Deserialize;
use std::sync::Arc;

use super::show::{AgentShowQueries, render_agent_show_page};
use crate::web::error::AppError;
use crate::{
    agents::strategy_prompts::{
        is_valid_prompt_kind, rollback_prompt_revision, upsert_agent_strategy_prompt,
    },
    harness::model::CAPABILITY_PROMPT_REVISION_SUBMIT,
    web::{AppState, auth::AuthenticatedUser, templates::AgentShowTab},
};
pub(in crate::web::routes) async fn agents_show_prompts(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    render_agent_show_page(
        &state,
        &user,
        &agent_key,
        AgentShowTab::Prompts,
        AgentShowQueries::default(),
    )
    .await
}
#[derive(Debug, Deserialize)]
pub(in crate::web::routes) struct RollbackPromptRevisionForm {
    pub revision_id: i64,
}
pub(in crate::web::routes) async fn agents_rollback_prompt_revision(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
    Form(form): Form<RollbackPromptRevisionForm>,
) -> Result<Response, AppError> {
    if !rollback_prompt_revision(&state.db_pool, &agent_key, form.revision_id).await? {
        return Ok((StatusCode::NOT_FOUND, "prompt revision not found").into_response());
    }
    Ok(Redirect::to(&format!("/agents/{agent_key}/prompts")).into_response())
}
#[derive(Debug, Deserialize)]
pub(in crate::web::routes) struct PromptImprovementForm {
    pub enabled: Option<String>,
}
pub(in crate::web::routes) async fn agents_update_prompt_improvement(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
    Form(form): Form<PromptImprovementForm>,
) -> Result<Response, AppError> {
    sqlx::query(
        "UPDATE harness_sub_agents
            SET enabled_capabilities = CASE
                WHEN $2 THEN CASE
                    WHEN enabled_capabilities @> jsonb_build_array($3::text) THEN enabled_capabilities
                    ELSE enabled_capabilities || jsonb_build_array($3::text)
                END
                ELSE enabled_capabilities - $3
            END,
            updated_at = now()
          WHERE agent_key = $1
            AND sub_agent_kind = 'daily_review'",
    )
    .bind(&agent_key)
    .bind(form.enabled.as_deref() == Some("true"))
    .bind(CAPABILITY_PROMPT_REVISION_SUBMIT)
    .execute(&state.db_pool)
    .await?;
    Ok(Redirect::to(&format!("/agents/{agent_key}/prompts")).into_response())
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
