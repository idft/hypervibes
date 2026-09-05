use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sqlx::query_as;
use uuid::Uuid;

use crate::{
    agents::prompts::{
        DEFAULT_ANALYSIS_STRATEGY_PROMPT, DEFAULT_REVIEW_STRATEGY_PROMPT,
        DEFAULT_TRADING_STRATEGY_PROMPT,
    },
    db::DbPool,
    harness::model::{SUB_AGENT_KIND_ANALYSIS, SUB_AGENT_KIND_REVIEW, SUB_AGENT_KIND_TRADING},
};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AgentStrategyPromptRow {
    pub revision_id: i64,
    pub target_sub_agent_id: i64,
    pub target_sub_agent_key: String,
    pub target_sub_agent_kind: String,
    pub prompt: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct PromptRevisionChange {
    pub target_sub_agent_id: i64,
    pub base_revision_id: i64,
    pub prompt: String,
}

pub async fn rollback_prompt_revision(
    pool: &DbPool,
    agent_key: &str,
    revision_id: i64,
) -> Result<bool> {
    let target: Option<(i64, String)> = sqlx::query_as(
        "SELECT target_sub_agent_id, prompt FROM agent_strategy_prompt_revisions WHERE id = $1 AND agent_key = $2",
    ).bind(revision_id).bind(agent_key).fetch_optional(pool).await?;
    let Some((target_sub_agent_id, prompt)) = target else {
        return Ok(false);
    };
    let current = get_agent_strategy_prompt(pool, agent_key, target_sub_agent_id)
        .await?
        .context("active prompt not found")?;
    if current.prompt == prompt {
        return Ok(true);
    }
    let mut tx = pool.begin().await?;
    let batch_id: i64 = sqlx::query_scalar("INSERT INTO agent_strategy_prompt_revision_batches (agent_key, source_type, rationale) VALUES ($1, 'rollback', $2) RETURNING id")
        .bind(agent_key).bind(format!("Rollback to revision {revision_id}")).fetch_one(&mut *tx).await?;
    let new_revision: i64 = sqlx::query_scalar("INSERT INTO agent_strategy_prompt_revisions (batch_id, agent_key, target_sub_agent_id, target_sub_agent_key, parent_revision_id, rollback_of_revision_id, prompt) VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING id")
        .bind(batch_id).bind(agent_key).bind(target_sub_agent_id).bind(&current.target_sub_agent_key).bind(current.revision_id).bind(revision_id).bind(&prompt).fetch_one(&mut *tx).await?;
    sqlx::query("UPDATE agent_strategy_prompt_active_revisions SET revision_id = $3, activated_at = now() WHERE agent_key = $1 AND target_sub_agent_id = $2")
        .bind(agent_key).bind(target_sub_agent_id).bind(new_revision).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(true)
}

pub async fn submit_review_revisions(
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
        !changes.is_empty() && changes.len() <= 8,
        "invalid revision changes"
    );
    let mut target_ids = std::collections::BTreeSet::new();
    for change in changes {
        anyhow::ensure!(
            change.base_revision_id > 0
                && !change.prompt.trim().is_empty()
                && change.prompt.len() <= 65_536,
            "invalid prompt revision change"
        );
        anyhow::ensure!(
            target_ids.insert(change.target_sub_agent_id),
            "duplicate prompt target"
        );
    }
    let mut tx = pool.begin().await?;
    let source_run = query_as::<_, (String, i64, String)>(
        "SELECT sub_agent_kind, sub_agent_id, sub_agent_key FROM harness_sub_agent_runs WHERE id = $1 AND agent_key = $2",
    )
    .bind(source_run_id)
    .bind(agent_key)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((sub_agent_kind, _, _)) = source_run else {
        anyhow::bail!("source run is not an owned review run");
    };
    anyhow::ensure!(
        sub_agent_kind == SUB_AGENT_KIND_REVIEW,
        "source run is not an owned review run"
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
    // Validate every target: same-agent ownership, analysis opt-in to Review
    // prompt updates (or the trading singleton), and a matching base
    // revision. The Review singleton may only revise Trading prompts and
    // opted-in Analysis prompts, never its own prompt or Coding's.
    for change in changes {
        let target: Option<(String, Vec<String>)> = sqlx::query_as(
            "SELECT jobs.sub_agent_kind, jobs.enabled_capabilities
               FROM harness_sub_agents AS jobs
              WHERE jobs.id = $1 AND jobs.agent_key = $2
              FOR UPDATE OF jobs",
        )
        .bind(change.target_sub_agent_id)
        .bind(agent_key)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((target_kind, enabled_capabilities)) = target else {
            anyhow::bail!("prompt revision target does not belong to agent");
        };
        match target_kind.as_str() {
            SUB_AGENT_KIND_TRADING => {}
            SUB_AGENT_KIND_ANALYSIS => {
                anyhow::ensure!(
                    enabled_capabilities.iter().any(|capability| capability
                        == crate::harness::model::CAPABILITY_REVIEW_PROMPT_UPDATE),
                    "analysis job has not opted in to review prompt updates"
                );
            }
            other => anyhow::bail!(
                "review cannot revise the {other} prompt; only Trading and opted-in Analysis prompts are eligible"
            ),
        }
        let current: AgentStrategyPromptRow = query_as(
            "SELECT active.revision_id, active.target_sub_agent_id, revisions.target_sub_agent_key, jobs.sub_agent_kind AS target_sub_agent_kind, revisions.prompt, active.activated_at AS updated_at
               FROM agent_strategy_prompt_active_revisions active
               JOIN agent_strategy_prompt_revisions revisions ON revisions.id = active.revision_id
              WHERE active.agent_key = $1 AND active.target_sub_agent_id = $2
              FOR UPDATE OF active",
        ).bind(agent_key).bind(change.target_sub_agent_id).fetch_one(&mut *tx).await?;
        anyhow::ensure!(
            current.revision_id == change.base_revision_id,
            "strategy prompt revision is stale"
        );
        anyhow::ensure!(
            current.prompt != change.prompt,
            "strategy prompt is unchanged"
        );
    }
    let batch_id: i64 = sqlx::query_scalar(
        "INSERT INTO agent_strategy_prompt_revision_batches (agent_key, source_type, source_run_id, rationale) VALUES ($1, 'review', $2, $3) RETURNING id",
    ).bind(agent_key).bind(source_run_id).bind(rationale.trim()).fetch_one(&mut *tx).await?;
    for memory_id in evidence_memory_ids {
        sqlx::query("INSERT INTO agent_strategy_prompt_revision_evidence (revision_batch_id, memory_id, agent_key) VALUES ($1, $2, $3)")
            .bind(batch_id).bind(memory_id).bind(agent_key).execute(&mut *tx).await?;
    }
    for change in changes {
        let current: AgentStrategyPromptRow = query_as(
            "SELECT active.revision_id, active.target_sub_agent_id, revisions.target_sub_agent_key, jobs.sub_agent_kind AS target_sub_agent_kind, revisions.prompt, active.activated_at AS updated_at
               FROM agent_strategy_prompt_active_revisions active
               JOIN agent_strategy_prompt_revisions revisions ON revisions.id = active.revision_id
              WHERE active.agent_key = $1 AND active.target_sub_agent_id = $2",
        ).bind(agent_key).bind(change.target_sub_agent_id).fetch_one(&mut *tx).await?;
        let revision_id: i64 = sqlx::query_scalar(
            "INSERT INTO agent_strategy_prompt_revisions (batch_id, agent_key, target_sub_agent_id, target_sub_agent_key, parent_revision_id, prompt) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
        )
        .bind(batch_id).bind(agent_key).bind(change.target_sub_agent_id).bind(&current.target_sub_agent_key).bind(current.revision_id).bind(&change.prompt).fetch_one(&mut *tx).await?;
        sqlx::query("UPDATE agent_strategy_prompt_active_revisions SET revision_id = $3, activated_at = now() WHERE agent_key = $1 AND target_sub_agent_id = $2")
            .bind(agent_key).bind(change.target_sub_agent_id).bind(revision_id).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(batch_id)
}

pub fn default_prompt_for_role(sub_agent_kind: &str) -> &'static str {
    match sub_agent_kind {
        SUB_AGENT_KIND_ANALYSIS => DEFAULT_ANALYSIS_STRATEGY_PROMPT,
        SUB_AGENT_KIND_TRADING => DEFAULT_TRADING_STRATEGY_PROMPT,
        SUB_AGENT_KIND_REVIEW => DEFAULT_REVIEW_STRATEGY_PROMPT,
        _ => "",
    }
}

pub async fn list_agent_strategy_prompts(
    pool: &DbPool,
    agent_key: &str,
) -> Result<Vec<AgentStrategyPromptRow>> {
    query_as::<_, AgentStrategyPromptRow>(
        "SELECT active.revision_id, active.target_sub_agent_id, revisions.target_sub_agent_key, jobs.sub_agent_kind AS target_sub_agent_kind, revisions.prompt, active.activated_at AS updated_at
           FROM agent_strategy_prompt_active_revisions active
           JOIN agent_strategy_prompt_revisions revisions ON revisions.id = active.revision_id
           JOIN harness_sub_agents jobs ON jobs.id = active.target_sub_agent_id AND jobs.agent_key = active.agent_key
          WHERE active.agent_key = $1
          ORDER BY revisions.target_sub_agent_key",
    )
    .bind(agent_key)
    .fetch_all(pool)
    .await
    .with_context(|| format!("failed to list strategy prompts for agent {agent_key}"))
}

pub async fn get_agent_strategy_prompt(
    pool: &DbPool,
    agent_key: &str,
    target_sub_agent_id: i64,
) -> Result<Option<AgentStrategyPromptRow>> {
    query_as::<_, AgentStrategyPromptRow>(
        "SELECT active.revision_id, active.target_sub_agent_id, revisions.target_sub_agent_key, jobs.sub_agent_kind AS target_sub_agent_kind, revisions.prompt, active.activated_at AS updated_at
           FROM agent_strategy_prompt_active_revisions active
           JOIN agent_strategy_prompt_revisions revisions ON revisions.id = active.revision_id
           JOIN harness_sub_agents jobs ON jobs.id = active.target_sub_agent_id AND jobs.agent_key = active.agent_key
          WHERE active.agent_key = $1
            AND active.target_sub_agent_id = $2",
    )
    .bind(agent_key)
    .bind(target_sub_agent_id)
    .fetch_optional(pool)
    .await
    .with_context(|| {
        format!("failed to load strategy prompt for target {target_sub_agent_id} agent {agent_key}")
    })
}

pub async fn upsert_agent_strategy_prompt(
    pool: &DbPool,
    agent_key: &str,
    target_sub_agent_id: i64,
    prompt: &str,
) -> Result<bool> {
    let Some(current) = get_agent_strategy_prompt(pool, agent_key, target_sub_agent_id).await?
    else {
        return Ok(false);
    };
    if current.prompt == prompt {
        return Ok(true);
    }
    create_prompt_revision(
        pool,
        agent_key,
        target_sub_agent_id,
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
    target_sub_agent_id: i64,
    prompt: &str,
    base_revision_id: i64,
    source_type: &str,
    rationale: &str,
) -> Result<i64> {
    anyhow::ensure!(prompt.len() <= 65_536, "invalid prompt content");
    let mut transaction = pool.begin().await?;
    let current = query_as::<_, AgentStrategyPromptRow>(
        "SELECT active.revision_id, active.target_sub_agent_id, revisions.target_sub_agent_key, jobs.sub_agent_kind AS target_sub_agent_kind, revisions.prompt, active.activated_at AS updated_at
           FROM agent_strategy_prompt_active_revisions active
           JOIN agent_strategy_prompt_revisions revisions ON revisions.id = active.revision_id
           JOIN harness_sub_agents jobs ON jobs.id = active.target_sub_agent_id AND jobs.agent_key = active.agent_key
          WHERE active.agent_key = $1 AND active.target_sub_agent_id = $2 FOR UPDATE",
    )
    .bind(agent_key)
    .bind(target_sub_agent_id)
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
        "INSERT INTO agent_strategy_prompt_revisions (batch_id, agent_key, target_sub_agent_id, target_sub_agent_key, parent_revision_id, prompt) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
    )
    .bind(batch_id).bind(agent_key).bind(target_sub_agent_id).bind(&current.target_sub_agent_key).bind(current.revision_id).bind(prompt).fetch_one(&mut *transaction).await?;
    sqlx::query("UPDATE agent_strategy_prompt_active_revisions SET revision_id = $3, activated_at = now() WHERE agent_key = $1 AND target_sub_agent_id = $2")
        .bind(agent_key).bind(target_sub_agent_id).bind(revision_id).execute(&mut *transaction).await?;
    transaction.commit().await?;
    Ok(revision_id)
}

pub async fn seed_initial_prompt_revision(
    pool: &DbPool,
    agent_key: &str,
    target_sub_agent_id: i64,
    target_sub_agent_key: &str,
    prompt: &str,
) -> Result<i64> {
    let batch_id: i64 = sqlx::query_scalar(
        "INSERT INTO agent_strategy_prompt_revision_batches (agent_key, source_type, rationale) VALUES ($1, 'migration', 'Initial strategy prompt') RETURNING id",
    )
    .bind(agent_key)
    .fetch_one(pool)
    .await?;
    let revision_id: i64 = sqlx::query_scalar(
        "INSERT INTO agent_strategy_prompt_revisions (batch_id, agent_key, target_sub_agent_id, target_sub_agent_key, prompt) VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(batch_id)
    .bind(agent_key)
    .bind(target_sub_agent_id)
    .bind(target_sub_agent_key)
    .bind(prompt)
    .fetch_one(pool)
    .await?;
    sqlx::query("INSERT INTO agent_strategy_prompt_active_revisions (agent_key, target_sub_agent_id, revision_id) VALUES ($1, $2, $3)")
        .bind(agent_key)
        .bind(target_sub_agent_id)
        .bind(revision_id)
        .execute(pool)
        .await?;
    Ok(revision_id)
}

#[cfg(test)]
pub async fn insert_default_strategy_prompts_for_agent(
    pool: &DbPool,
    agent_key: &str,
) -> Result<()> {
    let existing: Vec<i64> = sqlx::query_scalar(
        "SELECT jobs.id
           FROM harness_sub_agents AS jobs
          WHERE jobs.agent_key = $1
            AND NOT EXISTS (
                SELECT 1
                  FROM agent_strategy_prompt_active_revisions AS active
                 WHERE active.agent_key = jobs.agent_key
                   AND active.target_sub_agent_id = jobs.id
            )",
    )
    .bind(agent_key)
    .fetch_all(pool)
    .await?;
    for target_sub_agent_id in existing {
        let (sub_agent_key, sub_agent_kind): (String, String) = sqlx::query_as(
            "SELECT sub_agent_key, sub_agent_kind FROM harness_sub_agents WHERE id = $1 AND agent_key = $2",
        )
        .bind(target_sub_agent_id)
        .bind(agent_key)
        .fetch_one(pool)
        .await?;
        let default_prompt = default_prompt_for_role(&sub_agent_kind);
        seed_initial_prompt_revision(
            pool,
            agent_key,
            target_sub_agent_id,
            &sub_agent_key,
            default_prompt,
        )
        .await?;
    }

    Ok(())
}
