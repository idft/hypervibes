use anyhow::{Context, Result};
use serde_json::json;
use sqlx::{Error as SqlxError, Postgres, Transaction, query_as};

use crate::{
    agentic::model::{
        AgentMaintenanceTaskRow, MAINTENANCE_STATUS_FAILED, MAINTENANCE_STATUS_QUEUED,
        MAINTENANCE_STATUS_RUNNING, MAINTENANCE_STATUS_SUCCEEDED,
        MAINTENANCE_TASK_KIND_WORKSPACE_REGENERATE,
    },
    db::DbPool,
};

use super::common::truncate_error_summary;

const ACTIVE_MAINTENANCE_STATUSES: [&str; 2] =
    [MAINTENANCE_STATUS_QUEUED, MAINTENANCE_STATUS_RUNNING];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InsertWorkspaceMaintenanceTaskOutcome {
    Inserted { task_id: i64 },
    DuplicateActiveTask,
}

pub async fn insert_workspace_regenerate_task(
    pool: &DbPool,
    agent_key: &str,
    hard_reset: bool,
    reset_memories: bool,
) -> Result<InsertWorkspaceMaintenanceTaskOutcome> {
    let parameters = json!({
        "hard_reset": hard_reset,
        "reset_memories": hard_reset && reset_memories,
    });
    let row: Result<(i64,), SqlxError> = query_as(
        "INSERT INTO agentic_maintenance_tasks (
            agent_key,
            task_kind,
            parameters,
            status
         ) VALUES ($1, $2, $3, $4)
         RETURNING id",
    )
    .bind(agent_key)
    .bind(MAINTENANCE_TASK_KIND_WORKSPACE_REGENERATE)
    .bind(parameters)
    .bind(MAINTENANCE_STATUS_QUEUED)
    .fetch_one(pool)
    .await;

    match row {
        Ok((task_id,)) => Ok(InsertWorkspaceMaintenanceTaskOutcome::Inserted { task_id }),
        Err(SqlxError::Database(db_err)) if db_err.is_unique_violation() => {
            Ok(InsertWorkspaceMaintenanceTaskOutcome::DuplicateActiveTask)
        }
        Err(error) => Err(error).with_context(|| {
            format!("failed to insert workspace regenerate task for agent {agent_key}")
        }),
    }
}

pub async fn get_latest_workspace_regenerate_task(
    pool: &DbPool,
    agent_key: &str,
) -> Result<Option<AgentMaintenanceTaskRow>> {
    let row = query_as::<_, AgentMaintenanceTaskRow>(
        "SELECT id,
                agent_key,
                task_kind,
                parameters,
                status,
                error_summary,
                created_at,
                updated_at,
                started_at,
                finished_at
           FROM agentic_maintenance_tasks
          WHERE agent_key = $1
            AND task_kind = $2
          ORDER BY created_at DESC, id DESC
          LIMIT 1",
    )
    .bind(agent_key)
    .bind(MAINTENANCE_TASK_KIND_WORKSPACE_REGENERATE)
    .fetch_optional(pool)
    .await
    .with_context(|| format!("failed to load latest workspace maintenance task for {agent_key}"))?;

    Ok(row)
}

pub async fn get_next_queued_workspace_regenerate_task(
    pool: &DbPool,
) -> Result<Option<AgentMaintenanceTaskRow>> {
    let row = query_as::<_, AgentMaintenanceTaskRow>(
        "SELECT id,
                agent_key,
                task_kind,
                parameters,
                status,
                error_summary,
                created_at,
                updated_at,
                started_at,
                finished_at
           FROM agentic_maintenance_tasks
          WHERE task_kind = $1
            AND status = $2
          ORDER BY created_at ASC, id ASC
          LIMIT 1",
    )
    .bind(MAINTENANCE_TASK_KIND_WORKSPACE_REGENERATE)
    .bind(MAINTENANCE_STATUS_QUEUED)
    .fetch_optional(pool)
    .await
    .context("failed to load next queued workspace maintenance task")?;

    Ok(row)
}

pub async fn mark_maintenance_task_running(pool: &DbPool, task_id: i64) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE agentic_maintenance_tasks
            SET status = $2,
                started_at = COALESCE(started_at, now()),
                updated_at = now()
          WHERE id = $1
            AND status = $3",
    )
    .bind(task_id)
    .bind(MAINTENANCE_STATUS_RUNNING)
    .bind(MAINTENANCE_STATUS_QUEUED)
    .execute(pool)
    .await
    .with_context(|| format!("failed to mark maintenance task {task_id} running"))?;

    Ok(result.rows_affected() > 0)
}

pub async fn mark_maintenance_task_succeeded(pool: &DbPool, task_id: i64) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE agentic_maintenance_tasks
            SET status = $2,
                finished_at = now(),
                error_summary = NULL,
                updated_at = now()
          WHERE id = $1",
    )
    .bind(task_id)
    .bind(MAINTENANCE_STATUS_SUCCEEDED)
    .execute(pool)
    .await
    .with_context(|| format!("failed to mark maintenance task {task_id} succeeded"))?;

    Ok(result.rows_affected() > 0)
}

pub async fn mark_maintenance_task_failed(
    pool: &DbPool,
    task_id: i64,
    error_summary: &str,
) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE agentic_maintenance_tasks
            SET status = $2,
                finished_at = now(),
                error_summary = $3,
                updated_at = now()
          WHERE id = $1",
    )
    .bind(task_id)
    .bind(MAINTENANCE_STATUS_FAILED)
    .bind(truncate_error_summary(error_summary))
    .execute(pool)
    .await
    .with_context(|| format!("failed to mark maintenance task {task_id} failed"))?;

    Ok(result.rows_affected() > 0)
}

pub(crate) async fn agent_has_blocking_workspace_maintenance_tx(
    tx: &mut Transaction<'_, Postgres>,
    agent_key: &str,
) -> Result<bool> {
    let row: Option<(i32,)> = query_as(
        "SELECT 1
           FROM agentic_maintenance_tasks
          WHERE agent_key = $1
            AND task_kind = $2
            AND status = ANY($3)
          LIMIT 1",
    )
    .bind(agent_key)
    .bind(MAINTENANCE_TASK_KIND_WORKSPACE_REGENERATE)
    .bind(ACTIVE_MAINTENANCE_STATUSES)
    .fetch_optional(&mut **tx)
    .await
    .with_context(|| format!("failed to check workspace maintenance state for {agent_key}"))?;

    Ok(row.is_some())
}

#[cfg(test)]
pub async fn agent_has_blocking_workspace_maintenance(
    pool: &DbPool,
    agent_key: &str,
) -> Result<bool> {
    let mut tx = pool
        .begin()
        .await
        .context("failed to begin maintenance state transaction")?;
    let blocked = agent_has_blocking_workspace_maintenance_tx(&mut tx, agent_key).await?;
    tx.commit()
        .await
        .context("failed to commit maintenance state transaction")?;
    Ok(blocked)
}
