use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction, query_as};

use crate::{
    agentic::{
        job_key::{build_generated_hook_job_key, build_generated_job_key},
        model::{
            AgenticJobHookRow, AgenticJobScheduleRow, AgenticRunRow, DueOpenCodeHookRow,
            DueOpenCodeScheduleRow, HOOK_EVENT_ANALYSIS_BATCH_COMPLETED, JOB_KIND_ANALYSIS,
            JOB_KIND_MARKET_ANALYSIS, JOB_KIND_TRADING, RUN_STATUS_ABORTED, RUN_STATUS_FAILED,
            RUN_STATUS_QUEUED, RUN_STATUS_RUNNING, RUN_STATUS_SKIPPED, RUN_STATUS_SUCCEEDED,
        },
        timeframe::{
            DEFAULT_TRIGGER_DELAY_SECONDS, boundary_for_due_at, latest_due_at_or_before,
            next_due_after, parse_timeframe_seconds,
        },
    },
    db::DbPool,
};

const ERROR_SUMMARY_MAX_CHARS: usize = 500;
const ACTIVE_STATUSES: [&str; 2] = [RUN_STATUS_QUEUED, RUN_STATUS_RUNNING];

const DEFAULT_ANALYSIS_TIMEFRAME: &str = "15m";
const DEFAULT_ANALYSIS_TIMEFRAMES: [&str; 3] = ["15m", "1h", "1d"];
const DEFAULT_TRADING_TIMEFRAME: &str = "1m";
const DEFAULT_ANALYSIS_TIMEOUT_SECONDS: i32 = 900;
const DEFAULT_TRADING_TIMEOUT_SECONDS: i32 = 900;
const DEFAULT_MARKET_ANALYSIS_TIMEOUT_SECONDS: i32 = 900;

fn active_job_kinds_for_lane(job_kind: &str) -> &'static [&'static str] {
    match job_kind {
        JOB_KIND_TRADING => &[JOB_KIND_TRADING],
        JOB_KIND_ANALYSIS | JOB_KIND_MARKET_ANALYSIS => {
            &[JOB_KIND_ANALYSIS, JOB_KIND_MARKET_ANALYSIS]
        }
        _ => &[],
    }
}

