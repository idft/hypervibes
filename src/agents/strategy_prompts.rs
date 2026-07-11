use anyhow::{Context, Result};
use sqlx::query_as;

use crate::{
    agents::prompts::{
        DEFAULT_ANALYSIS_STRATEGY_PROMPT, DEFAULT_DAILY_REVIEW_STRATEGY_PROMPT,
        DEFAULT_MARKET_ANALYSIS_STRATEGY_PROMPT, DEFAULT_TRADING_STRATEGY_PROMPT,
    },
    db::DbPool,
};

pub const PROMPT_KIND_ANALYSIS: &str = "analysis";
pub const PROMPT_KIND_MARKET_ANALYSIS: &str = "market_analysis";
pub const PROMPT_KIND_TRADING: &str = "trading";
pub const PROMPT_KIND_DAILY_REVIEW: &str = "daily_review";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AgentStrategyPromptRow {
    pub prompt_kind: String,
    pub prompt: String,
}

pub fn prompt_kind_for_job_kind(job_kind: &str) -> Option<&'static str> {
    match job_kind {
        crate::agentic::model::JOB_KIND_ANALYSIS => Some(PROMPT_KIND_ANALYSIS),
        crate::agentic::model::JOB_KIND_MARKET_ANALYSIS => Some(PROMPT_KIND_MARKET_ANALYSIS),
        crate::agentic::model::JOB_KIND_TRADING => Some(PROMPT_KIND_TRADING),
        crate::agentic::model::JOB_KIND_DAILY_REVIEW => Some(PROMPT_KIND_DAILY_REVIEW),
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
    )
}

pub fn default_prompt_for_kind(prompt_kind: &str) -> &'static str {
    match prompt_kind {
        PROMPT_KIND_ANALYSIS => DEFAULT_ANALYSIS_STRATEGY_PROMPT,
        PROMPT_KIND_MARKET_ANALYSIS => DEFAULT_MARKET_ANALYSIS_STRATEGY_PROMPT,
        PROMPT_KIND_TRADING => DEFAULT_TRADING_STRATEGY_PROMPT,
        PROMPT_KIND_DAILY_REVIEW => DEFAULT_DAILY_REVIEW_STRATEGY_PROMPT,
        _ => "",
    }
}

pub fn all_prompt_kinds() -> [&'static str; 4] {
    [
        PROMPT_KIND_ANALYSIS,
        PROMPT_KIND_MARKET_ANALYSIS,
        PROMPT_KIND_TRADING,
        PROMPT_KIND_DAILY_REVIEW,
    ]
}

pub async fn list_agent_strategy_prompts(
    pool: &DbPool,
    agent_key: &str,
) -> Result<Vec<AgentStrategyPromptRow>> {
    query_as::<_, AgentStrategyPromptRow>(
        "SELECT agent_key, prompt_kind, prompt, created_at, updated_at
           FROM agent_strategy_prompts
          WHERE agent_key = $1
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
        "SELECT agent_key, prompt_kind, prompt, created_at, updated_at
           FROM agent_strategy_prompts
          WHERE agent_key = $1
            AND prompt_kind = $2",
    )
    .bind(agent_key)
    .bind(prompt_kind)
    .fetch_optional(pool)
    .await
    .with_context(|| {
        format!("failed to load strategy prompt {prompt_kind} for agent {agent_key}")
    })
}

pub async fn upsert_agent_strategy_prompt(
    pool: &DbPool,
    agent_key: &str,
    prompt_kind: &str,
    prompt: &str,
) -> Result<bool> {
    let result = sqlx::query(
        "INSERT INTO agent_strategy_prompts (
            agent_key,
            prompt_kind,
            prompt
         ) VALUES ($1, $2, $3)
         ON CONFLICT (agent_key, prompt_kind)
         DO UPDATE SET
            prompt = EXCLUDED.prompt,
            updated_at = now()",
    )
    .bind(agent_key)
    .bind(prompt_kind)
    .bind(prompt)
    .execute(pool)
    .await
    .with_context(|| format!("failed to upsert strategy prompt {prompt_kind} for {agent_key}"))?;

    Ok(result.rows_affected() > 0)
}

pub async fn insert_default_strategy_prompts_for_agent(pool: &DbPool, agent_key: &str) -> Result<()> {
    for prompt_kind in all_prompt_kinds() {
        sqlx::query(
            "INSERT INTO agent_strategy_prompts (
                agent_key,
                prompt_kind,
                prompt
             ) VALUES ($1, $2, $3)
             ON CONFLICT (agent_key, prompt_kind) DO NOTHING",
        )
        .bind(agent_key)
        .bind(prompt_kind)
        .bind(default_prompt_for_kind(prompt_kind))
        .execute(pool)
        .await
        .with_context(|| {
            format!("failed to insert default strategy prompt {prompt_kind} for {agent_key}")
        })?;
    }

    Ok(())
}
