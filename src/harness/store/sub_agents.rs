use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sqlx::{Postgres, Transaction, query_as};

use crate::{
    db::DbPool,
    harness::{
        model::{
            CAPABILITY_NOTIFICATION_SEND, CAPABILITY_PROMPT_REVISION_SUBMIT,
            HarnessDispatchSubAgentRow, HarnessSubAgentRow, HarnessSubAgentRunRow,
            RUN_STATUS_QUEUED, RUN_STATUS_SKIPPED, SUB_AGENT_KIND_ANALYSIS, SUB_AGENT_KIND_REVIEW,
            SUB_AGENT_KIND_TRADING,
        },
        sub_agent_key::{build_generated_event_sub_agent_key, build_generated_sub_agent_key},
        timeframe::{
            DEFAULT_TRIGGER_DELAY_SECONDS, boundary_for_due_at, latest_due_at_or_before,
            next_due_after, parse_timeframe_seconds,
        },
    },
};

use super::common::{insert_run_with_model_variant_in_tx, lock_agent_coordination_tx};
use super::recovery::has_active_run_in_lane_tx;

#[cfg(test)]
pub(crate) const DEFAULT_ANALYSIS_TIMEFRAME: &str = "15m";
#[cfg(test)]
const DEFAULT_ANALYSIS_TIMEFRAMES: [&str; 3] = ["15m", "1h", "1d"];
pub const DEFAULT_TRADING_TIMEFRAME: &str = "5m";
pub const DEFAULT_REVIEW_TIMEFRAME: &str = "1d";
#[cfg(test)]
pub(crate) const DEFAULT_ANALYSIS_TIMEOUT_SECONDS: i32 = 900;
#[cfg(test)]
pub(crate) const DEFAULT_TRADING_TIMEOUT_SECONDS: i32 = 900;
#[cfg(test)]
pub(crate) const DEFAULT_REVIEW_TIMEOUT_SECONDS: i32 = 900;

pub struct SingletonSubAgentConfig<'a> {
    pub enabled: bool,
    pub timeframe: &'a str,
    pub model_provider_id: Option<&'a str>,
    pub model_id: Option<&'a str>,
    pub model_variant: Option<&'a str>,
    pub timeout_seconds: i32,
}

#[cfg(test)]
/// Seed the canonical default sub-agents used by test fixtures.
///
/// This is idempotent: existing rows keyed by `(agent_key, sub_agent_key)`
/// are left untouched. The fixture gets disabled
/// `technical-15m`, `technical-1h`, `technical-1d` Analysis jobs, a
/// `trading-5m` job, and a `review-1d` job.
pub async fn insert_default_harness_sub_agents(pool: &DbPool, agent_key: &str) -> Result<()> {
    for timeframe in DEFAULT_ANALYSIS_TIMEFRAMES {
        insert_default_analysis_job(pool, agent_key, timeframe).await?;
    }

    insert_default_candle_job(
        pool,
        agent_key,
        SUB_AGENT_KIND_TRADING,
        DEFAULT_TRADING_TIMEFRAME,
        false,
        DEFAULT_TRADING_TIMEOUT_SECONDS,
    )
    .await?;

    insert_default_candle_job(
        pool,
        agent_key,
        SUB_AGENT_KIND_REVIEW,
        DEFAULT_REVIEW_TIMEFRAME,
        false,
        DEFAULT_REVIEW_TIMEOUT_SECONDS,
    )
    .await?;

    // Every durable role owns an active revision, including disabled defaults.
    // This runs after the rows exist so it is also safe for partially-created
    // development agents from before prompt revisions were introduced.
    crate::agents::strategy_prompts::insert_default_strategy_prompts_for_agent(pool, agent_key)
        .await?;

    Ok(())
}

