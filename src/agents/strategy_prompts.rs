use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sqlx::query_as;
use uuid::Uuid;

use crate::{
    agents::prompts::{
        DEFAULT_ANALYSIS_CODING_STRATEGY_PROMPT, DEFAULT_ANALYSIS_STRATEGY_PROMPT,
        DEFAULT_DAILY_REVIEW_STRATEGY_PROMPT, DEFAULT_MARKET_ANALYSIS_STRATEGY_PROMPT,
        DEFAULT_TRADING_STRATEGY_PROMPT,
    },
    db::DbPool,
};

pub const PROMPT_KIND_ANALYSIS: &str = "analysis";
pub const PROMPT_KIND_MARKET_ANALYSIS: &str = "market_analysis";
pub const PROMPT_KIND_TRADING: &str = "trading";
pub const PROMPT_KIND_DAILY_REVIEW: &str = "daily_review";
pub const PROMPT_KIND_ANALYSIS_CODING: &str = "analysis_coding";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AgentStrategyPromptRow {
    pub revision_id: i64,
    pub prompt_kind: String,
    pub prompt: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct PromptRevisionChange {
    pub prompt_kind: String,
    pub base_revision_id: i64,
    pub prompt: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PromptRevisionHistoryRow {
    pub id: i64,
    pub prompt_kind: String,
    pub prompt: String,
    pub source_type: String,
    pub source_run_id: Option<i64>,
    pub rationale: String,
    pub created_at: DateTime<Utc>,
}

pub async fn list_prompt_revision_history(
    pool: &DbPool,
    agent_key: &str,
) -> Result<Vec<PromptRevisionHistoryRow>> {
    query_as("SELECT revisions.id, revisions.prompt_kind, revisions.prompt, batches.source_type, batches.source_run_id, batches.rationale, revisions.created_at
              FROM agent_strategy_prompt_revisions revisions
              JOIN agent_strategy_prompt_revision_batches batches ON batches.id = revisions.batch_id
             WHERE revisions.agent_key = $1 ORDER BY revisions.id DESC")
        .bind(agent_key).fetch_all(pool).await.context("failed to list prompt revision history")
}

pub async fn rollback_prompt_revision(
    pool: &DbPool,
    agent_key: &str,
    revision_id: i64,
) -> Result<bool> {
    let target: Option<(String, String)> = sqlx::query_as(
        "SELECT prompt_kind, prompt FROM agent_strategy_prompt_revisions WHERE id = $1 AND agent_key = $2",
    ).bind(revision_id).bind(agent_key).fetch_optional(pool).await?;
    let Some((prompt_kind, prompt)) = target else {
        return Ok(false);
    };
    let current = get_agent_strategy_prompt(pool, agent_key, &prompt_kind)
        .await?
        .context("active prompt not found")?;
    if current.prompt == prompt {
        return Ok(true);
    }
    let mut tx = pool.begin().await?;
    let batch_id: i64 = sqlx::query_scalar("INSERT INTO agent_strategy_prompt_revision_batches (agent_key, source_type, rationale) VALUES ($1, 'rollback', $2) RETURNING id")
        .bind(agent_key).bind(format!("Rollback to revision {revision_id}")).fetch_one(&mut *tx).await?;
    let new_revision: i64 = sqlx::query_scalar("INSERT INTO agent_strategy_prompt_revisions (batch_id, agent_key, prompt_kind, parent_revision_id, rollback_of_revision_id, prompt) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id")
        .bind(batch_id).bind(agent_key).bind(&prompt_kind).bind(current.revision_id).bind(revision_id).bind(&prompt).fetch_one(&mut *tx).await?;
    sqlx::query("UPDATE agent_strategy_prompt_active_revisions SET revision_id = $3, activated_at = now() WHERE agent_key = $1 AND prompt_kind = $2")
        .bind(agent_key).bind(&prompt_kind).bind(new_revision).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(true)
}

pub async fn submit_daily_review_revisions(
    pool: &DbPool,
    agent_key: &str,
    source_run_id: i64,
    rationale: &str,
    evidence_memory_ids: &[Uuid],
    changes: &[PromptRevisionChange],
) -> Result<i64> {
    anyhow::ensure!(
        !rationale.trim().is_empty() && rationale.len() <= 8192,
        "invalid rationale"
    );
    anyhow::ensure!(
        !changes.is_empty() && changes.len() <= 3,
        "invalid revision changes"
    );
    let mut kinds = std::collections::BTreeSet::new();
    for change in changes {
        anyhow::ensure!(
            matches!(
                change.prompt_kind.as_str(),
                PROMPT_KIND_ANALYSIS | PROMPT_KIND_MARKET_ANALYSIS | PROMPT_KIND_TRADING
            ),
            "daily review cannot revise this prompt kind"
        );
        anyhow::ensure!(
            change.base_revision_id > 0
                && !change.prompt.trim().is_empty()
                && change.prompt.len() <= 65_536,
            "invalid prompt revision change"
        );
        anyhow::ensure!(
            kinds.insert(change.prompt_kind.as_str()),
            "duplicate prompt kind"
        );
    }
    let mut tx = pool.begin().await?;
    let is_daily_review: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM harness_sub_agent_runs WHERE id = $1 AND agent_key = $2 AND sub_agent_kind = 'daily_review')",
    ).bind(source_run_id).bind(agent_key).fetch_one(&mut *tx).await?;
    anyhow::ensure!(
        is_daily_review,
        "source run is not an owned daily-review run"
    );
    for memory_id in evidence_memory_ids {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM memory.records WHERE id = $1 AND agent_key = $2)",
        )
        .bind(memory_id)
        .bind(agent_key)
        .fetch_one(&mut *tx)
        .await?;
        anyhow::ensure!(exists, "evidence memory does not belong to agent");
    }
    let batch_id: i64 = sqlx::query_scalar(
        "INSERT INTO agent_strategy_prompt_revision_batches (agent_key, source_type, source_run_id, rationale) VALUES ($1, 'daily_review', $2, $3) RETURNING id",
    ).bind(agent_key).bind(source_run_id).bind(rationale.trim()).fetch_one(&mut *tx).await?;
    for memory_id in evidence_memory_ids {
        sqlx::query("INSERT INTO agent_strategy_prompt_revision_evidence (revision_batch_id, memory_id, agent_key) VALUES ($1, $2, $3)")
            .bind(batch_id).bind(memory_id).bind(agent_key).execute(&mut *tx).await?;
    }
    for change in changes {
        let current: AgentStrategyPromptRow = query_as(
            "SELECT active.revision_id, revisions.prompt_kind, revisions.prompt, active.activated_at AS updated_at FROM agent_strategy_prompt_active_revisions active JOIN agent_strategy_prompt_revisions revisions ON revisions.id = active.revision_id WHERE active.agent_key = $1 AND active.prompt_kind = $2 FOR UPDATE",
        ).bind(agent_key).bind(&change.prompt_kind).fetch_one(&mut *tx).await?;
        anyhow::ensure!(
            current.revision_id == change.base_revision_id,
            "strategy prompt revision is stale"
        );
        anyhow::ensure!(
            current.prompt != change.prompt,
            "strategy prompt is unchanged"
        );
        let revision_id: i64 = sqlx::query_scalar("INSERT INTO agent_strategy_prompt_revisions (batch_id, agent_key, prompt_kind, parent_revision_id, prompt) VALUES ($1, $2, $3, $4, $5) RETURNING id")
            .bind(batch_id).bind(agent_key).bind(&change.prompt_kind).bind(current.revision_id).bind(&change.prompt).fetch_one(&mut *tx).await?;
        sqlx::query("UPDATE agent_strategy_prompt_active_revisions SET revision_id = $3, activated_at = now() WHERE agent_key = $1 AND prompt_kind = $2")
            .bind(agent_key).bind(&change.prompt_kind).bind(revision_id).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(batch_id)
}

