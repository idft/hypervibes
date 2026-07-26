use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::Value;
use sqlx::query_as;

use crate::db::DbPool;

#[derive(Debug, Clone)]
pub struct OpenCodeSessionDetail {
    pub session: OpenCodeSessionRow,
    pub messages: Vec<OpenCodeMessageRow>,
    pub tool_executions: Vec<OpenCodeToolExecutionRow>,
    pub session_errors: Vec<OpenCodeSessionErrorRow>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct OpenCodeSessionRow {
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
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub tool_name: String,
    pub args: Option<Value>,
    pub result: Option<Value>,
    pub duration_ms: Option<i32>,
    pub success: Option<bool>,
    pub error: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct OpenCodeToolMessagePartRow {
    id: String,
    created_at: DateTime<Utc>,
    tool_name: Option<String>,
    content: Option<Value>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct OpenCodeSessionErrorRow {
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

    let tool_message_parts = query_as::<_, OpenCodeToolMessagePartRow>(
        "SELECT parts.id,
                parts.created_at,
                parts.tool_name,
                parts.content
           FROM opencode.message_parts AS parts
           JOIN opencode.messages AS messages ON messages.id = parts.message_id
          WHERE messages.session_id = $1
            AND parts.part_type = 'tool'
          ORDER BY parts.created_at ASC, parts.id ASC",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await
    .with_context(|| format!("failed to fetch OpenCode tool parts for session {session_id}"))?;

    let mut tool_executions = if tool_message_parts.is_empty() {
        query_as::<_, OpenCodeToolExecutionRow>(
            "SELECT id::text AS id,
                    created_at,
                    started_at,
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
        })?
    } else {
        tool_message_parts
            .into_iter()
            .map(tool_execution_from_message_part)
            .collect()
    };
    tool_executions.sort_by(|a, b| {
        a.started_at
            .unwrap_or(a.created_at)
            .cmp(&b.started_at.unwrap_or(b.created_at))
            .then_with(|| a.id.cmp(&b.id))
    });

    let session_errors = query_as::<_, OpenCodeSessionErrorRow>(
        "SELECT created_at,
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
        messages,
        tool_executions,
        session_errors,
    }))
}

fn tool_execution_from_message_part(part: OpenCodeToolMessagePartRow) -> OpenCodeToolExecutionRow {
    let content = part.content.as_ref();
    let state = content.and_then(|content| content.get("state"));
    let started_at = state
        .and_then(|state| state.pointer("/time/start"))
        .and_then(json_millis);
    let ended_at = state
        .and_then(|state| state.pointer("/time/end"))
        .and_then(json_millis);
    let duration_ms = started_at
        .zip(ended_at)
        .and_then(|(start, end)| (end - start).num_milliseconds().try_into().ok());
    let status = state
        .and_then(|state| state.get("status"))
        .and_then(Value::as_str);

    OpenCodeToolExecutionRow {
        id: part.id,
        created_at: part.created_at,
        started_at,
        tool_name: content
            .and_then(|content| content.get("tool"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or(part.tool_name)
            .unwrap_or_default(),
        args: state.and_then(|state| state.get("input")).cloned(),
        result: state
            .and_then(|state| state.get("output"))
            .cloned()
            .map(parse_json_string),
        duration_ms,
        success: match status {
            Some("completed") => Some(true),
            Some("error") => Some(false),
            _ => None,
        },
        error: state
            .and_then(|state| state.get("error"))
            .and_then(Value::as_str)
            .map(str::to_owned),
    }
}

fn json_millis(value: &Value) -> Option<DateTime<Utc>> {
    let millis = value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|millis| millis.try_into().ok()))?;
    DateTime::from_timestamp_millis(millis)
}

fn parse_json_string(value: Value) -> Value {
    let Value::String(text) = &value else {
        return value;
    };
    serde_json::from_str(text).unwrap_or(value)
}

/// Count OpenCode sessions in an active (`busy`/`retry`) status across
/// all directories. Used by the provider-config-reload maintenance job
/// to wait until it is safe to dispose OpenCode instances without
/// interrupting in-flight agent work.
pub async fn count_active_opencode_sessions(pool: &DbPool) -> Result<i64> {
    let row: (i64,) = query_as("SELECT count(*) FROM opencode.sessions WHERE status = ANY($1)")
        .bind(["busy", "retry"])
        .fetch_one(pool)
        .await
        .context("failed to count active OpenCode sessions")?;
    Ok(row.0)
}

/// Count every active session in one workspace. Unlike the sidebar-oriented
/// session listing, this query has no limit so maintenance cannot miss an
/// older active conversation.
pub async fn count_active_opencode_sessions_for_directory(
    pool: &DbPool,
    directory: &str,
) -> Result<i64> {
    let row: (i64,) = query_as(
        "SELECT count(*)
           FROM opencode.sessions
          WHERE directory = $1
            AND status = ANY($2)",
    )
    .bind(directory)
    .bind(["busy", "retry"])
    .fetch_one(pool)
    .await
    .with_context(|| {
        format!("failed to count active OpenCode sessions for directory {directory}")
    })?;
    Ok(row.0)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use sqlx::query;

    use super::*;

    #[tokio::test]
    async fn session_detail_uses_canonical_tool_parts_without_legacy_duplicates() {
        let pool = crate::test_db::pool().await;
        query("INSERT INTO opencode.sessions (id) VALUES ('session-modern')")
            .execute(&pool)
            .await
            .expect("insert session");
        query(
            "INSERT INTO opencode.messages (id, session_id, role)
             VALUES ('message-1', 'session-modern', 'assistant')",
        )
        .execute(&pool)
        .await
        .expect("insert message");
        query(
            "INSERT INTO opencode.message_parts
                (id, message_id, part_type, tool_name, content, created_at)
             VALUES
                ('part-error', 'message-1', 'tool', 'wrong-name', $1, '2023-11-14 22:13:22Z'),
                ('part-completed', 'message-1', 'tool', 'wrong-name', $2, '2023-11-14 22:13:21Z')",
        )
        .bind(json!({
            "tool": "bash",
            "state": {
                "status": "error",
                "input": {"command": "false"},
                "error": "command failed",
                "time": {"start": 1_700_000_002_000_i64, "end": 1_700_000_002_010_i64}
            }
        }))
        .bind(json!({
            "tool": "read",
            "state": {
                "status": "completed",
                "input": {"filePath": "/tmp/example"},
                "output": "{\"matches\":2}",
                "time": {"start": 1_700_000_001_000_i64, "end": 1_700_000_001_125_i64}
            }
        }))
        .execute(&pool)
        .await
        .expect("insert tool parts");
        query(
            "INSERT INTO opencode.tool_executions
                (session_id, correlation_id, tool_name, success)
             VALUES ('session-modern', 'duplicate', 'legacy-duplicate', true)",
        )
        .execute(&pool)
        .await
        .expect("insert legacy duplicate");

        let detail = get_session_detail(&pool, "session-modern")
            .await
            .expect("fetch session detail")
            .expect("session exists");

        assert_eq!(detail.tool_executions.len(), 2);
        let completed = &detail.tool_executions[0];
        assert_eq!(completed.id, "part-completed");
        assert_eq!(completed.tool_name, "read");
        assert_eq!(completed.args, Some(json!({"filePath": "/tmp/example"})));
        assert_eq!(completed.result, Some(json!({"matches": 2})));
        assert_eq!(completed.duration_ms, Some(125));
        assert_eq!(completed.success, Some(true));
        assert_eq!(
            completed.started_at,
            DateTime::from_timestamp_millis(1_700_000_001_000)
        );

        let failed = &detail.tool_executions[1];
        assert_eq!(failed.id, "part-error");
        assert_eq!(failed.success, Some(false));
        assert_eq!(failed.error.as_deref(), Some("command failed"));
        assert_eq!(failed.duration_ms, Some(10));
    }

    #[tokio::test]
    async fn session_detail_falls_back_to_legacy_tool_executions() {
        let pool = crate::test_db::pool().await;
        query("INSERT INTO opencode.sessions (id) VALUES ('session-legacy')")
            .execute(&pool)
            .await
            .expect("insert session");
        query(
            "INSERT INTO opencode.tool_executions
                (session_id, correlation_id, tool_name, args, result, duration_ms, success)
             VALUES ('session-legacy', 'legacy', 'bash', $1, $2, 25, true)",
        )
        .bind(json!({"command": "pwd"}))
        .bind(json!("/tmp"))
        .execute(&pool)
        .await
        .expect("insert legacy tool execution");

        let detail = get_session_detail(&pool, "session-legacy")
            .await
            .expect("fetch session detail")
            .expect("session exists");

        assert_eq!(detail.tool_executions.len(), 1);
        let tool = &detail.tool_executions[0];
        assert!(!tool.id.is_empty());
        assert_eq!(tool.tool_name, "bash");
        assert_eq!(tool.args, Some(json!({"command": "pwd"})));
        assert_eq!(tool.result, Some(json!("/tmp")));
        assert_eq!(tool.duration_ms, Some(25));
        assert_eq!(tool.success, Some(true));
    }
}
