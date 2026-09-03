use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, anyhow};
use sqlx::{Postgres, Transaction, query_as};

use crate::{
    agents::model::{AgentDetailRow, AgentListRow, AgentReadiness, AgentRegistryRow},
    db::DbPool,
    web::templates::shared::currency_logo_url,
};

#[cfg(test)]
/// List all active agents for store-level tests.
pub async fn list_agents(pool: &DbPool) -> Result<Vec<AgentListRow>> {
    let rows = query_as::<_, AgentListRow>(
        "SELECT display_name,
                 agents.agent_key,
                 agents.enabled,
                 trading_account_address,
                 environment,
                 api_key,
                 api_key_last_used_at
           FROM agents
           WHERE agents.lifecycle = 'active'
           ORDER BY agents.created_at DESC",
    )
    .fetch_all(pool)
    .await
    .context("failed to list agents")?;

    Ok(rows)
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AgentInstrumentOptionRow {
    pub instrument_id: String,
    pub selected: bool,
    pub logo_url: String,
}

#[derive(Debug, sqlx::FromRow)]
struct AgentInstrumentOptionRawRow {
    instrument_id: String,
    selected: bool,
}

/// List only the active agents owned by an operator.
pub async fn list_agents_for_user(pool: &DbPool, user_id: uuid::Uuid) -> Result<Vec<AgentListRow>> {
    let rows = query_as::<_, AgentListRow>(
     "SELECT display_name, agents.agent_key, agents.enabled, trading_account_address, environment,
                api_key, api_key_last_used_at
           FROM agents
          WHERE agents.lifecycle = 'active' AND agents.user_id = $1
          ORDER BY agents.created_at DESC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .context("failed to list user agents")?;
    Ok(rows)
}

/// Load the setup state that determines whether each active agent can run the
/// unattended analysis-to-trading chain.
pub async fn list_agent_readiness_for_user(
    pool: &DbPool,
    user_id: uuid::Uuid,
) -> Result<BTreeMap<String, AgentReadiness>> {
    let rows = query_as::<_, AgentReadiness>(
        "SELECT agents.agent_key,
                agents.lifecycle = 'active' AS active,
                agents.enabled,
                EXISTS (
                    SELECT 1
                      FROM agent_instruments
                      JOIN hyperliquid.instruments AS instruments
                        ON instruments.instrument_id = agent_instruments.instrument_id
                     WHERE agent_instruments.agent_key = agents.agent_key
                       AND instruments.market_type = 'perp'
                       AND instruments.active = true
                ) AS has_selected_instruments,
                EXISTS (
                    SELECT 1
                      FROM harness_sub_agents
                     WHERE harness_sub_agents.agent_key = agents.agent_key
                       AND harness_sub_agents.sub_agent_kind = 'analysis'
                       AND harness_sub_agents.enabled = true
                ) AS has_enabled_analysis_job,
                EXISTS (
                    SELECT 1
                      FROM harness_sub_agents
                     WHERE harness_sub_agents.agent_key = agents.agent_key
                       AND harness_sub_agents.sub_agent_kind = 'market_analysis'
                       AND harness_sub_agents.enabled = true
                ) AS has_enabled_market_analysis_job,
                EXISTS (
                    SELECT 1
                      FROM harness_sub_agents
                     WHERE harness_sub_agents.agent_key = agents.agent_key
                       AND harness_sub_agents.sub_agent_kind = 'trading'
                       AND harness_sub_agents.enabled = true
                ) AS has_enabled_trading_job,
                (
                    SELECT id
                     FROM harness_sub_agents
                     WHERE harness_sub_agents.agent_key = agents.agent_key
                       AND harness_sub_agents.sub_agent_kind = 'market_analysis'
                     ORDER BY id
                     LIMIT 1
                ) AS market_analysis_sub_agent_id,
                (
                    SELECT id
                      FROM harness_sub_agents
                     WHERE harness_sub_agents.agent_key = agents.agent_key
                       AND harness_sub_agents.sub_agent_kind = 'trading'
                     ORDER BY id
                     LIMIT 1
                ) AS trading_sub_agent_id
           FROM agents
          WHERE agents.user_id = $1
            AND agents.lifecycle = 'active'",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .context("failed to list agent readiness")?;

    Ok(rows
        .into_iter()
        .map(|readiness| (readiness.agent_key.clone(), readiness))
        .collect())
}

/// Load readiness for a single agent page.
pub async fn get_agent_readiness(pool: &DbPool, agent_key: &str) -> Result<Option<AgentReadiness>> {
    let rows = list_agent_readiness_for_agent_key(pool, agent_key).await?;
    Ok(rows.into_iter().next())
}

async fn list_agent_readiness_for_agent_key(
    pool: &DbPool,
    agent_key: &str,
) -> Result<Vec<AgentReadiness>> {
    let rows = query_as::<_, AgentReadiness>(
        "SELECT agents.agent_key,
                agents.lifecycle = 'active' AS active,
                agents.enabled,
                EXISTS (
                    SELECT 1
                      FROM agent_instruments
                      JOIN hyperliquid.instruments AS instruments
                        ON instruments.instrument_id = agent_instruments.instrument_id
                     WHERE agent_instruments.agent_key = agents.agent_key
                       AND instruments.market_type = 'perp'
                       AND instruments.active = true
                ) AS has_selected_instruments,
                EXISTS (
                    SELECT 1 FROM harness_sub_agents
                     WHERE harness_sub_agents.agent_key = agents.agent_key
                       AND harness_sub_agents.sub_agent_kind = 'analysis'
                       AND harness_sub_agents.enabled = true
                ) AS has_enabled_analysis_job,
                EXISTS (
                    SELECT 1 FROM harness_sub_agents
                     WHERE harness_sub_agents.agent_key = agents.agent_key
                       AND harness_sub_agents.sub_agent_kind = 'market_analysis'
                       AND harness_sub_agents.enabled = true
                ) AS has_enabled_market_analysis_job,
                EXISTS (
                    SELECT 1 FROM harness_sub_agents
                     WHERE harness_sub_agents.agent_key = agents.agent_key
                       AND harness_sub_agents.sub_agent_kind = 'trading'
                       AND harness_sub_agents.enabled = true
                ) AS has_enabled_trading_job,
                (
                    SELECT id
                      FROM harness_sub_agents
                     WHERE harness_sub_agents.agent_key = agents.agent_key
                       AND harness_sub_agents.sub_agent_kind = 'market_analysis'
                     ORDER BY id
                     LIMIT 1
                ) AS market_analysis_sub_agent_id,
                (
                    SELECT id
                      FROM harness_sub_agents
                     WHERE harness_sub_agents.agent_key = agents.agent_key
                       AND harness_sub_agents.sub_agent_kind = 'trading'
                     ORDER BY id
                     LIMIT 1
                ) AS trading_sub_agent_id
           FROM agents
          WHERE agents.agent_key = $1",
    )
    .bind(agent_key)
    .fetch_all(pool)
    .await
    .with_context(|| format!("failed to load readiness for agent {agent_key}"))?;

    Ok(rows)
}

/// Checks the ownership boundary before an operator route resolves child resources.
pub async fn agent_belongs_to_user(
    pool: &DbPool,
    agent_key: &str,
    user_id: uuid::Uuid,
) -> Result<bool> {
    let found: Option<(i32,)> =
        query_as("SELECT 1 FROM agents WHERE agent_key = $1 AND user_id = $2")
            .bind(agent_key)
            .bind(user_id)
            .fetch_optional(pool)
            .await
            .context("failed to check agent ownership")?;
    Ok(found.is_some())
}

/// Locked execution state for one agent. Callers that change execution
/// eligibility or submit an exchange request must hold this advisory lock until
/// their operation has reached a terminal exchange outcome.
#[derive(Debug, Clone, Copy)]
pub struct AgentExecutionState {
    pub enabled: bool,
    pub active: bool,
}

/// Lock an agent's execution state inside the caller's transaction.
///
/// The transaction-scoped advisory lock avoids conflicting with foreign-key
/// locks acquired while the gateway persists child order rows.
pub async fn lock_agent_execution_tx(
    tx: &mut Transaction<'_, Postgres>,
    agent_key: &str,
) -> Result<Option<AgentExecutionState>> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
        .bind(agent_key)
        .execute(&mut **tx)
        .await
        .with_context(|| format!("failed to acquire execution lock for agent {agent_key}"))?;
    let row: Option<(bool, String)> =
        query_as("SELECT enabled, lifecycle FROM agents WHERE agent_key = $1")
            .bind(agent_key)
            .fetch_optional(&mut **tx)
            .await
            .with_context(|| format!("failed to load execution state for agent {agent_key}"))?;

    Ok(row.map(|(enabled, lifecycle)| AgentExecutionState {
        enabled,
        active: lifecycle == crate::agents::model::AGENT_LIFECYCLE_ACTIVE,
    }))
}

/// Update the durable execution gate for one agent while holding its advisory lock.
pub async fn set_agent_enabled(pool: &DbPool, agent_key: &str, enabled: bool) -> Result<bool> {
    let mut tx = pool
        .begin()
        .await
        .context("failed to begin agent enabled update")?;
    if lock_agent_execution_tx(&mut tx, agent_key).await?.is_none() {
        tx.rollback().await?;
        return Ok(false);
    }

    sqlx::query("UPDATE agents SET enabled = $2, updated_at = now() WHERE agent_key = $1")
        .bind(agent_key)
        .bind(enabled)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("failed to update enabled state for agent {agent_key}"))?;
    tx.commit()
        .await
        .context("failed to commit agent enabled update")?;
    Ok(true)
}

/// Fetch a single agent by its unique agent key.
pub async fn get_agent(pool: &DbPool, agent_key: &str) -> Result<Option<AgentDetailRow>> {
    let row = query_as::<_, AgentDetailRow>(
        "SELECT display_name,
                 agents.agent_key,
                 agents.enabled,
                 agents.lifecycle,
                  trading_account_address,
                 environment,
                  api_key
           FROM agents
           WHERE agents.agent_key = $1",
    )
    .bind(agent_key)
    .fetch_optional(pool)
    .await
    .context("failed to fetch agent")?;

    Ok(row)
}

pub async fn list_agent_instrument_ids(pool: &DbPool, agent_key: &str) -> Result<Vec<String>> {
    let rows: Vec<(String,)> = query_as(
        "SELECT instruments.instrument_id
           FROM agent_instruments
           JOIN hyperliquid.instruments AS instruments
             ON instruments.instrument_id = agent_instruments.instrument_id
          WHERE agent_instruments.agent_key = $1
            AND instruments.market_type = 'perp'
            AND instruments.active = true
          ORDER BY instruments.instrument_id",
    )
    .bind(agent_key)
    .fetch_all(pool)
    .await
    .context("failed to list agent instrument ids")?;

    Ok(rows
        .into_iter()
        .map(|(instrument_id,)| instrument_id)
        .collect())
}

pub async fn list_agent_instrument_options(
    pool: &DbPool,
    agent_key: &str,
) -> Result<Vec<AgentInstrumentOptionRow>> {
    let rows = query_as::<_, AgentInstrumentOptionRawRow>(
        "SELECT instruments.instrument_id,
                (agent_instruments.instrument_id IS NOT NULL) AS selected
           FROM hyperliquid.instruments AS instruments
           LEFT JOIN agent_instruments
             ON agent_instruments.agent_key = $1
            AND agent_instruments.instrument_id = instruments.instrument_id
          WHERE instruments.market_type = 'perp'
            AND instruments.active = true
          ORDER BY instruments.instrument_id",
    )
    .bind(agent_key)
    .fetch_all(pool)
    .await
    .context("failed to list agent instrument options")?;

    Ok(rows
        .into_iter()
        .map(|row| AgentInstrumentOptionRow {
            logo_url: currency_logo_url(&row.instrument_id),
            instrument_id: row.instrument_id,
            selected: row.selected,
        })
        .collect())
}

pub async fn replace_agent_instruments(
    pool: &DbPool,
    agent_key: &str,
    instrument_ids: &[String],
) -> Result<bool> {
    let unique_ids: Vec<String> = instrument_ids
        .iter()
        .map(|instrument_id| instrument_id.trim())
        .filter(|instrument_id| !instrument_id.is_empty())
        .map(ToOwned::to_owned)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    let mut tx = pool
        .begin()
        .await
        .context("failed to begin agent instrument replacement transaction")?;

    let agent_exists: Option<(i32,)> = query_as("SELECT 1 FROM agents WHERE agent_key = $1")
        .bind(agent_key)
        .fetch_optional(&mut *tx)
        .await
        .context("failed to validate agent existence before replacing instruments")?;
    if agent_exists.is_none() {
        return Ok(false);
    }

    if !unique_ids.is_empty() {
        let valid_rows: Vec<(String,)> = query_as(
            "SELECT instrument_id
               FROM hyperliquid.instruments
              WHERE instrument_id = ANY($1)
                AND market_type = 'perp'
                AND active = true",
        )
        .bind(&unique_ids)
        .fetch_all(&mut *tx)
        .await
        .context("failed to validate selected instrument ids")?;

        let valid_ids: BTreeSet<String> = valid_rows
            .into_iter()
            .map(|(instrument_id,)| instrument_id)
            .collect();
        if valid_ids.len() != unique_ids.len() {
            let invalid_ids: Vec<String> = unique_ids
                .iter()
                .filter(|instrument_id| !valid_ids.contains(*instrument_id))
                .cloned()
                .collect();
            return Err(anyhow!(
                "invalid or inactive instrument ids: {}",
                invalid_ids.join(", ")
            ));
        }
    }

    sqlx::query("DELETE FROM agent_instruments WHERE agent_key = $1")
        .bind(agent_key)
        .execute(&mut *tx)
        .await
        .context("failed to clear existing agent instruments")?;

    for instrument_id in &unique_ids {
        sqlx::query(
            "INSERT INTO agent_instruments (agent_key, instrument_id)
             VALUES ($1, $2)",
        )
        .bind(agent_key)
        .bind(instrument_id)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("failed to insert agent instrument '{instrument_id}'"))?;
    }

    sqlx::query("UPDATE agents SET updated_at = now() WHERE agent_key = $1")
        .bind(agent_key)
        .execute(&mut *tx)
        .await
        .context("failed to update agent timestamp after replacing instruments")?;

    tx.commit()
        .await
        .context("failed to commit agent instrument replacement transaction")?;

    Ok(true)
}

