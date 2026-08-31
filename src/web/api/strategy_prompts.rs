use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
};
use chrono::{DateTime, Utc};

use crate::{
    agents::{
        AuthenticatedAgent,
        strategy_prompts::{
            AgentStrategyPromptRow, PromptRevisionChange, get_agent_strategy_prompt,
            is_valid_prompt_kind, list_agent_strategy_prompts, submit_daily_review_revisions,
            upsert_agent_strategy_prompt,
        },
    },
    harness::model::RunApiScope,
    web::AppState,
};

use super::error::ApiError;

#[derive(Debug, serde::Serialize)]
pub(super) struct StrategyPromptResponse {
    revision_id: i64,
    prompt_kind: String,
    prompt: String,
    updated_at: DateTime<Utc>,
}

impl From<AgentStrategyPromptRow> for StrategyPromptResponse {
    fn from(row: AgentStrategyPromptRow) -> Self {
        Self {
            revision_id: row.revision_id,
            prompt_kind: row.prompt_kind,
            prompt: row.prompt,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct UpdateStrategyPromptRequest {
    prompt: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SubmitPromptRevisionRequest {
    rationale: String,
    evidence_memory_ids: Vec<uuid::Uuid>,
    changes: Vec<SubmitPromptRevisionChange>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SubmitPromptRevisionChange {
    prompt_kind: String,
    base_revision_id: i64,
    prompt: String,
}

/// `GET /api/v1/strategy-prompts`
pub(super) async fn list_strategy_prompts(
    State(state): State<Arc<AppState>>,
    agent: AuthenticatedAgent,
) -> Result<Json<Vec<StrategyPromptResponse>>, ApiError> {
    if agent.is_run_credential() {
        super::require_run_api_scope(&agent, RunApiScope::PromptRead)?;
    }
    let prompts = list_agent_strategy_prompts(&state.db_pool, &agent.agent_key)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(
        prompts
            .into_iter()
            .map(StrategyPromptResponse::from)
            .collect(),
    ))
}

/// `GET /api/v1/strategy-prompts/{prompt_kind}`
pub(super) async fn get_strategy_prompt(
    State(state): State<Arc<AppState>>,
    agent: AuthenticatedAgent,
    Path(prompt_kind): Path<String>,
) -> Result<Json<StrategyPromptResponse>, ApiError> {
    if agent.is_run_credential() {
        super::require_run_api_scope(&agent, RunApiScope::PromptRead)?;
    }
    let prompt_kind = validate_prompt_kind(&prompt_kind)?;
    let prompt = get_agent_strategy_prompt(&state.db_pool, &agent.agent_key, prompt_kind)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound("strategy prompt not found"))?;
    Ok(Json(prompt.into()))
}

/// `PUT /api/v1/strategy-prompts/{prompt_kind}`
pub(super) async fn update_strategy_prompt(
    State(state): State<Arc<AppState>>,
    agent: AuthenticatedAgent,
    Path(prompt_kind): Path<String>,
    Json(input): Json<UpdateStrategyPromptRequest>,
) -> Result<Json<StrategyPromptResponse>, ApiError> {
    super::require_permanent_agent_credential(&agent)?;
    let prompt_kind = validate_prompt_kind(&prompt_kind)?;
    upsert_agent_strategy_prompt(
        &state.db_pool,
        &agent.agent_key,
        prompt_kind,
        input.prompt.trim(),
    )
    .await
    .map_err(ApiError::Internal)?;
    let prompt = get_agent_strategy_prompt(&state.db_pool, &agent.agent_key, prompt_kind)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound("strategy prompt not found"))?;
    Ok(Json(prompt.into()))
}

/// `POST /api/v1/strategy-prompts/revisions`
pub(super) async fn submit_prompt_revision(
    State(state): State<Arc<AppState>>,
    agent: AuthenticatedAgent,
    Json(input): Json<SubmitPromptRevisionRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    super::require_run_api_scope(&agent, RunApiScope::PromptRevisionSubmit)?;
    let (source_run_id, _) = agent.run_provenance().ok_or(ApiError::Forbidden(
        "prompt revisions require a run credential",
    ))?;
    let changes = input
        .changes
        .into_iter()
        .map(|change| PromptRevisionChange {
            prompt_kind: change.prompt_kind,
            base_revision_id: change.base_revision_id,
            prompt: change.prompt,
        })
        .collect::<Vec<_>>();
    let batch_id = submit_daily_review_revisions(
        &state.db_pool,
        &agent.agent_key,
        source_run_id,
        &input.rationale,
        &input.evidence_memory_ids,
        &changes,
    )
    .await
    .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    Ok(Json(serde_json::json!({"batch_id": batch_id})))
}

fn validate_prompt_kind(prompt_kind: &str) -> Result<&str, ApiError> {
    if is_valid_prompt_kind(prompt_kind) {
        Ok(prompt_kind)
    } else {
        Err(ApiError::BadRequest("invalid prompt kind".to_string()))
    }
}