#[cfg(test)]
async fn insert_default_analysis_job(
    pool: &DbPool,
    agent_key: &str,
    timeframe: &str,
) -> Result<()> {
    let sub_agent_key = format!("technical-{timeframe}");
    let now = Utc::now();
    let next_run_at = next_due_after(now, timeframe, DEFAULT_TRIGGER_DELAY_SECONDS)
        .with_context(|| format!("invalid default timeframe {timeframe:?}"))?;

    sqlx::query(
        "INSERT INTO harness_sub_agents (
            agent_key,
            sub_agent_key,
            sub_agent_kind,
            enabled,
            timeframe,
             trigger_delay_seconds,
             next_run_at,
             timeout_seconds,
             enabled_capabilities
            ) VALUES ($1, $2, 'analysis', false, $3, $4, $5, $6, '[]'::jsonb)
         ON CONFLICT (agent_key, sub_agent_key) DO NOTHING",
    )
    .bind(agent_key)
    .bind(&sub_agent_key)
    .bind(timeframe)
    .bind(DEFAULT_TRIGGER_DELAY_SECONDS)
    .bind(next_run_at)
    .bind(DEFAULT_ANALYSIS_TIMEOUT_SECONDS)
    .execute(pool)
    .await
    .with_context(|| {
        format!("failed to insert default {sub_agent_key} job for agent {agent_key}")
    })?;

    Ok(())
}

/// Create a user-configured Analysis job. The `sub_agent_key` is the durable
/// identity of the job and must be a bounded ASCII slug. Multiple Analysis
/// jobs may share a schedule/timeframe.
#[allow(clippy::too_many_arguments)]
pub async fn insert_analysis_sub_agent_with_model_variant(
    pool: &DbPool,
    agent_key: &str,
    sub_agent_key: &str,
    enabled: bool,
    timeframe: &str,
    trigger_delay_seconds: i32,
    model_provider_id: Option<&str>,
    model_id: Option<&str>,
    model_variant: Option<&str>,
    timeout_seconds: i32,
) -> Result<i64> {
    parse_timeframe_seconds(timeframe)
        .with_context(|| format!("invalid timeframe {timeframe:?}"))?;
    if trigger_delay_seconds < 0 {
        return Err(anyhow::anyhow!(
            "trigger_delay_seconds must be non-negative"
        ));
    }
    let sub_agent_key = sub_agent_key.trim();
    anyhow::ensure!(
        crate::harness::sub_agent_key::is_valid_user_sub_agent_key(sub_agent_key),
        "sub-agent key must be a bounded ASCII slug of letters, digits, hyphens, and underscores"
    );

    let now = Utc::now();
    let next_run_at = next_due_after(now, timeframe, trigger_delay_seconds)
        .with_context(|| format!("failed to compute next_run_at for {timeframe:?}"))?;

    let row: (i64,) = query_as(
        "INSERT INTO harness_sub_agents (
            agent_key,
            sub_agent_key,
            sub_agent_kind,
            enabled,
            timeframe,
            trigger_delay_seconds,
            next_run_at,
            model_provider_id,
             model_id,
             model_variant,
              timeout_seconds,
               enabled_capabilities
            ) VALUES ($1, $2, 'analysis', $3, $4, $5, $6, $7, $8, $9, $10, '[]'::jsonb)
         RETURNING id",
    )
    .bind(agent_key)
    .bind(sub_agent_key)
    .bind(enabled)
    .bind(timeframe)
    .bind(trigger_delay_seconds)
    .bind(next_run_at)
    .bind(model_provider_id)
    .bind(model_id)
    .bind(model_variant)
    .bind(timeout_seconds)
    .fetch_one(pool)
    .await
    .with_context(|| format!("failed to insert job {sub_agent_key} for agent {agent_key}"))?;

    Ok(row.0)
}

/// List only the Analysis jobs owned by an agent.
pub async fn list_analysis_sub_agents(
    pool: &DbPool,
    agent_key: &str,
) -> Result<Vec<HarnessSubAgentRow>> {
    query_as(
        "SELECT id, agent_key, sub_agent_key, sub_agent_kind, enabled, timeframe,
                 next_run_at, model_provider_id, model_id,
                   model_variant, timeout_seconds,
                  enabled_capabilities,
                  created_at, updated_at
           FROM harness_sub_agents
          WHERE agent_key = $1
            AND sub_agent_kind = 'analysis'
           ORDER BY sub_agent_key",
    )
    .bind(agent_key)
    .fetch_all(pool)
    .await
    .with_context(|| format!("failed to list analysis jobs for agent {agent_key}"))
}

