use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction, query_as};

use crate::agentic::model::{
    JOB_KIND_ANALYSIS, JOB_KIND_ANALYSIS_CODING, JOB_KIND_DAILY_REVIEW,
    JOB_KIND_MARKET_ANALYSIS, JOB_KIND_TRADING, RUN_STATUS_FAILED, RUN_STATUS_QUEUED,
    RUN_STATUS_RUNNING, RUN_STATUS_SUCCEEDED,
};

use super::common::ACTIVE_STATUSES;

const OPENCODE_STATUS_IDLE: &str = "idle";
pub(crate) const ORPHANED_QUEUED_RUN_SUMMARY: &str =
    "queued run orphaned by app restart after timeout";
pub(crate) const ORPHANED_RUNNING_RUN_SUMMARY: &str =
    "running run orphaned by app restart after timeout";

pub(crate) fn active_job_kinds_for_lane(job_kind: &str) -> &'static [&'static str] {
    match job_kind {
        JOB_KIND_TRADING => &[JOB_KIND_TRADING],
        JOB_KIND_ANALYSIS | JOB_KIND_MARKET_ANALYSIS | JOB_KIND_DAILY_REVIEW => &[
            JOB_KIND_ANALYSIS,
            JOB_KIND_MARKET_ANALYSIS,
            JOB_KIND_DAILY_REVIEW,
        ],
        JOB_KIND_ANALYSIS_CODING => &[],
        _ => &[],
    }
}

pub(crate) async fn has_active_run_in_lane_tx(
    tx: &mut Transaction<'_, Postgres>,
    agent_key: &str,
    job_kind: &str,
    now: DateTime<Utc>,
) -> Result<bool> {
    let recovered = recover_inactive_runs_in_lane_tx(tx, agent_key, job_kind, now).await?;
    if recovered > 0 {
        tracing::info!(
            agent_key,
            job_kind,
            recovered,
            "recovered inactive agentic runs before lane active check"
        );
    }

    let lane_job_kinds = active_job_kinds_for_lane(job_kind);
    let active: Option<(i32,)> = query_as(
        "SELECT 1 FROM agentic_runs
          WHERE agent_key = $1
            AND status = ANY($2)
            AND job_kind = ANY($3)
          LIMIT 1",
    )
    .bind(agent_key)
    .bind(&ACTIVE_STATUSES)
    .bind(lane_job_kinds)
    .fetch_optional(&mut **tx)
    .await
    .context("failed to check for active run in lane")?;

    Ok(active.is_some())
}

