use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::json;
use sqlx::{Error as SqlxError, Postgres, Transaction, query_as};
use uuid::Uuid;

use crate::{
    agentic::model::{
        AgentMaintenanceTaskRow, CODING_PROMOTION_PHASES, GlobalMaintenanceTaskRow,
        MAINTENANCE_PHASE_COMPLETED, MAINTENANCE_STATUS_FAILED, MAINTENANCE_STATUS_QUEUED,
        MAINTENANCE_STATUS_RUNNING, MAINTENANCE_STATUS_SUCCEEDED,
        MAINTENANCE_TASK_KIND_ANALYSIS_CODING, MAINTENANCE_TASK_KIND_PROVIDER_CONFIG_RELOAD,
        MAINTENANCE_TASK_KIND_WORKSPACE_REGENERATE, RUN_STATUS_QUEUED,
    },
    db::DbPool,
};

use super::common::{
    HookRunInsertMode, insert_run_with_model_variant_in_tx, lock_agent_coordination_tx,
    truncate_error_summary,
};

const ACTIVE_MAINTENANCE_STATUSES: [&str; 2] =
    [MAINTENANCE_STATUS_QUEUED, MAINTENANCE_STATUS_RUNNING];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InsertWorkspaceMaintenanceTaskOutcome {
    Inserted { task_id: i64 },
    DuplicateActiveTask,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InsertAnalysisCodingTaskOutcome {
    Inserted {
        task_id: i64,
        run_id: i64,
        scheduled_for: DateTime<Utc>,
    },
    AlreadyQueued,
    BlockedByMaintenance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodingTriggerMode {
    Manual,
    Automatic,
}

impl CodingTriggerMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Automatic => "automatic",
        }
    }
}

pub struct AnalysisCodingTaskRequest<'a> {
    pub agent_key: &'a str,
    pub hook_id: i64,
    pub trigger_mode: CodingTriggerMode,
    pub source_run_id: Option<i64>,
    pub source_memory_id: Option<Uuid>,
    pub operator_prompt: Option<&'a str>,
    pub requested_mode: Option<&'a str>,
}

type CodingHookRow = (
    i64,
    String,
    String,
    String,
    bool,
    Option<String>,
    Option<String>,
    Option<String>,
    i32,
    String,
);

