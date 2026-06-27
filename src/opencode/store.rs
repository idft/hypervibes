use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::Value;
use sqlx::query_as;
use uuid::Uuid;

use crate::db::DbPool;

#[derive(Debug, Clone)]
pub struct OpenCodeSessionDetail {
    pub session: OpenCodeSessionRow,
    pub commands: Vec<OpenCodeCommandRow>,
    pub messages: Vec<OpenCodeMessageRow>,
    pub tool_executions: Vec<OpenCodeToolExecutionRow>,
    pub session_errors: Vec<OpenCodeSessionErrorRow>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct OpenCodeSessionRow {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub directory: Option<String>,
    pub title: Option<String>,
    pub status: Option<String>,
    pub model_provider: String,
    pub model_id: String,
    pub share_url: Option<String>,
    pub input_tokens: i32,
    pub output_tokens: i32,
    pub cache_read_tokens: i32,
    pub cache_write_tokens: i32,
    pub reasoning_tokens: i32,
    pub context_tokens: i32,
    pub peak_context_tokens: i32,
    pub estimated_cost: Decimal,
    pub compaction_count: i32,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct OpenCodeCommandRow {
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
    pub command_name: String,
    pub command_args: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct OpenCodeMessageRow {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub role: String,
    pub model_provider: Option<String>,
    pub model_id: Option<String>,
    pub text: Option<String>,
    pub summary: Option<String>,
    pub system_prompt: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct OpenCodeToolExecutionRow {
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub tool_name: String,
    pub args: Option<Value>,
    pub result: Option<Value>,
    pub duration_ms: Option<i32>,
    pub success: Option<bool>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct OpenCodeSessionErrorRow {
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
    pub error_type: Option<String>,
    pub error_message: Option<String>,
    pub error_data: Option<Value>,
}

pub async fn get_session_detail(
    pool: &DbPool,
    session_id: &str,
) -> Result<Option<OpenCodeSessionDetail>> {
    let session = query_as::<_, OpenCodeSessionRow>(
        "SELECT id,
                created_at,
                updated_at,
                directory,
                title,
                status,
                COALESCE(model_provider, '') AS model_provider,
                COALESCE(model_id, '') AS model_id,
                share_url,
                COALESCE(input_tokens, 0) AS input_tokens,
                COALESCE(output_tokens, 0) AS output_tokens,
                COALESCE(cache_read_tokens, 0) AS cache_read_tokens,
                COALESCE(cache_write_tokens, 0) AS cache_write_tokens,
                COALESCE(reasoning_tokens, 0) AS reasoning_tokens,
                COALESCE(context_tokens, 0) AS context_tokens,
                COALESCE(peak_context_tokens, 0) AS peak_context_tokens,
                COALESCE(estimated_cost, 0)::numeric(10, 6) AS estimated_cost,
                COALESCE(compaction_count, 0) AS compaction_count
           FROM opencode.sessions
          WHERE id = $1",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await
    .with_context(|| format!("failed to fetch OpenCode session {session_id}"))?;

    let Some(session) = session else {
        return Ok(None);
    };

    let commands = query_as::<_, OpenCodeCommandRow>(
        "SELECT id,
                created_at,
                command_name,
                command_args
           FROM opencode.commands
          WHERE session_id = $1
          ORDER BY created_at ASC, id ASC",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await
    .with_context(|| format!("failed to fetch OpenCode commands for session {session_id}"))?;

    let messages = query_as::<_, OpenCodeMessageRow>(
        "SELECT id,
                created_at,
                role,
                model_provider,
                model_id,
                text,
                summary,
                system_prompt
           FROM opencode.messages
          WHERE session_id = $1
          ORDER BY created_at ASC, id ASC",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await
    .with_context(|| format!("failed to fetch OpenCode messages for session {session_id}"))?;

    let tool_executions = query_as::<_, OpenCodeToolExecutionRow>(
        "SELECT id,
                created_at,
                started_at,
                completed_at,
                tool_name,
                args,
                result,
                duration_ms,
                success,
                error
           FROM opencode.tool_executions
          WHERE session_id = $1
          ORDER BY COALESCE(started_at, created_at) ASC, id ASC",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await
    .with_context(|| {
        format!("failed to fetch OpenCode tool executions for session {session_id}")
    })?;

    let session_errors = query_as::<_, OpenCodeSessionErrorRow>(
        "SELECT id,
                created_at,
                error_type,
                error_message,
                error_data
           FROM opencode.session_errors
          WHERE session_id = $1
          ORDER BY created_at ASC, id ASC",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await
    .with_context(|| format!("failed to fetch OpenCode session errors for session {session_id}"))?;

    Ok(Some(OpenCodeSessionDetail {
        session,
        commands,
        messages,
        tool_executions,
        session_errors,
    }))
}
