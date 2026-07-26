use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use sqlx::query_as;
use uuid::Uuid;

use crate::{
    agent_conversations::model::{
        AgentConversationListRow, AgentConversationRow, AgentConversationToolPolicyRow,
        CreateAgentConversation, TOOL_GROUP_MEMORY_WRITES, TOOL_GROUP_ORDERS, TOOL_POLICY_ALLOW,
        TOOL_POLICY_CONFIRM, TOOL_POLICY_DENY, UpdateAgentConversationModel,
    },
    db::DbPool,
};

#[derive(Debug, Clone, sqlx::FromRow)]
struct AgentConversationDbRow {
    id: Uuid,
    agent_key: String,
    opencode_session_id: String,
    channel: String,
    external_conversation_key: Option<String>,
    title: String,
    model_provider_id: String,
    model_id: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, sqlx::FromRow)]
struct AgentConversationListDbRow {
    id: Uuid,
    agent_key: String,
    opencode_session_id: String,
    channel: String,
    external_conversation_key: Option<String>,
    title: String,
    model_provider_id: String,
    model_id: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    opencode_status: Option<String>,
    opencode_updated_at: Option<DateTime<Utc>>,
    input_tokens: Option<i32>,
    output_tokens: Option<i32>,
    cache_read_tokens: Option<i32>,
    cache_write_tokens: Option<i32>,
    reasoning_tokens: Option<i32>,
    context_tokens: Option<i32>,
    peak_context_tokens: Option<i32>,
    estimated_cost: Option<Decimal>,
    compaction_count: Option<i32>,
}

/// Create an app-owned mapping after OpenCode has created its session. The
/// default state-changing tool policies are persisted atomically with it.
pub async fn create_conversation_with_default_policies(
    pool: &DbPool,
    input: &CreateAgentConversation,
) -> Result<AgentConversationRow> {
    validate_input(input.validate())?;
    let mut tx = pool
        .begin()
        .await
        .context("failed to begin agent conversation create transaction")?;
    let row = query_as::<_, AgentConversationDbRow>(
        "INSERT INTO agent_conversations (
             id, agent_key, opencode_session_id, channel, external_conversation_key,
             title, model_provider_id, model_id, created_at, updated_at
         )
         SELECT gen_random_uuid(), $1, $2, $3, $4, $5, $6, $7, now(), now()
          WHERE EXISTS (SELECT 1 FROM agents WHERE agent_key = $1)
         RETURNING id, agent_key, opencode_session_id, channel, external_conversation_key,
                   title, model_provider_id, model_id, created_at, updated_at",
    )
    .bind(input.agent_key.trim())
    .bind(input.opencode_session_id.trim())
    .bind(input.channel.trim())
    .bind(input.external_conversation_key.as_deref().map(str::trim))
    .bind(input.title.trim())
    .bind(input.model_provider_id.trim())
    .bind(input.model_id.trim())
    .fetch_optional(&mut *tx)
    .await
    .context("failed to insert agent conversation")?
    .ok_or_else(|| anyhow!("agent does not exist"))?;

    for tool_group in [TOOL_GROUP_ORDERS, TOOL_GROUP_MEMORY_WRITES] {
        sqlx::query(
            "INSERT INTO agent_conversation_tool_policies (conversation_id, tool_group, policy) \
             VALUES ($1, $2, $3)",
        )
        .bind(row.id)
        .bind(tool_group)
        .bind(TOOL_POLICY_CONFIRM)
        .execute(&mut *tx)
        .await
        .context("failed to insert default agent conversation tool policy")?;
    }

    let tool_policies = list_tool_policies_in_tx(&mut tx, row.id, &row.agent_key).await?;
    tx.commit()
        .await
        .context("failed to commit agent conversation create transaction")?;
    Ok(agent_conversation_from_db(row, tool_policies))
}