/// Queue an coding run and its durable maintenance task atomically.
/// The agent row is locked before the hook row and all active-task checks.
pub async fn insert_analysis_coding_task_and_run(
    pool: &DbPool,
    request: AnalysisCodingTaskRequest<'_>,
) -> Result<InsertAnalysisCodingTaskOutcome> {
    let AnalysisCodingTaskRequest {
        agent_key,
        hook_id,
        trigger_mode,
        source_run_id,
        source_memory_id,
        operator_prompt,
        requested_mode,
    } = request;
    let mut tx = pool
        .begin()
        .await
        .context("failed to begin coding queue transaction")?;
    lock_agent_coordination_tx(&mut tx, agent_key).await?;

    let hook: Option<CodingHookRow> = query_as(
        "SELECT id, agent_key, job_key, job_kind, enabled,
                    model_provider_id, model_id, model_variant, timeout_seconds, operator_prompt
               FROM agentic_job_hooks
              WHERE agent_key = $1 AND id = $2
              FOR UPDATE",
    )
    .bind(agent_key)
    .bind(hook_id)
    .fetch_optional(&mut *tx)
    .await
    .context("failed to lock coding hook")?;
    let Some((
        hook_id,
        hook_agent_key,
        job_key,
        job_kind,
        enabled,
        provider,
        model,
        model_variant,
        timeout_seconds,
        hook_prompt,
    )) = hook
    else {
        anyhow::bail!("coding hook not found")
    };
    if hook_agent_key != agent_key || job_kind != "analysis_coding" {
        anyhow::bail!("hook is not an analysis coding hook")
    }
    if trigger_mode == CodingTriggerMode::Automatic && !enabled {
        anyhow::bail!("analysis coding hook is disabled")
    }
    if let Some(mode) = requested_mode
        && !matches!(mode, "auto" | "bootstrap" | "manual_improvement")
    {
        anyhow::bail!("invalid analysis coding mode")
    }
    let (Some(provider), Some(model)) = (provider, model) else {
        anyhow::bail!("analysis coding requires an explicit provider and model")
    };

    if let Some(source_memory_id) = source_memory_id {
        let duplicate: Option<(i64,)> = query_as(
            "SELECT id FROM agentic_maintenance_tasks
               WHERE task_kind = $1 AND source_memory_id = $2
               LIMIT 1",
        )
        .bind(MAINTENANCE_TASK_KIND_ANALYSIS_CODING)
        .bind(source_memory_id)
        .fetch_optional(&mut *tx)
        .await
        .context("failed to check duplicate coding trigger")?;
        if duplicate.is_some() {
            tx.rollback()
                .await
                .context("failed to roll back duplicate coding trigger")?;
            return Ok(InsertAnalysisCodingTaskOutcome::AlreadyQueued);
        }
    }

    if agent_has_blocking_workspace_maintenance_for_mode_tx(
        &mut tx,
        agent_key,
        HookRunInsertMode::CodingTrigger,
    )
    .await?
    {
        tx.rollback()
            .await
            .context("failed to roll back blocked coding task")?;
        return Ok(InsertAnalysisCodingTaskOutcome::BlockedByMaintenance);
    }

    let now = Utc::now();
    let run_id = insert_run_with_model_variant_in_tx(
        &mut tx,
        None,
        Some(hook_id),
        agent_key,
        &job_key,
        &job_kind,
        None,
        RUN_STATUS_QUEUED,
        None,
        Some(&provider),
        Some(&model),
        model_variant.as_deref(),
        now,
        None,
        None,
        timeout_seconds,
        None,
    )
    .await?;
    let parameters = json!({
        "trigger_mode": trigger_mode.as_str(),
        "mode": requested_mode.unwrap_or("auto"),
        "hook_id": hook_id,
        "operator_prompt": operator_prompt.unwrap_or(""),
        "hook_prompt": hook_prompt,
        "timeout_seconds": timeout_seconds,
        "model_provider_id": provider,
        "model_id": model,
        "model_variant": model_variant,
    });
    let task: (i64,) = query_as(
        "INSERT INTO agentic_maintenance_tasks
             (agent_key, task_kind, parameters, status, phase, run_id,
              source_run_id, source_memory_id)
          VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
          RETURNING id",
    )
    .bind(agent_key)
    .bind(MAINTENANCE_TASK_KIND_ANALYSIS_CODING)
    .bind(parameters)
    .bind(MAINTENANCE_STATUS_QUEUED)
    .bind(crate::agentic::model::MAINTENANCE_PHASE_QUEUED)
    .bind(run_id)
    .bind(source_run_id)
    .bind(source_memory_id)
    .fetch_one(&mut *tx)
    .await
    .context("failed to insert coding maintenance task")?;
    tx.commit()
        .await
        .context("failed to commit coding queue transaction")?;
    Ok(InsertAnalysisCodingTaskOutcome::Inserted {
        task_id: task.0,
        run_id,
        scheduled_for: now,
    })
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
    let mut tx = pool
        .begin()
        .await
        .context("failed to begin workspace maintenance insert")?;
    lock_agent_coordination_tx(&mut tx, agent_key).await?;
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
    .fetch_one(&mut *tx)
    .await;

    match row {
        Ok((task_id,)) => {
            tx.commit()
                .await
                .context("failed to commit workspace maintenance insert")?;
            Ok(InsertWorkspaceMaintenanceTaskOutcome::Inserted { task_id })
        }
        Err(SqlxError::Database(db_err)) if db_err.is_unique_violation() => {
            tx.rollback()
                .await
                .context("failed to roll back duplicate workspace maintenance insert")?;
            Ok(InsertWorkspaceMaintenanceTaskOutcome::DuplicateActiveTask)
        }
        Err(error) => {
            tx.rollback().await.ok();
            Err(error).with_context(|| {
                format!("failed to insert workspace regenerate task for agent {agent_key}")
            })
        }
    }
}

#[cfg(test)]
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
                phase,
                error_summary,
                run_id,
                source_run_id,
                source_memory_id,
                heartbeat_at,
                attempt_count,
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

