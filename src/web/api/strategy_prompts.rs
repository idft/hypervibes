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
            AgentStrategyPromptRow, get_agent_strategy_prompt, is_valid_prompt_kind,
            list_agent_strategy_prompts, upsert_agent_strategy_prompt,
        },
    },
    web::AppState,
};

use super::error::ApiError;

#[derive(Debug, serde::Serialize)]
pub(super) struct StrategyPromptResponse {
    prompt_kind: String,
    prompt: String,
    updated_at: DateTime<Utc>,
}

impl From<AgentStrategyPromptRow> for StrategyPromptResponse {
    fn from(row: AgentStrategyPromptRow) -> Self {
        Self {
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

/// `GET /api/v1/strategy-prompts`
pub(super) async fn list_strategy_prompts(
    State(state): State<Arc<AppState>>,
    agent: AuthenticatedAgent,
) -> Result<Json<Vec<StrategyPromptResponse>>, ApiError> {
    super::require_permanent_agent_credential(&agent)?;
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
    super::require_permanent_agent_credential(&agent)?;
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

fn validate_prompt_kind(prompt_kind: &str) -> Result<&str, ApiError> {
    if is_valid_prompt_kind(prompt_kind) {
        Ok(prompt_kind)
    } else {
        Err(ApiError::BadRequest("invalid prompt kind".to_string()))
    }
}