/// List only app-mapped conversations for an agent. OpenCode mirror fields are
/// optional because the database plugin can lag session creation.
pub async fn list_agent_conversations(
    pool: &DbPool,
    agent_key: &str,
) -> Result<Vec<AgentConversationListRow>> {
    let rows = query_as::<_, AgentConversationListDbRow>(
        "SELECT conversations.id,
                conversations.agent_key,
                conversations.opencode_session_id,
                conversations.channel,
                conversations.external_conversation_key,
                conversations.title,
                conversations.model_provider_id,
                conversations.model_id,
                conversations.created_at,
                conversations.updated_at,
                sessions.status AS opencode_status,
                sessions.updated_at AS opencode_updated_at,
                sessions.input_tokens,
                sessions.output_tokens,
                sessions.cache_read_tokens,
                sessions.cache_write_tokens,
                sessions.reasoning_tokens,
                sessions.context_tokens,
                sessions.peak_context_tokens,
                sessions.estimated_cost,
                sessions.compaction_count
           FROM agent_conversations AS conversations
      LEFT JOIN opencode.sessions AS sessions
             ON sessions.id = conversations.opencode_session_id
          WHERE conversations.agent_key = $1
          ORDER BY COALESCE(sessions.updated_at, conversations.updated_at) DESC,
                   conversations.id DESC",
    )
    .bind(agent_key)
    .fetch_all(pool)
    .await
    .context("failed to list agent conversations")?;

    let mut conversations = Vec::with_capacity(rows.len());
    for row in rows {
        let tool_policies = list_tool_policies(pool, row.id, agent_key).await?;
        conversations.push(AgentConversationListRow {
            id: row.id,
            agent_key: row.agent_key,
            opencode_session_id: row.opencode_session_id,
            channel: row.channel,
            external_conversation_key: row.external_conversation_key,
            title: row.title,
            model_provider_id: row.model_provider_id,
            model_id: row.model_id,
            created_at: row.created_at,
            updated_at: row.updated_at,
            tool_policies,
            opencode_status: row.opencode_status,
            opencode_updated_at: row.opencode_updated_at,
            input_tokens: row.input_tokens,
            output_tokens: row.output_tokens,
            cache_read_tokens: row.cache_read_tokens,
            cache_write_tokens: row.cache_write_tokens,
            reasoning_tokens: row.reasoning_tokens,
            context_tokens: row.context_tokens,
            peak_context_tokens: row.peak_context_tokens,
            estimated_cost: row.estimated_cost,
            compaction_count: row.compaction_count,
        });
    }
    Ok(conversations)
}

pub async fn get_agent_conversation(
    pool: &DbPool,
    agent_key: &str,
    conversation_id: Uuid,
) -> Result<Option<AgentConversationRow>> {
    let row = query_as::<_, AgentConversationDbRow>(
        "SELECT id, agent_key, opencode_session_id, channel, external_conversation_key,
                title, model_provider_id, model_id, created_at, updated_at
           FROM agent_conversations
          WHERE id = $1 AND agent_key = $2",
    )
    .bind(conversation_id)
    .bind(agent_key)
    .fetch_optional(pool)
    .await
    .context("failed to fetch agent conversation")?;
    let Some(row) = row else {
        return Ok(None);
    };
    let tool_policies = list_tool_policies(pool, row.id, agent_key).await?;
    Ok(Some(agent_conversation_from_db(row, tool_policies)))
}