pub async fn get_latest_maintenance_task(
    pool: &DbPool,
    agent_key: &str,
) -> Result<Option<AgentMaintenanceTaskRow>> {
    query_as::<_, AgentMaintenanceTaskRow>(
        "SELECT id, agent_key, task_kind, parameters, status, phase,
                error_summary, run_id, source_run_id, source_memory_id,
                heartbeat_at, attempt_count, created_at, updated_at,
                started_at, finished_at
           FROM agentic_maintenance_tasks
          WHERE agent_key = $1
          ORDER BY created_at DESC, id DESC
          LIMIT 1",
    )
    .bind(agent_key)
    .fetch_optional(pool)
    .await
    .with_context(|| format!("failed to load latest maintenance task for {agent_key}"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InsertGlobalMaintenanceTaskOutcome {
    Inserted { task_id: i64 },
    DuplicateActiveTask,
}

pub async fn insert_provider_config_reload_task(
    pool: &DbPool,
) -> Result<InsertGlobalMaintenanceTaskOutcome> {
    let row: Result<(i64,), SqlxError> = query_as(
        "INSERT INTO agentic_maintenance_tasks (agent_key, task_kind, parameters, status)
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
           FROM agentic_maintenance_tasks
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
           FROM agentic_maintenance_tasks
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
        "UPDATE agentic_maintenance_tasks
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

pub async fn get_next_queued_workspace_regenerate_task(
    pool: &DbPool,
) -> Result<Option<AgentMaintenanceTaskRow>> {
    let row = query_as::<_, AgentMaintenanceTaskRow>(
        "SELECT id,
                agent_key,
                task_kind,
                parameters,
                status,
                phase,
                error_summary,
                run_id,
                source_run_id,
                source_memory_id,
                heartbeat_at,
                attempt_count,
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

pub async fn list_queued_maintenance_candidates(
    pool: &DbPool,
    limit: i64,
) -> Result<Vec<AgentMaintenanceTaskRow>> {
    query_as::<_, AgentMaintenanceTaskRow>(
        "SELECT id, agent_key, task_kind, parameters, status, phase,
                error_summary, run_id, source_run_id, source_memory_id,
                heartbeat_at, attempt_count, created_at, updated_at,
                started_at, finished_at
           FROM agentic_maintenance_tasks
          WHERE status = $1
            AND agent_key IS NOT NULL
          ORDER BY created_at ASC, id ASC
          LIMIT $2",
    )
    .bind(MAINTENANCE_STATUS_QUEUED)
    .bind(limit)
    .fetch_all(pool)
    .await
    .context("failed to list queued maintenance candidates")
}

pub async fn mark_maintenance_task_running(pool: &DbPool, task_id: i64) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE agentic_maintenance_tasks
            SET status = $2,
                phase = CASE WHEN task_kind = $4 THEN 'preparing' ELSE phase END,
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
    .bind(MAINTENANCE_TASK_KIND_ANALYSIS_CODING)
    .execute(pool)
    .await
    .with_context(|| format!("failed to mark maintenance task {task_id} running"))?;

    Ok(result.rows_affected() > 0)
}

pub async fn compare_and_set_maintenance_phase(
    pool: &DbPool,
    task_id: i64,
    expected_phase: &str,
    next_phase: &str,
) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE agentic_maintenance_tasks
            SET phase = $3, updated_at = now(), heartbeat_at = now()
          WHERE id = $1 AND phase = $2",
    )
    .bind(task_id)
    .bind(expected_phase)
    .bind(next_phase)
    .execute(pool)
    .await
    .with_context(|| format!("failed to advance maintenance task {task_id} phase"))?;
    Ok(result.rows_affected() > 0)
}

pub async fn heartbeat_maintenance_task(pool: &DbPool, task_id: i64) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE agentic_maintenance_tasks
            SET heartbeat_at = now(), updated_at = now()
          WHERE id = $1 AND status = $2",
    )
    .bind(task_id)
    .bind(MAINTENANCE_STATUS_RUNNING)
    .execute(pool)
    .await
    .with_context(|| format!("failed to heartbeat maintenance task {task_id}"))?;
    Ok(result.rows_affected() > 0)
}

pub async fn list_stale_running_maintenance_tasks(
    pool: &DbPool,
    before: DateTime<Utc>,
    limit: i64,
) -> Result<Vec<AgentMaintenanceTaskRow>> {
    query_as::<_, AgentMaintenanceTaskRow>(
        "SELECT id, agent_key, task_kind, parameters, status, phase,
                error_summary, run_id, source_run_id, source_memory_id,
                heartbeat_at, attempt_count, created_at, updated_at,
                started_at, finished_at
           FROM agentic_maintenance_tasks
          WHERE status = $1
            AND agent_key IS NOT NULL
            AND COALESCE(heartbeat_at, started_at, updated_at) < $2
          ORDER BY updated_at ASC, id ASC
          LIMIT $3",
    )
    .bind(MAINTENANCE_STATUS_RUNNING)
    .bind(before)
    .bind(limit)
    .fetch_all(pool)
    .await
    .context("failed to list stale maintenance tasks")
}

pub async fn agent_has_active_live_runs(pool: &DbPool, agent_key: &str) -> Result<bool> {
    let row: (bool,) = query_as(
        "SELECT EXISTS (
             SELECT 1 FROM agentic_runs
              WHERE agent_key = $1
                AND status = ANY($2)
                AND job_kind <> $3
         )",
    )
    .bind(agent_key)
    .bind(crate::agentic::store::common::ACTIVE_STATUSES)
    .bind(crate::agentic::model::JOB_KIND_ANALYSIS_CODING)
    .fetch_one(pool)
    .await
    .context("failed to check active live runs")?;
    Ok(row.0)
}

pub async fn mark_maintenance_task_succeeded(pool: &DbPool, task_id: i64) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE agentic_maintenance_tasks
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
        "UPDATE agentic_maintenance_tasks
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

pub(crate) async fn agent_has_blocking_workspace_maintenance_tx(
    tx: &mut Transaction<'_, Postgres>,
    agent_key: &str,
) -> Result<bool> {
    let row: Option<(i32,)> = query_as(
        "SELECT 1
           FROM agentic_maintenance_tasks
         WHERE agent_key = $1
           AND (
                 (task_kind = $2 AND status = ANY($3))
              OR (task_kind = $4 AND phase = ANY($5) AND status = ANY($3))
           )
           LIMIT 1",
    )
    .bind(agent_key)
    .bind(MAINTENANCE_TASK_KIND_WORKSPACE_REGENERATE)
    .bind(ACTIVE_MAINTENANCE_STATUSES)
    .bind(MAINTENANCE_TASK_KIND_ANALYSIS_CODING)
    .bind(CODING_PROMOTION_PHASES)
    .fetch_optional(&mut **tx)
    .await
    .with_context(|| format!("failed to check workspace maintenance state for {agent_key}"))?;

    Ok(row.is_some())
}

pub(crate) async fn agent_has_blocking_workspace_maintenance_for_mode_tx(
    tx: &mut Transaction<'_, Postgres>,
    agent_key: &str,
    mode: HookRunInsertMode,
) -> Result<bool> {
    let row: Option<(i32,)> = match mode {
        HookRunInsertMode::Manual => {
            query_as(
                "SELECT 1 FROM agentic_maintenance_tasks
              WHERE agent_key = $1
                AND ((task_kind = $2 AND status = ANY($3))
                  OR (task_kind = $4 AND phase = ANY($5) AND status = ANY($3)))
              LIMIT 1",
            )
            .bind(agent_key)
            .bind(MAINTENANCE_TASK_KIND_WORKSPACE_REGENERATE)
            .bind(ACTIVE_MAINTENANCE_STATUSES)
            .bind(MAINTENANCE_TASK_KIND_ANALYSIS_CODING)
            .bind(CODING_PROMOTION_PHASES)
            .fetch_optional(&mut **tx)
            .await?
        }
        HookRunInsertMode::AnalysisContinuation => {
            query_as(
                "SELECT 1 FROM agentic_maintenance_tasks
              WHERE agent_key = $1
                AND task_kind = $2 AND phase = ANY($3) AND status = ANY($4)
              LIMIT 1",
            )
            .bind(agent_key)
            .bind(MAINTENANCE_TASK_KIND_ANALYSIS_CODING)
            .bind([
                crate::agentic::model::MAINTENANCE_PHASE_PROMOTING,
                crate::agentic::model::MAINTENANCE_PHASE_SMOKE_TESTING,
                crate::agentic::model::MAINTENANCE_PHASE_ROLLING_BACK,
            ])
            .bind(ACTIVE_MAINTENANCE_STATUSES)
            .fetch_optional(&mut **tx)
            .await?
        }
        HookRunInsertMode::CodingTrigger => {
            query_as(
                "SELECT 1 FROM agentic_maintenance_tasks
              WHERE agent_key = $1 AND status = ANY($2)
              LIMIT 1",
            )
            .bind(agent_key)
            .bind(ACTIVE_MAINTENANCE_STATUSES)
            .fetch_optional(&mut **tx)
            .await?
        }
    };
    Ok(row.is_some())
}

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