pub(crate) async fn recover_inactive_runs_in_lane_tx(
    tx: &mut Transaction<'_, Postgres>,
    agent_key: &str,
    job_kind: &str,
    now: DateTime<Utc>,
) -> Result<u64> {
    let lane_job_kinds = active_job_kinds_for_lane(job_kind);
    if lane_job_kinds.is_empty() {
        return Ok(0);
    }

    let recovered_succeeded = sqlx::query(
        "UPDATE agentic_runs AS runs
            SET status = $3,
                finished_at = COALESCE(runs.finished_at, sessions.updated_at, now()),
                error_summary = NULL,
                updated_at = now()
           FROM opencode.sessions AS sessions
           WHERE runs.agent_key = $1
             AND runs.job_kind = ANY($2)
             AND runs.job_kind <> 'analysis_coding'
            AND runs.status = $4
            AND runs.backend_run_ref IS NOT NULL
            AND sessions.id = runs.backend_run_ref
            AND sessions.status = $5
            AND EXISTS (
                SELECT 1
                  FROM opencode.commands AS commands
                 WHERE commands.session_id = runs.backend_run_ref
            )",
    )
    .bind(agent_key)
    .bind(lane_job_kinds)
    .bind(RUN_STATUS_SUCCEEDED)
    .bind(RUN_STATUS_RUNNING)
    .bind(OPENCODE_STATUS_IDLE)
    .execute(&mut **tx)
    .await
    .context("failed to recover idle OpenCode runs in lane")?
    .rows_affected();

    let recovered_queued = sqlx::query(
        "UPDATE agentic_runs
            SET status = $3,
                finished_at = $4,
                error_summary = $5,
                updated_at = now()
           WHERE agent_key = $1
             AND job_kind = ANY($2)
             AND job_kind <> 'analysis_coding'
            AND status = $6
            AND started_at IS NULL
            AND finished_at IS NULL
            AND created_at + (timeout_seconds * interval '1 second') <= $4",
    )
    .bind(agent_key)
    .bind(lane_job_kinds)
    .bind(RUN_STATUS_FAILED)
    .bind(now)
    .bind(ORPHANED_QUEUED_RUN_SUMMARY)
    .bind(RUN_STATUS_QUEUED)
    .execute(&mut **tx)
    .await
    .context("failed to recover orphaned queued runs in lane")?
    .rows_affected();

    let recovered_running = sqlx::query(
        "UPDATE agentic_runs
            SET status = $3,
                finished_at = $4,
                error_summary = $5,
                updated_at = now()
           WHERE agent_key = $1
             AND job_kind = ANY($2)
             AND job_kind <> 'analysis_coding'
            AND status = $6
            AND finished_at IS NULL
            AND COALESCE(started_at, created_at) + (timeout_seconds * interval '1 second') <= $4",
    )
    .bind(agent_key)
    .bind(lane_job_kinds)
    .bind(RUN_STATUS_FAILED)
    .bind(now)
    .bind(ORPHANED_RUNNING_RUN_SUMMARY)
    .bind(RUN_STATUS_RUNNING)
    .execute(&mut **tx)
    .await
    .context("failed to recover orphaned running runs in lane")?
    .rows_affected();

    Ok(recovered_succeeded + recovered_queued + recovered_running)
}

/// Run a global recovery pass across every agent.
///
/// This is the periodic backstop that complements the per-lane
/// [`recover_inactive_runs_in_lane_tx`] recovery. Lane recovery only
/// fires when a new run is claimed for the same agent; without this
/// periodic pass, a `running` run whose dispatch worker has died would
/// block its lane until the next claim attempt. Periodic recovery
/// turns those orphans into terminal `failed`/`succeeded` rows on a
/// fixed cadence so subsequent schedules dispatch normally.
pub async fn recover_inactive_runs_all(pool: &PgPool, now: DateTime<Utc>) -> Result<u64> {
    let mut tx = pool
        .begin()
        .await
        .context("failed to begin global agentic run recovery transaction")?;

    let recovered_succeeded = sqlx::query(
        "UPDATE agentic_runs AS runs
            SET status = $1,
                finished_at = COALESCE(runs.finished_at, sessions.updated_at, now()),
                error_summary = NULL,
                updated_at = now()
           FROM opencode.sessions AS sessions
          WHERE runs.status = $2
            AND runs.job_kind <> 'analysis_coding'
            AND runs.backend_run_ref IS NOT NULL
            AND sessions.id = runs.backend_run_ref
            AND sessions.status = $3
            AND EXISTS (
                SELECT 1
                  FROM opencode.commands AS commands
                 WHERE commands.session_id = runs.backend_run_ref
            )",
    )
    .bind(RUN_STATUS_SUCCEEDED)
    .bind(RUN_STATUS_RUNNING)
    .bind(OPENCODE_STATUS_IDLE)
    .execute(&mut *tx)
    .await
    .context("failed to recover idle OpenCode runs globally")?
    .rows_affected();

    let recovered_queued = sqlx::query(
        "UPDATE agentic_runs
            SET status = $1,
                finished_at = $2,
                error_summary = $3,
                updated_at = $2
          WHERE status = $4
            AND job_kind <> 'analysis_coding'
            AND started_at IS NULL
            AND finished_at IS NULL
            AND created_at + (timeout_seconds * interval '1 second') <= $2",
    )
    .bind(RUN_STATUS_FAILED)
    .bind(now)
    .bind(ORPHANED_QUEUED_RUN_SUMMARY)
    .bind(RUN_STATUS_QUEUED)
    .execute(&mut *tx)
    .await
    .context("failed to recover orphaned queued runs globally")?
    .rows_affected();

    let recovered_running = sqlx::query(
        "UPDATE agentic_runs
            SET status = $1,
                finished_at = $2,
                error_summary = $3,
                updated_at = $2
          WHERE status = $4
            AND job_kind <> 'analysis_coding'
            AND finished_at IS NULL
            AND COALESCE(started_at, created_at) + (timeout_seconds * interval '1 second') <= $2",
    )
    .bind(RUN_STATUS_FAILED)
    .bind(now)
    .bind(ORPHANED_RUNNING_RUN_SUMMARY)
    .bind(RUN_STATUS_RUNNING)
    .execute(&mut *tx)
    .await
    .context("failed to recover orphaned running runs globally")?
    .rows_affected();

    tx.commit()
        .await
        .context("failed to commit global agentic run recovery transaction")?;

    Ok(recovered_succeeded + recovered_queued + recovered_running)
}

