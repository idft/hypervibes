#[cfg(test)]
use crate::agentic::model::RUN_STATUS_ABORTED;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
#[cfg(test)]
use sqlx::PgPool;
use sqlx::query_as;

use crate::{
    agentic::model::{
        AgenticRunRow, RUN_STATUS_FAILED, RUN_STATUS_QUEUED, RUN_STATUS_RUNNING,
        RUN_STATUS_SKIPPED, RUN_STATUS_SUCCEEDED,
    },
    agentic::timeframe::{boundary_for_due_at, latest_due_at_or_before},
    db::DbPool,
};

use super::common::{
    ACTIVE_STATUSES, HookRunInsertMode, insert_run_in_tx, lock_agent_coordination_tx,
    truncate_error_summary,
};
use super::hooks::HookForUpdate;
use super::recovery::{has_active_run_in_lane_tx, recover_inactive_agent_runs_tx};
use super::schedules::ScheduleForUpdate;
use super::workspace::{
    agent_has_blocking_workspace_maintenance_for_mode_tx,
    agent_has_blocking_workspace_maintenance_tx,
};

pub async fn list_active_agent_runs(pool: &DbPool, agent_key: &str) -> Result<Vec<AgenticRunRow>> {
    let mut tx = pool
        .begin()
        .await
        .context("failed to begin active-run listing transaction")?;

    let recovered = recover_inactive_agent_runs_tx(&mut tx, agent_key, Utc::now()).await?;
    if recovered > 0 {
        tracing::info!(
            agent_key,
            recovered,
            "recovered inactive agentic runs before agent-wide active check"
        );
    }

    let rows = query_as::<_, AgenticRunRow>(
        "SELECT id,
                schedule_id,
                hook_id,
                agent_key,
                job_key,
                job_kind,
                timeframe,
                status,
                backend_run_ref,
                model_provider_id,
                model_id,
                scheduled_for,
                started_at,
                finished_at,
                timeout_seconds,
                error_summary,
                created_at,
                updated_at
           FROM agentic_runs
          WHERE agent_key = $1
            AND status = ANY($2)
          ORDER BY created_at ASC, id ASC",
    )
    .bind(agent_key)
    .bind(ACTIVE_STATUSES)
    .fetch_all(&mut *tx)
    .await
    .with_context(|| format!("failed to list active runs for agent {agent_key}"))?;

    tx.commit()
        .await
        .context("failed to commit active-run listing transaction")?;

    Ok(rows)
}

pub async fn agent_has_active_runs(pool: &DbPool, agent_key: &str) -> Result<bool> {
    Ok(!list_active_agent_runs(pool, agent_key).await?.is_empty())
}

/// List the most recent runs for an agent.
#[cfg(test)]
pub async fn list_agent_runs(
    pool: &DbPool,
    agent_key: &str,
    limit: i64,
) -> Result<Vec<AgenticRunRow>> {
    list_agent_runs_page(pool, agent_key, limit, 0).await
}

/// List a page of recent runs for an agent.
pub async fn list_agent_runs_page(
    pool: &DbPool,
    agent_key: &str,
    limit: i64,
    offset: i64,
) -> Result<Vec<AgenticRunRow>> {
    let rows = query_as::<_, AgenticRunRow>(
        "SELECT id,
                schedule_id,
                hook_id,
                agent_key,
                job_key,
                job_kind,
                timeframe,
                status,
                backend_run_ref,
                model_provider_id,
                model_id,
                scheduled_for,
                started_at,
                finished_at,
                timeout_seconds,
                error_summary,
                created_at,
                updated_at
           FROM agentic_runs
          WHERE agent_key = $1
          ORDER BY created_at DESC
          LIMIT $2
         OFFSET $3",
    )
    .bind(agent_key)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
    .with_context(|| format!("failed to list runs for agent {agent_key}"))?;

    Ok(rows)
}

/// Count runs recorded for an agent.
pub async fn count_agent_runs(pool: &DbPool, agent_key: &str) -> Result<i64> {
    let (count,): (i64,) = query_as("SELECT COUNT(*) FROM agentic_runs WHERE agent_key = $1")
        .bind(agent_key)
        .fetch_one(pool)
        .await
        .with_context(|| format!("failed to count runs for agent {agent_key}"))?;

    Ok(count)
}