async fn has_active_run_in_lane_tx(
    tx: &mut Transaction<'_, Postgres>,
    agent_key: &str,
    job_kind: &str,
) -> Result<bool> {
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

fn default_analysis_job_key() -> String {
    build_generated_job_key(JOB_KIND_ANALYSIS, DEFAULT_ANALYSIS_TIMEFRAME)
}

fn default_trading_job_key() -> String {
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

    insert_default_opencode_hook(pool, agent_key).await?;

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

async fn insert_default_opencode_hook(pool: &DbPool, agent_key: &str) -> Result<()> {
    let job_key = build_generated_hook_job_key(JOB_KIND_MARKET_ANALYSIS);

    sqlx::query(
        "INSERT INTO agentic_job_hooks (
            agent_key,
            job_key,
            job_kind,
            hook_event,
            enabled,
            timeout_seconds,
            operator_prompt
         ) VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (agent_key, job_kind, hook_event) DO NOTHING",
    )
    .bind(agent_key)
    .bind(&job_key)
    .bind(JOB_KIND_MARKET_ANALYSIS)
    .bind(HOOK_EVENT_ANALYSIS_BATCH_COMPLETED)
    .bind(false)
    .bind(DEFAULT_MARKET_ANALYSIS_TIMEOUT_SECONDS)
    .bind("")
    .execute(pool)
    .await
    .with_context(|| format!("failed to insert default {job_key} hook for agent {agent_key}"))?;

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

pub async fn list_agent_hooks(pool: &DbPool, agent_key: &str) -> Result<Vec<AgenticJobHookRow>> {
    let rows = query_as::<_, AgenticJobHookRow>(
        "SELECT id,
                agent_key,
                job_key,
                job_kind,
                hook_event,
                enabled,
                model_provider_id,
                model_id,
                timeout_seconds,
                operator_prompt,
                created_at,
                updated_at
           FROM agentic_job_hooks
          WHERE agent_key = $1
          ORDER BY hook_event, job_key",
    )
    .bind(agent_key)
    .fetch_all(pool)
    .await
    .with_context(|| format!("failed to list hooks for agent {agent_key}"))?;

    Ok(rows)
}

pub async fn get_agent_hook(
    pool: &DbPool,
    agent_key: &str,
    hook_id: i64,
) -> Result<Option<AgenticJobHookRow>> {
    let row = query_as::<_, AgenticJobHookRow>(
        "SELECT id,
                agent_key,
                job_key,
                job_kind,
                hook_event,
                enabled,
                model_provider_id,
                model_id,
                timeout_seconds,
                operator_prompt,
                created_at,
                updated_at
           FROM agentic_job_hooks
          WHERE agent_key = $1
            AND id = $2",
    )
    .bind(agent_key)
    .bind(hook_id)
    .fetch_optional(pool)
    .await
    .with_context(|| format!("failed to load hook {hook_id} for agent {agent_key}"))?;

    Ok(row)
}

pub async fn get_enabled_hook_for_event(
    pool: &DbPool,
    agent_key: &str,
    hook_event: &str,
) -> Result<Option<AgenticJobHookRow>> {
    let row = query_as::<_, AgenticJobHookRow>(
        "SELECT id,
                agent_key,
                job_key,
                job_kind,
                hook_event,
                enabled,
                model_provider_id,
                model_id,
                timeout_seconds,
                operator_prompt,
                created_at,
                updated_at
           FROM agentic_job_hooks
          WHERE agent_key = $1
            AND hook_event = $2
            AND enabled = true",
    )
    .bind(agent_key)
    .bind(hook_event)
    .fetch_optional(pool)
    .await
    .with_context(|| format!("failed to load enabled hook {hook_event} for agent {agent_key}"))?;

    Ok(row)
}

pub async fn insert_agent_hook(
    pool: &DbPool,
    agent_key: &str,
    job_kind: &str,
    hook_event: &str,
    enabled: bool,
    model_provider_id: Option<&str>,
    model_id: Option<&str>,
    timeout_seconds: i32,
    operator_prompt: &str,
) -> Result<i64> {
    let job_key = build_generated_hook_job_key(job_kind);
    let row: (i64,) = query_as(
        "INSERT INTO agentic_job_hooks (
            agent_key,
            job_key,
            job_kind,
            hook_event,
            enabled,
            model_provider_id,
            model_id,
            timeout_seconds,
            operator_prompt
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         RETURNING id",
    )
    .bind(agent_key)
    .bind(&job_key)
    .bind(job_kind)
    .bind(hook_event)
    .bind(enabled)
    .bind(model_provider_id)
    .bind(model_id)
    .bind(timeout_seconds)
    .bind(operator_prompt)
    .fetch_one(pool)
    .await
    .with_context(|| format!("failed to insert hook {job_key} for agent {agent_key}"))?;

    Ok(row.0)
}

pub async fn set_hook_enabled(
    pool: &DbPool,
    agent_key: &str,
    hook_id: i64,
    enabled: bool,
) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE agentic_job_hooks
            SET enabled = $3,
                updated_at = now()
          WHERE agent_key = $1
            AND id = $2",
    )
    .bind(agent_key)
    .bind(hook_id)
    .bind(enabled)
    .execute(pool)
    .await
    .with_context(|| format!("failed to toggle hook {hook_id} for agent {agent_key}"))?;

    Ok(result.rows_affected() > 0)
}

pub async fn delete_agent_hook(pool: &DbPool, agent_key: &str, hook_id: i64) -> Result<bool> {
    let result = sqlx::query(
        "DELETE FROM agentic_job_hooks
          WHERE agent_key = $1
            AND id = $2",
    )
    .bind(agent_key)
    .bind(hook_id)
    .execute(pool)
    .await
    .with_context(|| format!("failed to delete hook {hook_id} for agent {agent_key}"))?;

    Ok(result.rows_affected() > 0)
}

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

