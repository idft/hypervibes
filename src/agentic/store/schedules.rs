use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sqlx::{Postgres, Transaction, query_as};

use crate::{
    agentic::{
        job_key::build_generated_job_key,
        model::{
            AgenticJobScheduleRow, AgenticRunRow, DueOpenCodeScheduleRow, JOB_KIND_ANALYSIS,
            JOB_KIND_DAILY_REVIEW, JOB_KIND_TRADING, RUN_STATUS_QUEUED, RUN_STATUS_SKIPPED,
        },
        timeframe::{
            DEFAULT_TRIGGER_DELAY_SECONDS, boundary_for_due_at, latest_due_at_or_before,
            next_due_after, parse_timeframe_seconds,
        },
    },
    db::DbPool,
};

use super::common::{insert_run_in_tx, lock_agent_coordination_tx};
use super::hooks::insert_default_opencode_hooks;
use super::recovery::has_active_run_in_lane_tx;
use super::workspace::agent_has_blocking_workspace_maintenance_tx;

#[cfg(test)]
pub(crate) const DEFAULT_ANALYSIS_TIMEFRAME: &str = "15m";
const DEFAULT_ANALYSIS_TIMEFRAMES: [&str; 3] = ["15m", "1h", "1d"];
pub(crate) const DEFAULT_TRADING_TIMEFRAME: &str = "1m";
pub(crate) const DEFAULT_DAILY_REVIEW_TIMEFRAME: &str = "1d";
pub(crate) const DEFAULT_ANALYSIS_TIMEOUT_SECONDS: i32 = 900;
pub(crate) const DEFAULT_TRADING_TIMEOUT_SECONDS: i32 = 900;
pub(crate) const DEFAULT_DAILY_REVIEW_TIMEOUT_SECONDS: i32 = 900;

#[cfg(test)]
pub(crate) fn default_analysis_job_key() -> String {
    build_generated_job_key(JOB_KIND_ANALYSIS, DEFAULT_ANALYSIS_TIMEFRAME)
}

#[cfg(test)]
pub(crate) fn default_trading_job_key() -> String {
    build_generated_job_key(JOB_KIND_TRADING, DEFAULT_TRADING_TIMEFRAME)
}

/// Seed the canonical default schedules and hook for a newly-created OpenCode agent.
///
/// This is idempotent: existing `(agent_key, job_kind, timeframe)` rows
/// are left untouched, and the default market-analysis hook is inserted only
/// when it does not already exist. New agents always get disabled
/// `analysis-15m`, `analysis-1h`, `analysis-1d`, and `trading-1m` rows plus
/// a disabled `market-analysis` hook.
pub async fn insert_default_opencode_schedules(pool: &DbPool, agent_key: &str) -> Result<()> {
    for timeframe in DEFAULT_ANALYSIS_TIMEFRAMES {
        insert_default_opencode_schedule(
            pool,
            agent_key,
            JOB_KIND_ANALYSIS,
            timeframe,
            false,
            DEFAULT_ANALYSIS_TIMEOUT_SECONDS,
        )
        .await?;
    }

    insert_default_opencode_schedule(
        pool,
        agent_key,
        JOB_KIND_TRADING,
        DEFAULT_TRADING_TIMEFRAME,
        false,
        DEFAULT_TRADING_TIMEOUT_SECONDS,
    )
    .await?;

    insert_default_opencode_schedule(
        pool,
        agent_key,
        JOB_KIND_DAILY_REVIEW,
        DEFAULT_DAILY_REVIEW_TIMEFRAME,
        false,
        DEFAULT_DAILY_REVIEW_TIMEOUT_SECONDS,
    )
    .await?;

    insert_default_opencode_hooks(pool, agent_key).await?;

    Ok(())
}

