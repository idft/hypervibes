use anyhow::{Context, Result, bail};
use sqlx::query_as;
use uuid::Uuid;

use crate::{
    agent_conversations::model::AgentConversationWorkspaceRow, db::DbPool,
    harness::model::CAPABILITY_SCHEMA_VERSION,
};

/// Reserve durable workspace state for an internal conversation before a
/// future conversation workspace is materialized. The caller supplies only the
/// internal UUID, never a gateway or external chat key.
pub async fn prepare_conversation_workspace(
    pool: &DbPool,
    agent_key: &str,
    conversation_id: Uuid,
    capability_schema_version: i32,
) -> Result<AgentConversationWorkspaceRow> {
    validate_capability_schema_version(capability_schema_version)?;
    let mut tx = pool
        .begin()
        .await
        .context("failed to begin conversation workspace preparation")?;
    let owned: Option<(Uuid,)> = query_as(
        "SELECT id
           FROM agent_conversations
          WHERE id = $1 AND agent_key = $2
          FOR UPDATE",
    )
    .bind(conversation_id)
    .bind(agent_key)
    .fetch_optional(&mut *tx)
    .await
    .context("failed to validate conversation workspace ownership")?;
    if owned.is_none() {
        bail!("conversation workspace does not belong to agent");
    }

    sqlx::query(
        "INSERT INTO agent_conversation_workspaces (
             conversation_id, capability_schema_version, workspace_status
         ) VALUES ($1, $2, 'preparing')
         ON CONFLICT (conversation_id) DO NOTHING",
    )
    .bind(conversation_id)
    .bind(capability_schema_version)
    .execute(&mut *tx)
    .await
    .context("failed to insert conversation workspace state")?;

    let workspace = get_conversation_workspace_in_tx(&mut tx, agent_key, conversation_id)
        .await?
        .expect("inserted conversation workspace must be readable");
    if workspace.capability_schema_version != capability_schema_version {
        bail!("conversation workspace capability binding conflicts with existing state");
    }
    if workspace.workspace_status == "deleted" {
        bail!("conversation workspace has already been deleted");
    }
    tx.commit()
        .await
        .context("failed to commit conversation workspace preparation")?;
    Ok(workspace)
}

pub async fn get_conversation_workspace(
    pool: &DbPool,
    agent_key: &str,
    conversation_id: Uuid,
) -> Result<Option<AgentConversationWorkspaceRow>> {
    let workspace = query_as::<_, AgentConversationWorkspaceRow>(
        "SELECT workspaces.conversation_id,
                workspaces.capability_schema_version,
                workspaces.workspace_status,
                workspaces.workspace_created_at,
                workspaces.runtime_secrets_scrubbed_at,
                workspaces.deletion_started_at,
                workspaces.deleted_at,
                workspaces.observed_size_bytes,
                workspaces.observed_file_count,
                workspaces.error_summary,
                workspaces.created_at,
                workspaces.updated_at
           FROM agent_conversation_workspaces AS workspaces
           JOIN agent_conversations AS conversations
             ON conversations.id = workspaces.conversation_id
          WHERE workspaces.conversation_id = $1
            AND conversations.agent_key = $2",
    )
    .bind(conversation_id)
    .bind(agent_key)
    .fetch_optional(pool)
    .await
    .context("failed to load conversation workspace state")?;
    if let Some(workspace) = &workspace {
        validate_capability_schema_version(workspace.capability_schema_version)?;
    }
    Ok(workspace)
}

pub async fn mark_conversation_workspace_ready(
    pool: &DbPool,
    agent_key: &str,
    conversation_id: Uuid,
) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE agent_conversation_workspaces AS workspaces
            SET workspace_status = 'ready',
                workspace_created_at = COALESCE(workspace_created_at, now())
           FROM agent_conversations AS conversations
          WHERE workspaces.conversation_id = $1
            AND conversations.id = workspaces.conversation_id
            AND conversations.agent_key = $2
            AND workspaces.workspace_status IN ('preparing', 'ready')",
    )
    .bind(conversation_id)
    .bind(agent_key)
    .execute(pool)
    .await
    .context("failed to mark conversation workspace ready")?;
    Ok(result.rows_affected() > 0)
}

