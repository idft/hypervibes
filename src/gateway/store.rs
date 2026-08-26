use anyhow::{Context, Result};
use sqlx::query_as;

use crate::{
    db::DbPool,
    gateway::model::{AgentGatewayRow, GATEWAY_TYPE_TELEGRAM, TelegramGatewayConfig},
};

/// Upsert a gateway row, creating or updating the enabled flag and the
/// `config` JSONB column atomically. Per-gateway-type helpers below wrap this
/// with the appropriate serialization.
pub async fn upsert_gateway(
    pool: &DbPool,
    agent_key: &str,
    gateway_type: &str,
    enabled: bool,
    config: &serde_json::Value,
) -> Result<AgentGatewayRow> {
    let row = query_as::<_, AgentGatewayRow>(
        "INSERT INTO agent_gateways (agent_key, gateway_type, enabled, config)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (agent_key, gateway_type)
           DO UPDATE SET enabled = EXCLUDED.enabled,
                         config = EXCLUDED.config,
                         updated_at = now()
          RETURNING agent_key, enabled, config",
    )
    .bind(agent_key)
    .bind(gateway_type)
    .bind(enabled)
    .bind(config)
    .fetch_one(pool)
    .await
    .context("failed to upsert agent gateway")?;
    Ok(row)
}

/// Set only the `enabled` flag on an existing gateway row. Returns `false`
/// when the gateway does not exist.
pub async fn set_gateway_enabled(
    pool: &DbPool,
    agent_key: &str,
    gateway_type: &str,
    enabled: bool,
) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE agent_gateways
            SET enabled = $3
          WHERE agent_key = $1 AND gateway_type = $2",
    )
    .bind(agent_key)
    .bind(gateway_type)
    .bind(enabled)
    .execute(pool)
    .await
    .context("failed to update agent gateway enabled flag")?;
    Ok(result.rows_affected() > 0)
}

/// Persist a new Telegram config blob, creating or updating the gateway row.
/// The row is left disabled when first created so the operator must
/// explicitly enable it after pasting a token.
pub async fn upsert_telegram_config(
    pool: &DbPool,
    agent_key: &str,
    config: &TelegramGatewayConfig,
) -> Result<AgentGatewayRow> {
    let value = serde_json::to_value(config).context("failed to serialize telegram config")?;
    let existing = get_gateway(pool, agent_key, GATEWAY_TYPE_TELEGRAM).await?;
    let enabled = existing.as_ref().is_some_and(|row| row.enabled);
    upsert_gateway(pool, agent_key, GATEWAY_TYPE_TELEGRAM, enabled, &value).await
}

/// Load a single gateway row.
pub async fn get_gateway(
    pool: &DbPool,
    agent_key: &str,
    gateway_type: &str,
) -> Result<Option<AgentGatewayRow>> {
    let row = query_as::<_, AgentGatewayRow>(
        "SELECT agent_key, enabled, config
           FROM agent_gateways
          WHERE agent_key = $1 AND gateway_type = $2",
    )
    .bind(agent_key)
    .bind(gateway_type)
    .fetch_optional(pool)
    .await
    .context("failed to fetch agent gateway")?;
    Ok(row)
}

/// List every enabled Telegram gateway. Used by the gateway service to spawn
/// per-agent polling tasks.
pub async fn list_enabled_telegram_gateways(pool: &DbPool) -> Result<Vec<AgentGatewayRow>> {
    let rows = query_as::<_, AgentGatewayRow>(
        "SELECT agent_key, enabled, config
           FROM agent_gateways
          WHERE gateway_type = $1 AND enabled = TRUE",
    )
    .bind(GATEWAY_TYPE_TELEGRAM)
    .fetch_all(pool)
    .await
    .context("failed to list enabled telegram gateways")?;
    Ok(rows)
}

/// Clear the bound chat identity from a Telegram gateway config, leaving the
/// encrypted bot token in place. Used by the unlink handler.
pub async fn clear_telegram_chat(pool: &DbPool, agent_key: &str) -> Result<bool> {
    let Some(row) = get_gateway(pool, agent_key, GATEWAY_TYPE_TELEGRAM).await? else {
        return Ok(false);
    };
    let mut config = TelegramGatewayConfig::from_value(&row.config);
    config.chat_id = None;
    config.chat_username = None;
    let value = serde_json::to_value(&config).context("failed to serialize telegram config")?;
    let result = sqlx::query(
        "UPDATE agent_gateways
            SET config = $3
          WHERE agent_key = $1 AND gateway_type = $2",
    )
    .bind(agent_key)
    .bind(GATEWAY_TYPE_TELEGRAM)
    .bind(value)
    .execute(pool)
    .await
    .context("failed to clear telegram chat binding")?;
    Ok(result.rows_affected() > 0)
}