async fn insert_default_opencode_schedule(
    pool: &DbPool,
    agent_key: &str,
    job_kind: &str,
    timeframe: &str,
    enabled: bool,
    timeout_seconds: i32,
) -> Result<()> {
    let job_key = build_generated_job_key(job_kind, timeframe);
    let now = Utc::now();
    let next_run_at = next_due_after(now, timeframe, DEFAULT_TRIGGER_DELAY_SECONDS)
        .with_context(|| format!("invalid default timeframe {timeframe:?}"))?;

    sqlx::query(
        "INSERT INTO agentic_job_schedules (
            agent_key,
            job_key,
            job_kind,
            enabled,
            timeframe,
            trigger_delay_seconds,
            next_run_at,
            timeout_seconds,
            operator_prompt
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         ON CONFLICT (agent_key, job_kind, timeframe) DO NOTHING",
    )
    .bind(agent_key)
    .bind(&job_key)
    .bind(job_kind)
    .bind(enabled)
    .bind(timeframe)
    .bind(DEFAULT_TRIGGER_DELAY_SECONDS)
    .bind(next_run_at)
    .bind(timeout_seconds)
    .bind("")
    .execute(pool)
    .await
    .with_context(|| {
        format!("failed to insert default {job_key} schedule for agent {agent_key}")
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
                timeframe,
                trigger_delay_seconds,
                next_run_at,
                model_provider_id,
                model_id,
                timeout_seconds,
                operator_prompt,
                created_at,
                updated_at
           FROM agentic_job_schedules
          WHERE agent_key = $1
          ORDER BY job_kind, timeframe, job_key",
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
                timeframe,
                trigger_delay_seconds,
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

/// Insert a new schedule. The `job_key` is generated from
/// `job_kind` + `timeframe` and the schedule's first `next_run_at` is
/// computed from the same timeframe and trigger delay.
#[allow(clippy::too_many_arguments)]
pub async fn insert_agent_schedule(
    pool: &DbPool,
    agent_key: &str,
    job_kind: &str,
    enabled: bool,
    timeframe: &str,
    trigger_delay_seconds: i32,
    model_provider_id: Option<&str>,
    model_id: Option<&str>,
    timeout_seconds: i32,
    operator_prompt: &str,
) -> Result<i64> {
    parse_timeframe_seconds(timeframe)
        .with_context(|| format!("invalid timeframe {timeframe:?}"))?;
    if trigger_delay_seconds < 0 {
        return Err(anyhow::anyhow!(
            "trigger_delay_seconds must be non-negative"
        ));
    }

    let job_key = build_generated_job_key(job_kind, timeframe);
    let now = Utc::now();
    let next_run_at = next_due_after(now, timeframe, trigger_delay_seconds)
        .with_context(|| format!("failed to compute next_run_at for {timeframe:?}"))?;

    let row: (i64,) = query_as(
        "INSERT INTO agentic_job_schedules (
            agent_key,
            job_key,
            job_kind,
            enabled,
            timeframe,
            trigger_delay_seconds,
            next_run_at,
            model_provider_id,
            model_id,
            timeout_seconds,
            operator_prompt
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
         RETURNING id",
    )
    .bind(agent_key)
    .bind(&job_key)
    .bind(job_kind)
    .bind(enabled)
    .bind(timeframe)
    .bind(trigger_delay_seconds)
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

pub async fn delete_agent_schedule(
    pool: &DbPool,
    agent_key: &str,
    schedule_id: i64,
) -> Result<bool> {
    let result = sqlx::query(
        "DELETE FROM agentic_job_schedules
          WHERE agent_key = $1
            AND id = $2",
    )
    .bind(agent_key)
    .bind(schedule_id)
    .execute(pool)
    .await
    .with_context(|| format!("failed to delete schedule {schedule_id} for agent {agent_key}"))?;

    Ok(result.rows_affected() > 0)
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
            AND schedule_id = $2
          ORDER BY created_at DESC
          LIMIT $3",
    )
    .bind(agent_key)
    .bind(schedule_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .with_context(|| format!("failed to list runs for schedule {schedule_id} agent {agent_key}"))?;

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
            AND id = $2
            AND ($3 = false OR (model_provider_id IS NOT NULL AND model_id IS NOT NULL))",
    )
    .bind(agent_key)
    .bind(schedule_id)
    .bind(enabled)
    .execute(pool)
    .await
    .with_context(|| format!("failed to toggle schedule {schedule_id} for agent {agent_key}"))?;

    Ok(result.rows_affected() > 0)
}

pub async fn set_schedule_model(
    pool: &DbPool,
    agent_key: &str,
    schedule_id: i64,
    model_provider_id: Option<&str>,
    model_id: Option<&str>,
) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE agentic_job_schedules
            SET model_provider_id = $3,
                model_id = $4,
                updated_at = now()
          WHERE agent_key = $1
            AND id = $2",
    )
    .bind(agent_key)
    .bind(schedule_id)
    .bind(model_provider_id)
    .bind(model_id)
    .execute(pool)
    .await
    .with_context(|| {
        format!("failed to update model for schedule {schedule_id} agent {agent_key}")
    })?;

    Ok(result.rows_affected() > 0)
}

pub async fn set_schedule_timeout(
    pool: &DbPool,
    agent_key: &str,
    schedule_id: i64,
    timeout_seconds: i32,
) -> Result<bool> {
    if timeout_seconds <= 0 {
        anyhow::bail!("timeout_seconds must be positive, got {timeout_seconds}");
    }
    let result = sqlx::query(
        "UPDATE agentic_job_schedules
            SET timeout_seconds = $3,
                updated_at = now()
          WHERE agent_key = $1
            AND id = $2",
    )
    .bind(agent_key)
    .bind(schedule_id)
    .bind(timeout_seconds)
    .execute(pool)
    .await
    .with_context(|| {
        format!("failed to update timeout for schedule {schedule_id} agent {agent_key}")
    })?;

    Ok(result.rows_affected() > 0)
}

/// Change a schedule's timeframe and re-anchor its next run to the next
/// boundary for that timeframe. Keeping these fields together prevents an
/// edited schedule from firing at a boundary from its previous cadence.
pub async fn set_schedule_timeframe(
    pool: &DbPool,
    agent_key: &str,
    schedule_id: i64,
    timeframe: &str,
) -> Result<bool> {
    let timeframe = timeframe.trim();
    parse_timeframe_seconds(timeframe)
        .with_context(|| format!("invalid timeframe {timeframe:?}"))?;

    let mut tx = pool
        .begin()
        .await
        .context("failed to begin schedule timeframe update transaction")?;
    lock_agent_coordination_tx(&mut tx, agent_key).await?;
    let schedule: Option<(String, i32)> = query_as(
        "SELECT job_kind, trigger_delay_seconds
           FROM agentic_job_schedules
          WHERE agent_key = $1
            AND id = $2
          FOR UPDATE",
    )
    .bind(agent_key)
    .bind(schedule_id)
    .fetch_optional(&mut *tx)
    .await
    .with_context(|| format!("failed to lock schedule {schedule_id} for agent {agent_key}"))?;

    let Some((job_kind, trigger_delay_seconds)) = schedule else {
        tx.rollback()
            .await
            .context("failed to roll back missing schedule timeframe update")?;
        return Ok(false);
    };

    let next_run_at = next_due_after(Utc::now(), timeframe, trigger_delay_seconds)?;
    let job_key = build_generated_job_key(&job_kind, timeframe);
    sqlx::query(
        "UPDATE agentic_job_schedules
            SET job_key = $3,
                timeframe = $4,
                next_run_at = $5,
                updated_at = now()
          WHERE agent_key = $1
            AND id = $2",
    )
    .bind(agent_key)
    .bind(schedule_id)
    .bind(&job_key)
    .bind(timeframe)
    .bind(next_run_at)
    .execute(&mut *tx)
    .await
    .with_context(|| {
        format!("failed to update timeframe for schedule {schedule_id} agent {agent_key}")
    })?;
    tx.commit()
        .await
        .context("failed to commit schedule timeframe update")?;

    Ok(true)
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
                schedules.timeframe,
                schedules.trigger_delay_seconds,
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
              AND agents.lifecycle = 'active'
               AND agents.backend_kind = 'opencode'
               AND agents.lifecycle = 'active'
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
                schedules.timeframe,
                schedules.trigger_delay_seconds,
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
    /// The schedule was due but a previous run for the same agent was
    /// still active. A `skipped` run row was inserted and returned; no
    /// backend dispatch should happen.
    Skipped { run_id: i64 },
    /// The schedule was no longer due (concurrent claim, disabled,
    /// missing, etc.). No row was written.
    NotDue,
    /// The schedule remained due, but queued/running workspace maintenance
    /// prevents dispatch until the agent is available again.
    BlockedByMaintenance,
}

/// Atomically advance the schedule, optionally inserting a `queued` or
/// `skipped` `agentic_runs` row, and return the outcome.
///
/// The schedule fires on UTC candle boundaries (per its `timeframe`)
/// plus a fixed `trigger_delay_seconds` of slack. When the schedule is
/// overdue after downtime, the next future boundary is computed
/// without replaying missed runs.
pub async fn claim_due_schedule(
    pool: &DbPool,
    schedule_id: i64,
    now: DateTime<Utc>,
) -> Result<ClaimedScheduleRun> {
    let mut tx = pool
        .begin()
        .await
        .context("failed to begin claim transaction")?;

    let agent_key: Option<(String,)> =
        query_as("SELECT agent_key FROM agentic_job_schedules WHERE id = $1")
            .bind(schedule_id)
            .fetch_optional(&mut *tx)
            .await
            .context("failed to load schedule agent for claim")?;
    let Some((agent_key,)) = agent_key else {
        tx.rollback()
            .await
            .context("failed to roll back missing-schedule claim")?;
        return Ok(ClaimedScheduleRun::NotDue);
    };
    lock_agent_coordination_tx(&mut tx, &agent_key).await?;

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

    let timeframe = schedule.timeframe.clone();
    let trigger_delay_seconds = schedule.trigger_delay_seconds;

    let latest_due = match latest_due_at_or_before(now, &timeframe, trigger_delay_seconds)
        .with_context(|| {
            format!(
                "invalid timeframe {timeframe:?} on schedule {}",
                schedule.id
            )
        })? {
        Some(latest) => latest,
        None => {
            // We are not yet at the first valid due instant. Skip and
            // re-anchor to the next future boundary.
            advance_schedule(&mut tx, &schedule, now).await?;
            tx.commit()
                .await
                .context("failed to commit early-skip claim")?;
            return Ok(ClaimedScheduleRun::NotDue);
        }
    };

    if schedule.next_run_at < latest_due {
        // Schedule is stale after downtime. Skip and re-anchor.
        advance_schedule(&mut tx, &schedule, now).await?;
        tx.commit()
            .await
            .context("failed to commit stale-skip claim")?;
        return Ok(ClaimedScheduleRun::NotDue);
    }

    if agent_has_blocking_workspace_maintenance_tx(&mut tx, &schedule.agent_key).await? {
        tx.rollback()
            .await
            .context("failed to roll back maintenance-blocked claim")?;
        return Ok(ClaimedScheduleRun::BlockedByMaintenance);
    }

    let scheduled_for = boundary_for_due_at(schedule.next_run_at, trigger_delay_seconds);

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
            Some(&timeframe),
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
        ClaimedScheduleRun::Skipped { run_id }
    } else {
        let run_id = insert_run_in_tx(
            &mut tx,
            Some(schedule.id),
            None,
            &schedule.agent_key,
            &schedule.job_key,
            &schedule.job_kind,
            Some(&timeframe),
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
        ClaimedScheduleRun::Dispatch { run_id }
    };

    advance_schedule(&mut tx, &schedule, now).await?;

    tx.commit()
        .await
        .context("failed to commit claim transaction")?;

    Ok(outcome)
}

async fn advance_schedule(
    tx: &mut Transaction<'_, Postgres>,
    schedule: &ScheduleForUpdate,
    now: DateTime<Utc>,
) -> Result<()> {
    let next_run_at = next_due_after(now, &schedule.timeframe, schedule.trigger_delay_seconds)
        .with_context(|| {
            format!(
                "failed to compute next_run_at for timeframe {:?} on schedule {}",
                schedule.timeframe, schedule.id
            )
        })?;
    sqlx::query(
        "UPDATE agentic_job_schedules
            SET next_run_at = $2,
                updated_at = now()
          WHERE id = $1",
    )
    .bind(schedule.id)
    .bind(next_run_at)
    .execute(&mut **tx)
    .await
    .context("failed to advance schedule next_run_at")?;
    Ok(())
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct ScheduleForUpdate {
    pub(crate) id: i64,
    pub(crate) agent_key: String,
    pub(crate) job_key: String,
    pub(crate) job_kind: String,
    pub(crate) enabled: bool,
    pub(crate) timeframe: String,
    pub(crate) trigger_delay_seconds: i32,
    pub(crate) next_run_at: DateTime<Utc>,
    pub(crate) model_provider_id: Option<String>,
    pub(crate) model_id: Option<String>,
    pub(crate) timeout_seconds: i32,
}
