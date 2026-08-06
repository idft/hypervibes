use crate::harness::model::RUN_STATUS_ABORTED;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
#[cfg(test)]
use sqlx::PgPool;
use sqlx::query_as;

use crate::{
    db::DbPool,
    harness::model::{
        HarnessRunRow, RUN_STATUS_FAILED, RUN_STATUS_QUEUED, RUN_STATUS_RUNNING,
        RUN_STATUS_SKIPPED, RUN_STATUS_SUCCEEDED,
    },
    harness::timeframe::{boundary_for_due_at, latest_due_at_or_before},
};

use super::common::{
    ACTIVE_STATUSES, EventRunInsertMode, insert_run_with_model_variant_in_tx,
    lock_agent_coordination_tx, truncate_error_summary,
};
use super::jobs::CandleJobForUpdate;
use super::recovery::{has_active_run_in_lane_tx, recover_inactive_agent_runs_tx};
use super::workspace::{
    agent_has_blocking_workspace_maintenance_for_mode_tx,
    agent_has_blocking_workspace_maintenance_tx,
};

#[derive(sqlx::FromRow)]
struct EventJobForUpdate {
    id: i64,
    agent_key: String,
    job_key: String,
    job_kind: String,
    trigger_type: String,
    timeout_seconds: i32,
    model_provider_id: Option<String>,
    model_id: Option<String>,
    model_variant: Option<String>,
}

