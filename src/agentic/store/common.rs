use anyhow::{Context, Result};
use chrono::Utc;
use sqlx::query_as;

use crate::{
    agentic::model::{RUN_STATUS_QUEUED, RUN_STATUS_RUNNING},
    db::DbPool,
};

pub(crate) const ACTIVE_STATUSES: [&str; 2] = [RUN_STATUS_QUEUED, RUN_STATUS_RUNNING];

pub(crate) const ERROR_SUMMARY_MAX_CHARS: usize = 500;

/// Toggle every schedule and hook for an agent to the same enabled state.
pub async fn set_all_agent_jobs_enabled(
    pool: &DbPool,
    agent_key: &str,
    enabled: bool,
) -> Result<()> {
    let mut tx = pool
        .begin()
        .await
        .context("failed to start jobs toggle transaction")?;

    sqlx::query(
        "UPDATE agentic_job_schedules
            SET enabled = $2,
                updated_at = now()
          WHERE agent_key = $1",
    )
    .bind(agent_key)
    .bind(enabled)
    .execute(&mut *tx)
    .await
    .with_context(|| format!("failed to toggle schedules for agent {agent_key}"))?;

    sqlx::query(
        "UPDATE agentic_job_hooks
            SET enabled = $2,
                updated_at = now()
          WHERE agent_key = $1",
    )
    .bind(agent_key)
    .bind(enabled)
    .execute(&mut *tx)
    .await
    .with_context(|| format!("failed to toggle hooks for agent {agent_key}"))?;

    tx.commit()
        .await
        .context("failed to commit jobs toggle transaction")?;

    Ok(())
}

pub(crate) fn truncate_error_summary(value: &str) -> String {
    if value.chars().count() <= ERROR_SUMMARY_MAX_CHARS {
        return value.to_string();
    }
    let mut out: String = value.chars().take(ERROR_SUMMARY_MAX_CHARS).collect();
    out.push('…');
    out
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn insert_run_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    schedule_id: Option<i64>,
    hook_id: Option<i64>,
    agent_key: &str,
    job_key: &str,
    job_kind: &str,
    timeframe: Option<&str>,
    status: &str,
    backend_run_ref: Option<&str>,
    model_provider_id: Option<&str>,
    model_id: Option<&str>,
    scheduled_for: chrono::DateTime<Utc>,
    started_at: Option<chrono::DateTime<Utc>>,
    finished_at: Option<chrono::DateTime<Utc>>,
    timeout_seconds: i32,
    error_summary: Option<&str>,
) -> Result<i64> {
    let truncated = error_summary.map(truncate_error_summary);
    let row: (i64,) = query_as(
        "INSERT INTO agentic_runs (
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
            error_summary
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
         RETURNING id",
    )
    .bind(schedule_id)
    .bind(hook_id)
    .bind(agent_key)
    .bind(job_key)
    .bind(job_kind)
    .bind(timeframe)
    .bind(status)
    .bind(backend_run_ref)
    .bind(model_provider_id)
    .bind(model_id)
    .bind(scheduled_for)
    .bind(started_at)
    .bind(finished_at)
    .bind(timeout_seconds)
    .bind(truncated)
    .fetch_one(&mut **tx)
    .await
    .context("failed to insert agentic_runs row")?;

    Ok(row.0)
}
