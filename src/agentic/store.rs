use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction, query_as};

use crate::{
    agentic::model::{
        AgenticJobScheduleRow, AgenticRunRow, DueOpenCodeScheduleRow, JOB_KIND_ANALYSIS,
        JOB_KIND_TRADING, RUN_STATUS_ABORTED, RUN_STATUS_FAILED, RUN_STATUS_QUEUED,
        RUN_STATUS_RUNNING, RUN_STATUS_SKIPPED, RUN_STATUS_SUCCEEDED,
    },
    db::DbPool,
};

const ERROR_SUMMARY_MAX_CHARS: usize = 500;
const ACTIVE_STATUSES: [&str; 2] = [RUN_STATUS_QUEUED, RUN_STATUS_RUNNING];

/// Seed the canonical default schedules for a newly-created OpenCode agent.
///
/// This is idempotent: existing `(agent_key, job_key)` rows are left
/// untouched. New agents always get both `analysis-15m` (enabled) and
/// `trading-1m` (disabled).
pub async fn insert_default_opencode_schedules(pool: &DbPool, agent_key: &str) -> Result<()> {
    let now = Utc::now();

    sqlx::query(
        "INSERT INTO agentic_job_schedules (
            agent_key,
            job_key,
            job_kind,
            enabled,
            interval_seconds,
            next_run_at,
            timeout_seconds,
            operator_prompt
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT (agent_key, job_key) DO NOTHING",
    )
    .bind(agent_key)
    .bind("analysis-15m")
    .bind(JOB_KIND_ANALYSIS)
    .bind(true)
    .bind(900_i32)
    .bind(now)
    .bind(600_i32)
    .bind("")
    .execute(pool)
    .await
    .with_context(|| {
        format!("failed to insert default analysis-15m schedule for agent {agent_key}")
    })?;

    sqlx::query(
        "INSERT INTO agentic_job_schedules (
            agent_key,
            job_key,
            job_kind,
            enabled,
            interval_seconds,
            next_run_at,
            timeout_seconds,
            operator_prompt
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT (agent_key, job_key) DO NOTHING",
    )
    .bind(agent_key)
    .bind("trading-1m")
    .bind(JOB_KIND_TRADING)
    .bind(false)
    .bind(60_i32)
    .bind(now)
    .bind(45_i32)
    .bind("")
    .execute(pool)
    .await
    .with_context(|| {
        format!("failed to insert default trading-1m schedule for agent {agent_key}")
    })?;

    Ok(())
}

/// List the schedules for an agent, ordered for stable UI rendering.
pub async fn list_agent_schedules(
    pool: &DbPool,
    agent_key: &str,
) -> Result<Vec<AgenticJobScheduleRow>> {
    let rows = query_as::<_, AgenticJobScheduleRow>(
        "SELECT id,
                agent_key,
                job_key,
                job_kind,
                enabled,
                interval_seconds,
                next_run_at,
                model_provider_id,
                model_id,
                timeout_seconds,
                operator_prompt,
                created_at,
                updated_at
           FROM agentic_job_schedules
          WHERE agent_key = $1
          ORDER BY job_kind, job_key",
    )
    .bind(agent_key)
    .fetch_all(pool)
    .await
    .with_context(|| format!("failed to list schedules for agent {agent_key}"))?;

    Ok(rows)
}

/// Load a single job row for an agent.
pub async fn get_agent_schedule(
    pool: &DbPool,
    agent_key: &str,
    schedule_id: i64,
) -> Result<Option<AgenticJobScheduleRow>> {
    let row = query_as::<_, AgenticJobScheduleRow>(
        "SELECT id,
                agent_key,
                job_key,
                job_kind,
                enabled,
                interval_seconds,
                next_run_at,
                model_provider_id,
                model_id,
                timeout_seconds,
                operator_prompt,
                created_at,
                updated_at
           FROM agentic_job_schedules
          WHERE agent_key = $1
            AND id = $2",
    )
    .bind(agent_key)
    .bind(schedule_id)
    .fetch_optional(pool)
    .await
    .with_context(|| format!("failed to load schedule {schedule_id} for agent {agent_key}"))?;

    Ok(row)
}