pub async fn record_conversation_workspace_secret_scrub(
    pool: &DbPool,
    agent_key: &str,
    conversation_id: Uuid,
) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE agent_conversation_workspaces AS workspaces
            SET runtime_secrets_scrubbed_at = COALESCE(runtime_secrets_scrubbed_at, now())
           FROM agent_conversations AS conversations
          WHERE workspaces.conversation_id = $1
            AND conversations.id = workspaces.conversation_id
            AND conversations.agent_key = $2
            AND workspaces.workspace_status <> 'deleted'",
    )
    .bind(conversation_id)
    .bind(agent_key)
    .execute(pool)
    .await
    .context("failed to record conversation workspace runtime-secret scrub")?;
    Ok(result.rows_affected() > 0)
}

pub async fn record_conversation_workspace_stats(
    pool: &DbPool,
    agent_key: &str,
    conversation_id: Uuid,
    size_bytes: u64,
    file_count: u64,
) -> Result<bool> {
    let size_bytes = i64::try_from(size_bytes).context("workspace size exceeds database range")?;
    let file_count =
        i64::try_from(file_count).context("workspace file count exceeds database range")?;
    let result = sqlx::query(
        "UPDATE agent_conversation_workspaces AS workspaces
            SET observed_size_bytes = $3,
                observed_file_count = $4
           FROM agent_conversations AS conversations
          WHERE workspaces.conversation_id = $1
            AND conversations.id = workspaces.conversation_id
            AND conversations.agent_key = $2
            AND workspaces.workspace_status <> 'deleted'",
    )
    .bind(conversation_id)
    .bind(agent_key)
    .bind(size_bytes)
    .bind(file_count)
    .execute(pool)
    .await
    .context("failed to record conversation workspace statistics")?;
    Ok(result.rows_affected() > 0)
}

async fn get_conversation_workspace_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    agent_key: &str,
    conversation_id: Uuid,
) -> Result<Option<AgentConversationWorkspaceRow>> {
    let workspace = query_as::<_, AgentConversationWorkspaceRow>(
        "SELECT workspaces.conversation_id,
                workspaces.capability_schema_version,
                workspaces.workspace_status,
                workspaces.workspace_created_at,
                workspaces.runtime_secrets_scrubbed_at,
                workspaces.deletion_started_at,
                workspaces.deleted_at,
                workspaces.observed_size_bytes,
                workspaces.observed_file_count,
                workspaces.error_summary,
                workspaces.created_at,
                workspaces.updated_at
           FROM agent_conversation_workspaces AS workspaces
           JOIN agent_conversations AS conversations
             ON conversations.id = workspaces.conversation_id
          WHERE workspaces.conversation_id = $1
            AND conversations.agent_key = $2",
    )
    .bind(conversation_id)
    .bind(agent_key)
    .fetch_optional(&mut **tx)
    .await
    .context("failed to load conversation workspace state in transaction")?;
    if let Some(workspace) = &workspace {
        validate_capability_schema_version(workspace.capability_schema_version)?;
    }
    Ok(workspace)
}