/// Insert a full registry row.
pub async fn insert_agent(pool: &DbPool, row: &AgentRegistryRow) -> Result<()> {
    sqlx::query(
        "INSERT INTO agents (
             agent_key,
             user_id,
             created_at,
             updated_at,
             enabled,
             lifecycle,
             display_name,
                 trading_account_address,
             environment,
            api_key,
             api_key_last_used_at,
             runtime_config
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
    )
    .bind(&row.agent_key)
    .bind(row.user_id)
    .bind(row.created_at)
    .bind(row.updated_at)
    .bind(row.enabled)
    .bind(&row.lifecycle)
    .bind(&row.display_name)
    .bind(&row.trading_account_address)
    .bind(&row.environment)
    .bind(&row.api_key)
    .bind(row.api_key_last_used_at)
    .bind(&row.runtime_config)
    .execute(pool)
    .await
    .context("failed to insert agent")?;

    Ok(())
}

/// Delete a single agent by its unique agent key.
///
/// Returns `true` if a row was deleted, `false` if the agent did not exist.
pub async fn delete_agent(pool: &DbPool, agent_key: &str) -> Result<bool> {
    let mut tx = pool
        .begin()
        .await
        .context("failed to start delete-agent transaction")?;

    let agent: Option<(String, String)> = query_as(
        "SELECT agent_key, environment
           FROM agents
          WHERE agent_key = $1
          FOR UPDATE",
    )
    .bind(agent_key)
    .fetch_optional(&mut *tx)
    .await
    .context("failed to lock agent for deletion")?;

    if agent.is_none() {
        return Ok(false);
    }

    sqlx::query("DELETE FROM agents WHERE agent_key = $1")
        .bind(agent_key)
        .execute(&mut *tx)
        .await
        .context("failed to delete agent")?;

    tx.commit()
        .await
        .context("failed to commit delete-agent transaction")?;

    Ok(true)
}