pub(crate) async fn recover_inactive_agent_runs_tx(
    tx: &mut Transaction<'_, Postgres>,
    agent_key: &str,
    now: DateTime<Utc>,
) -> Result<u64> {
    let recovered_succeeded = sqlx::query(
        "UPDATE agentic_runs AS runs
            SET status = $2,
                finished_at = COALESCE(runs.finished_at, sessions.updated_at, now()),
                error_summary = NULL,
                updated_at = now()
           FROM opencode.sessions AS sessions
           WHERE runs.agent_key = $1
             AND runs.status = $3
             AND runs.job_kind <> 'analysis_coding'
            AND runs.backend_run_ref IS NOT NULL
            AND sessions.id = runs.backend_run_ref
            AND sessions.status = $4
            AND EXISTS (
                SELECT 1
                  FROM opencode.commands AS commands
                 WHERE commands.session_id = runs.backend_run_ref
            )",
    )
    .bind(agent_key)
    .bind(RUN_STATUS_SUCCEEDED)
    .bind(RUN_STATUS_RUNNING)
    .bind(OPENCODE_STATUS_IDLE)
    .execute(&mut **tx)
    .await
    .context("failed to recover idle OpenCode runs for agent")?
    .rows_affected();

    let recovered_queued = sqlx::query(
        "UPDATE agentic_runs
            SET status = $2,
                finished_at = $3,
                error_summary = $4,
                updated_at = now()
           WHERE agent_key = $1
             AND status = $5
             AND job_kind <> 'analysis_coding'
            AND started_at IS NULL
            AND finished_at IS NULL
            AND created_at + (timeout_seconds * interval '1 second') <= $3",
    )
    .bind(agent_key)
    .bind(RUN_STATUS_FAILED)
    .bind(now)
    .bind(ORPHANED_QUEUED_RUN_SUMMARY)
    .bind(RUN_STATUS_QUEUED)
    .execute(&mut **tx)
    .await
    .context("failed to recover orphaned queued runs for agent")?
    .rows_affected();

    let recovered_running = sqlx::query(
        "UPDATE agentic_runs
            SET status = $2,
                finished_at = $3,
                error_summary = $4,
                updated_at = now()
           WHERE agent_key = $1
             AND status = $5
             AND job_kind <> 'analysis_coding'
            AND finished_at IS NULL
            AND COALESCE(started_at, created_at) + (timeout_seconds * interval '1 second') <= $3",
    )
    .bind(agent_key)
    .bind(RUN_STATUS_FAILED)
    .bind(now)
    .bind(ORPHANED_RUNNING_RUN_SUMMARY)
    .bind(RUN_STATUS_RUNNING)
    .execute(&mut **tx)
    .await
    .context("failed to recover orphaned running runs for agent")?
    .rows_affected();

    Ok(recovered_succeeded + recovered_queued + recovered_running)
}