/// Persist the bound chat identity onto a Telegram gateway config. Used
/// after the operator confirms the link flow.
pub async fn set_telegram_chat(
    pool: &DbPool,
    agent_key: &str,
    chat_id: i64,
    chat_username: Option<&str>,
) -> Result<bool> {
    let Some(row) = get_gateway(pool, agent_key, GATEWAY_TYPE_TELEGRAM).await? else {
        return Ok(false);
    };
    let mut config = TelegramGatewayConfig::from_value(&row.config);
    config.chat_id = Some(chat_id);
    config.chat_username = chat_username.map(ToOwned::to_owned);
    let value = serde_json::to_value(&config).context("failed to serialize telegram config")?;
    let result = sqlx::query(
        "UPDATE agent_gateways
            SET config = $3
          WHERE agent_key = $1 AND gateway_type = $2",
    )
    .bind(agent_key)
    .bind(GATEWAY_TYPE_TELEGRAM)
    .bind(value)
    .execute(pool)
    .await
    .context("failed to persist telegram chat binding")?;
    Ok(result.rows_affected() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_db;

    async fn seed_agent(pool: &DbPool, key: &str) {
        let now = chrono::Utc::now();
        crate::agents::store::insert_agent(
            pool,
            &crate::agents::model::AgentRegistryRow {
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

    #[tokio::test]
    async fn upsert_telegram_config_creates_disabled_row_then_preserves_enabled_flag() {
        let pool = test_db::pool().await;
        let key = format!(
            "gateway-upsert-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        seed_agent(&pool, &key).await;

        let config = TelegramGatewayConfig {
            bot_token_ciphertext: Some(vec![1, 2, 3]),
            bot_token_key_id: Some("test".to_string()),
            bot_username: Some("testbot".to_string()),
            ..Default::default()
        };
        let created = upsert_telegram_config(&pool, &key, &config)
            .await
            .expect("create telegram config");
        assert!(!created.enabled);

        set_gateway_enabled(&pool, &key, GATEWAY_TYPE_TELEGRAM, true)
            .await
            .expect("enable gateway");
        let updated = upsert_telegram_config(&pool, &key, &config)
            .await
            .expect("update telegram config");
        assert!(
            updated.enabled,
            "enabled flag must be preserved on config update"
        );
    }

    #[tokio::test]
    async fn set_telegram_chat_updates_chat_id_and_username() {
        let pool = test_db::pool().await;
        let key = format!(
            "gateway-chat-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        seed_agent(&pool, &key).await;

        let config = TelegramGatewayConfig {
            bot_token_ciphertext: Some(vec![1, 2, 3]),
            ..Default::default()
        };
        upsert_telegram_config(&pool, &key, &config)
            .await
            .expect("create config");

        assert!(
            set_telegram_chat(&pool, &key, -100_123, Some("operator"))
                .await
                .expect("set chat")
        );
        let stored = get_gateway(&pool, &key, GATEWAY_TYPE_TELEGRAM)
            .await
            .expect("get gateway")
            .expect("gateway exists");
        let telegram = TelegramGatewayConfig::from_value(&stored.config);
        assert_eq!(telegram.chat_id, Some(-100_123));
        assert_eq!(telegram.chat_username.as_deref(), Some("operator"));
        assert_eq!(telegram.bot_token_ciphertext, Some(vec![1, 2, 3]));
    }

    #[tokio::test]
    async fn clear_telegram_chat_preserves_bot_token() {
        let pool = test_db::pool().await;
        let key = format!(
            "gateway-clear-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        seed_agent(&pool, &key).await;

        let config = TelegramGatewayConfig {
            bot_token_ciphertext: Some(vec![9, 9, 9]),
            chat_id: Some(42),
            chat_username: Some("user".to_string()),
            ..Default::default()
        };
        upsert_telegram_config(&pool, &key, &config)
            .await
            .expect("create config");

        assert!(clear_telegram_chat(&pool, &key).await.expect("clear chat"));
        let stored = get_gateway(&pool, &key, GATEWAY_TYPE_TELEGRAM)
            .await
            .expect("get gateway")
            .expect("gateway exists");
        let telegram = TelegramGatewayConfig::from_value(&stored.config);
        assert_eq!(telegram.chat_id, None);
        assert_eq!(telegram.chat_username, None);
        assert_eq!(telegram.bot_token_ciphertext, Some(vec![9, 9, 9]));
    }

    #[tokio::test]
    async fn list_enabled_telegram_gateways_returns_only_enabled_rows() {
        let pool = test_db::pool().await;
        let key_enabled = format!(
            "gateway-enabled-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        let key_disabled = format!(
            "gateway-disabled-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        seed_agent(&pool, &key_enabled).await;
        seed_agent(&pool, &key_disabled).await;

        let config = TelegramGatewayConfig {
            bot_token_ciphertext: Some(vec![1, 2, 3]),
            ..Default::default()
        };
        upsert_telegram_config(&pool, &key_enabled, &config)
            .await
            .expect("create enabled config");
        upsert_telegram_config(&pool, &key_disabled, &config)
            .await
            .expect("create disabled config");
        set_gateway_enabled(&pool, &key_enabled, GATEWAY_TYPE_TELEGRAM, true)
            .await
            .expect("enable first");

        let rows = list_enabled_telegram_gateways(&pool)
            .await
            .expect("list enabled");
        assert!(rows.iter().any(|row| row.agent_key == key_enabled));
        assert!(!rows.iter().any(|row| row.agent_key == key_disabled));
    }
}