#[cfg(test)]
pub async fn list_agent_sub_agents(
    pool: &DbPool,
    agent_key: &str,
) -> Result<Vec<HarnessSubAgentRow>> {
    query_as(
        "SELECT id, agent_key, sub_agent_key, sub_agent_kind, enabled, timeframe,
                 next_run_at, model_provider_id, model_id, model_variant,
                  timeout_seconds, enabled_capabilities, created_at, updated_at
           FROM harness_sub_agents
          WHERE agent_key = $1
          ORDER BY next_run_at NULLS LAST, sub_agent_kind, timeframe, id",
    )
    .bind(agent_key)
    .fetch_all(pool)
    .await
    .with_context(|| format!("failed to list harness jobs for agent {agent_key}"))
}

/// Load the singleton job of a given durable role (`trading` or `review`).
pub async fn get_singleton_sub_agent(
    pool: &DbPool,
    agent_key: &str,
    sub_agent_kind: &str,
) -> Result<Option<HarnessSubAgentRow>> {
    anyhow::ensure!(
        matches!(
            sub_agent_kind,
            SUB_AGENT_KIND_TRADING | SUB_AGENT_KIND_REVIEW
        ),
        "unsupported singleton sub-agent kind"
    );
    query_as(
        "SELECT id, agent_key, sub_agent_key, sub_agent_kind, enabled, timeframe,
                 next_run_at, model_provider_id, model_id,
                   model_variant, timeout_seconds,
                  enabled_capabilities,
                  created_at, updated_at
           FROM harness_sub_agents
          WHERE agent_key = $1
            AND sub_agent_kind = $2",
    )
    .bind(agent_key)
    .bind(sub_agent_kind)
    .fetch_optional(pool)
    .await
    .with_context(|| format!("failed to load {sub_agent_kind} job for agent {agent_key}"))
}

/// Create the missing singleton job for a durable role. Returns an error if
/// one already exists.
pub async fn insert_singleton_sub_agent(
    pool: &DbPool,
    agent_key: &str,
    sub_agent_kind: &str,
    config: SingletonSubAgentConfig<'_>,
) -> Result<i64> {
    anyhow::ensure!(
        matches!(
            sub_agent_kind,
            SUB_AGENT_KIND_TRADING | SUB_AGENT_KIND_REVIEW
        ),
        "unsupported singleton sub-agent kind"
    );
    let sub_agent_key = match sub_agent_kind {
        SUB_AGENT_KIND_TRADING | SUB_AGENT_KIND_REVIEW => {
            build_generated_sub_agent_key(sub_agent_kind, config.timeframe)
        }
        _ => build_generated_event_sub_agent_key(sub_agent_kind),
    };
    let timeframe = match sub_agent_kind {
        SUB_AGENT_KIND_TRADING | SUB_AGENT_KIND_REVIEW => Some(config.timeframe),
        _ => None,
    };
    let now = Utc::now();
    let next_run_at = timeframe
        .map(|timeframe| {
            next_due_after(now, timeframe, DEFAULT_TRIGGER_DELAY_SECONDS)
                .with_context(|| format!("invalid default timeframe {timeframe:?}"))
        })
        .transpose()?;
    let row: (i64,) = query_as(
        "INSERT INTO harness_sub_agents (
            agent_key, sub_agent_key, sub_agent_kind, enabled, timeframe,
            trigger_delay_seconds, next_run_at, model_provider_id, model_id,
            model_variant, timeout_seconds,
            enabled_capabilities
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
         RETURNING id",
    )
    .bind(agent_key)
    .bind(&sub_agent_key)
    .bind(sub_agent_kind)
    .bind(config.enabled)
    .bind(timeframe)
    .bind(DEFAULT_TRIGGER_DELAY_SECONDS)
    .bind(next_run_at)
    .bind(config.model_provider_id)
    .bind(config.model_id)
    .bind(config.model_variant)
    .bind(config.timeout_seconds)
    .bind(serde_json::json!(default_capabilities_for_kind(
        sub_agent_kind
    )))
    .fetch_one(pool)
    .await
    .with_context(|| {
        format!("failed to insert {sub_agent_kind} singleton for agent {agent_key}")
    })?;
    Ok(row.0)
}

