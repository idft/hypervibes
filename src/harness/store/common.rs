use anyhow::{Context, Result};
use chrono::Utc;
use sqlx::query_as;

use crate::{
    db::DbPool,
    harness::model::{RUN_STATUS_QUEUED, RUN_STATUS_RUNNING, SUB_AGENT_KIND_CODING},
};

pub(crate) const ACTIVE_STATUSES: [&str; 2] = [RUN_STATUS_QUEUED, RUN_STATUS_RUNNING];

pub(crate) const ERROR_SUMMARY_MAX_CHARS: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EventRunInsertMode {
    Manual,
    CodingTrigger,
}

/// Serialize state transitions that can start work or activate maintenance
/// for one agent. Callers must acquire this before locking a schedule, event,
/// or maintenance task row.
pub(crate) async fn lock_agent_coordination_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    agent_key: &str,
) -> Result<()> {
    let exists: Option<(String,)> =
        query_as("SELECT agent_key FROM agents WHERE agent_key = $1 FOR UPDATE")
            .bind(agent_key)
            .fetch_optional(&mut **tx)
            .await
            .with_context(|| format!("failed to lock agent coordination row for {agent_key}"))?;
    if exists.is_none() {
        anyhow::bail!("agent {agent_key} not found")
    }
    Ok(())
}

/// Toggle every sub-agent for an agent to the same enabled state.
///
/// Bulk enable is intentionally conservative with respect to autonomous
/// code modification: bulk enabling jobs must
/// NOT enable the Coding role. That role has to be
/// enabled by hand and pinned to an explicit strong provider/model
/// before any automatic code coding can fire. Bulk disable still
/// turns every event (including coding) off.
pub async fn set_all_agent_sub_agents_enabled(
    pool: &DbPool,
    agent_key: &str,
    enabled: bool,
) -> Result<()> {
    let mut tx = pool
        .begin()
        .await
        .context("failed to start sub-agent toggle transaction")?;
    lock_agent_coordination_tx(&mut tx, agent_key).await?;

    sqlx::query(
        "UPDATE harness_sub_agents
            SET enabled = $2,
                updated_at = now()
          WHERE agent_key = $1
            AND ($2 = false OR (
                model_provider_id IS NOT NULL
                AND model_id IS NOT NULL
                AND sub_agent_kind <> $3
            ))",
    )
    .bind(agent_key)
    .bind(enabled)
    .bind(SUB_AGENT_KIND_CODING)
    .execute(&mut *tx)
    .await
    .with_context(|| format!("failed to toggle sub-agents for agent {agent_key}"))?;

    tx.commit()
        .await
        .context("failed to commit sub-agent toggle transaction")?;

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
pub(crate) async fn insert_run_with_model_variant_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    sub_agent_id: i64,
    agent_key: &str,
    sub_agent_key: &str,
    sub_agent_kind: &str,
    timeframe: Option<&str>,
    status: &str,
    backend_run_ref: Option<&str>,
    model_provider_id: Option<&str>,
    model_id: Option<&str>,
    model_variant: Option<&str>,
    scheduled_for: chrono::DateTime<Utc>,
    started_at: Option<chrono::DateTime<Utc>>,
    finished_at: Option<chrono::DateTime<Utc>>,
    timeout_seconds: i32,
    error_summary: Option<&str>,
) -> Result<i64> {
    let truncated = error_summary.map(truncate_error_summary);
    let row: (i64,) = query_as(
        "INSERT INTO harness_sub_agent_runs (
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
            error_summary
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
         RETURNING id",
    )
    .bind(sub_agent_id)
    .bind(agent_key)
    .bind(sub_agent_key)
    .bind(sub_agent_kind)
    .bind(timeframe)
    .bind(status)
    .bind(backend_run_ref)
    .bind(model_provider_id)
    .bind(model_id)
    .bind(model_variant)
    .bind(scheduled_for)
    .bind(started_at)
    .bind(finished_at)
    .bind(timeout_seconds)
    .bind(truncated)
    .fetch_one(&mut **tx)
    .await
    .context("failed to insert harness_sub_agent_runs row")?;

    Ok(row.0)
}