/// Resolve the unique OpenCode session mapping. Session ids are generated by
/// OpenCode and cannot be supplied with an agent key by an untrusted caller.
pub async fn get_conversation_by_opencode_session_id(
    pool: &DbPool,
    session_id: &str,
) -> Result<Option<AgentConversationRow>> {
    let row = query_as::<_, AgentConversationDbRow>(
        "SELECT id, agent_key, opencode_session_id, channel, external_conversation_key,
                title, model_provider_id, model_id, created_at, updated_at
           FROM agent_conversations
          WHERE opencode_session_id = $1",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await
    .context("failed to fetch agent conversation by OpenCode session id")?;
    let Some(row) = row else {
        return Ok(None);
    };
    let tool_policies = list_tool_policies(pool, row.id, &row.agent_key).await?;
    Ok(Some(agent_conversation_from_db(row, tool_policies)))
}

pub async fn update_conversation_title(
    pool: &DbPool,
    agent_key: &str,
    conversation_id: Uuid,
    title: &str,
) -> Result<bool> {
    let title = title.trim();
    if title.is_empty() {
        bail!("conversation title is required");
    }
    let result = sqlx::query(
        "UPDATE agent_conversations
            SET title = $3
          WHERE id = $1 AND agent_key = $2",
    )
    .bind(conversation_id)
    .bind(agent_key)
    .bind(title)
    .execute(pool)
    .await
    .context("failed to update agent conversation title")?;
    Ok(result.rows_affected() > 0)
}

pub async fn update_conversation_model(
    pool: &DbPool,
    agent_key: &str,
    conversation_id: Uuid,
    provider_id: &str,
    model_id: &str,
) -> Result<bool> {
    let update = UpdateAgentConversationModel {
        model_provider_id: provider_id.to_string(),
        model_id: model_id.to_string(),
    };
    validate_input(update.validate())?;
    let result = sqlx::query(
        "UPDATE agent_conversations
            SET model_provider_id = $3,
                model_id = $4
          WHERE id = $1 AND agent_key = $2",
    )
    .bind(conversation_id)
    .bind(agent_key)
    .bind(update.model_provider_id.trim())
    .bind(update.model_id.trim())
    .execute(pool)
    .await
    .context("failed to update agent conversation model")?;
    Ok(result.rows_affected() > 0)
}

pub async fn replace_conversation_tool_policies(
    pool: &DbPool,
    agent_key: &str,
    conversation_id: Uuid,
    policies: &[AgentConversationToolPolicyRow],
) -> Result<bool> {
    validate_tool_policies(policies)?;
    let mut tx = pool
        .begin()
        .await
        .context("failed to begin conversation policy replacement transaction")?;
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
    .context("failed to validate agent conversation ownership")?;
    if owned.is_none() {
        return Ok(false);
    }

    sqlx::query(
        "DELETE FROM agent_conversation_tool_policies AS policies
          USING agent_conversations AS conversations
          WHERE policies.conversation_id = conversations.id
            AND policies.conversation_id = $1
            AND conversations.agent_key = $2",
    )
    .bind(conversation_id)
    .bind(agent_key)
    .execute(&mut *tx)
    .await
    .context("failed to clear agent conversation tool policies")?;
    for policy in policies {
        sqlx::query(
            "INSERT INTO agent_conversation_tool_policies (conversation_id, tool_group, policy)
             VALUES ($1, $2, $3)",
        )
        .bind(conversation_id)
        .bind(policy.tool_group.trim())
        .bind(policy.policy.trim())
        .execute(&mut *tx)
        .await
        .context("failed to insert agent conversation tool policy")?;
    }
    tx.commit()
        .await
        .context("failed to commit conversation policy replacement transaction")?;
    Ok(true)
}

pub async fn delete_conversation_mapping(
    pool: &DbPool,
    agent_key: &str,
    conversation_id: Uuid,
) -> Result<bool> {
    let result = sqlx::query("DELETE FROM agent_conversations WHERE id = $1 AND agent_key = $2")
        .bind(conversation_id)
        .bind(agent_key)
        .execute(pool)
        .await
        .context("failed to delete agent conversation mapping")?;
    Ok(result.rows_affected() > 0)
}

pub async fn list_agent_conversation_opencode_session_ids(
    pool: &DbPool,
    agent_key: &str,
) -> Result<Vec<String>> {
    let rows: Vec<(String,)> = query_as(
        "SELECT opencode_session_id
           FROM agent_conversations
          WHERE agent_key = $1
          ORDER BY created_at DESC, id DESC",
    )
    .bind(agent_key)
    .fetch_all(pool)
    .await
    .context("failed to list agent conversation OpenCode session ids")?;
    Ok(rows.into_iter().map(|(session_id,)| session_id).collect())
}

fn validate_input(validation: Result<(), Vec<String>>) -> Result<()> {
    validation.map_err(|errors| anyhow!(errors.join(" ")))
}

fn validate_tool_policies(policies: &[AgentConversationToolPolicyRow]) -> Result<()> {
    if policies.len() != 2 {
        bail!("conversation tool policies must contain both known tool groups exactly once");
    }
    let mut orders = 0;
    let mut memory_writes = 0;
    for policy in policies {
        match policy.tool_group.trim() {
            TOOL_GROUP_ORDERS => orders += 1,
            TOOL_GROUP_MEMORY_WRITES => memory_writes += 1,
            other => bail!("unknown conversation tool group: {other}"),
        }
        match policy.policy.trim() {
            TOOL_POLICY_DENY | TOOL_POLICY_CONFIRM | TOOL_POLICY_ALLOW => {}
            other => bail!("unknown conversation tool policy: {other}"),
        }
    }
    if orders != 1 || memory_writes != 1 {
        bail!("conversation tool policies must contain both known tool groups exactly once");
    }
    Ok(())
}

fn agent_conversation_from_db(
    row: AgentConversationDbRow,
    tool_policies: Vec<AgentConversationToolPolicyRow>,
) -> AgentConversationRow {
    AgentConversationRow {
        id: row.id,
        agent_key: row.agent_key,
        opencode_session_id: row.opencode_session_id,
        channel: row.channel,
        external_conversation_key: row.external_conversation_key,
        title: row.title,
        model_provider_id: row.model_provider_id,
        model_id: row.model_id,
        created_at: row.created_at,
        updated_at: row.updated_at,
        tool_policies,
    }
}

async fn list_tool_policies(
    pool: &DbPool,
    conversation_id: Uuid,
    agent_key: &str,
) -> Result<Vec<AgentConversationToolPolicyRow>> {
    query_as(
        "SELECT policies.conversation_id,
                policies.tool_group,
                policies.policy,
                policies.created_at,
                policies.updated_at
           FROM agent_conversation_tool_policies AS policies
           JOIN agent_conversations AS conversations
             ON conversations.id = policies.conversation_id
          WHERE policies.conversation_id = $1
            AND conversations.agent_key = $2
          ORDER BY policies.tool_group ASC",
    )
    .bind(conversation_id)
    .bind(agent_key)
    .fetch_all(pool)
    .await
    .context("failed to list agent conversation tool policies")
}

async fn list_tool_policies_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    conversation_id: Uuid,
    agent_key: &str,
) -> Result<Vec<AgentConversationToolPolicyRow>> {
    query_as(
        "SELECT policies.conversation_id,
                policies.tool_group,
                policies.policy,
                policies.created_at,
                policies.updated_at
           FROM agent_conversation_tool_policies AS policies
           JOIN agent_conversations AS conversations
             ON conversations.id = policies.conversation_id
          WHERE policies.conversation_id = $1
            AND conversations.agent_key = $2
          ORDER BY policies.tool_group ASC",
    )
    .bind(conversation_id)
    .bind(agent_key)
    .fetch_all(&mut **tx)
    .await
    .context("failed to list agent conversation tool policies")
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::{
        agent_conversations::model::{CONVERSATION_CHANNEL_WEB, TOOL_POLICY_ALLOW},
        agents::{model::AgentRegistryRow, store::insert_agent},
        test_db,
    };

    async fn seed_agent(pool: &DbPool, key: &str) {
        let now = Utc::now();
        insert_agent(
            pool,
            &AgentRegistryRow {
                agent_key: key.to_string(),
                user_id: test_db::test_user_id(),
                created_at: now,
                updated_at: now,
                enabled: true,
                lifecycle: crate::agents::model::AGENT_LIFECYCLE_ACTIVE.to_string(),
                display_name: key.to_string(),
                trading_account_address: Some(format!("0x{:040x}", uuid::Uuid::new_v4().as_u128())),
                environment: "live".to_string(),
                api_key: format!("vta_{key}"),
                api_key_last_used_at: None,
                runtime_config: serde_json::json!({}),
            },
        )
        .await
        .expect("insert agent");
    }

    fn create_input(agent_key: &str, session_id: &str) -> CreateAgentConversation {
        CreateAgentConversation {
            agent_key: agent_key.to_string(),
            opencode_session_id: session_id.to_string(),
            channel: CONVERSATION_CHANNEL_WEB.to_string(),
            external_conversation_key: None,
            title: "New conversation".to_string(),
            model_provider_id: "anthropic".to_string(),
            model_id: "claude-sonnet-4".to_string(),
        }
    }

    #[tokio::test]
    async fn create_lists_default_policies_without_an_opencode_mirror() {
        let pool = test_db::pool().await;
        let key = format!(
            "conversation-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        seed_agent(&pool, &key).await;

        let created = create_conversation_with_default_policies(
            &pool,
            &create_input(&key, "ses_conversation_default"),
        )
        .await
        .expect("create conversation");

        assert_eq!(created.tool_policies.len(), 2);
        assert!(
            created
                .tool_policies
                .iter()
                .all(|policy| policy.policy == TOOL_POLICY_CONFIRM)
        );
        let listed = list_agent_conversations(&pool, &key)
            .await
            .expect("list conversations");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, created.id);
        assert!(listed[0].opencode_status.is_none());
        assert_eq!(
            get_conversation_by_opencode_session_id(&pool, "ses_conversation_default")
                .await
                .expect("get conversation")
                .expect("conversation exists")
                .id,
            created.id
        );
    }

    #[tokio::test]
    async fn ownership_and_policy_replacement_are_scoped_and_validated() {
        let pool = test_db::pool().await;
        let owner = format!(
            "conversation-owner-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        let other = format!(
            "conversation-other-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        seed_agent(&pool, &owner).await;
        seed_agent(&pool, &other).await;
        let created = create_conversation_with_default_policies(
            &pool,
            &create_input(&owner, "ses_conversation_scoped"),
        )
        .await
        .expect("create conversation");

        assert!(
            get_agent_conversation(&pool, &other, created.id)
                .await
                .expect("get scoped conversation")
                .is_none()
        );
        assert!(
            !update_conversation_title(&pool, &other, created.id, "Other title")
                .await
                .expect("update foreign title")
        );

        let invalid = vec![
            created.tool_policies[0].clone(),
            created.tool_policies[0].clone(),
        ];
        assert!(
            replace_conversation_tool_policies(&pool, &owner, created.id, &invalid)
                .await
                .is_err()
        );

        let mut replacement = created.tool_policies.clone();
        replacement[0].policy = TOOL_POLICY_ALLOW.to_string();
        assert!(
            replace_conversation_tool_policies(&pool, &owner, created.id, &replacement)
                .await
                .expect("replace policies")
        );
        let stored = get_agent_conversation(&pool, &owner, created.id)
            .await
            .expect("get owner conversation")
            .expect("conversation exists");
        assert!(
            stored
                .tool_policies
                .iter()
                .any(|policy| policy.policy == TOOL_POLICY_ALLOW)
        );
        assert!(
            !delete_conversation_mapping(&pool, &other, created.id)
                .await
                .expect("delete foreign conversation")
        );
    }
}