#[cfg(test)]
async fn insert_default_candle_job(
    pool: &DbPool,
    agent_key: &str,
    sub_agent_kind: &str,
    timeframe: &str,
    enabled: bool,
    timeout_seconds: i32,
) -> Result<()> {
    let sub_agent_key = build_generated_sub_agent_key(sub_agent_kind, timeframe);
    let now = Utc::now();
    let next_run_at = next_due_after(now, timeframe, DEFAULT_TRIGGER_DELAY_SECONDS)
        .with_context(|| format!("invalid default timeframe {timeframe:?}"))?;

    sqlx::query(
        "INSERT INTO harness_sub_agents (
            agent_key,
            sub_agent_key,
            sub_agent_kind,
            enabled,
            timeframe,
             trigger_delay_seconds,
             next_run_at,
              timeout_seconds,
               enabled_capabilities
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         ON CONFLICT (agent_key, sub_agent_key) DO NOTHING",
    )
    .bind(agent_key)
    .bind(&sub_agent_key)
    .bind(sub_agent_kind)
    .bind(enabled)
    .bind(timeframe)
    .bind(DEFAULT_TRIGGER_DELAY_SECONDS)
    .bind(next_run_at)
    .bind(timeout_seconds)
    .bind(serde_json::json!(default_capabilities_for_kind(
        sub_agent_kind
    )))
    .execute(pool)
    .await
    .with_context(|| {
        format!("failed to insert default {sub_agent_key} job for agent {agent_key}")
    })?;

    Ok(())
}

/// Load a single job row for an agent.
pub async fn get_agent_sub_agent(
    pool: &DbPool,
    agent_key: &str,
    sub_agent_id: i64,
) -> Result<Option<HarnessSubAgentRow>> {
    let row = query_as::<_, HarnessSubAgentRow>(
        "SELECT id,
                 agent_key,
                 sub_agent_key,
                  sub_agent_kind,
                 enabled,
                 timeframe,
                 next_run_at,
                model_provider_id,
                model_id,
                  model_variant,
                  timeout_seconds,
                   enabled_capabilities,
                   created_at,
                updated_at
           FROM harness_sub_agents
          WHERE agent_key = $1
            AND id = $2",
    )
    .bind(agent_key)
    .bind(sub_agent_id)
    .fetch_optional(pool)
    .await
    .with_context(|| format!("failed to load job {sub_agent_id} for agent {agent_key}"))?;

    Ok(row)
}

#[cfg(test)]
pub async fn get_enabled_sub_agent(
    pool: &DbPool,
    agent_key: &str,
    sub_agent_kind: &str,
) -> Result<Option<HarnessSubAgentRow>> {
    query_as(
        "SELECT id, agent_key, sub_agent_key, sub_agent_kind, enabled, timeframe,
                 next_run_at, model_provider_id, model_id, model_variant,
                 timeout_seconds, enabled_capabilities, created_at, updated_at
           FROM harness_sub_agents
          WHERE agent_key = $1
            AND sub_agent_kind = $2
            AND enabled = true
          ORDER BY id
          LIMIT 1",
    )
    .bind(agent_key)
    .bind(sub_agent_kind)
    .fetch_optional(pool)
    .await
    .with_context(|| format!("failed to load enabled {sub_agent_kind} job for agent {agent_key}"))
}

/// List a page of the most recent runs for a single job.
pub async fn list_sub_agent_runs_page(
    pool: &DbPool,
    agent_key: &str,
    sub_agent_id: i64,
    limit: i64,
    offset: i64,
) -> Result<Vec<HarnessSubAgentRunRow>> {
    let rows = query_as::<_, HarnessSubAgentRunRow>(
        "SELECT id,
                sub_agent_id,
                agent_key,
                sub_agent_key,
                 sub_agent_kind,
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
           FROM harness_sub_agent_runs
           WHERE agent_key = $1
             AND sub_agent_id = $2
           ORDER BY created_at DESC
           LIMIT $3
          OFFSET $4",
    )
    .bind(agent_key)
    .bind(sub_agent_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
    .with_context(|| format!("failed to list runs for job {sub_agent_id} agent {agent_key}"))?;

    Ok(rows)
}

/// Count runs recorded for a single job.
pub async fn count_sub_agent_runs(
    pool: &DbPool,
    agent_key: &str,
    sub_agent_id: i64,
) -> Result<i64> {
    let (count,): (i64,) = query_as(
        "SELECT COUNT(*) FROM harness_sub_agent_runs WHERE agent_key = $1 AND sub_agent_id = $2",
    )
    .bind(agent_key)
    .bind(sub_agent_id)
    .fetch_one(pool)
    .await
    .with_context(|| format!("failed to count runs for job {sub_agent_id} agent {agent_key}"))?;

    Ok(count)
}

/// Toggle a single job's `enabled` flag.
///
/// Returns `true` when a row was updated, `false` when the (agent_key,
/// sub_agent_id) pair did not match an existing row.
pub async fn set_sub_agent_enabled(
    pool: &DbPool,
    agent_key: &str,
    sub_agent_id: i64,
    enabled: bool,
) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE harness_sub_agents
            SET enabled = $3,
                updated_at = now()
          WHERE agent_key = $1
            AND id = $2
            AND ($3 = false OR (model_provider_id IS NOT NULL AND model_id IS NOT NULL))",
    )
    .bind(agent_key)
    .bind(sub_agent_id)
    .bind(enabled)
    .execute(pool)
    .await
    .with_context(|| format!("failed to toggle job {sub_agent_id} for agent {agent_key}"))?;

    Ok(result.rows_affected() > 0)
}