pub fn prompt_kind_for_sub_agent_kind(sub_agent_kind: &str) -> Option<&'static str> {
    match sub_agent_kind {
        crate::harness::model::SUB_AGENT_KIND_ANALYSIS => Some(PROMPT_KIND_ANALYSIS),
        crate::harness::model::SUB_AGENT_KIND_MARKET_ANALYSIS => Some(PROMPT_KIND_MARKET_ANALYSIS),
        crate::harness::model::SUB_AGENT_KIND_TRADING => Some(PROMPT_KIND_TRADING),
        crate::harness::model::SUB_AGENT_KIND_DAILY_REVIEW => Some(PROMPT_KIND_DAILY_REVIEW),
        crate::harness::model::SUB_AGENT_KIND_ANALYSIS_CODING => Some(PROMPT_KIND_ANALYSIS_CODING),
        _ => None,
    }
}

pub fn is_valid_prompt_kind(value: &str) -> bool {
    matches!(
        value,
        PROMPT_KIND_ANALYSIS
            | PROMPT_KIND_MARKET_ANALYSIS
            | PROMPT_KIND_TRADING
            | PROMPT_KIND_DAILY_REVIEW
            | PROMPT_KIND_ANALYSIS_CODING
    )
}

pub fn default_prompt_for_kind(prompt_kind: &str) -> &'static str {
    match prompt_kind {
        PROMPT_KIND_ANALYSIS => DEFAULT_ANALYSIS_STRATEGY_PROMPT,
        PROMPT_KIND_MARKET_ANALYSIS => DEFAULT_MARKET_ANALYSIS_STRATEGY_PROMPT,
        PROMPT_KIND_TRADING => DEFAULT_TRADING_STRATEGY_PROMPT,
        PROMPT_KIND_DAILY_REVIEW => DEFAULT_DAILY_REVIEW_STRATEGY_PROMPT,
        PROMPT_KIND_ANALYSIS_CODING => DEFAULT_ANALYSIS_CODING_STRATEGY_PROMPT,
        _ => "",
    }
}