pub async fn insert_agent_schedule(
    pool: &DbPool,
    agent_key: &str,
    job_key: &str,
    job_kind: &str,
    enabled: bool,
    interval_seconds: i32,
    next_run_at: DateTime<Utc>,
    model_provider_id: Option<&str>,
    model_id: Option<&str>,
    timeout_seconds: i32,
    operator_prompt: &str,
) -> Result<i64> {
    let row: (i64,) = query_as(
        "INSERT INTO agentic_job_schedules (
            agent_key,
            job_key,
            job_kind,
            enabled,
            interval_seconds,
            next_run_at,
            model_provider_id,
            model_id,
            timeout_seconds,
            operator_prompt
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
         RETURNING id",
    )
    .bind(agent_key)
    .bind(job_key)
    .bind(job_kind)
    .bind(enabled)
    .bind(interval_seconds)
    .bind(next_run_at)
    .bind(model_provider_id)
    .bind(model_id)
    .bind(timeout_seconds)
    .bind(operator_prompt)
    .fetch_one(pool)
    .await
    .with_context(|| format!("failed to insert schedule {job_key} for agent {agent_key}"))?;

    Ok(row.0)
}

/// List the most recent runs for an agent.
pub async fn list_agent_runs(
    pool: &DbPool,
    agent_key: &str,
    limit: i64,
) -> Result<Vec<AgenticRunRow>> {
    let rows = query_as::<_, AgenticRunRow>(
        "SELECT id,
                schedule_id,
                agent_key,
                job_key,
                job_kind,
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
          LIMIT $2",
    )
    .bind(agent_key)
    .bind(limit)
    .fetch_all(pool)
    .await
    .with_context(|| format!("failed to list runs for agent {agent_key}"))?;

    Ok(rows)
}

/// List the most recent runs for a single job.
pub async fn list_schedule_runs(
    pool: &DbPool,
    agent_key: &str,
    schedule_id: i64,
    limit: i64,
) -> Result<Vec<AgenticRunRow>> {
    let rows = query_as::<_, AgenticRunRow>(
        "SELECT id,
                schedule_id,
                agent_key,
                job_key,
                job_kind,
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
            AND schedule_id = $2
          ORDER BY created_at DESC
          LIMIT $3",
    )
    .bind(agent_key)
    .bind(schedule_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .with_context(|| {
        format!("failed to list runs for schedule {schedule_id} agent {agent_key}")
    })?;

    Ok(rows)
}

/// Toggle a single schedule's `enabled` flag.
///
/// Returns `true` when a row was updated, `false` when the (agent_key,
/// schedule_id) pair did not match an existing row.
pub async fn set_schedule_enabled(
    pool: &DbPool,
    agent_key: &str,
    schedule_id: i64,
    enabled: bool,
) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE agentic_job_schedules
            SET enabled = $3,
                updated_at = now()
          WHERE agent_key = $1
            AND id = $2",
    )
    .bind(agent_key)
    .bind(schedule_id)
    .bind(enabled)
    .execute(pool)
    .await
    .with_context(|| format!("failed to toggle schedule {schedule_id} for agent {agent_key}"))?;

    Ok(result.rows_affected() > 0)
}

/// List OpenCode schedules that are due and dispatchable.
///
/// The join is intentionally strict: disabled agents, disabled runtimes,
/// and missing `base_url` values are excluded so the scheduler only sees
/// runs that can succeed.
pub async fn list_due_opencode_schedules(
    pool: &DbPool,
    now: DateTime<Utc>,
    limit: i64,
) -> Result<Vec<DueOpenCodeScheduleRow>> {
    let rows = query_as::<_, DueOpenCodeScheduleRow>(
        "SELECT schedules.id AS schedule_id,
                agents.agent_key,
                agents.display_name,
                schedules.job_key,
                schedules.job_kind,
                schedules.interval_seconds,
                schedules.next_run_at,
                schedules.model_provider_id,
                schedules.model_id,
                schedules.timeout_seconds,
                schedules.operator_prompt,
                agents.runtime_id,
                runtimes.name AS runtime_name,
                runtimes.base_url AS runtime_base_url,
                agents.runtime_config
           FROM agentic_job_schedules AS schedules
           JOIN agents
             ON agents.agent_key = schedules.agent_key
           JOIN agent_runtimes AS runtimes
             ON runtimes.id = agents.runtime_id
          WHERE schedules.enabled = true
            AND schedules.next_run_at <= $1
            AND agents.enabled = true
            AND agents.backend_kind = 'opencode'
            AND runtimes.enabled = true
            AND runtimes.base_url IS NOT NULL
            AND length(runtimes.base_url) > 0
          ORDER BY schedules.next_run_at ASC, schedules.id ASC
          LIMIT $2",
    )
    .bind(now)
    .bind(limit)
    .fetch_all(pool)
    .await
    .context("failed to list due OpenCode schedules")?;

    Ok(rows)
}