/// Updates the notification capability assignment used by future runs. Existing
/// run snapshots retain the capability state they were dispatched with.
pub async fn set_sub_agent_notification_send_enabled(
    pool: &DbPool,
    agent_key: &str,
    sub_agent_id: i64,
    enabled: bool,
) -> Result<bool> {
    let Some(job) = get_agent_sub_agent(pool, agent_key, sub_agent_id).await? else {
        return Ok(false);
    };
    let mut capabilities = job.enabled_capabilities;
    capabilities.retain(|capability| capability != CAPABILITY_NOTIFICATION_SEND);
    if enabled {
        capabilities.push(CAPABILITY_NOTIFICATION_SEND.to_string());
    }
    set_sub_agent_capabilities(pool, agent_key, sub_agent_id, capabilities).await
}

/// Replaces the named capability assignment for future runs. Role ceilings are
/// validated before persistence; current run snapshots remain unchanged.
pub async fn set_sub_agent_capabilities(
    pool: &DbPool,
    agent_key: &str,
    sub_agent_id: i64,
    enabled_capabilities: Vec<String>,
) -> Result<bool> {
    let Some(job) = get_agent_sub_agent(pool, agent_key, sub_agent_id).await? else {
        return Ok(false);
    };
    let enabled_capabilities = crate::harness::model::validate_sub_agent_capabilities(
        &job.sub_agent_kind,
        &enabled_capabilities,
    )?;
    let result = sqlx::query(
        r#"UPDATE harness_sub_agents
               SET enabled_capabilities = $3,
                  updated_at = now()
           WHERE agent_key = $1
             AND id = $2"#,
    )
    .bind(agent_key)
    .bind(sub_agent_id)
    .bind(serde_json::json!(enabled_capabilities))
    .execute(pool)
    .await
    .with_context(|| {
        format!("failed to update notification capability for job {sub_agent_id} agent {agent_key}")
    })?;

    Ok(result.rows_affected() > 0)
}

fn default_capabilities_for_kind(sub_agent_kind: &str) -> Vec<&'static str> {
    match sub_agent_kind {
        SUB_AGENT_KIND_TRADING => vec![CAPABILITY_NOTIFICATION_SEND],
        SUB_AGENT_KIND_REVIEW => vec![CAPABILITY_PROMPT_REVISION_SUBMIT],
        _ => Vec::new(),
    }
}

pub async fn set_sub_agent_model_with_variant(
    pool: &DbPool,
    agent_key: &str,
    sub_agent_id: i64,
    model_provider_id: Option<&str>,
    model_id: Option<&str>,
    model_variant: Option<&str>,
) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE harness_sub_agents
            SET model_provider_id = $3,
                model_id = $4,
                model_variant = $5,
                enabled = CASE
                    WHEN $3 IS NULL AND $4 IS NULL THEN false
                    ELSE enabled
                END,
                updated_at = now()
           WHERE agent_key = $1
             AND id = $2",
    )
    .bind(agent_key)
    .bind(sub_agent_id)
    .bind(model_provider_id)
    .bind(model_id)
    .bind(model_variant)
    .execute(pool)
    .await
    .with_context(|| format!("failed to update model for job {sub_agent_id} agent {agent_key}"))?;

    Ok(result.rows_affected() > 0)
}