pub fn all_prompt_kinds() -> [&'static str; 5] {
    [
        PROMPT_KIND_ANALYSIS,
        PROMPT_KIND_MARKET_ANALYSIS,
        PROMPT_KIND_TRADING,
        PROMPT_KIND_DAILY_REVIEW,
        PROMPT_KIND_ANALYSIS_CODING,
    ]
}

pub async fn list_agent_strategy_prompts(
    pool: &DbPool,
    agent_key: &str,
) -> Result<Vec<AgentStrategyPromptRow>> {
    query_as::<_, AgentStrategyPromptRow>(
        "SELECT active.revision_id, revisions.prompt_kind, revisions.prompt, active.activated_at AS updated_at
           FROM agent_strategy_prompt_active_revisions active
           JOIN agent_strategy_prompt_revisions revisions ON revisions.id = active.revision_id
          WHERE active.agent_key = $1
          ORDER BY prompt_kind",
    )
    .bind(agent_key)
    .fetch_all(pool)
    .await
    .with_context(|| format!("failed to list strategy prompts for agent {agent_key}"))
}

pub async fn get_agent_strategy_prompt(
    pool: &DbPool,
    agent_key: &str,
    prompt_kind: &str,
) -> Result<Option<AgentStrategyPromptRow>> {
    query_as::<_, AgentStrategyPromptRow>(
        "SELECT active.revision_id, revisions.prompt_kind, revisions.prompt, active.activated_at AS updated_at
           FROM agent_strategy_prompt_active_revisions active
           JOIN agent_strategy_prompt_revisions revisions ON revisions.id = active.revision_id
          WHERE active.agent_key = $1
            AND active.prompt_kind = $2",
    )
    .bind(agent_key)
    .bind(prompt_kind)
    .fetch_optional(pool)
    .await
    .with_context(|| format!("failed to load strategy prompt {prompt_kind} for agent {agent_key}"))
}