/// List the most recent runs for an agent.
pub async fn list_agent_runs(
    pool: &DbPool,
    agent_key: &str,
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

/// List the most recent runs for a single hook.
pub async fn list_hook_runs(
    pool: &DbPool,
    agent_key: &str,
    hook_id: i64,
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
            AND hook_id = $2
          ORDER BY created_at DESC
          LIMIT $3",
    )
    .bind(agent_key)
    .bind(hook_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .with_context(|| format!("failed to list runs for hook {hook_id} agent {agent_key}"))?;

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

pub async fn set_hook_model(
    pool: &DbPool,
    agent_key: &str,
    hook_id: i64,
    model_provider_id: Option<&str>,
    model_id: Option<&str>,
) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE agentic_job_hooks
            SET model_provider_id = $3,
                model_id = $4,
                updated_at = now()
          WHERE agent_key = $1
            AND id = $2",
    )
    .bind(agent_key)
    .bind(hook_id)
    .bind(model_provider_id)
    .bind(model_id)
    .execute(pool)
    .await
    .with_context(|| format!("failed to update model for hook {hook_id} agent {agent_key}"))?;

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

pub async fn set_hook_timeout(
    pool: &DbPool,
    agent_key: &str,
    hook_id: i64,
    timeout_seconds: i32,
) -> Result<bool> {
    if timeout_seconds <= 0 {
        anyhow::bail!("timeout_seconds must be positive, got {timeout_seconds}");
    }
    let result = sqlx::query(
        "UPDATE agentic_job_hooks
            SET timeout_seconds = $3,
                updated_at = now()
          WHERE agent_key = $1
            AND id = $2",
    )
    .bind(agent_key)
    .bind(hook_id)
    .bind(timeout_seconds)
    .execute(pool)
    .await
    .with_context(|| format!("failed to update timeout for hook {hook_id} agent {agent_key}"))?;

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

pub async fn get_opencode_hook_for_dispatch(
    pool: &DbPool,
    agent_key: &str,
    hook_id: i64,
) -> Result<Option<DueOpenCodeHookRow>> {
    let row = query_as::<_, DueOpenCodeHookRow>(
        "SELECT hooks.id AS hook_id,
                agents.agent_key,
                agents.display_name,
                hooks.job_key,
                hooks.job_kind,
                hooks.hook_event,
                hooks.model_provider_id,
                hooks.model_id,
                hooks.timeout_seconds,
                hooks.operator_prompt,
                agents.runtime_id,
                runtimes.name AS runtime_name,
                runtimes.base_url AS runtime_base_url,
                agents.runtime_config
           FROM agentic_job_hooks AS hooks
           JOIN agents
             ON agents.agent_key = hooks.agent_key
           JOIN agent_runtimes AS runtimes
             ON runtimes.id = agents.runtime_id
          WHERE hooks.agent_key = $1
            AND hooks.id = $2
            AND agents.backend_kind = 'opencode'
            AND runtimes.enabled = true
            AND runtimes.base_url IS NOT NULL
            AND length(runtimes.base_url) > 0",
    )
    .bind(agent_key)
    .bind(hook_id)
    .fetch_optional(pool)
    .await
    .with_context(|| {
        format!("failed to load OpenCode dispatch metadata for hook {hook_id} agent {agent_key}")
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

    let schedule: Option<ScheduleForUpdate> = query_as(
        "SELECT id,
                agent_key,
                job_key,
                job_kind,
                enabled,
                timeframe,
                trigger_delay_seconds,
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

    let scheduled_for = boundary_for_due_at(schedule.next_run_at, trigger_delay_seconds);

    let outcome =
        if has_active_run_in_lane_tx(&mut tx, &schedule.agent_key, &schedule.job_kind).await? {
            let error_summary = "previous run still active";
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
                None,
                &schedule.agent_key,
                &schedule.job_key,
                &schedule.job_kind,
                Some(&timeframe),
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
struct ScheduleForUpdate {
    id: i64,
    agent_key: String,
    job_key: String,
    job_kind: String,
    enabled: bool,
    timeframe: String,
    trigger_delay_seconds: i32,
    next_run_at: DateTime<Utc>,
    timeout_seconds: i32,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct HookForUpdate {
    id: i64,
    agent_key: String,
    job_key: String,
    job_kind: String,
    #[allow(dead_code)]
    hook_event: String,
    #[allow(dead_code)]
    enabled: bool,
    timeout_seconds: i32,
}

#[allow(clippy::too_many_arguments)]
async fn insert_run_in_tx(
    tx: &mut Transaction<'_, Postgres>,
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

    let schedule: Option<ScheduleForUpdate> = query_as(
        "SELECT id,
                agent_key,
                job_key,
                job_kind,
                enabled,
                timeframe,
                trigger_delay_seconds,
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
    let outcome =
        if has_active_run_in_lane_tx(&mut tx, &schedule.agent_key, &schedule.job_kind).await? {
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
                None,
                &schedule.agent_key,
                &schedule.job_key,
                &schedule.job_kind,
                Some(&schedule.timeframe),
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

pub async fn insert_queued_hook_run(
    pool: &DbPool,
    agent_key: &str,
    hook_id: i64,
) -> Result<QueuedHookRun> {
    let mut tx = pool
        .begin()
        .await
        .context("failed to begin manual queued hook run insert")?;

    let hook: Option<HookForUpdate> = query_as(
        "SELECT id,
                agent_key,
                job_key,
                job_kind,
                hook_event,
                enabled,
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

    let now = Utc::now();
    let outcome = if has_active_run_in_lane_tx(&mut tx, &hook.agent_key, &hook.job_kind).await? {
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
            None,
            None,
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
            None,
            None,
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
                timeframe,
                trigger_delay_seconds,
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

        query(
            "UPDATE agentic_job_schedules
                SET enabled = true
              WHERE agent_key = $1
                AND job_key = $2",
        )
        .bind(key)
        .bind(default_analysis_job_key())
        .execute(pool)
        .await
        .expect("enable seeded analysis schedule");

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
                AND job_key = $2",
        )
        .bind(key)
        .bind(default_analysis_job_key())
        .fetch_one(pool)
        .await
        .expect("fetch schedule id");
        id
    }

    #[tokio::test]
    async fn default_schedules_insert_expected_rows_with_disabled_defaults() {
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
        assert_eq!(rows.len(), 4);

        let analysis = rows
            .iter()
            .find(|row| row.job_key == default_analysis_job_key())
            .expect("analysis schedule present");
        assert!(!analysis.enabled);
        assert_eq!(analysis.job_kind, JOB_KIND_ANALYSIS);
        assert_eq!(analysis.timeframe, DEFAULT_ANALYSIS_TIMEFRAME);
        assert_eq!(
            analysis.trigger_delay_seconds,
            DEFAULT_TRIGGER_DELAY_SECONDS
        );
        assert_eq!(analysis.timeout_seconds, DEFAULT_ANALYSIS_TIMEOUT_SECONDS);

        let analysis_1h = rows
            .iter()
            .find(|row| row.job_key == "analysis-1h")
            .expect("1h analysis schedule present");
        assert!(!analysis_1h.enabled);
        assert_eq!(analysis_1h.job_kind, JOB_KIND_ANALYSIS);
        assert_eq!(analysis_1h.timeframe, "1h");

        let analysis_1d = rows
            .iter()
            .find(|row| row.job_key == "analysis-1d")
            .expect("1d analysis schedule present");
        assert!(!analysis_1d.enabled);
        assert_eq!(analysis_1d.job_kind, JOB_KIND_ANALYSIS);
        assert_eq!(analysis_1d.timeframe, "1d");

        let trading = rows
            .iter()
            .find(|row| row.job_key == default_trading_job_key())
            .expect("trading schedule present");
        assert!(!trading.enabled);
        assert_eq!(trading.job_kind, JOB_KIND_TRADING);
        assert_eq!(trading.timeframe, DEFAULT_TRADING_TIMEFRAME);
        assert_eq!(trading.trigger_delay_seconds, DEFAULT_TRIGGER_DELAY_SECONDS);
        assert_eq!(trading.timeout_seconds, DEFAULT_TRADING_TIMEOUT_SECONDS);

        let hooks = list_agent_hooks(&pool, &key).await.expect("list hooks");
        assert_eq!(hooks.len(), 1);
        let hook = hooks.first().expect("default hook present");
        assert_eq!(hook.job_key, "market-analysis");
        assert_eq!(hook.job_kind, JOB_KIND_MARKET_ANALYSIS);
        assert_eq!(
            hook.hook_event,
            crate::agentic::model::HOOK_EVENT_ANALYSIS_BATCH_COMPLETED
        );
        assert!(!hook.enabled);
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
        assert_eq!(count.0, 4);

        let hook_count: (i64,) =
            query_as("SELECT COUNT(*) FROM agentic_job_hooks WHERE agent_key = $1")
                .bind(&key)
                .fetch_one(&pool)
                .await
                .expect("hook count");
        assert_eq!(hook_count.0, 1);
    }

    #[tokio::test]
    async fn default_schedules_align_next_run_at_to_future_boundary() {
        let pool = test_db::pool().await;
        let key = format!(
            "default-aligned-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        insert_agent(&pool, &sample_agent(&key))
            .await
            .expect("insert agent");

        let now = Utc::now();
        insert_default_opencode_schedules(&pool, &key)
            .await
            .expect("insert defaults");

        let rows = list_agent_schedules(&pool, &key)
            .await
            .expect("list schedules");
        let analysis = rows
            .iter()
            .find(|row| row.job_key == default_analysis_job_key())
            .expect("analysis schedule present");
        assert!(
            analysis.next_run_at > now,
            "expected future next_run_at, got {:?} <= {:?}",
            analysis.next_run_at,
            now
        );
        // The next due instant for a 15m schedule from any moment is a
        // 15-minute UTC boundary + 1s.
        let expected = next_due_after(
            now,
            DEFAULT_ANALYSIS_TIMEFRAME,
            DEFAULT_TRIGGER_DELAY_SECONDS,
        )
        .expect("next due");
        assert_eq!(analysis.next_run_at, expected);
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

        let schedule_id = insert_agent_schedule(
            &pool,
            &key,
            JOB_KIND_ANALYSIS,
            true,
            "1h",
            1,
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
        assert_eq!(row.timeframe, "1h");
        assert_eq!(row.trigger_delay_seconds, 1);
        assert_eq!(row.timeout_seconds, 600);
        assert_eq!(row.model_provider_id.as_deref(), Some("anthropic"));
        assert_eq!(row.model_id.as_deref(), Some("claude-sonnet-4"));
        assert_eq!(row.operator_prompt, "Check higher timeframe structure.");
    }

    #[tokio::test]
    async fn insert_agent_schedule_rejects_invalid_timeframe() {
        let pool = test_db::pool().await;
        let key = format!(
            "custom-bad-tf-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        insert_agent(&pool, &sample_agent(&key))
            .await
            .expect("insert agent");

        let result = insert_agent_schedule(
            &pool,
            &key,
            JOB_KIND_ANALYSIS,
            true,
            "15s",
            1,
            None,
            None,
            600,
            "",
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn insert_agent_schedule_rejects_duplicate_job_kind_timeframe() {
        let pool = test_db::pool().await;
        let key = format!(
            "custom-dup-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        insert_agent(&pool, &sample_agent(&key))
            .await
            .expect("insert agent");

        insert_agent_schedule(
            &pool,
            &key,
            JOB_KIND_ANALYSIS,
            true,
            "1h",
            1,
            None,
            None,
            600,
            "",
        )
        .await
        .expect("first insert");

        let result = insert_agent_schedule(
            &pool,
            &key,
            JOB_KIND_ANALYSIS,
            true,
            "1h",
            1,
            None,
            None,
            600,
            "",
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn insert_agent_hook_generates_market_analysis_key_and_rejects_duplicates() {
        let pool = test_db::pool().await;
        let key = format!("hook-dup-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0));
        insert_agent(&pool, &sample_agent(&key))
            .await
            .expect("insert agent");

        let hook_id = insert_agent_hook(
            &pool,
            &key,
            JOB_KIND_MARKET_ANALYSIS,
            crate::agentic::model::HOOK_EVENT_ANALYSIS_BATCH_COMPLETED,
            true,
            None,
            None,
            600,
            "",
        )
        .await
        .expect("insert hook");

        let hook = get_agent_hook(&pool, &key, hook_id)
            .await
            .expect("get hook")
            .expect("hook present");
        assert_eq!(hook.job_key, "market-analysis");

        let duplicate = insert_agent_hook(
            &pool,
            &key,
            JOB_KIND_MARKET_ANALYSIS,
            crate::agentic::model::HOOK_EVENT_ANALYSIS_BATCH_COMPLETED,
            true,
            None,
            None,
            600,
            "",
        )
        .await;
        assert!(duplicate.is_err());
    }

    #[tokio::test]
    async fn insert_queued_hook_run_inserts_manual_dispatch_run_without_timeframe() {
        let pool = test_db::pool().await;
        let key = format!("hook-run-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0));
        insert_agent(&pool, &sample_agent(&key))
            .await
            .expect("insert agent");
        let hook_id = insert_agent_hook(
            &pool,
            &key,
            JOB_KIND_MARKET_ANALYSIS,
            crate::agentic::model::HOOK_EVENT_ANALYSIS_BATCH_COMPLETED,
            true,
            None,
            None,
            600,
            "",
        )
        .await
        .expect("insert hook");

        let outcome = insert_queued_hook_run(&pool, &key, hook_id)
            .await
            .expect("insert queued hook run");
        let run_id = match outcome {
            QueuedHookRun::Dispatch { run_id, .. } => run_id,
            other => panic!("expected Dispatch, got {other:?}"),
        };

        let run = get_run(&pool, run_id)
            .await
            .expect("get run")
            .expect("run present");
        assert_eq!(run.schedule_id, None);
        assert_eq!(run.hook_id, Some(hook_id));
        assert_eq!(run.job_key, "market-analysis");
        assert_eq!(run.job_kind, JOB_KIND_MARKET_ANALYSIS);
        assert_eq!(run.timeframe, None);
    }

    #[tokio::test]
    async fn agentic_runs_exactly_one_source_check_rejects_both_and_neither_sources() {
        let pool = test_db::pool().await;
        let key = format!(
            "run-source-check-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        let schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;
        let schedule = list_agent_schedules(&pool, &key)
            .await
            .expect("list schedules")
            .into_iter()
            .find(|row| row.id == schedule_id)
            .expect("schedule present");
        let hook_id = list_agent_hooks(&pool, &key)
            .await
            .expect("list hooks")
            .first()
            .expect("default hook present")
            .id;

        let both_sources = query(
            "INSERT INTO agentic_runs (
                schedule_id,
                hook_id,
                agent_key,
                job_key,
                job_kind,
                timeframe,
                status,
                scheduled_for,
                timeout_seconds
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(schedule_id)
        .bind(hook_id)
        .bind(&key)
        .bind(&schedule.job_key)
        .bind(&schedule.job_kind)
        .bind(&schedule.timeframe)
        .bind(RUN_STATUS_QUEUED)
        .bind(Utc::now())
        .bind(schedule.timeout_seconds)
        .execute(&pool)
        .await;
        assert!(both_sources.is_err(), "expected both-source insert to fail");

        let neither_source = query(
            "INSERT INTO agentic_runs (
                schedule_id,
                hook_id,
                agent_key,
                job_key,
                job_kind,
                timeframe,
                status,
                scheduled_for,
                timeout_seconds
             ) VALUES (NULL, NULL, $1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(&key)
        .bind(&schedule.job_key)
        .bind(&schedule.job_kind)
        .bind(&schedule.timeframe)
        .bind(RUN_STATUS_QUEUED)
        .bind(Utc::now())
        .bind(schedule.timeout_seconds)
        .execute(&pool)
        .await;
        assert!(
            neither_source.is_err(),
            "expected neither-source insert to fail"
        );
    }

    #[tokio::test]
    async fn deleting_schedule_cascades_run_rows() {
        let pool = test_db::pool().await;
        let key = format!(
            "delete-schedule-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        let schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;
        let run_id = insert_test_run(&pool, schedule_id, RUN_STATUS_QUEUED)
            .await
            .expect("insert run");

        let deleted = delete_agent_schedule(&pool, &key, schedule_id)
            .await
            .expect("delete schedule");
        assert!(deleted);
        assert!(get_run(&pool, run_id).await.expect("get run").is_none());
    }

    #[tokio::test]
    async fn deleting_hook_cascades_run_rows() {
        let pool = test_db::pool().await;
        let key = format!(
            "delete-hook-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        insert_agent(&pool, &sample_agent(&key))
            .await
            .expect("insert agent");
        let hook_id = insert_agent_hook(
            &pool,
            &key,
            JOB_KIND_MARKET_ANALYSIS,
            crate::agentic::model::HOOK_EVENT_ANALYSIS_BATCH_COMPLETED,
            true,
            None,
            None,
            600,
            "",
        )
        .await
        .expect("insert hook");
        let run_id = match insert_queued_hook_run(&pool, &key, hook_id)
            .await
            .expect("insert queued hook run")
        {
            QueuedHookRun::Dispatch { run_id, .. } => run_id,
            other => panic!("expected Dispatch, got {other:?}"),
        };

        let deleted = delete_agent_hook(&pool, &key, hook_id)
            .await
            .expect("delete hook");
        assert!(deleted);
        assert!(get_run(&pool, run_id).await.expect("get run").is_none());
    }

    #[tokio::test]
    async fn set_all_agent_jobs_enabled_toggles_schedules_and_hooks_together() {
        let pool = test_db::pool().await;
        let key = format!(
            "toggle-all-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        insert_agent(&pool, &sample_agent(&key))
            .await
            .expect("insert agent");
        insert_default_opencode_schedules(&pool, &key)
            .await
            .expect("insert defaults");

        set_all_agent_jobs_enabled(&pool, &key, true)
            .await
            .expect("enable all");

        let schedules = list_agent_schedules(&pool, &key)
            .await
            .expect("list schedules");
        assert!(schedules.iter().all(|row| row.enabled));
        let hooks = list_agent_hooks(&pool, &key).await.expect("list hooks");
        assert!(hooks.iter().all(|row| row.enabled));

        set_all_agent_jobs_enabled(&pool, &key, false)
            .await
            .expect("disable all");

        let schedules = list_agent_schedules(&pool, &key)
            .await
            .expect("list schedules again");
        assert!(schedules.iter().all(|row| !row.enabled));
        let hooks = list_agent_hooks(&pool, &key)
            .await
            .expect("list hooks again");
        assert!(hooks.iter().all(|row| !row.enabled));
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

        query("UPDATE agentic_job_schedules SET enabled = false WHERE job_key = $1")
            .bind(default_analysis_job_key())
            .execute(&pool)
            .await
            .expect("disable analysis");
        let future = Utc::now() + chrono::Duration::seconds(3600);
        query("UPDATE agentic_job_schedules SET next_run_at = $1 WHERE job_key = $2")
            .bind(future)
            .bind(default_trading_job_key())
            .execute(&pool)
            .await
            .expect("push trading future");
        query("UPDATE agentic_job_schedules SET enabled = true WHERE job_key = $1")
            .bind(default_trading_job_key())
            .execute(&pool)
            .await
            .expect("enable trading");

        let now = Utc::now();
        let due = list_due_opencode_schedules(&pool, now, 20)
            .await
            .expect("list due");

        assert!(
            due.iter().all(|row| row.agent_key != key),
            "expected no rows for disabled/future agent, got {due:?}"
        );

        query(
            "UPDATE agentic_job_schedules SET enabled = true, next_run_at = $1 WHERE job_key = $2",
        )
        .bind(now - chrono::Duration::seconds(1))
        .bind(default_analysis_job_key())
        .execute(&pool)
        .await
        .expect("re-enable analysis due");
        let due = list_due_opencode_schedules(&pool, now, 20)
            .await
            .expect("list due again");
        let analysis_due = due
            .iter()
            .find(|row| row.agent_key == key && row.job_key == default_analysis_job_key())
            .expect("analysis row should be due");
        assert!(analysis_due.runtime_base_url.contains("14096"));
        assert_eq!(analysis_due.timeframe, DEFAULT_ANALYSIS_TIMEFRAME);
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

        let past = Utc::now() - chrono::Duration::seconds(60);
        query(
            "UPDATE agentic_job_schedules SET enabled = true, next_run_at = $1 WHERE job_key = $2",
        )
        .bind(past)
        .bind(default_analysis_job_key())
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
    async fn claim_due_schedule_inserts_queued_run_and_aligns_next_run_at() {
        let pool = test_db::pool().await;
        let key = format!("claim-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0));
        let schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;

        // Set the schedule to a previous 15m due boundary so it is
        // due "now" no matter when the test actually runs.
        let now = Utc::now();
        let due_boundary = latest_due_at_or_before(
            now,
            DEFAULT_ANALYSIS_TIMEFRAME,
            DEFAULT_TRIGGER_DELAY_SECONDS,
        )
        .expect("compute latest due")
        .expect("should have a previous due boundary");
        query("UPDATE agentic_job_schedules SET next_run_at = $1 WHERE id = $2")
            .bind(due_boundary)
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
        assert_eq!(run.timeframe.as_deref(), Some(DEFAULT_ANALYSIS_TIMEFRAME));
        assert_eq!(
            run.scheduled_for,
            boundary_for_due_at(due_boundary, DEFAULT_TRIGGER_DELAY_SECONDS)
        );

        // The next_run_at should be aligned to the next 15m boundary
        // after `now`, regardless of how late the claim was.
        let (next_run_at,): (DateTime<Utc>,) =
            query_as("SELECT next_run_at FROM agentic_job_schedules WHERE id = $1")
                .bind(schedule_id)
                .fetch_one(&pool)
                .await
                .expect("fetch next_run_at");
        let expected = next_due_after(
            now,
            DEFAULT_ANALYSIS_TIMEFRAME,
            DEFAULT_TRIGGER_DELAY_SECONDS,
        )
        .expect("compute next due");
        assert_eq!(next_run_at, expected);
    }

    #[tokio::test]
    async fn claim_due_schedule_advances_stale_schedule_without_dispatching() {
        let pool = test_db::pool().await;
        let key = format!("stale-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0));
        let schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;

        // Push next_run_at well into the past, simulating a long
        // downtime.
        let long_ago = Utc::now() - chrono::Duration::hours(2);
        query("UPDATE agentic_job_schedules SET next_run_at = $1 WHERE id = $2")
            .bind(long_ago)
            .bind(schedule_id)
            .execute(&pool)
            .await
            .expect("set stale");

        let now = Utc::now();
        let outcome = claim_due_schedule(&pool, schedule_id, now)
            .await
            .expect("claim");
        assert!(matches!(outcome, ClaimedScheduleRun::NotDue));

        let (next_run_at,): (DateTime<Utc>,) =
            query_as("SELECT next_run_at FROM agentic_job_schedules WHERE id = $1")
                .bind(schedule_id)
                .fetch_one(&pool)
                .await
                .expect("fetch next_run_at");
        let expected = next_due_after(
            now,
            DEFAULT_ANALYSIS_TIMEFRAME,
            DEFAULT_TRIGGER_DELAY_SECONDS,
        )
        .expect("compute next due");
        assert_eq!(next_run_at, expected);
    }

    #[tokio::test]
    async fn claim_due_schedule_inserts_skipped_run_when_active_run_exists_for_same_agent() {
        let pool = test_db::pool().await;
        let key = format!("skip-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0));
        let schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;

        let now = Utc::now();
        let due_boundary = latest_due_at_or_before(
            now,
            DEFAULT_ANALYSIS_TIMEFRAME,
            DEFAULT_TRIGGER_DELAY_SECONDS,
        )
        .expect("compute latest due")
        .expect("should have a previous due boundary");
        query("UPDATE agentic_job_schedules SET next_run_at = $1 WHERE id = $2")
            .bind(due_boundary)
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
        assert_eq!(run.timeframe.as_deref(), Some(DEFAULT_ANALYSIS_TIMEFRAME));
    }

    #[tokio::test]
    async fn claim_due_schedule_returns_not_due_when_schedule_disabled() {
        let pool = test_db::pool().await;
        let key = format!("notdue-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0));
        let schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;

        let now = Utc::now();
        let due = next_due_after(
            now,
            DEFAULT_ANALYSIS_TIMEFRAME,
            DEFAULT_TRIGGER_DELAY_SECONDS,
        )
        .expect("compute next due");
        query("UPDATE agentic_job_schedules SET enabled = false, next_run_at = $1 WHERE id = $2")
            .bind(due)
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
    async fn insert_queued_run_inserts_manual_dispatch_run_and_does_not_advance_schedule() {
        let pool = test_db::pool().await;
        let key = format!("manual-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0));
        let schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;

        let (before_next_run_at,): (DateTime<Utc>,) =
            query_as("SELECT next_run_at FROM agentic_job_schedules WHERE id = $1")
                .bind(schedule_id)
                .fetch_one(&pool)
                .await
                .expect("fetch before next_run_at");

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
        assert_eq!(run.timeframe.as_deref(), Some(DEFAULT_ANALYSIS_TIMEFRAME));
        let delta = (run.scheduled_for - scheduled_for)
            .num_microseconds()
            .unwrap_or(i64::MAX);
        assert!(delta.abs() <= 1, "scheduled_for delta too large: {delta}us");

        let (after_next_run_at,): (DateTime<Utc>,) =
            query_as("SELECT next_run_at FROM agentic_job_schedules WHERE id = $1")
                .bind(schedule_id)
                .fetch_one(&pool)
                .await
                .expect("fetch after next_run_at");
        assert_eq!(
            before_next_run_at, after_next_run_at,
            "manual run must not mutate next_run_at"
        );
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
    async fn mark_run_failed_preserves_existing_backend_ref_when_not_provided() {
        let pool = test_db::pool().await;
        let key = format!(
            "run-fail-preserve-ref-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        let schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;
        let run_id = insert_test_run(&pool, schedule_id, RUN_STATUS_QUEUED)
            .await
            .expect("seed run");

        mark_run_running(&pool, run_id, Some("ses_keep"))
            .await
            .expect("mark running");
        mark_run_failed(&pool, run_id, "boom", None)
            .await
            .expect("mark failed");

        let run = get_run(&pool, run_id)
            .await
            .expect("fetch run")
            .expect("run present");
        assert_eq!(run.status, RUN_STATUS_FAILED);
        assert_eq!(run.backend_run_ref.as_deref(), Some("ses_keep"));
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

    #[tokio::test]
    async fn set_schedule_timeout_updates_value() {
        let pool = test_db::pool().await;
        let key = format!(
            "sched-timeout-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        let schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;

        let updated = set_schedule_timeout(&pool, &key, schedule_id, 1234)
            .await
            .expect("update timeout");
        assert!(updated);

        let schedule = get_agent_schedule(&pool, &key, schedule_id)
            .await
            .expect("fetch schedule")
            .expect("schedule present");
        assert_eq!(schedule.timeout_seconds, 1234);
    }

    #[tokio::test]
    async fn set_schedule_timeout_rejects_non_positive() {
        let pool = test_db::pool().await;
        let key = format!(
            "sched-timeout-bad-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        let schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;

        let err = set_schedule_timeout(&pool, &key, schedule_id, 0)
            .await
            .expect_err("zero should fail");
        assert!(err.to_string().contains("positive"));
    }

    #[tokio::test]
    async fn set_schedule_timeout_missing_returns_false() {
        let pool = test_db::pool().await;
        let err = set_schedule_timeout(&pool, "no-such-agent", 999, 900)
            .await
            .expect("update returns false for missing schedule");
        assert!(!err);
    }

    #[tokio::test]
    async fn set_hook_timeout_updates_value() {
        let pool = test_db::pool().await;
        let key = format!(
            "hook-timeout-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        insert_agent(&pool, &sample_agent(&key))
            .await
            .expect("insert agent");

        let hook_id = insert_agent_hook(
            &pool,
            &key,
            JOB_KIND_MARKET_ANALYSIS,
            crate::agentic::model::HOOK_EVENT_ANALYSIS_BATCH_COMPLETED,
            true,
            None,
            None,
            600,
            "",
        )
        .await
        .expect("insert hook");

        let updated = set_hook_timeout(&pool, &key, hook_id, 777)
            .await
            .expect("update timeout");
        assert!(updated);

        let hook = get_agent_hook(&pool, &key, hook_id)
            .await
            .expect("fetch hook")
            .expect("hook present");
        assert_eq!(hook.timeout_seconds, 777);
    }

    #[tokio::test]
    async fn set_hook_timeout_rejects_non_positive() {
        let pool = test_db::pool().await;
        let key = format!(
            "hook-timeout-bad-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        insert_agent(&pool, &sample_agent(&key))
            .await
            .expect("insert agent");

        let hook_id = insert_agent_hook(
            &pool,
            &key,
            JOB_KIND_MARKET_ANALYSIS,
            crate::agentic::model::HOOK_EVENT_ANALYSIS_BATCH_COMPLETED,
            true,
            None,
            None,
            600,
            "",
        )
        .await
        .expect("insert hook");

        let err = set_hook_timeout(&pool, &key, hook_id, -5)
            .await
            .expect_err("negative should fail");
        assert!(err.to_string().contains("positive"));
    }
}