pub async fn set_sub_agent_timeout(
    pool: &DbPool,
    agent_key: &str,
    sub_agent_id: i64,
    timeout_seconds: i32,
) -> Result<bool> {
    if timeout_seconds <= 0 {
        anyhow::bail!("timeout_seconds must be positive, got {timeout_seconds}");
    }
    let result = sqlx::query(
        "UPDATE harness_sub_agents
            SET timeout_seconds = $3,
                updated_at = now()
          WHERE agent_key = $1
            AND id = $2",
    )
    .bind(agent_key)
    .bind(sub_agent_id)
    .bind(timeout_seconds)
    .execute(pool)
    .await
    .with_context(|| {
        format!("failed to update timeout for job {sub_agent_id} agent {agent_key}")
    })?;

    Ok(result.rows_affected() > 0)
}

/// Change a job's timeframe and re-anchor its next run to the next
/// boundary for that timeframe. Keeping these fields together prevents an
/// edited job from firing at a boundary from its previous cadence.
/// Analysis jobs keep their user-provided sub-agent key; other candle jobs
/// derive their key from kind and timeframe.
pub async fn set_candle_sub_agent_timeframe(
    pool: &DbPool,
    agent_key: &str,
    sub_agent_id: i64,
    timeframe: &str,
) -> Result<bool> {
    let timeframe = timeframe.trim();
    parse_timeframe_seconds(timeframe)
        .with_context(|| format!("invalid timeframe {timeframe:?}"))?;

    let mut tx = pool
        .begin()
        .await
        .context("failed to begin job timeframe update transaction")?;
    lock_agent_coordination_tx(&mut tx, agent_key).await?;
    let job: Option<(String, i32)> = query_as(
        "SELECT sub_agent_kind, trigger_delay_seconds
           FROM harness_sub_agents
           WHERE agent_key = $1
             AND id = $2
              AND timeframe IS NOT NULL
           FOR UPDATE",
    )
    .bind(agent_key)
    .bind(sub_agent_id)
    .fetch_optional(&mut *tx)
    .await
    .with_context(|| format!("failed to lock job {sub_agent_id} for agent {agent_key}"))?;

    let Some((sub_agent_kind, trigger_delay_seconds)) = job else {
        tx.rollback()
            .await
            .context("failed to roll back missing job timeframe update")?;
        return Ok(false);
    };

    let next_run_at = next_due_after(Utc::now(), timeframe, trigger_delay_seconds)?;
    let sub_agent_key = if sub_agent_kind == SUB_AGENT_KIND_ANALYSIS {
        None
    } else {
        Some(build_generated_sub_agent_key(&sub_agent_kind, timeframe))
    };
    let updated = sqlx::query(
        "UPDATE harness_sub_agents
            SET sub_agent_key = COALESCE($3, sub_agent_key),
                timeframe = $4,
                next_run_at = $5,
                updated_at = now()
          WHERE agent_key = $1
            AND id = $2",
    )
    .bind(agent_key)
    .bind(sub_agent_id)
    .bind(sub_agent_key)
    .bind(timeframe)
    .bind(next_run_at)
    .execute(&mut *tx)
    .await;
    match updated {
        Ok(_) => {}
        Err(sqlx::Error::Database(error))
            if error.is_unique_violation() && sub_agent_kind == SUB_AGENT_KIND_ANALYSIS =>
        {
            anyhow::bail!("another analysis job already uses this sub-agent key");
        }
        Err(error) => return Err(error).context("failed to update analysis job timeframe"),
    }
    tx.commit()
        .await
        .context("failed to commit job timeframe update")?;

    Ok(true)
}

/// List OpenCode jobs that are due and dispatchable.
///
/// Disabled agents and jobs are excluded so the jobr only sees
/// runs that can be claimed.
pub async fn list_due_candle_sub_agents(
    pool: &DbPool,
    now: DateTime<Utc>,
    limit: i64,
    opencode_base_url: &str,
) -> Result<Vec<HarnessDispatchSubAgentRow>> {
    let rows = query_as::<_, HarnessDispatchSubAgentRow>(
        "SELECT jobs.id AS sub_agent_id,
                agents.agent_key,
                agents.display_name,
                 jobs.sub_agent_key,
                 jobs.sub_agent_kind,
                 jobs.timeframe,
                jobs.trigger_delay_seconds,
                jobs.next_run_at,
                jobs.model_provider_id,
                jobs.model_id,
                 jobs.model_variant,
                  jobs.timeout_seconds,
                   jobs.enabled_capabilities,
                 $3::text AS opencode_base_url,
                agents.runtime_config
           FROM harness_sub_agents AS jobs
           JOIN agents
             ON agents.agent_key = jobs.agent_key
             WHERE jobs.enabled = true
               AND jobs.next_run_at IS NOT NULL
             AND jobs.next_run_at <= $1
              AND agents.enabled = true
               AND agents.lifecycle = 'active'
          ORDER BY jobs.next_run_at ASC, jobs.id ASC
          LIMIT $2",
    )
    .bind(now)
    .bind(limit)
    .bind(opencode_base_url)
    .fetch_all(pool)
    .await
    .context("failed to list due OpenCode jobs")?;

    Ok(rows)
}