pub async fn upsert_agent_strategy_prompt(
    pool: &DbPool,
    agent_key: &str,
    prompt_kind: &str,
    prompt: &str,
) -> Result<bool> {
    let Some(current) = get_agent_strategy_prompt(pool, agent_key, prompt_kind).await? else {
        return Ok(false);
    };
    if current.prompt == prompt {
        return Ok(true);
    }
    create_prompt_revision(
        pool,
        agent_key,
        prompt_kind,
        prompt,
        current.revision_id,
        "manual",
        "",
    )
    .await?;
    Ok(true)
}

pub async fn create_prompt_revision(
    pool: &DbPool,
    agent_key: &str,
    prompt_kind: &str,
    prompt: &str,
    base_revision_id: i64,
    source_type: &str,
    rationale: &str,
) -> Result<i64> {
    anyhow::ensure!(is_valid_prompt_kind(prompt_kind), "invalid prompt kind");
    anyhow::ensure!(prompt.len() <= 65_536, "invalid prompt content");
    let mut transaction = pool.begin().await?;
    let current = query_as::<_, AgentStrategyPromptRow>(
        "SELECT active.revision_id, revisions.prompt_kind, revisions.prompt, active.activated_at AS updated_at
           FROM agent_strategy_prompt_active_revisions active
           JOIN agent_strategy_prompt_revisions revisions ON revisions.id = active.revision_id
          WHERE active.agent_key = $1 AND active.prompt_kind = $2 FOR UPDATE",
    )
    .bind(agent_key)
    .bind(prompt_kind)
    .fetch_optional(&mut *transaction)
    .await?;
    let current = current.context("strategy prompt not found")?;
    anyhow::ensure!(
        current.revision_id == base_revision_id,
        "strategy prompt revision is stale"
    );
    anyhow::ensure!(current.prompt != prompt, "strategy prompt is unchanged");
    let batch_id: i64 = sqlx::query_scalar(
        "INSERT INTO agent_strategy_prompt_revision_batches (agent_key, source_type, rationale) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(agent_key).bind(source_type).bind(rationale).fetch_one(&mut *transaction).await?;
    let revision_id: i64 = sqlx::query_scalar(
        "INSERT INTO agent_strategy_prompt_revisions (batch_id, agent_key, prompt_kind, parent_revision_id, prompt) VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(batch_id).bind(agent_key).bind(prompt_kind).bind(current.revision_id).bind(prompt).fetch_one(&mut *transaction).await?;
    sqlx::query("UPDATE agent_strategy_prompt_active_revisions SET revision_id = $3, activated_at = now() WHERE agent_key = $1 AND prompt_kind = $2")
        .bind(agent_key).bind(prompt_kind).bind(revision_id).execute(&mut *transaction).await?;
    transaction.commit().await?;
    Ok(revision_id)
}

pub async fn insert_default_strategy_prompts_for_agent(
    pool: &DbPool,
    agent_key: &str,
) -> Result<()> {
    let existing: Vec<String> = sqlx::query_scalar(
        "SELECT prompt_kind FROM agent_strategy_prompt_active_revisions WHERE agent_key = $1",
    )
    .bind(agent_key)
    .fetch_all(pool)
    .await?;
    for prompt_kind in all_prompt_kinds() {
        if existing
            .iter()
            .any(|existing_kind| existing_kind == prompt_kind)
        {
            continue;
        }
        let batch_id: i64 = sqlx::query_scalar("INSERT INTO agent_strategy_prompt_revision_batches (agent_key, source_type, rationale) VALUES ($1, 'migration', 'Initial strategy prompt') RETURNING id")
            .bind(agent_key).fetch_one(pool).await?;
        let revision_id: i64 = sqlx::query_scalar("INSERT INTO agent_strategy_prompt_revisions (batch_id, agent_key, prompt_kind, prompt) VALUES ($1, $2, $3, $4) RETURNING id")
            .bind(batch_id).bind(agent_key).bind(prompt_kind).bind(default_prompt_for_kind(prompt_kind)).fetch_one(pool).await?;
        sqlx::query("INSERT INTO agent_strategy_prompt_active_revisions (agent_key, prompt_kind, revision_id) VALUES ($1, $2, $3)")
            .bind(agent_key).bind(prompt_kind).bind(revision_id).execute(pool).await?;
    }

    Ok(())
}