pub async fn list_active_agent_runs(pool: &DbPool, agent_key: &str) -> Result<Vec<HarnessRunRow>> {
    let mut tx = pool
        .begin()
        .await
        .context("failed to begin active-run listing transaction")?;

    let recovered = recover_inactive_agent_runs_tx(&mut tx, agent_key, Utc::now()).await?;
    if recovered > 0 {
        tracing::info!(
            agent_key,
            recovered,
            "recovered inactive harness runs before agent-wide active check"
        );
    }

    let rows = query_as::<_, HarnessRunRow>(
        "SELECT id,
                job_id,
                agent_key,
                job_key,
                job_kind,
                trigger_type,
                timeframe,
                status,
                backend_run_ref,
                model_provider_id,
                model_id,
                model_variant,
                scheduled_for,
                started_at,
                finished_at,
                timeout_seconds,
                error_summary,
                created_at,
                updated_at
           FROM harness_runs
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
) -> Result<Vec<HarnessRunRow>> {
    list_agent_runs_page(pool, agent_key, limit, 0).await
}

/// List a page of recent runs for an agent.
pub async fn list_agent_runs_page(
    pool: &DbPool,
    agent_key: &str,
    limit: i64,
    offset: i64,
) -> Result<Vec<HarnessRunRow>> {
    let rows = query_as::<_, HarnessRunRow>(
        "SELECT id,
                job_id,
                agent_key,
                job_key,
                job_kind,
                trigger_type,
                timeframe,
                status,
                backend_run_ref,
                model_provider_id,
                model_id,
                model_variant,
                scheduled_for,
                started_at,
                finished_at,
                timeout_seconds,
                error_summary,
                created_at,
                updated_at
           FROM harness_runs
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
    let (count,): (i64,) = query_as("SELECT COUNT(*) FROM harness_runs WHERE agent_key = $1")
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
        "UPDATE harness_runs
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
        "UPDATE harness_runs
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
        "UPDATE harness_runs
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
pub async fn mark_run_aborted(
    pool: &DbPool,
    run_id: i64,
    error_summary: &str,
    backend_run_ref: Option<&str>,
) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE harness_runs
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
pub async fn get_run(pool: &DbPool, run_id: i64) -> Result<Option<HarnessRunRow>> {
    let row: Option<HarnessRunRow> = query_as(
        "SELECT id,
                job_id,
                agent_key,
                job_key,
                job_kind,
                trigger_type,
                timeframe,
                status,
                backend_run_ref,
                model_provider_id,
                model_id,
                model_variant,
                scheduled_for,
                started_at,
                finished_at,
                timeout_seconds,
                error_summary,
                created_at,
                updated_at
           FROM harness_runs
          WHERE id = $1",
    )
    .bind(run_id)
    .fetch_optional(pool)
    .await
    .with_context(|| format!("failed to fetch run {run_id}"))?;

    Ok(row)
}

/// Insert a brand-new `queued` run for a job. Used by manual
/// `Run now` actions. Does not advance `harness_jobs.next_run_at`.
///
/// If a previous run for the same agent is still active, inserts a
/// `skipped` run row instead and does not dispatch.
pub async fn insert_queued_manual_run(
    pool: &DbPool,
    agent_key: &str,
    job_id: i64,
) -> Result<QueuedJobRun> {
    let mut tx = pool
        .begin()
        .await
        .context("failed to begin manual queued run insert")?;
    lock_agent_coordination_tx(&mut tx, agent_key).await?;

    let job: Option<CandleJobForUpdate> = query_as(
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
                model_variant,
                timeout_seconds
           FROM harness_jobs
          WHERE agent_key = $1
            AND id = $2
          FOR UPDATE",
    )
    .bind(agent_key)
    .bind(job_id)
    .fetch_optional(&mut *tx)
    .await
    .context("failed to lock job for manual run")?;

    let Some(job) = job else {
        tx.rollback()
            .await
            .context("failed to roll back missing-job manual run")?;
        return Ok(QueuedJobRun::Missing);
    };

    if agent_has_blocking_workspace_maintenance_tx(&mut tx, &job.agent_key).await? {
        tx.rollback()
            .await
            .context("failed to roll back maintenance-blocked manual run")?;
        return Ok(QueuedJobRun::BlockedByMaintenance);
    }

    let now = Utc::now();
    let scheduled_for = latest_due_at_or_before(now, &job.timeframe, job.trigger_delay_seconds)?
        .map(|due| boundary_for_due_at(due, job.trigger_delay_seconds))
        .unwrap_or(now);
    let wait_for_lane =
        has_active_run_in_lane_tx(&mut tx, &job.agent_key, &job.job_kind, now).await?;
    let run_id = insert_run_with_model_variant_in_tx(
        &mut tx,
        job.id,
        &job.agent_key,
        &job.job_key,
        &job.job_kind,
        "candle_closed",
        Some(&job.timeframe),
        RUN_STATUS_QUEUED,
        None,
        job.model_provider_id.as_deref(),
        job.model_id.as_deref(),
        job.model_variant.as_deref(),
        scheduled_for,
        None,
        None,
        job.timeout_seconds,
        None,
    )
    .await?;
    let outcome = QueuedJobRun::Dispatch {
        run_id,
        scheduled_for,
        wait_for_lane,
    };

    tx.commit()
        .await
        .context("failed to commit manual queued run")?;

    Ok(outcome)
}

#[derive(Debug, Clone)]
pub enum QueuedJobRun {
    Dispatch {
        run_id: i64,
        scheduled_for: DateTime<Utc>,
        wait_for_lane: bool,
    },
    Skipped {
        run_id: i64,
    },
    Missing,
    BlockedByMaintenance,
}

/// Whether a manual queued run must wait for an earlier run in its lane.
pub async fn has_prior_active_run_in_lane(
    pool: &DbPool,
    agent_key: &str,
    job_kind: &str,
    run_id: i64,
) -> Result<bool> {
    let lane_job_kinds = super::recovery::active_job_kinds_for_lane(job_kind);
    let active: Option<(i32,)> = query_as(
        "SELECT 1 FROM harness_runs
          WHERE agent_key = $1
            AND job_kind = ANY($2)
            AND (status = $3 OR (status = $4 AND id < $5))
          LIMIT 1",
    )
    .bind(agent_key)
    .bind(lane_job_kinds)
    .bind(RUN_STATUS_RUNNING)
    .bind(RUN_STATUS_QUEUED)
    .bind(run_id)
    .fetch_optional(pool)
    .await
    .context("failed to check for prior active run in lane")?;
    Ok(active.is_some())
}

pub async fn insert_queued_event_run(
    pool: &DbPool,
    agent_key: &str,
    job_id: i64,
) -> Result<QueuedJobRun> {
    insert_queued_event_run_with_mode(pool, agent_key, job_id, EventRunInsertMode::Manual).await
}

pub async fn insert_queued_event_run_for_automatic_dispatch(
    pool: &DbPool,
    agent_key: &str,
    job_id: i64,
) -> Result<QueuedJobRun> {
    insert_queued_event_run_with_mode(
        pool,
        agent_key,
        job_id,
        EventRunInsertMode::AnalysisContinuation,
    )
    .await
}

pub(crate) async fn insert_queued_event_run_with_mode(
    pool: &DbPool,
    agent_key: &str,
    job_id: i64,
    mode: EventRunInsertMode,
) -> Result<QueuedJobRun> {
    let mut tx = pool
        .begin()
        .await
        .context("failed to begin manual queued event run insert")?;
    lock_agent_coordination_tx(&mut tx, agent_key).await?;

    let event: Option<EventJobForUpdate> = query_as(
        "SELECT id,
                agent_key,
                job_key,
                job_kind,
                trigger_type,
                enabled,
                model_provider_id,
                model_id,
                model_variant,
                timeout_seconds
           FROM harness_jobs
          WHERE agent_key = $1
            AND id = $2
          FOR UPDATE",
    )
    .bind(agent_key)
    .bind(job_id)
    .fetch_optional(&mut *tx)
    .await
    .context("failed to lock event for manual run")?;

    let Some(event) = event else {
        tx.rollback()
            .await
            .context("failed to roll back missing-event manual run")?;
        return Ok(QueuedJobRun::Missing);
    };

    if agent_has_blocking_workspace_maintenance_for_mode_tx(&mut tx, &event.agent_key, mode).await?
    {
        tx.rollback()
            .await
            .context("failed to roll back maintenance-blocked manual event run")?;
        return Ok(QueuedJobRun::BlockedByMaintenance);
    }

    let now = Utc::now();
    let outcome =
        if has_active_run_in_lane_tx(&mut tx, &event.agent_key, &event.job_kind, now).await? {
            let run_id = insert_run_with_model_variant_in_tx(
                &mut tx,
                event.id,
                &event.agent_key,
                &event.job_key,
                &event.job_kind,
                &event.trigger_type,
                None,
                RUN_STATUS_SKIPPED,
                None,
                event.model_provider_id.as_deref(),
                event.model_id.as_deref(),
                event.model_variant.as_deref(),
                now,
                None,
                Some(now),
                event.timeout_seconds,
                Some("previous run still active"),
            )
            .await?;
            QueuedJobRun::Skipped { run_id }
        } else {
            let run_id = insert_run_with_model_variant_in_tx(
                &mut tx,
                event.id,
                &event.agent_key,
                &event.job_key,
                &event.job_kind,
                &event.trigger_type,
                None,
                RUN_STATUS_QUEUED,
                None,
                event.model_provider_id.as_deref(),
                event.model_id.as_deref(),
                event.model_variant.as_deref(),
                now,
                None,
                None,
                event.timeout_seconds,
                None,
            )
            .await?;
            QueuedJobRun::Dispatch {
                run_id,
                scheduled_for: now,
                wait_for_lane: false,
            }
        };

    tx.commit()
        .await
        .context("failed to commit manual queued event run")?;

    Ok(outcome)
}

#[cfg(test)]
#[derive(sqlx::FromRow)]
struct TestRunJob {
    id: i64,
    agent_key: String,
    job_key: String,
    job_kind: String,
    trigger_type: String,
    timeframe: Option<String>,
    model_provider_id: Option<String>,
    model_id: Option<String>,
    model_variant: Option<String>,
    timeout_seconds: i32,
}

/// Small helper to keep the row insert signature in one place for tests.
#[cfg(test)]
pub async fn insert_test_run(pool: &PgPool, job_id: i64, status: &str) -> Result<i64> {
    let job: TestRunJob = query_as(
        "SELECT id,
                agent_key,
                job_key,
                job_kind,
                trigger_type,
                timeframe,
                model_provider_id,
                model_id,
                model_variant,
                timeout_seconds
           FROM harness_jobs
          WHERE id = $1",
    )
    .bind(job_id)
    .fetch_one(pool)
    .await
    .context("failed to load job for test run")?;

    let row: (i64,) = query_as(
        "INSERT INTO harness_runs (
            job_id,
            agent_key,
            job_key,
            job_kind,
            trigger_type,
                timeframe,
             status,
             model_provider_id,
             model_id,
             model_variant,
             scheduled_for,
             timeout_seconds
           ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
         RETURNING id",
    )
    .bind(job.id)
    .bind(&job.agent_key)
    .bind(&job.job_key)
    .bind(&job.job_kind)
    .bind(&job.trigger_type)
    .bind(job.timeframe)
    .bind(status)
    .bind(job.model_provider_id)
    .bind(job.model_id)
    .bind(job.model_variant)
    .bind(Utc::now())
    .bind(job.timeout_seconds)
    .fetch_one(pool)
    .await
    .context("failed to insert test run")?;

    Ok(row.0)
}