/// Load one OpenCode job with all metadata required for dispatch.
///
/// This is used by manual `Run now` actions, so it intentionally does not
/// require the job itself to be enabled or due.
pub async fn get_dispatch_sub_agent(
    pool: &DbPool,
    agent_key: &str,
    sub_agent_id: i64,
    opencode_base_url: &str,
) -> Result<Option<HarnessDispatchSubAgentRow>> {
    let row = query_as::<_, HarnessDispatchSubAgentRow>(
        "SELECT jobs.id AS sub_agent_id,
                agents.agent_key,
                agents.display_name,
                 jobs.sub_agent_key,
                 jobs.sub_agent_kind,
                 jobs.timeframe,
                jobs.trigger_delay_seconds,
                jobs.next_run_at,
                jobs.model_provider_id,
                jobs.model_id,
                 jobs.model_variant,
                  jobs.timeout_seconds,
                   jobs.enabled_capabilities,
                 $3::text AS opencode_base_url,
                agents.runtime_config
           FROM harness_sub_agents AS jobs
           JOIN agents
             ON agents.agent_key = jobs.agent_key
            WHERE jobs.agent_key = $1
                AND jobs.id = $2
                AND agents.enabled = true
                AND agents.lifecycle = 'active'",
    )
    .bind(agent_key)
    .bind(sub_agent_id)
    .bind(opencode_base_url)
    .fetch_optional(pool)
    .await
    .with_context(|| {
        format!(
            "failed to load OpenCode dispatch metadata for job {sub_agent_id} agent {agent_key}"
        )
    })?;

    Ok(row)
}

#[derive(Debug, Clone)]
pub enum ClaimedCandleSubAgentRun {
    /// The job was due and is now claimed for dispatch. The run is
    /// in `queued` state with the returned id.
    Dispatch { run_id: i64 },
    /// The job was due but a previous run for the same agent was
    /// still active. A `skipped` run row was inserted and returned; no
    /// backend dispatch should happen.
    Skipped { run_id: i64 },
    /// The job was no longer due (concurrent claim, disabled,
    /// missing, etc.). No row was written.
    NotDue,
}