/// Load one OpenCode schedule with all metadata required for dispatch.
///
/// This is used by manual `Run now` actions, so it intentionally does not
/// require the schedule itself to be enabled or due.
pub async fn get_opencode_schedule_for_dispatch(
    pool: &DbPool,
    agent_key: &str,
    schedule_id: i64,
) -> Result<Option<DueOpenCodeScheduleRow>> {
    let row = query_as::<_, DueOpenCodeScheduleRow>(
        "SELECT schedules.id AS schedule_id,
                agents.agent_key,
                agents.display_name,
                schedules.job_key,
                schedules.job_kind,
                schedules.interval_seconds,
                schedules.next_run_at,
                schedules.model_provider_id,
                schedules.model_id,
                schedules.timeout_seconds,
                schedules.operator_prompt,
                agents.runtime_id,
                runtimes.name AS runtime_name,
                runtimes.base_url AS runtime_base_url,
                agents.runtime_config
           FROM agentic_job_schedules AS schedules
           JOIN agents
             ON agents.agent_key = schedules.agent_key
           JOIN agent_runtimes AS runtimes
             ON runtimes.id = agents.runtime_id
          WHERE schedules.agent_key = $1
            AND schedules.id = $2
            AND agents.backend_kind = 'opencode'
            AND runtimes.enabled = true
            AND runtimes.base_url IS NOT NULL
            AND length(runtimes.base_url) > 0",
    )
    .bind(agent_key)
    .bind(schedule_id)
    .fetch_optional(pool)
    .await
    .with_context(|| {
        format!(
            "failed to load OpenCode dispatch metadata for schedule {schedule_id} agent {agent_key}"
        )
    })?;

    Ok(row)
}

#[derive(Debug, Clone)]
pub enum ClaimedScheduleRun {
    /// The schedule was due and is now claimed for dispatch. The run is
    /// in `queued` state with the returned id.
    Dispatch { run_id: i64 },
    /// The schedule was due but a previous run was still active. A
    /// `skipped` run row was inserted and returned; no backend dispatch
    /// should happen.
    Skipped { run_id: i64 },
    /// The schedule was no longer due (concurrent claim, disabled,
    /// missing, etc.). No row was written.
    NotDue,
}

