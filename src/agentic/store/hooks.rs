use anyhow::{Context, Result};
use sqlx::query_as;

use crate::{
    agentic::{
        job_key::build_generated_hook_job_key,
        model::{
            AgenticJobHookRow, AgenticRunRow, DueOpenCodeHookRow,
            HOOK_EVENT_ANALYSIS_BATCH_COMPLETED, HOOK_EVENT_DAILY_REVIEW_COMPLETED,
            JOB_KIND_ANALYSIS_CODING, JOB_KIND_MARKET_ANALYSIS,
        },
    },
    db::DbPool,
};

const DEFAULT_MARKET_ANALYSIS_TIMEOUT_SECONDS: i32 = 900;
const DEFAULT_CODING_TIMEOUT_SECONDS: i32 = 1800;

/// Seed the default OpenCode agent hooks. Both the market-analysis hook
/// (used after a successful analysis batch) and the
/// analysis-coding hook (used after a successful daily-review that
/// requests code work) are seeded disabled and without a model. The
/// operator must explicitly enable them; analysis-coding also
/// requires the operator to pick a strong provider/model before it
/// will queue automatically. This single insert is called both by the
/// default-schedule seeding path for new agents and by the migration
/// backfill for existing agents.
pub(crate) async fn insert_default_opencode_hooks(pool: &DbPool, agent_key: &str) -> Result<()> {
    let market_job_key = build_generated_hook_job_key(JOB_KIND_MARKET_ANALYSIS);

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
    .bind(&market_job_key)
    .bind(JOB_KIND_MARKET_ANALYSIS)
    .bind(HOOK_EVENT_ANALYSIS_BATCH_COMPLETED)
    .bind(false)
    .bind(DEFAULT_MARKET_ANALYSIS_TIMEOUT_SECONDS)
    .bind("")
    .execute(pool)
    .await
    .with_context(|| {
        format!("failed to insert default {market_job_key} hook for agent {agent_key}")
    })?;

    let coding_job_key = build_generated_hook_job_key(JOB_KIND_ANALYSIS_CODING);

    sqlx::query(
        "INSERT INTO agentic_job_hooks (
            agent_key,
            job_key,
            job_kind,
            hook_event,
            enabled,
             model_provider_id,
             model_id,
             model_variant,
             timeout_seconds,
             operator_prompt
          ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
         ON CONFLICT (agent_key, job_kind, hook_event) DO NOTHING",
    )
    .bind(agent_key)
    .bind(&coding_job_key)
    .bind(JOB_KIND_ANALYSIS_CODING)
    .bind(HOOK_EVENT_DAILY_REVIEW_COMPLETED)
    .bind(false)
    .bind(None::<&str>)
    .bind(None::<&str>)
    .bind(None::<&str>)
    .bind(DEFAULT_CODING_TIMEOUT_SECONDS)
    .bind("")
    .execute(pool)
    .await
    .with_context(|| {
        format!("failed to insert default {coding_job_key} hook for agent {agent_key}")
    })?;

    Ok(())
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
                model_variant,
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
                model_variant,
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
                model_variant,
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

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
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
    insert_agent_hook_with_model_variant(
        pool,
        agent_key,
        job_kind,
        hook_event,
        enabled,
        model_provider_id,
        model_id,
        None,
        timeout_seconds,
        operator_prompt,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_agent_hook_with_model_variant(
    pool: &DbPool,
    agent_key: &str,
    job_kind: &str,
    hook_event: &str,
    enabled: bool,
    model_provider_id: Option<&str>,
    model_id: Option<&str>,
    model_variant: Option<&str>,
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
            model_variant,
            timeout_seconds,
            operator_prompt
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
         RETURNING id",
    )
    .bind(agent_key)
    .bind(&job_key)
    .bind(job_kind)
    .bind(hook_event)
    .bind(enabled)
    .bind(model_provider_id)
    .bind(model_id)
    .bind(model_variant)
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
            AND id = $2
            AND ($3 = false OR (model_provider_id IS NOT NULL AND model_id IS NOT NULL))",
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

pub async fn set_hook_model_with_variant(
    pool: &DbPool,
    agent_key: &str,
    hook_id: i64,
    model_provider_id: Option<&str>,
    model_id: Option<&str>,
    model_variant: Option<&str>,
) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE agentic_job_hooks
            SET model_provider_id = $3,
                model_id = $4,
                model_variant = $5,
                updated_at = now()
          WHERE agent_key = $1
            AND id = $2",
    )
    .bind(agent_key)
    .bind(hook_id)
    .bind(model_provider_id)
    .bind(model_id)
    .bind(model_variant)
    .execute(pool)
    .await
    .with_context(|| format!("failed to update model for hook {hook_id} agent {agent_key}"))?;

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

pub async fn get_opencode_hook_for_dispatch(
    pool: &DbPool,
    agent_key: &str,
    hook_id: i64,
    opencode_base_url: &str,
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
                hooks.model_variant,
                hooks.timeout_seconds,
                hooks.operator_prompt,
                $3::text AS opencode_base_url,
                agents.runtime_config
           FROM agentic_job_hooks AS hooks
           JOIN agents
             ON agents.agent_key = hooks.agent_key
           WHERE hooks.agent_key = $1
              AND hooks.id = $2
                AND agents.lifecycle = 'active'",
    )
    .bind(agent_key)
    .bind(hook_id)
    .bind(opencode_base_url)
    .fetch_optional(pool)
    .await
    .with_context(|| {
        format!("failed to load OpenCode dispatch metadata for hook {hook_id} agent {agent_key}")
    })?;

    Ok(row)
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
                model_variant,
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

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct HookForUpdate {
    pub(crate) id: i64,
    pub(crate) agent_key: String,
    pub(crate) job_key: String,
    pub(crate) job_kind: String,
    pub(crate) timeout_seconds: i32,
    pub(crate) model_provider_id: Option<String>,
    pub(crate) model_id: Option<String>,
    pub(crate) model_variant: Option<String>,
}
