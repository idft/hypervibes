use std::sync::Arc;

use anyhow::Error as AnyhowError;
use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{agents::AuthenticatedAgent, web::AppState};

use super::error::ApiError;

#[derive(Debug, Deserialize, Serialize)]
pub(super) struct CodingReportRequest {
    pub task_id: i64,
    pub schema_version: u32,
    pub outcome: String,
    pub summary: String,
    pub rationale: String,
    pub changed_paths: Vec<String>,
    pub evidence_memory_ids: Vec<String>,
    pub validation_notes: String,
}

async fn owns_coding_task(
    state: &AppState,
    agent_key: &str,
    task_id: i64,
) -> Result<bool, ApiError> {
    let row: (bool,) = sqlx::query_as(
        "SELECT EXISTS (
             SELECT 1 FROM agentic_maintenance_tasks
              WHERE id = $1 AND agent_key = $2 AND task_kind = 'analysis_coding'
         )",
    )
    .bind(task_id)
    .bind(agent_key)
    .fetch_one(&state.db_pool)
    .await
    .map_err(|error| ApiError::Internal(AnyhowError::from(error)))?;
    Ok(row.0)
}

pub(super) async fn submit_coding_report(
    State(state): State<Arc<AppState>>,
    agent: AuthenticatedAgent,
    Json(input): Json<CodingReportRequest>,
) -> Result<Response, ApiError> {
    if !owns_coding_task(&state, &agent.agent_key, input.task_id).await? {
        return Ok((StatusCode::NOT_FOUND, "coding task not found").into_response());
    }
    if input.schema_version != 1
        || !matches!(input.outcome.as_str(), "changed" | "no_change")
        || input.summary.trim().is_empty()
        || input.rationale.trim().is_empty()
        || input.validation_notes.trim().is_empty()
        || input.summary.chars().count() > 4_000
        || input.rationale.chars().count() > 4_000
        || input.validation_notes.chars().count() > 4_000
    {
        return Err(ApiError::Validation("invalid coding report".into()));
    }
    if input.changed_paths.iter().any(|path| {
        path.starts_with('/')
            || path.starts_with("scripts/user/")
            || path.contains("..")
            || path.contains('\0')
    }) {
        return Err(ApiError::Validation("invalid coding report path".into()));
    }
    let report = serde_json::to_value(input)
        .map_err(|error| ApiError::Internal(AnyhowError::from(error)))?;
    state
        .workspace_controller
        .store_report(
            &agent.agent_key,
            report
                .get("task_id")
                .and_then(Value::as_i64)
                .expect("report task_id was validated"),
            report,
        )
        .await
        .map_err(ApiError::Internal)?;
    Ok((StatusCode::CREATED, Json(Value::Bool(true))).into_response())
}