/// Move a run into the `running` state. Optionally records a
/// `backend_run_ref` if one is already known.
pub async fn mark_run_running(
    pool: &DbPool,
    run_id: i64,
    backend_run_ref: Option<&str>,
) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE agentic_runs
            SET status = $2,
                started_at = COALESCE(started_at, now()),
                backend_run_ref = COALESCE($3, backend_run_ref),
                updated_at = now()
          WHERE id = $1",
    )
    .bind(run_id)
    .bind(RUN_STATUS_RUNNING)
    .bind(backend_run_ref)
    .execute(pool)
    .await
    .with_context(|| format!("failed to mark run {run_id} running"))?;

    Ok(result.rows_affected() > 0)
}

/// Mark a run as succeeded. Optionally records a final `backend_run_ref`.
pub async fn mark_run_succeeded(
    pool: &DbPool,
    run_id: i64,
    backend_run_ref: Option<&str>,
) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE agentic_runs
            SET status = $2,
                finished_at = now(),
                backend_run_ref = COALESCE($3, backend_run_ref),
                error_summary = NULL,
                updated_at = now()
          WHERE id = $1",
    )
    .bind(run_id)
    .bind(RUN_STATUS_SUCCEEDED)
    .bind(backend_run_ref)
    .execute(pool)
    .await
    .with_context(|| format!("failed to mark run {run_id} succeeded"))?;

    Ok(result.rows_affected() > 0)
}

/// Mark a run as failed with a short, sanitized error summary.
pub async fn mark_run_failed(
    pool: &DbPool,
    run_id: i64,
    error_summary: &str,
    backend_run_ref: Option<&str>,
) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE agentic_runs
            SET status = $2,
                finished_at = now(),
                error_summary = $3,
                backend_run_ref = COALESCE($4, backend_run_ref),
                updated_at = now()
          WHERE id = $1",
    )
    .bind(run_id)
    .bind(RUN_STATUS_FAILED)
    .bind(truncate_error_summary(error_summary))
    .bind(backend_run_ref)
    .execute(pool)
    .await
    .with_context(|| format!("failed to mark run {run_id} failed"))?;

    Ok(result.rows_affected() > 0)
}

/// Mark a run as aborted with a short, sanitized error summary.
#[cfg(test)]
pub async fn mark_run_aborted(
    pool: &DbPool,
    run_id: i64,
    error_summary: &str,
    backend_run_ref: Option<&str>,
) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE agentic_runs
            SET status = $2,
                finished_at = now(),
                error_summary = $3,
                backend_run_ref = COALESCE($4, backend_run_ref),
                updated_at = now()
          WHERE id = $1",
    )
    .bind(run_id)
    .bind(RUN_STATUS_ABORTED)
    .bind(truncate_error_summary(error_summary))
    .bind(backend_run_ref)
    .execute(pool)
    .await
    .with_context(|| format!("failed to mark run {run_id} aborted"))?;

    Ok(result.rows_affected() > 0)
}

/// Look up the run row by id, returning `None` if it has been deleted
/// (e.g., by an agent cascading delete).
pub async fn get_run(pool: &DbPool, run_id: i64) -> Result<Option<AgenticRunRow>> {
    let row: Option<AgenticRunRow> = query_as(
        "SELECT id,
                schedule_id,
                hook_id,
                agent_key,
                job_key,
                job_kind,
                timeframe,
                status,
                backend_run_ref,
                model_provider_id,
                model_id,
                scheduled_for,
                started_at,
                finished_at,
                timeout_seconds,
                error_summary,
                created_at,
                updated_at
           FROM agentic_runs
          WHERE id = $1",
    )
    .bind(run_id)
    .fetch_optional(pool)
    .await
    .with_context(|| format!("failed to fetch run {run_id}"))?;

    Ok(row)
}

