use std::{fs, sync::Arc};

use anyhow::Error as AnyhowError;
use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    agents::AuthenticatedAgent, opencode::coding_workspace::candidate_root, web::AppState,
};

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
        return Err(ApiError::Validation(
            "invalid coding report path".into(),
        ));
    }
    let candidate = candidate_root(
        &state.opencode_workspace_config,
        &agent.agent_key,
        input.task_id,
    )
    .map_err(ApiError::Internal)?;
    let report_path = candidate
        .parent()
        .ok_or_else(|| ApiError::Internal(anyhow::anyhow!("invalid coding task path")))?
        .join("coding-report.json");
    let temporary = report_path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(&input)
        .map_err(|error| ApiError::Internal(AnyhowError::from(error)))?;
    fs::create_dir_all(report_path.parent().expect("report parent"))
        .map_err(|error| ApiError::Internal(AnyhowError::from(error)))?;
    fs::write(&temporary, bytes).map_err(|error| ApiError::Internal(AnyhowError::from(error)))?;
    fs::rename(&temporary, &report_path)
        .map_err(|error| ApiError::Internal(AnyhowError::from(error)))?;
    Ok((StatusCode::CREATED, Json(Value::Bool(true))).into_response())
}