/// Atomically advance the schedule, optionally inserting a `queued` or
/// `skipped` `agentic_runs` row, and return the outcome.
///
/// Uses `SELECT ... FOR UPDATE` on the schedule row to serialize claims
/// across scheduler instances.
pub async fn claim_due_schedule(
    pool: &DbPool,
    schedule_id: i64,
    now: DateTime<Utc>,
) -> Result<ClaimedScheduleRun> {
    let mut tx = pool
        .begin()
        .await
        .context("failed to begin claim transaction")?;

    let schedule: Option<ScheduleForUpdate> = query_as(
        "SELECT id,
                agent_key,
                job_key,
                job_kind,
                enabled,
                interval_seconds,
                next_run_at,
                timeout_seconds
           FROM agentic_job_schedules
          WHERE id = $1
          FOR UPDATE",
    )
    .bind(schedule_id)
    .fetch_optional(&mut *tx)
    .await
    .context("failed to lock schedule for claim")?;

    let Some(schedule) = schedule else {
        tx.rollback()
            .await
            .context("failed to roll back missing-schedule claim")?;
        return Ok(ClaimedScheduleRun::NotDue);
    };

    if !schedule.enabled || schedule.next_run_at > now {
        tx.rollback()
            .await
            .context("failed to roll back not-due claim")?;
        return Ok(ClaimedScheduleRun::NotDue);
    }

    let scheduled_for = schedule.next_run_at;
    let next_run_at = now + chrono::Duration::seconds(schedule.interval_seconds as i64);

    let active: Option<(i32,)> = query_as(
        "SELECT 1 FROM agentic_runs
          WHERE schedule_id = $1
            AND status = ANY($2)
          LIMIT 1",
    )
    .bind(schedule.id)
    .bind(&ACTIVE_STATUSES)
    .fetch_optional(&mut *tx)
    .await
    .context("failed to check for active run on schedule")?;

    let outcome = if active.is_some() {
        let error_summary = "previous run still active";
        let run_id = insert_run_in_tx(
            &mut tx,
            Some(schedule.id),
            &schedule.agent_key,
            &schedule.job_key,
            &schedule.job_kind,
            RUN_STATUS_SKIPPED,
            None,
            None,
            None,
            scheduled_for,
            None,
            Some(now),
            schedule.timeout_seconds,
            Some(error_summary),
        )
        .await?;
        ClaimedScheduleRun::Skipped { run_id }
    } else {
        let run_id = insert_run_in_tx(
            &mut tx,
            Some(schedule.id),
            &schedule.agent_key,
            &schedule.job_key,
            &schedule.job_kind,
            RUN_STATUS_QUEUED,
            None,
            None,
            None,
            scheduled_for,
            None,
            None,
            schedule.timeout_seconds,
            None,
        )
        .await?;
        ClaimedScheduleRun::Dispatch { run_id }
    };

    sqlx::query(
        "UPDATE agentic_job_schedules
            SET next_run_at = $2,
                updated_at = now()
          WHERE id = $1",
    )
    .bind(schedule.id)
    .bind(next_run_at)
    .execute(&mut *tx)
    .await
    .context("failed to advance schedule next_run_at")?;

    tx.commit()
        .await
        .context("failed to commit claim transaction")?;

    Ok(outcome)
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct ScheduleForUpdate {
    id: i64,
    agent_key: String,
    job_key: String,
    job_kind: String,
    enabled: bool,
    interval_seconds: i32,
    next_run_at: DateTime<Utc>,
    timeout_seconds: i32,
}

#[allow(clippy::too_many_arguments)]
async fn insert_run_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    schedule_id: Option<i64>,
    agent_key: &str,
    job_key: &str,
    job_kind: &str,
    status: &str,
    backend_run_ref: Option<&str>,
    model_provider_id: Option<&str>,
    model_id: Option<&str>,
    scheduled_for: DateTime<Utc>,
    started_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
    timeout_seconds: i32,
    error_summary: Option<&str>,
) -> Result<i64> {
    let truncated = error_summary.map(truncate_error_summary);
    let row: (i64,) = query_as(
        "INSERT INTO agentic_runs (
            schedule_id,
            agent_key,
            job_key,
            job_kind,
            status,
            backend_run_ref,
            model_provider_id,
            model_id,
            scheduled_for,
            started_at,
            finished_at,
            timeout_seconds,
            error_summary
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
         RETURNING id",
    )
    .bind(schedule_id)
    .bind(agent_key)
    .bind(job_key)
    .bind(job_kind)
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

fn truncate_error_summary(value: &str) -> String {
    if value.chars().count() <= ERROR_SUMMARY_MAX_CHARS {
        return value.to_string();
    }
    let mut out: String = value.chars().take(ERROR_SUMMARY_MAX_CHARS).collect();
    out.push('…');
    out
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
                agent_key,
                job_key,
                job_kind,
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
/// `Run now` actions.
pub async fn insert_queued_run(
    pool: &DbPool,
    agent_key: &str,
    schedule_id: i64,
) -> Result<QueuedScheduleRun> {
    let mut tx = pool
        .begin()
        .await
        .context("failed to begin manual queued run insert")?;

    let schedule: Option<ScheduleForUpdate> = query_as(
        "SELECT id,
                agent_key,
                job_key,
                job_kind,
                enabled,
                interval_seconds,
                next_run_at,
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

    let now = Utc::now();
    let active: Option<(i32,)> = query_as(
        "SELECT 1 FROM agentic_runs
          WHERE schedule_id = $1
            AND status = ANY($2)
          LIMIT 1",
    )
    .bind(schedule.id)
    .bind(&ACTIVE_STATUSES)
    .fetch_optional(&mut *tx)
    .await
    .context("failed to check for active run on manual schedule run")?;

    let outcome = if active.is_some() {
        let run_id = insert_run_in_tx(
            &mut tx,
            Some(schedule.id),
            &schedule.agent_key,
            &schedule.job_key,
            &schedule.job_kind,
            RUN_STATUS_SKIPPED,
            None,
            None,
            None,
            now,
            None,
            Some(now),
            schedule.timeout_seconds,
            Some("previous run still active"),
        )
        .await?;
        QueuedScheduleRun::Skipped { run_id }
    } else {
        let run_id = insert_run_in_tx(
            &mut tx,
            Some(schedule.id),
            &schedule.agent_key,
            &schedule.job_key,
            &schedule.job_kind,
            RUN_STATUS_QUEUED,
            None,
            None,
            None,
            now,
            None,
            None,
            schedule.timeout_seconds,
            None,
        )
        .await?;
        QueuedScheduleRun::Dispatch {
            run_id,
            scheduled_for: now,
        }
    };

    let next_run_at = now + chrono::Duration::seconds(schedule.interval_seconds as i64);
    sqlx::query(
        "UPDATE agentic_job_schedules
            SET next_run_at = $2,
                updated_at = now()
          WHERE id = $1",
    )
    .bind(schedule.id)
    .bind(next_run_at)
    .execute(&mut *tx)
    .await
    .context("failed to advance schedule after manual run")?;

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
    Skipped {
        run_id: i64,
    },
    Missing,
}

/// Small helper to keep the row insert signature in one place for tests.
#[allow(dead_code)]
pub async fn insert_test_run(pool: &PgPool, schedule_id: i64, status: &str) -> Result<i64> {
    let schedule: ScheduleForUpdate = query_as(
        "SELECT id,
                agent_key,
                job_key,
                job_kind,
                enabled,
                interval_seconds,
                next_run_at,
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
            status,
            scheduled_for,
            timeout_seconds
         ) VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING id",
    )
    .bind(schedule.id)
    .bind(&schedule.agent_key)
    .bind(&schedule.job_key)
    .bind(&schedule.job_kind)
    .bind(status)
    .bind(Utc::now())
    .bind(schedule.timeout_seconds)
    .fetch_one(pool)
    .await
    .context("failed to insert test run")?;

    Ok(row.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::Utc;
    use sqlx::query;

    use crate::{
        agents::{
            crypto::{EncryptionKey, encrypt},
            keys::derive_wallet_address,
            model::{AgentRegistryRow, BACKEND_KIND_OPENCODE},
            store::insert_agent,
        },
        test_db,
    };

    fn deterministic_private_key(key: &str) -> String {
        use rand::rngs::StdRng;
        use rand::{Rng, SeedableRng};

        let seed = key
            .bytes()
            .fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
        let mut rng = StdRng::seed_from_u64(seed);
        let bytes: [u8; 32] = rng.r#gen();
        format!("0x{}", hex::encode(bytes))
    }

    fn sample_agent(key: &str) -> AgentRegistryRow {
        let enc = EncryptionKey::new(
            "test",
            [
                0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22,
                23, 24, 25, 26, 27, 28, 29, 30, 31,
            ],
        );
        let private_key = deterministic_private_key(key);
        let ciphertext = encrypt(&enc, &private_key).unwrap();
        let wallet = derive_wallet_address(&private_key).unwrap();
        let now = Utc::now();

        AgentRegistryRow {
            agent_key: key.to_string(),
            created_at: now,
            updated_at: now,
            enabled: true,
            display_name: format!("Test {key}"),
            analysis_prompt: String::new(),
            trading_prompt: String::new(),
            wallet_address: wallet,
            environment: "live".to_string(),
            api_key: format!("vta_{key}"),
            api_key_last_used_at: None,
            backend_kind: BACKEND_KIND_OPENCODE.to_string(),
            runtime_id: "opencode-local".to_string(),
            runtime_config: serde_json::json!({}),
            analysis_context_last_used_at: None,
            trading_context_last_used_at: None,
            hyperliquid_private_key_ciphertext: ciphertext,
            hyperliquid_private_key_key_id: "test".to_string(),
        }
    }

    async fn seed_agent_and_schedule(pool: &DbPool, key: &str, schedule_id_offset: i64) -> i64 {
        insert_agent(pool, &sample_agent(key))
            .await
            .expect("insert agent");
        insert_default_opencode_schedules(pool, key)
            .await
            .expect("insert default schedules");

        if schedule_id_offset > 0 {
            query(
                "UPDATE agentic_job_schedules
                    SET next_run_at = $2
                  WHERE id = $1",
            )
            .bind(schedule_id_offset)
            .bind(Utc::now() + chrono::Duration::seconds(60))
            .execute(pool)
            .await
            .expect("update offset schedule");
        }

        let (id,): (i64,) = query_as(
            "SELECT id FROM agentic_job_schedules
              WHERE agent_key = $1
              ORDER BY id ASC LIMIT 1",
        )
        .bind(key)
        .fetch_one(pool)
        .await
        .expect("fetch schedule id");
        id
    }

    #[tokio::test]
    async fn default_schedules_insert_two_rows_with_expected_defaults() {
        let pool = test_db::pool().await;
        let key = format!(
            "default-sched-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        insert_agent(&pool, &sample_agent(&key))
            .await
            .expect("insert agent");

        insert_default_opencode_schedules(&pool, &key)
            .await
            .expect("insert defaults");

        let rows = list_agent_schedules(&pool, &key)
            .await
            .expect("list schedules");
        assert_eq!(rows.len(), 2);

        let analysis = rows
            .iter()
            .find(|row| row.job_key == "analysis-15m")
            .expect("analysis schedule present");
        assert!(analysis.enabled);
        assert_eq!(analysis.job_kind, JOB_KIND_ANALYSIS);
        assert_eq!(analysis.interval_seconds, 900);
        assert_eq!(analysis.timeout_seconds, 600);

        let trading = rows
            .iter()
            .find(|row| row.job_key == "trading-1m")
            .expect("trading schedule present");
        assert!(!trading.enabled);
        assert_eq!(trading.job_kind, JOB_KIND_TRADING);
        assert_eq!(trading.interval_seconds, 60);
        assert_eq!(trading.timeout_seconds, 45);
    }

    #[tokio::test]
    async fn default_schedules_are_idempotent() {
        let pool = test_db::pool().await;
        let key = format!(
            "idempotent-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        insert_agent(&pool, &sample_agent(&key))
            .await
            .expect("insert agent");

        insert_default_opencode_schedules(&pool, &key)
            .await
            .expect("first insert");
        insert_default_opencode_schedules(&pool, &key)
            .await
            .expect("second insert");

        let count: (i64,) =
            query_as("SELECT COUNT(*) FROM agentic_job_schedules WHERE agent_key = $1")
                .bind(&key)
                .fetch_one(&pool)
                .await
                .expect("count");
        assert_eq!(count.0, 2);
    }

    #[tokio::test]
    async fn insert_agent_schedule_persists_custom_schedule() {
        let pool = test_db::pool().await;
        let key = format!(
            "custom-sched-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        insert_agent(&pool, &sample_agent(&key))
            .await
            .expect("insert agent");

        let next_run_at = Utc::now();
        let schedule_id = insert_agent_schedule(
            &pool,
            &key,
            "analysis-1h",
            JOB_KIND_ANALYSIS,
            true,
            3600,
            next_run_at,
            Some("anthropic"),
            Some("claude-sonnet-4"),
            600,
            "Check higher timeframe structure.",
        )
        .await
        .expect("insert schedule");

        let rows = list_agent_schedules(&pool, &key)
            .await
            .expect("list schedules");
        let row = rows
            .iter()
            .find(|row| row.id == schedule_id)
            .expect("inserted schedule present");
        assert_eq!(row.job_key, "analysis-1h");
        assert_eq!(row.job_kind, JOB_KIND_ANALYSIS);
        assert!(row.enabled);
        assert_eq!(row.interval_seconds, 3600);
        assert_eq!(row.timeout_seconds, 600);
        assert_eq!(row.model_provider_id.as_deref(), Some("anthropic"));
        assert_eq!(row.model_id.as_deref(), Some("claude-sonnet-4"));
        assert_eq!(row.operator_prompt, "Check higher timeframe structure.");
    }

    #[tokio::test]
    async fn list_due_opencode_schedules_excludes_disabled_and_other_backends() {
        let pool = test_db::pool().await;
        let key = format!("due-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0));
        insert_agent(&pool, &sample_agent(&key))
            .await
            .expect("insert agent");
        insert_default_opencode_schedules(&pool, &key)
            .await
            .expect("defaults");

        // Both schedules start enabled and due (next_run_at = now()).
        // We disable analysis and bump trading into the future, then expect
        // only the analysis schedule to appear.
        query("UPDATE agentic_job_schedules SET enabled = false WHERE job_key = 'analysis-15m'")
            .execute(&pool)
            .await
            .expect("disable analysis");
        let future = Utc::now() + chrono::Duration::seconds(3600);
        query("UPDATE agentic_job_schedules SET next_run_at = $1 WHERE job_key = 'trading-1m'")
            .bind(future)
            .execute(&pool)
            .await
            .expect("push trading future");
        query("UPDATE agentic_job_schedules SET enabled = true WHERE job_key = 'trading-1m'")
            .execute(&pool)
            .await
            .expect("enable trading");

        let now = Utc::now();
        let due = list_due_opencode_schedules(&pool, now, 20)
            .await
            .expect("list due");

        // The result contains whatever the seeded opencode-local runtime
        // plus possibly other agents in this test DB. We only assert that
        // this specific agent contributes zero rows because both schedules
        // are either disabled or in the future.
        assert!(
            due.iter().all(|row| row.agent_key != key),
            "expected no rows for disabled/future agent, got {due:?}"
        );

        // Now make the analysis schedule due and enabled, and expect one row.
        query("UPDATE agentic_job_schedules SET enabled = true, next_run_at = $1 WHERE job_key = 'analysis-15m'")
            .bind(now - chrono::Duration::seconds(1))
            .execute(&pool)
            .await
            .expect("re-enable analysis due");
        let due = list_due_opencode_schedules(&pool, now, 20)
            .await
            .expect("list due again");
        let analysis_due = due
            .iter()
            .find(|row| row.agent_key == key && row.job_key == "analysis-15m")
            .expect("analysis row should be due");
        assert!(analysis_due.runtime_base_url.contains("14096"));
    }

    #[tokio::test]
    async fn list_due_opencode_schedules_filters_by_agent_enabled_flag() {
        let pool = test_db::pool().await;
        let key = format!(
            "agent-disabled-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        insert_agent(&pool, &sample_agent(&key))
            .await
            .expect("insert agent");
        insert_default_opencode_schedules(&pool, &key)
            .await
            .expect("defaults");

        // Push the analysis schedule into the past, but disable the agent.
        let past = Utc::now() - chrono::Duration::seconds(60);
        query("UPDATE agentic_job_schedules SET enabled = true, next_run_at = $1 WHERE job_key = 'analysis-15m'")
            .bind(past)
            .execute(&pool)
            .await
            .expect("set analysis due");
        query("UPDATE agents SET enabled = false WHERE agent_key = $1")
            .bind(&key)
            .execute(&pool)
            .await
            .expect("disable agent");

        let now = Utc::now();
        let due = list_due_opencode_schedules(&pool, now, 20)
            .await
            .expect("list due");
        assert!(
            due.iter().all(|row| row.agent_key != key),
            "disabled parent agent should be excluded"
        );
    }

    #[tokio::test]
    async fn claim_due_schedule_inserts_queued_run_and_advances_next_run_at() {
        let pool = test_db::pool().await;
        let key = format!("claim-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0));
        let schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;

        // Set the schedule to be due.
        let now = Utc::now();
        query("UPDATE agentic_job_schedules SET next_run_at = $1 WHERE id = $2")
            .bind(now - chrono::Duration::seconds(5))
            .bind(schedule_id)
            .execute(&pool)
            .await
            .expect("set due");

        let outcome = claim_due_schedule(&pool, schedule_id, now)
            .await
            .expect("claim");
        let run_id = match outcome {
            ClaimedScheduleRun::Dispatch { run_id } => run_id,
            other => panic!("expected Dispatch, got {other:?}"),
        };

        let run = get_run(&pool, run_id)
            .await
            .expect("fetch run")
            .expect("run present");
        assert_eq!(run.status, RUN_STATUS_QUEUED);
        assert_eq!(run.agent_key, key);

        // The next_run_at should have advanced by interval_seconds.
        let (next_run_at,): (DateTime<Utc>,) =
            query_as("SELECT next_run_at FROM agentic_job_schedules WHERE id = $1")
                .bind(schedule_id)
                .fetch_one(&pool)
                .await
                .expect("fetch next_run_at");
        let expected = now + chrono::Duration::seconds(900);
        // Allow a 2s slack for clock drift between now snapshots.
        let delta = (next_run_at - expected).num_seconds().abs();
        assert!(delta <= 2, "next_run_at delta too large: {delta}s");
    }

    #[tokio::test]
    async fn claim_due_schedule_inserts_skipped_run_when_active_run_exists() {
        let pool = test_db::pool().await;
        let key = format!("skip-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0));
        let schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;

        // Set the schedule to be due, then insert a fake running run.
        let now = Utc::now();
        query("UPDATE agentic_job_schedules SET next_run_at = $1 WHERE id = $2")
            .bind(now - chrono::Duration::seconds(5))
            .bind(schedule_id)
            .execute(&pool)
            .await
            .expect("set due");
        insert_test_run(&pool, schedule_id, RUN_STATUS_RUNNING)
            .await
            .expect("seed active run");

        let outcome = claim_due_schedule(&pool, schedule_id, now)
            .await
            .expect("claim");
        let run_id = match outcome {
            ClaimedScheduleRun::Skipped { run_id } => run_id,
            other => panic!("expected Skipped, got {other:?}"),
        };

        let run = get_run(&pool, run_id)
            .await
            .expect("fetch run")
            .expect("run present");
        assert_eq!(run.status, RUN_STATUS_SKIPPED);
        assert_eq!(
            run.error_summary.as_deref(),
            Some("previous run still active")
        );
        assert!(run.finished_at.is_some());
    }

    #[tokio::test]
    async fn claim_due_schedule_returns_not_due_when_schedule_disabled() {
        let pool = test_db::pool().await;
        let key = format!("notdue-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0));
        let schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;

        let now = Utc::now();
        query("UPDATE agentic_job_schedules SET enabled = false, next_run_at = $1 WHERE id = $2")
            .bind(now - chrono::Duration::seconds(5))
            .bind(schedule_id)
            .execute(&pool)
            .await
            .expect("disable");

        let outcome = claim_due_schedule(&pool, schedule_id, now)
            .await
            .expect("claim");
        assert!(matches!(outcome, ClaimedScheduleRun::NotDue));
    }

    #[tokio::test]
    async fn insert_queued_run_inserts_manual_dispatch_run_and_advances_next_run_at() {
        let pool = test_db::pool().await;
        let key = format!("manual-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0));
        let schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;

        let before = Utc::now();
        let outcome = insert_queued_run(&pool, &key, schedule_id)
            .await
            .expect("manual run");
        let (run_id, scheduled_for) = match outcome {
            QueuedScheduleRun::Dispatch {
                run_id,
                scheduled_for,
            } => (run_id, scheduled_for),
            other => panic!("expected Dispatch, got {other:?}"),
        };
        let after = Utc::now();

        let run = get_run(&pool, run_id)
            .await
            .expect("fetch run")
            .expect("run present");
        assert_eq!(run.status, RUN_STATUS_QUEUED);
        assert!(run.scheduled_for >= before);
        assert!(run.scheduled_for <= after);
        let delta = (run.scheduled_for - scheduled_for)
            .num_microseconds()
            .unwrap_or(i64::MAX);
        assert!(delta.abs() <= 1, "scheduled_for delta too large: {delta}us");

        let (next_run_at,): (DateTime<Utc>,) =
            query_as("SELECT next_run_at FROM agentic_job_schedules WHERE id = $1")
                .bind(schedule_id)
                .fetch_one(&pool)
                .await
                .expect("fetch next_run_at");
        assert!(next_run_at >= scheduled_for + chrono::Duration::seconds(899));
    }

    #[tokio::test]
    async fn insert_queued_run_inserts_skipped_run_when_previous_run_is_active() {
        let pool = test_db::pool().await;
        let key = format!(
            "manual-skip-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        let schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;
        insert_test_run(&pool, schedule_id, RUN_STATUS_RUNNING)
            .await
            .expect("seed active run");

        let outcome = insert_queued_run(&pool, &key, schedule_id)
            .await
            .expect("manual run");
        let run_id = match outcome {
            QueuedScheduleRun::Skipped { run_id } => run_id,
            other => panic!("expected Skipped, got {other:?}"),
        };

        let run = get_run(&pool, run_id)
            .await
            .expect("fetch run")
            .expect("run present");
        assert_eq!(run.status, RUN_STATUS_SKIPPED);
        assert_eq!(
            run.error_summary.as_deref(),
            Some("previous run still active")
        );
        assert!(run.finished_at.is_some());
    }

    #[tokio::test]
    async fn mark_run_running_sets_status_and_started_at() {
        let pool = test_db::pool().await;
        let key = format!(
            "run-running-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        let schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;
        let run_id = insert_test_run(&pool, schedule_id, RUN_STATUS_QUEUED)
            .await
            .expect("seed run");

        let updated = mark_run_running(&pool, run_id, Some("ses_abc"))
            .await
            .expect("mark running");
        assert!(updated);

        let run = get_run(&pool, run_id)
            .await
            .expect("fetch run")
            .expect("run present");
        assert_eq!(run.status, RUN_STATUS_RUNNING);
        assert!(run.started_at.is_some());
        assert_eq!(run.backend_run_ref.as_deref(), Some("ses_abc"));
    }

    #[tokio::test]
    async fn mark_run_succeeded_records_finished_at_and_backend_ref() {
        let pool = test_db::pool().await;
        let key = format!("run-ok-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0));
        let schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;
        let run_id = insert_test_run(&pool, schedule_id, RUN_STATUS_RUNNING)
            .await
            .expect("seed run");

        let updated = mark_run_succeeded(&pool, run_id, Some("ses_xyz"))
            .await
            .expect("mark succeeded");
        assert!(updated);

        let run = get_run(&pool, run_id)
            .await
            .expect("fetch run")
            .expect("run present");
        assert_eq!(run.status, RUN_STATUS_SUCCEEDED);
        assert!(run.finished_at.is_some());
        assert_eq!(run.backend_run_ref.as_deref(), Some("ses_xyz"));
    }

    #[tokio::test]
    async fn mark_run_failed_truncates_error_summary() {
        let pool = test_db::pool().await;
        let key = format!("run-fail-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0));
        let schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;
        let run_id = insert_test_run(&pool, schedule_id, RUN_STATUS_RUNNING)
            .await
            .expect("seed run");

        let huge = "x".repeat(ERROR_SUMMARY_MAX_CHARS + 100);
        let updated = mark_run_failed(&pool, run_id, &huge, Some("ses_zzz"))
            .await
            .expect("mark failed");
        assert!(updated);

        let run = get_run(&pool, run_id)
            .await
            .expect("fetch run")
            .expect("run present");
        assert_eq!(run.status, RUN_STATUS_FAILED);
        assert!(run.finished_at.is_some());
        let summary = run.error_summary.expect("error summary");
        assert!(summary.chars().count() <= ERROR_SUMMARY_MAX_CHARS + 1);
        assert!(summary.ends_with('…'));
    }

    #[tokio::test]
    async fn mark_run_aborted_marks_status_finished_at() {
        let pool = test_db::pool().await;
        let key = format!(
            "run-abort-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        let schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;
        let run_id = insert_test_run(&pool, schedule_id, RUN_STATUS_RUNNING)
            .await
            .expect("seed run");

        let updated = mark_run_aborted(&pool, run_id, "timeout", None)
            .await
            .expect("mark aborted");
        assert!(updated);

        let run = get_run(&pool, run_id)
            .await
            .expect("fetch run")
            .expect("run present");
        assert_eq!(run.status, RUN_STATUS_ABORTED);
        assert!(run.finished_at.is_some());
    }
}