/// Insert a brand-new `queued` run for a schedule. Used by manual
/// `Run now` actions. Does not advance `agentic_job_schedules.next_run_at`.
///
/// If a previous run for the same agent is still active, inserts a
/// `skipped` run row instead and does not dispatch.
pub async fn insert_queued_run(
    pool: &DbPool,
    agent_key: &str,
    schedule_id: i64,
) -> Result<QueuedScheduleRun> {
    let mut tx = pool
        .begin()
        .await
        .context("failed to begin manual queued run insert")?;
    lock_agent_coordination_tx(&mut tx, agent_key).await?;

    let schedule: Option<ScheduleForUpdate> = query_as(
        "SELECT id,
                agent_key,
                job_key,
                job_kind,
                enabled,
                timeframe,
                trigger_delay_seconds,
                next_run_at,
                model_provider_id,
                model_id,
                timeout_seconds
           FROM agentic_job_schedules
          WHERE agent_key = $1
            AND id = $2
          FOR UPDATE",
    )
    .bind(agent_key)
    .bind(schedule_id)
    .fetch_optional(&mut *tx)
    .await
    .context("failed to lock schedule for manual run")?;

    let Some(schedule) = schedule else {
        tx.rollback()
            .await
            .context("failed to roll back missing-schedule manual run")?;
        return Ok(QueuedScheduleRun::Missing);
    };

    if agent_has_blocking_workspace_maintenance_tx(&mut tx, &schedule.agent_key).await? {
        tx.rollback()
            .await
            .context("failed to roll back maintenance-blocked manual run")?;
        return Ok(QueuedScheduleRun::BlockedByMaintenance);
    }

    let now = Utc::now();
    let scheduled_for =
        latest_due_at_or_before(now, &schedule.timeframe, schedule.trigger_delay_seconds)?
            .map(|due| boundary_for_due_at(due, schedule.trigger_delay_seconds))
            .unwrap_or(now);
    let outcome = if has_active_run_in_lane_tx(
        &mut tx,
        &schedule.agent_key,
        &schedule.job_kind,
        now,
    )
    .await?
    {
        let run_id = insert_run_in_tx(
            &mut tx,
            Some(schedule.id),
            None,
            &schedule.agent_key,
            &schedule.job_key,
            &schedule.job_kind,
            Some(&schedule.timeframe),
            RUN_STATUS_SKIPPED,
            None,
            schedule.model_provider_id.as_deref(),
            schedule.model_id.as_deref(),
            scheduled_for,
            None,
            Some(now),
            schedule.timeout_seconds,
            Some("previous run still active"),
        )
        .await?;
        let _ = run_id;
        QueuedScheduleRun::Skipped
    } else {
        let run_id = insert_run_in_tx(
            &mut tx,
            Some(schedule.id),
            None,
            &schedule.agent_key,
            &schedule.job_key,
            &schedule.job_kind,
            Some(&schedule.timeframe),
            RUN_STATUS_QUEUED,
            None,
            schedule.model_provider_id.as_deref(),
            schedule.model_id.as_deref(),
            scheduled_for,
            None,
            None,
            schedule.timeout_seconds,
            None,
        )
        .await?;
        QueuedScheduleRun::Dispatch {
            run_id,
            scheduled_for,
        }
    };

    tx.commit()
        .await
        .context("failed to commit manual queued run")?;

    Ok(outcome)
}

#[derive(Debug, Clone)]
pub enum QueuedScheduleRun {
    Dispatch {
        run_id: i64,
        scheduled_for: DateTime<Utc>,
    },
    Skipped,
    Missing,
    BlockedByMaintenance,
}

pub async fn insert_queued_hook_run(
    pool: &DbPool,
    agent_key: &str,
    hook_id: i64,
) -> Result<QueuedHookRun> {
    insert_queued_hook_run_with_mode(pool, agent_key, hook_id, HookRunInsertMode::Manual).await
}

pub async fn insert_queued_hook_run_for_automatic_dispatch(
    pool: &DbPool,
    agent_key: &str,
    hook_id: i64,
) -> Result<QueuedHookRun> {
    insert_queued_hook_run_with_mode(
        pool,
        agent_key,
        hook_id,
        HookRunInsertMode::AnalysisContinuation,
    )
    .await
}