fn validate_capability_schema_version(capability_schema_version: i32) -> Result<()> {
    if capability_schema_version != CAPABILITY_SCHEMA_VERSION {
        bail!("unsupported conversation capability schema version");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::{
        agent_conversations::{
            model::{CONVERSATION_CHANNEL_WEB, CreateAgentConversation},
            store::create_conversation_with_default_policies,
        },
        agents::{model::AgentRegistryRow, store::insert_agent},
        test_db,
    };

    async fn seed_conversation(pool: &DbPool, agent_key: &str) -> uuid::Uuid {
        let now = Utc::now();
        insert_agent(
            pool,
            &AgentRegistryRow {
                agent_key: agent_key.to_string(),
                user_id: test_db::test_user_id(),
                created_at: now,
                updated_at: now,
                enabled: true,
                lifecycle: crate::agents::model::AGENT_LIFECYCLE_ACTIVE.to_string(),
                display_name: agent_key.to_string(),
                trading_account_address: Some(format!("0x{:040x}", uuid::Uuid::new_v4().as_u128())),
                environment: "live".to_string(),
                api_key: format!("vta_{agent_key}"),
                api_key_last_used_at: None,
                runtime_config: serde_json::json!({}),
            },
        )
        .await
        .expect("insert agent");
        create_conversation_with_default_policies(
            pool,
            &CreateAgentConversation {
                agent_key: agent_key.to_string(),
                opencode_session_id: format!("ses_{agent_key}"),
                channel: CONVERSATION_CHANNEL_WEB.to_string(),
                external_conversation_key: None,
                title: "Conversation".to_string(),
                model_provider_id: "anthropic".to_string(),
                model_id: "claude-sonnet-4".to_string(),
                model_variant: None,
            },
        )
        .await
        .expect("create conversation")
        .id
    }

    #[tokio::test]
    async fn preparation_is_idempotent_and_uses_internal_conversation_ownership() {
        let pool = test_db::pool().await;
        let agent_key = format!(
            "conversation-workspace-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        let id = seed_conversation(&pool, &agent_key).await;
        assert!(
            sqlx::query(
                "INSERT INTO agent_conversation_workspaces (conversation_id, capability_schema_version) \
                 VALUES ($1, $2)",
            )
            .bind(id)
            .bind(CAPABILITY_SCHEMA_VERSION + 1)
            .execute(&pool)
            .await
            .is_err(),
            "unsupported capability schema versions must be rejected by the database"
        );

        let first =
            prepare_conversation_workspace(&pool, &agent_key, id, CAPABILITY_SCHEMA_VERSION)
                .await
                .expect("prepare conversation workspace");
        let second =
            prepare_conversation_workspace(&pool, &agent_key, id, CAPABILITY_SCHEMA_VERSION)
                .await
                .expect("repeat preparation");

        assert_eq!(first.conversation_id, id);
        assert_eq!(first.workspace_status, "preparing");
        assert_eq!(first.conversation_id, second.conversation_id);
        assert!(
            prepare_conversation_workspace(&pool, "other-agent", id, CAPABILITY_SCHEMA_VERSION)
                .await
                .is_err()
        );
        assert!(
            prepare_conversation_workspace(&pool, &agent_key, id, CAPABILITY_SCHEMA_VERSION + 1,)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn conversation_workspace_lifecycle_updates_are_scoped() {
        let pool = test_db::pool().await;
        let agent_key = format!(
            "conversation-workspace-life-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        let id = seed_conversation(&pool, &agent_key).await;
        prepare_conversation_workspace(&pool, &agent_key, id, CAPABILITY_SCHEMA_VERSION)
            .await
            .expect("prepare conversation workspace");

        assert!(
            mark_conversation_workspace_ready(&pool, &agent_key, id)
                .await
                .expect("mark ready")
        );
        assert!(
            record_conversation_workspace_secret_scrub(&pool, &agent_key, id)
                .await
                .expect("record scrub")
        );
        assert!(
            record_conversation_workspace_stats(&pool, &agent_key, id, 99, 3)
                .await
                .expect("record stats")
        );

        let workspace = get_conversation_workspace(&pool, &agent_key, id)
            .await
            .expect("get workspace")
            .expect("workspace exists");
        assert_eq!(workspace.workspace_status, "ready");
        assert!(workspace.workspace_created_at.is_some());
        assert!(workspace.runtime_secrets_scrubbed_at.is_some());
        assert_eq!(workspace.observed_size_bytes, Some(99));
        assert_eq!(workspace.observed_file_count, Some(3));
    }
}