/// Resolve an `agent_key` from the API key presented in the
/// `Authorization: Bearer <api_key>` header.
///
/// Returns `Ok(None)` when the key does not match any row. Used by the
/// API-key authentication extractor to scope incoming requests to a single
/// agent.
pub async fn resolve_agent_key_by_api_key(pool: &DbPool, api_key: &str) -> Result<Option<String>> {
    let row: Option<(String,)> = query_as("SELECT agent_key FROM agents WHERE api_key = $1")
        .bind(api_key)
        .fetch_optional(pool)
        .await
        .context("failed to resolve agent_key by api_key")?;

    Ok(row.map(|(k,)| k))
}

/// Best-effort update of `api_key_last_used_at` for an authenticated agent.
///
/// Called by the API-key auth extractor on every successful authentication
/// so the operator UI can show when each key was last used. Errors are
/// surfaced to the caller so they can be logged, but authentication must
/// not fail when this update fails.
pub async fn touch_api_key_last_used(pool: &DbPool, api_key: &str) -> Result<()> {
    sqlx::query("UPDATE agents SET api_key_last_used_at = now() WHERE api_key = $1")
        .bind(api_key)
        .execute(pool)
        .await
        .context("failed to touch api_key_last_used_at")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    use crate::test_db;

    fn sample_agent(key: &str) -> AgentRegistryRow {
        let now = Utc::now();
        AgentRegistryRow {
            agent_key: key.to_string(),
            user_id: crate::test_db::test_user_id(),
            created_at: now,
            updated_at: now,
            enabled: true,
            lifecycle: crate::agents::model::AGENT_LIFECYCLE_ACTIVE.to_string(),
            display_name: format!("Test {key}"),
            trading_account_address: Some(format!("0x{:040x}", key.len() as u64 + 1)),
            environment: "live".to_string(),
            api_key: format!("vta_{key}"),
            api_key_last_used_at: None,
            runtime_config: serde_json::json!({}),
        }
    }

    async fn seed_instrument(pool: &DbPool, instrument_id: &str, market_type: &str, active: bool) {
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO hyperliquid.instruments (
                instrument_id,
                name,
                market_type,
                base_asset,
                quote_asset,
                settlement_asset,
                asset_index,
                price_decimals,
                size_decimals,
                lot_size,
                max_leverage,
                is_hip3,
                active,
                created_at,
                updated_at
            ) VALUES (
                $1, $1, $2, $1, 'USD', 'USDC', 1, 2, 3, 0.001, 50, false, $3, $4, $4
            )
            ON CONFLICT (instrument_id) DO UPDATE
                SET name = EXCLUDED.name,
                    market_type = EXCLUDED.market_type,
                    active = EXCLUDED.active,
                    updated_at = EXCLUDED.updated_at",
        )
        .bind(instrument_id)
        .bind(market_type)
        .bind(active)
        .bind(now)
        .execute(pool)
        .await
        .expect("insert instrument");
    }

    #[tokio::test]
    async fn list_agents_returns_inserted_rows() {
        let pool = test_db::pool().await;

        let key = format!("list-test-{}", Utc::now().timestamp_millis());
        let row = sample_agent(&key);
        insert_agent(&pool, &row).await.expect("insert agent");

        let agents = list_agents(&pool).await.expect("list agents");
        assert!(agents.iter().any(|a| a.agent_key == key));
    }

    #[tokio::test]
    async fn readiness_tracks_the_complete_agent_trading_chain() {
        let pool = test_db::pool().await;
        let key = format!("readiness-test-{}", Utc::now().timestamp_millis());
        let agent = sample_agent(&key);
        insert_agent(&pool, &agent).await.expect("insert agent");
        crate::harness::store::insert_default_harness_sub_agents(&pool, &key)
            .await
            .expect("insert default jobs");

        let initial = get_agent_readiness(&pool, &key)
            .await
            .expect("load initial readiness")
            .expect("agent readiness");
        assert!(initial.enabled);
        assert!(!initial.has_selected_instruments);
        assert!(!initial.has_enabled_analysis_job);
        assert!(!initial.has_enabled_market_analysis_job);
        assert!(!initial.has_enabled_trading_job);
        assert!(!initial.is_ready_for_agent_trading());

        seed_instrument(&pool, "BTC", "perp", true).await;
        replace_agent_instruments(&pool, &key, &["BTC".to_string()])
            .await
            .expect("select BTC");
        sqlx::query(
            "UPDATE harness_sub_agents
                SET model_provider_id = 'test-provider',
                    model_id = 'test-model',
                    enabled = true
              WHERE agent_key = $1
                AND sub_agent_kind IN ('analysis', 'market_analysis', 'trading')",
        )
        .bind(&key)
        .execute(&pool)
        .await
        .expect("configure required jobs");

        let readiness = get_agent_readiness(&pool, &key)
            .await
            .expect("load configured readiness")
            .expect("agent readiness");
        assert!(readiness.has_selected_instruments);
        assert!(readiness.has_enabled_analysis_job);
        assert!(readiness.has_enabled_market_analysis_job);
        assert!(readiness.has_enabled_trading_job);
        assert!(readiness.is_ready_for_agent_trading());

        let all_readiness = list_agent_readiness_for_user(&pool, agent.user_id)
            .await
            .expect("list user readiness");
        assert!(all_readiness.contains_key(&key));
    }

    #[tokio::test]
    async fn user_agent_queries_do_not_cross_the_ownership_boundary() {
        let pool = test_db::pool().await;
        let owner = crate::test_db::test_user_id();
        let other_user = uuid::Uuid::new_v4();
        sqlx::query("INSERT INTO users (id, wallet_address) VALUES ($1, $2)")
            .bind(other_user)
            .bind(format!("0x{:040x}", other_user.as_u128()))
            .execute(&pool)
            .await
            .expect("insert second user");

        let own_key = format!("owned-agent-{}", Utc::now().timestamp_millis());
        let foreign_key = format!("foreign-agent-{}", Utc::now().timestamp_millis());
        let own = sample_agent(&own_key);
        let mut foreign = sample_agent(&foreign_key);
        foreign.user_id = other_user;
        insert_agent(&pool, &own).await.expect("insert owned agent");
        insert_agent(&pool, &foreign)
            .await
            .expect("insert foreign agent");

        let agents = list_agents_for_user(&pool, owner)
            .await
            .expect("list owned agents");
        assert!(agents.iter().any(|agent| agent.agent_key == own_key));
        assert!(!agents.iter().any(|agent| agent.agent_key == foreign_key));
        assert!(
            agent_belongs_to_user(&pool, &own_key, owner)
                .await
                .expect("check owner")
        );
        assert!(
            !agent_belongs_to_user(&pool, &foreign_key, owner)
                .await
                .expect("reject foreign owner")
        );
    }

    #[tokio::test]
    async fn delete_agent_removes_row() {
        let pool = test_db::pool().await;

        let key = format!("delete-test-{}", Utc::now().timestamp_millis());
        let row = sample_agent(&key);
        insert_agent(&pool, &row).await.expect("insert agent");

        let deleted = delete_agent(&pool, &key).await.expect("delete agent");
        assert!(deleted, "expected agent to be deleted");

        let missing = delete_agent(&pool, &key)
            .await
            .expect("delete missing agent");
        assert!(!missing, "expected false for already-deleted agent");

        let agent = get_agent(&pool, &key).await.expect("fetch agent");
        assert!(agent.is_none(), "expected deleted agent to be gone");
    }

    #[tokio::test]
    async fn duplicate_agent_key_is_rejected() {
        let pool = test_db::pool().await;

        let key = format!("dup-test-{}", Utc::now().timestamp_millis());
        let row = sample_agent(&key);
        insert_agent(&pool, &row).await.expect("first insert");

        let mut second = sample_agent(&format!("{}-second", key));
        second.agent_key = key.clone();
        let err = insert_agent(&pool, &second).await.unwrap_err();

        let db_err = err
            .downcast_ref::<sqlx::Error>()
            .and_then(|e| e.as_database_error())
            .map(|e| e.constraint().map(|c| c.to_string()));
        assert!(
            matches!(db_err, Some(Some(ref c)) if c.contains("agent_key") || c.contains("agents_pkey")),
            "expected unique violation on agent_key, got {:?}",
            db_err
        );
    }

    #[tokio::test]
    async fn resolve_agent_key_by_api_key_round_trip() {
        let pool = test_db::pool().await;

        let key = format!("apikey-rt-{}", Utc::now().timestamp_millis());
        let row = sample_agent(&key);
        insert_agent(&pool, &row).await.expect("insert agent");

        let resolved = resolve_agent_key_by_api_key(&pool, &row.api_key)
            .await
            .expect("resolve");
        assert_eq!(resolved.as_deref(), Some(key.as_str()));

        // Unknown key returns None.
        let missing = resolve_agent_key_by_api_key(&pool, "vta_does-not-exist")
            .await
            .expect("resolve missing");
        assert!(missing.is_none());
    }

    #[tokio::test]
    async fn touch_api_key_last_used_sets_timestamp() {
        let pool = test_db::pool().await;

        let key = format!("apikey-touch-{}", Utc::now().timestamp_millis());
        let row = sample_agent(&key);
        insert_agent(&pool, &row).await.expect("insert agent");

        let (before,): (Option<chrono::DateTime<Utc>>,) =
            sqlx::query_as("SELECT api_key_last_used_at FROM agents WHERE agent_key = $1")
                .bind(&key)
                .fetch_one(&pool)
                .await
                .expect("fetch");
        assert!(before.is_none());

        touch_api_key_last_used(&pool, &row.api_key)
            .await
            .expect("touch");

        let (after,): (Option<chrono::DateTime<Utc>>,) =
            sqlx::query_as("SELECT api_key_last_used_at FROM agents WHERE agent_key = $1")
                .bind(&key)
                .fetch_one(&pool)
                .await
                .expect("fetch");
        assert!(after.is_some());
    }

    #[tokio::test]
    async fn replace_agent_instruments_round_trips_selected_ids() {
        let pool = test_db::pool().await;
        let key = format!(
            "agent-instruments-roundtrip-{}",
            Utc::now().timestamp_millis()
        );
        insert_agent(&pool, &sample_agent(&key))
            .await
            .expect("insert agent");
        seed_instrument(&pool, "BTC", "perp", true).await;
        seed_instrument(&pool, "ETH", "perp", true).await;
        seed_instrument(&pool, "SOL", "perp", true).await;

        let updated = replace_agent_instruments(
            &pool,
            &key,
            &[
                "BTC".to_string(),
                "ETH".to_string(),
                "BTC".to_string(),
                "  ETH  ".to_string(),
            ],
        )
        .await
        .expect("replace instruments");
        assert!(updated);

        let selected = list_agent_instrument_ids(&pool, &key)
            .await
            .expect("list selected instruments");
        assert_eq!(selected, vec!["BTC".to_string(), "ETH".to_string()]);

        let options = list_agent_instrument_options(&pool, &key)
            .await
            .expect("list instrument options");
        assert_eq!(options.len(), 3);
        assert_eq!(options[0].instrument_id, "BTC");
        assert!(options[0].selected);
        assert_eq!(options[1].instrument_id, "ETH");
        assert!(options[1].selected);
        assert_eq!(options[2].instrument_id, "SOL");
        assert!(!options[2].selected);
    }

    #[tokio::test]
    async fn replace_agent_instruments_allows_empty_selection() {
        let pool = test_db::pool().await;
        let key = format!("agent-instruments-empty-{}", Utc::now().timestamp_millis());
        insert_agent(&pool, &sample_agent(&key))
            .await
            .expect("insert agent");
        seed_instrument(&pool, "BTC", "perp", true).await;

        replace_agent_instruments(&pool, &key, &["BTC".to_string()])
            .await
            .expect("seed selection");
        replace_agent_instruments(&pool, &key, &[])
            .await
            .expect("clear selection");

        let selected = list_agent_instrument_ids(&pool, &key)
            .await
            .expect("list selected instruments");
        assert!(selected.is_empty());
    }

    #[tokio::test]
    async fn deleting_agent_cascades_agent_instruments() {
        let pool = test_db::pool().await;
        let key = format!(
            "agent-instruments-cascade-{}",
            Utc::now().timestamp_millis()
        );
        insert_agent(&pool, &sample_agent(&key))
            .await
            .expect("insert agent");
        seed_instrument(&pool, "BTC", "perp", true).await;

        replace_agent_instruments(&pool, &key, &["BTC".to_string()])
            .await
            .expect("replace instruments");
        delete_agent(&pool, &key).await.expect("delete agent");

        let count: (i64,) = query_as("SELECT COUNT(*) FROM agent_instruments WHERE agent_key = $1")
            .bind(&key)
            .fetch_one(&pool)
            .await
            .expect("count agent instruments");
        assert_eq!(count.0, 0);
    }

    #[tokio::test]
    async fn deleting_agent_cascades_hyperliquid_rows_but_preserves_instruments() {
        let pool = test_db::pool().await;
        let key = format!("agent-hl-cascade-{}", Utc::now().timestamp_millis());
        let agent = sample_agent(&key);
        let trading_account_address = agent
            .trading_account_address
            .clone()
            .expect("trading account");
        let environment = agent.environment.clone();
        insert_agent(&pool, &agent).await.expect("insert agent");

        seed_instrument(&pool, "BTC", "perp", true).await;

        sqlx::query(
            "INSERT INTO hyperliquid.sync_state
                (account_address, environment, stream_name, status, metadata)
             VALUES ($1, $2, 'fills', 'healthy', '{}'::jsonb)",
        )
        .bind(&trading_account_address)
        .bind(&environment)
        .execute(&pool)
        .await
        .expect("insert sync_state");

        sqlx::query(
            "INSERT INTO hyperliquid.trade_fills
                (hash, account_address, environment, event_time, source_stream, instrument_id,
                 fill_time, direction, side, price, size, trade_id, payload, ingest_source, inserted_at)
             VALUES ($1, $2, $3, now(), 'fills', 'BTC', now(), 'open_long', 'buy', 1, 1, $4, '{}'::jsonb, 'test', now())",
        )
        .bind(format!("fill-hash-{key}"))
        .bind(&trading_account_address)
        .bind(&environment)
        .bind(format!("trade-{key}"))
        .execute(&pool)
        .await
        .expect("insert trade_fill");

        sqlx::query(
            "INSERT INTO hyperliquid.funding_events
                (account_address, environment, instrument_id, event_time, source_stream,
                 usdc, payload, ingest_source, inserted_at)
             VALUES ($1, $2, 'BTC', now(), 'funding', 1, '{}'::jsonb, 'test', now())",
        )
        .bind(&trading_account_address)
        .bind(&environment)
        .execute(&pool)
        .await
        .expect("insert funding_event");

        sqlx::query(
            "INSERT INTO hyperliquid.ledger_events
                (hash, account_address, environment, event_time, event_type, source_stream,
                 ledger_type, payload, ingest_source, inserted_at)
             VALUES ($1, $2, $3, now(), 'ledger', 'ledger', 'deposit', '{}'::jsonb, 'test', now())",
        )
        .bind(format!("ledger-hash-{key}"))
        .bind(&trading_account_address)
        .bind(&environment)
        .execute(&pool)
        .await
        .expect("insert ledger_event");

        sqlx::query(
            "INSERT INTO hyperliquid.historical_orders
                (account_address, environment, order_id, event_time, instrument_id, payload, ingest_source, inserted_at)
             VALUES ($1, $2, $3, now(), 'BTC', '{}'::jsonb, 'test', now())",
        )
        .bind(&trading_account_address)
        .bind(&environment)
        .bind(format!("historical-{key}"))
        .execute(&pool)
        .await
        .expect("insert historical_order");

        let order_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO hyperliquid.orders
                (id, created_at, updated_at, agent_key, account_address, environment, symbol,
                 instrument_id, side, order_kind, requested_size, cloid, status)
             VALUES ($1, now(), now(), $2, $3, $4, 'BTC', 'BTC', 'buy', 'limit', 1, $5, 'pending_submission')",
        )
        .bind(order_id)
        .bind(&key)
        .bind(&trading_account_address)
        .bind(&environment)
        .bind(format!("cloid-{key}"))
        .execute(&pool)
        .await
        .expect("insert order");

        sqlx::query(
            "INSERT INTO hyperliquid.order_events
                (id, order_id, account_address, environment, status, status_timestamp, source, payload, inserted_at)
             VALUES ($1, $2, $3, $4, 'pending_submission', now(), 'http_response', '{}'::jsonb, now())",
        )
        .bind(Uuid::new_v4())
        .bind(order_id)
        .bind(&trading_account_address)
        .bind(&environment)
        .execute(&pool)
        .await
        .expect("insert order_event");

        let deleted = delete_agent(&pool, &key).await.expect("delete agent");
        assert!(deleted);

        let agent_count: (i64,) = query_as("SELECT COUNT(*) FROM agents WHERE agent_key = $1")
            .bind(&key)
            .fetch_one(&pool)
            .await
            .expect("count agents");
        let sync_state_count: (i64,) = query_as(
            "SELECT COUNT(*) FROM hyperliquid.sync_state WHERE account_address = $1 AND environment = $2",
        )
        .bind(&trading_account_address)
        .bind(&environment)
        .fetch_one(&pool)
        .await
        .expect("count sync_state");
        let trade_fills_count: (i64,) = query_as(
            "SELECT COUNT(*) FROM hyperliquid.trade_fills WHERE account_address = $1 AND environment = $2",
        )
        .bind(&trading_account_address)
        .bind(&environment)
        .fetch_one(&pool)
        .await
        .expect("count trade_fills");
        let funding_count: (i64,) = query_as(
            "SELECT COUNT(*) FROM hyperliquid.funding_events WHERE account_address = $1 AND environment = $2",
        )
        .bind(&trading_account_address)
        .bind(&environment)
        .fetch_one(&pool)
        .await
        .expect("count funding_events");
        let ledger_count: (i64,) = query_as(
            "SELECT COUNT(*) FROM hyperliquid.ledger_events WHERE account_address = $1 AND environment = $2",
        )
        .bind(&trading_account_address)
        .bind(&environment)
        .fetch_one(&pool)
        .await
        .expect("count ledger_events");
        let historical_count: (i64,) = query_as(
            "SELECT COUNT(*) FROM hyperliquid.historical_orders WHERE account_address = $1 AND environment = $2",
        )
        .bind(&trading_account_address)
        .bind(&environment)
        .fetch_one(&pool)
        .await
        .expect("count historical_orders");
        let order_count: (i64,) =
            query_as("SELECT COUNT(*) FROM hyperliquid.orders WHERE agent_key = $1")
                .bind(&key)
                .fetch_one(&pool)
                .await
                .expect("count orders");
        let order_events_count: (i64,) =
            query_as("SELECT COUNT(*) FROM hyperliquid.order_events WHERE order_id = $1")
                .bind(order_id)
                .fetch_one(&pool)
                .await
                .expect("count order_events");
        let instrument_count: (i64,) =
            query_as("SELECT COUNT(*) FROM hyperliquid.instruments WHERE instrument_id = 'BTC'")
                .fetch_one(&pool)
                .await
                .expect("count instruments");

        assert_eq!(agent_count.0, 0);
        assert_eq!(sync_state_count.0, 0);
        assert_eq!(trade_fills_count.0, 0);
        assert_eq!(funding_count.0, 0);
        assert_eq!(ledger_count.0, 0);
        assert_eq!(historical_count.0, 0);
        assert_eq!(order_count.0, 0);
        assert_eq!(order_events_count.0, 0);
        assert_eq!(instrument_count.0, 1);
    }

    #[tokio::test]
    async fn replace_agent_instruments_rejects_invalid_or_inactive_ids() {
        let pool = test_db::pool().await;
        let key = format!(
            "agent-instruments-invalid-{}",
            Utc::now().timestamp_millis()
        );
        insert_agent(&pool, &sample_agent(&key))
            .await
            .expect("insert agent");
        seed_instrument(&pool, "BTC", "perp", true).await;
        seed_instrument(&pool, "ETH", "perp", false).await;
        seed_instrument(&pool, "SOL", "spot", true).await;

        let err = replace_agent_instruments(
            &pool,
            &key,
            &["BTC".to_string(), "ETH".to_string(), "SOL".to_string()],
        )
        .await
        .expect_err("invalid selection should fail");
        assert!(
            err.to_string().contains("ETH") && err.to_string().contains("SOL"),
            "unexpected error: {err:#}"
        );

        let selected = list_agent_instrument_ids(&pool, &key)
            .await
            .expect("list selected instruments");
        assert!(selected.is_empty());
    }
}