pub(crate) async fn insert_queued_hook_run_with_mode(
    pool: &DbPool,
    agent_key: &str,
    hook_id: i64,
    mode: HookRunInsertMode,
) -> Result<QueuedHookRun> {
    let mut tx = pool
        .begin()
        .await
        .context("failed to begin manual queued hook run insert")?;
    lock_agent_coordination_tx(&mut tx, agent_key).await?;

    let hook: Option<HookForUpdate> = query_as(
        "SELECT id,
                agent_key,
                job_key,
                job_kind,
                hook_event,
                enabled,
                model_provider_id,
                model_id,
                timeout_seconds
           FROM agentic_job_hooks
          WHERE agent_key = $1
            AND id = $2
          FOR UPDATE",
    )
    .bind(agent_key)
    .bind(hook_id)
    .fetch_optional(&mut *tx)
    .await
    .context("failed to lock hook for manual run")?;

    let Some(hook) = hook else {
        tx.rollback()
            .await
            .context("failed to roll back missing-hook manual run")?;
        return Ok(QueuedHookRun::Missing);
    };

    if agent_has_blocking_workspace_maintenance_for_mode_tx(&mut tx, &hook.agent_key, mode).await? {
        tx.rollback()
            .await
            .context("failed to roll back maintenance-blocked manual hook run")?;
        return Ok(QueuedHookRun::BlockedByMaintenance);
    }

    let now = Utc::now();
    let outcome =
        if has_active_run_in_lane_tx(&mut tx, &hook.agent_key, &hook.job_kind, now).await? {
            let run_id = insert_run_in_tx(
                &mut tx,
                None,
                Some(hook.id),
                &hook.agent_key,
                &hook.job_key,
                &hook.job_kind,
                None,
                RUN_STATUS_SKIPPED,
                None,
                hook.model_provider_id.as_deref(),
                hook.model_id.as_deref(),
                now,
                None,
                Some(now),
                hook.timeout_seconds,
                Some("previous run still active"),
            )
            .await?;
            QueuedHookRun::Skipped { run_id }
        } else {
            let run_id = insert_run_in_tx(
                &mut tx,
                None,
                Some(hook.id),
                &hook.agent_key,
                &hook.job_key,
                &hook.job_kind,
                None,
                RUN_STATUS_QUEUED,
                None,
                hook.model_provider_id.as_deref(),
                hook.model_id.as_deref(),
                now,
                None,
                None,
                hook.timeout_seconds,
                None,
            )
            .await?;
            QueuedHookRun::Dispatch {
                run_id,
                scheduled_for: now,
            }
        };

    tx.commit()
        .await
        .context("failed to commit manual queued hook run")?;

    Ok(outcome)
}

#[derive(Debug, Clone)]
pub enum QueuedHookRun {
    Dispatch {
        run_id: i64,
        scheduled_for: DateTime<Utc>,
    },
    Skipped {
        run_id: i64,
    },
    Missing,
    BlockedByMaintenance,
}

/// Small helper to keep the row insert signature in one place for tests.
#[cfg(test)]
pub async fn insert_test_run(pool: &PgPool, schedule_id: i64, status: &str) -> Result<i64> {
    let schedule: ScheduleForUpdate = query_as(
        "SELECT id,
                agent_key,
                job_key,
                job_kind,
                enabled,
                timeframe,
                trigger_delay_seconds,
                next_run_at,
                model_provider_id,
                model_id,
                timeout_seconds
           FROM agentic_job_schedules
          WHERE id = $1",
    )
    .bind(schedule_id)
    .fetch_one(pool)
    .await
    .context("failed to load schedule for test run")?;

    let row: (i64,) = query_as(
        "INSERT INTO agentic_runs (
            schedule_id,
            agent_key,
            job_key,
            job_kind,
            timeframe,
            status,
            scheduled_for,
            timeout_seconds
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         RETURNING id",
    )
    .bind(schedule.id)
    .bind(&schedule.agent_key)
    .bind(&schedule.job_key)
    .bind(&schedule.job_kind)
    .bind(&schedule.timeframe)
    .bind(status)
    .bind(Utc::now())
    .bind(schedule.timeout_seconds)
    .fetch_one(pool)
    .await
    .context("failed to insert test run")?;

    Ok(row.0)
}
