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
           RETURNING agent_key, config",
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

/// Persist a new Telegram config blob, creating or updating the gateway row.
/// The gateway is enabled whenever the config contains token material.
pub async fn upsert_telegram_config(
    pool: &DbPool,
    agent_key: &str,
    config: &TelegramGatewayConfig,
) -> Result<AgentGatewayRow> {
    let value = serde_json::to_value(config).context("failed to serialize telegram config")?;
    let enabled = config.is_ready();
    upsert_gateway(pool, agent_key, GATEWAY_TYPE_TELEGRAM, enabled, &value).await
}

/// Load a single gateway row.
pub async fn get_gateway(
    pool: &DbPool,
    agent_key: &str,
    gateway_type: &str,
) -> Result<Option<AgentGatewayRow>> {
    let row = query_as::<_, AgentGatewayRow>(
        "SELECT agent_key, config
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

/// List every Telegram gateway with token material. Token presence is the
/// source of truth for Telegram activation; the persisted enabled flag is
/// maintained for generic gateway-row compatibility.
pub async fn list_configured_telegram_gateways(pool: &DbPool) -> Result<Vec<AgentGatewayRow>> {
    let rows = query_as::<_, AgentGatewayRow>(
        "SELECT agent_key, config
           FROM agent_gateways
          WHERE gateway_type = $1
            AND config->>'bot_token_ciphertext' IS NOT NULL
            AND config->>'bot_token_key_id' IS NOT NULL",
    )
    .bind(GATEWAY_TYPE_TELEGRAM)
    .fetch_all(pool)
    .await
    .context("failed to list configured telegram gateways")?;
    Ok(rows)
}

/// Remove all Telegram credentials and bindings for an agent.
pub async fn disconnect_telegram_gateway(pool: &DbPool, agent_key: &str) -> Result<bool> {
    let value = serde_json::to_value(TelegramGatewayConfig::default())
        .context("failed to serialize empty telegram config")?;
    let result = sqlx::query(
        "UPDATE agent_gateways
            SET enabled = FALSE, config = $3
          WHERE agent_key = $1 AND gateway_type = $2",
    )
    .bind(agent_key)
    .bind(GATEWAY_TYPE_TELEGRAM)
    .bind(value)
    .execute(pool)
    .await
    .context("failed to disconnect telegram gateway")?;
    Ok(result.rows_affected() > 0)
}

/// Persist the bound chat identity onto a Telegram gateway config. Used when
/// Telegram receives the `/start` message for a pending link.
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
            SET enabled = $4, config = $3
          WHERE agent_key = $1 AND gateway_type = $2",
    )
    .bind(agent_key)
    .bind(GATEWAY_TYPE_TELEGRAM)
    .bind(value)
    .bind(config.is_ready())
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

    async fn gateway_enabled(pool: &DbPool, key: &str) -> bool {
        sqlx::query_scalar("SELECT enabled FROM agent_gateways WHERE agent_key = $1")
            .bind(key)
            .fetch_one(pool)
            .await
            .expect("read gateway enabled flag")
    }

    #[tokio::test]
    async fn upsert_telegram_config_enables_gateway_when_token_material_is_present() {
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
        upsert_telegram_config(&pool, &key, &config)
            .await
            .expect("create telegram config");
        assert!(gateway_enabled(&pool, &key).await);
        upsert_telegram_config(&pool, &key, &config)
            .await
            .expect("update telegram config");
        assert!(gateway_enabled(&pool, &key).await);
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
    async fn list_configured_telegram_gateways_returns_only_rows_with_token_material() {
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
            bot_token_key_id: Some("test".to_string()),
            ..Default::default()
        };
        upsert_telegram_config(&pool, &key_enabled, &config)
            .await
            .expect("create enabled config");
        upsert_telegram_config(&pool, &key_disabled, &TelegramGatewayConfig::default())
            .await
            .expect("create disabled config");
        sqlx::query(
            "UPDATE agent_gateways SET enabled = FALSE WHERE agent_key = $1 AND gateway_type = $2",
        )
        .bind(&key_enabled)
        .bind(GATEWAY_TYPE_TELEGRAM)
        .execute(&pool)
        .await
        .expect("disable configured gateway row");

        let rows = list_configured_telegram_gateways(&pool)
            .await
            .expect("list configured");
        assert!(rows.iter().any(|row| row.agent_key == key_enabled));
        assert!(!rows.iter().any(|row| row.agent_key == key_disabled));
    }

    #[tokio::test]
    async fn disconnect_telegram_gateway_clears_config_and_disables_row() {
        let pool = test_db::pool().await;
        let key = format!(
            "gateway-disconnect-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        seed_agent(&pool, &key).await;
        let config = TelegramGatewayConfig {
            bot_token_ciphertext: Some(vec![1, 2, 3]),
            bot_token_key_id: Some("test".to_string()),
            chat_id: Some(42),
            chat_username: Some("user".to_string()),
            bot_username: Some("bot".to_string()),
        };
        upsert_telegram_config(&pool, &key, &config)
            .await
            .expect("create config");

        assert!(
            disconnect_telegram_gateway(&pool, &key)
                .await
                .expect("disconnect gateway")
        );
        let stored = get_gateway(&pool, &key, GATEWAY_TYPE_TELEGRAM)
            .await
            .expect("get gateway")
            .expect("gateway exists");
        assert!(!gateway_enabled(&pool, &key).await);
        let config = TelegramGatewayConfig::from_value(&stored.config);
        assert!(config.bot_token_ciphertext.is_none());
        assert!(config.bot_token_key_id.is_none());
        assert!(config.bot_username.is_none());
        assert!(config.chat_id.is_none());
        assert!(config.chat_username.is_none());
    }
}