/// Atomically advance the job, optionally inserting a `queued` or
/// `skipped` `harness_sub_agent_runs` row, and return the outcome.
///
/// The job fires on UTC candle boundaries (per its `timeframe`)
/// plus a fixed `trigger_delay_seconds` of slack. When the job is
/// overdue after downtime, the next future boundary is computed
/// without replaying missed runs.
pub async fn claim_due_candle_sub_agent(
    pool: &DbPool,
    sub_agent_id: i64,
    now: DateTime<Utc>,
) -> Result<ClaimedCandleSubAgentRun> {
    let mut tx = pool
        .begin()
        .await
        .context("failed to begin claim transaction")?;

    let agent_key: Option<(String,)> =
        query_as("SELECT agent_key FROM harness_sub_agents WHERE id = $1")
            .bind(sub_agent_id)
            .fetch_optional(&mut *tx)
            .await
            .context("failed to load job agent for claim")?;
    let Some((agent_key,)) = agent_key else {
        tx.rollback()
            .await
            .context("failed to roll back missing-job claim")?;
        return Ok(ClaimedCandleSubAgentRun::NotDue);
    };
    lock_agent_coordination_tx(&mut tx, &agent_key).await?;

    let job: Option<CandleSubAgentForUpdate> = query_as(
        "SELECT id,
                agent_key,
                sub_agent_key,
                sub_agent_kind,
                enabled,
                timeframe,
                trigger_delay_seconds,
                next_run_at,
                model_provider_id,
                model_id,
                model_variant,
                timeout_seconds
           FROM harness_sub_agents
           WHERE id = $1
              AND timeframe IS NOT NULL
          FOR UPDATE",
    )
    .bind(sub_agent_id)
    .fetch_optional(&mut *tx)
    .await
    .context("failed to lock job for claim")?;

    let Some(job) = job else {
        tx.rollback()
            .await
            .context("failed to roll back missing-job claim")?;
        return Ok(ClaimedCandleSubAgentRun::NotDue);
    };

    if !job.enabled || job.next_run_at > now {
        tx.rollback()
            .await
            .context("failed to roll back not-due claim")?;
        return Ok(ClaimedCandleSubAgentRun::NotDue);
    }

    let timeframe = job.timeframe.clone();
    let trigger_delay_seconds = job.trigger_delay_seconds;

    let latest_due = match latest_due_at_or_before(now, &timeframe, trigger_delay_seconds)
        .with_context(|| format!("invalid timeframe {timeframe:?} on job {}", job.id))?
    {
        Some(latest) => latest,
        None => {
            // We are not yet at the first valid due instant. Skip and
            // re-anchor to the next future boundary.
            advance_candle_job(&mut tx, &job, now).await?;
            tx.commit()
                .await
                .context("failed to commit early-skip claim")?;
            return Ok(ClaimedCandleSubAgentRun::NotDue);
        }
    };

    if job.next_run_at < latest_due {
        // CandleSubAgent is stale after downtime. Skip and re-anchor.
        advance_candle_job(&mut tx, &job, now).await?;
        tx.commit()
            .await
            .context("failed to commit stale-skip claim")?;
        return Ok(ClaimedCandleSubAgentRun::NotDue);
    }

    let scheduled_for = boundary_for_due_at(job.next_run_at, trigger_delay_seconds);

    let outcome =
        if has_active_run_in_lane_tx(&mut tx, &job.agent_key, &job.sub_agent_kind, now).await? {
            let run_id = insert_run_with_model_variant_in_tx(
                &mut tx,
                job.id,
                &job.agent_key,
                &job.sub_agent_key,
                &job.sub_agent_kind,
                Some(&timeframe),
                RUN_STATUS_SKIPPED,
                None,
                job.model_provider_id.as_deref(),
                job.model_id.as_deref(),
                job.model_variant.as_deref(),
                scheduled_for,
                None,
                Some(now),
                job.timeout_seconds,
                Some("previous run still active"),
            )
            .await?;
            ClaimedCandleSubAgentRun::Skipped { run_id }
        } else {
            let run_id = insert_run_with_model_variant_in_tx(
                &mut tx,
                job.id,
                &job.agent_key,
                &job.sub_agent_key,
                &job.sub_agent_kind,
                Some(&timeframe),
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
            ClaimedCandleSubAgentRun::Dispatch { run_id }
        };

    advance_candle_job(&mut tx, &job, now).await?;

    tx.commit()
        .await
        .context("failed to commit claim transaction")?;

    Ok(outcome)
}

async fn advance_candle_job(
    tx: &mut Transaction<'_, Postgres>,
    job: &CandleSubAgentForUpdate,
    now: DateTime<Utc>,
) -> Result<()> {
    let next_run_at =
        next_due_after(now, &job.timeframe, job.trigger_delay_seconds).with_context(|| {
            format!(
                "failed to compute next_run_at for timeframe {:?} on job {}",
                job.timeframe, job.id
            )
        })?;
    sqlx::query(
        "UPDATE harness_sub_agents
            SET next_run_at = $2,
                updated_at = now()
          WHERE id = $1",
    )
    .bind(job.id)
    .bind(next_run_at)
    .execute(&mut **tx)
    .await
    .context("failed to advance job next_run_at")?;
    Ok(())
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct CandleSubAgentForUpdate {
    pub(crate) id: i64,
    pub(crate) agent_key: String,
    pub(crate) sub_agent_key: String,
    pub(crate) sub_agent_kind: String,
    pub(crate) enabled: bool,
    pub(crate) timeframe: String,
    pub(crate) trigger_delay_seconds: i32,
    pub(crate) next_run_at: DateTime<Utc>,
    pub(crate) model_provider_id: Option<String>,
    pub(crate) model_id: Option<String>,
    pub(crate) model_variant: Option<String>,
    pub(crate) timeout_seconds: i32,
}
