use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sqlx::{Error as SqlxError, query_as};

use crate::{
    db::DbPool,
    harness::model::{
        GlobalMaintenanceTaskRow, MAINTENANCE_PHASE_COMPLETED, MAINTENANCE_STATUS_FAILED,
        MAINTENANCE_STATUS_QUEUED, MAINTENANCE_STATUS_RUNNING, MAINTENANCE_STATUS_SUCCEEDED,
        MAINTENANCE_TASK_KIND_PROVIDER_CONFIG_RELOAD,
    },
};

use super::common::truncate_error_summary;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InsertGlobalMaintenanceTaskOutcome {
    Inserted { task_id: i64 },
    DuplicateActiveTask,
}

pub async fn insert_provider_config_reload_task(
    pool: &DbPool,
) -> Result<InsertGlobalMaintenanceTaskOutcome> {
    let row: Result<(i64,), SqlxError> = query_as(
        "INSERT INTO harness_maintenance_tasks (agent_key, task_kind, parameters, status)
         VALUES (NULL, $1, '{}'::jsonb, $2)
         RETURNING id",
    )
    .bind(MAINTENANCE_TASK_KIND_PROVIDER_CONFIG_RELOAD)
    .bind(MAINTENANCE_STATUS_QUEUED)
    .fetch_one(pool)
    .await;
    match row {
        Ok((task_id,)) => Ok(InsertGlobalMaintenanceTaskOutcome::Inserted { task_id }),
        Err(SqlxError::Database(db_err)) if db_err.is_unique_violation() => {
            Ok(InsertGlobalMaintenanceTaskOutcome::DuplicateActiveTask)
        }
        Err(error) => Err(error).context("failed to insert provider config reload task"),
    }
}

pub async fn get_latest_provider_config_reload_task(
    pool: &DbPool,
) -> Result<Option<GlobalMaintenanceTaskRow>> {
    query_as::<_, GlobalMaintenanceTaskRow>(
        "SELECT id, status, error_summary
           FROM harness_maintenance_tasks
          WHERE task_kind = $1
          ORDER BY created_at DESC, id DESC
          LIMIT 1",
    )
    .bind(MAINTENANCE_TASK_KIND_PROVIDER_CONFIG_RELOAD)
    .fetch_optional(pool)
    .await
    .context("failed to load latest provider config reload task")
}

pub async fn get_next_queued_provider_config_reload_task(
    pool: &DbPool,
) -> Result<Option<GlobalMaintenanceTaskRow>> {
    query_as::<_, GlobalMaintenanceTaskRow>(
        "SELECT id, status, error_summary
           FROM harness_maintenance_tasks
          WHERE task_kind = $1
            AND status = $2
          ORDER BY created_at ASC, id ASC
          LIMIT 1",
    )
    .bind(MAINTENANCE_TASK_KIND_PROVIDER_CONFIG_RELOAD)
    .bind(MAINTENANCE_STATUS_QUEUED)
    .fetch_optional(pool)
    .await
    .context("failed to load next queued provider config reload task")
}

/// Return stale global reload work to the queue after a scheduler process
/// exits while a dispose request is in flight. Disposing twice is safe, while
/// leaving the task running would prevent every later reload from being queued.
pub async fn requeue_stale_provider_config_reload_tasks(
    pool: &DbPool,
    before: DateTime<Utc>,
) -> Result<u64> {
    let result = sqlx::query(
        "UPDATE harness_maintenance_tasks
            SET status = $2,
                phase = $2,
                heartbeat_at = NULL,
                updated_at = now()
          WHERE task_kind = $1
            AND status = $3
            AND COALESCE(heartbeat_at, started_at, updated_at) < $4",
    )
    .bind(MAINTENANCE_TASK_KIND_PROVIDER_CONFIG_RELOAD)
    .bind(MAINTENANCE_STATUS_QUEUED)
    .bind(MAINTENANCE_STATUS_RUNNING)
    .bind(before)
    .execute(pool)
    .await
    .context("failed to requeue stale provider config reload tasks")?;

    Ok(result.rows_affected())
}

pub async fn mark_maintenance_task_running(pool: &DbPool, task_id: i64) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE harness_maintenance_tasks
            SET status = $2,
                phase = phase,
                attempt_count = attempt_count + 1,
                heartbeat_at = now(),
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
        "UPDATE harness_maintenance_tasks
            SET status = $2,
                phase = $3,
                finished_at = now(),
                error_summary = NULL,
                updated_at = now()
          WHERE id = $1",
    )
    .bind(task_id)
    .bind(MAINTENANCE_STATUS_SUCCEEDED)
    .bind(MAINTENANCE_PHASE_COMPLETED)
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
        "UPDATE harness_maintenance_tasks
             SET status = $2,
                 phase = $3,
                 finished_at = now(),
                 error_summary = $4,
                 updated_at = now()
          WHERE id = $1",
    )
    .bind(task_id)
    .bind(MAINTENANCE_STATUS_FAILED)
    .bind(MAINTENANCE_PHASE_COMPLETED)
    .bind(truncate_error_summary(error_summary))
    .execute(pool)
    .await
    .with_context(|| format!("failed to mark maintenance task {task_id} failed"))?;

    Ok(result.rows_affected() > 0)
}
