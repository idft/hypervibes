use std::collections::BTreeSet;

use anyhow::{Context, Result, anyhow};
use sqlx::query_as;

use crate::{
    agents::model::{
        AgentDetailRow, AgentListRow, AgentRegistryRow, AgentRuntimeRow, CreateAgentRuntimeForm,
    },
    db::DbPool,
};

/// Deprecated support for the temporary `/api/v1/job-context` API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobContextKind {
    Analysis,
    Trading,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AgentInstrumentOptionRow {
    pub instrument_id: String,
    pub selected: bool,
}

/// List all agents ordered by creation time, newest first.
pub async fn list_agents(pool: &DbPool) -> Result<Vec<AgentListRow>> {
    let rows = query_as::<_, AgentListRow>(
        "SELECT display_name,
                agents.agent_key,
                agents.enabled,
                wallet_address,
                environment,
                api_key,
                api_key_last_used_at,
                agents.backend_kind,
                agents.runtime_id,
                agent_runtimes.name AS runtime_name,
                agent_runtimes.base_url AS runtime_base_url
           FROM agents
           JOIN agent_runtimes
             ON agent_runtimes.id = agents.runtime_id
          ORDER BY agents.created_at DESC",
    )
    .fetch_all(pool)
    .await
    .context("failed to list agents")?;

    Ok(rows)
}

pub async fn list_agent_runtimes(pool: &DbPool) -> Result<Vec<AgentRuntimeRow>> {
    let rows = query_as::<_, AgentRuntimeRow>(
        "SELECT id,
                created_at,
                updated_at,
                name,
                backend_kind,
                enabled,
                base_url,
                runtime_config
           FROM agent_runtimes
          ORDER BY backend_kind, name",
    )
    .fetch_all(pool)
    .await
    .context("failed to list agent runtimes")?;

    Ok(rows)
}

pub async fn list_enabled_agent_runtimes(pool: &DbPool) -> Result<Vec<AgentRuntimeRow>> {
    let rows = query_as::<_, AgentRuntimeRow>(
        "SELECT id,
                created_at,
                updated_at,
                name,
                backend_kind,
                enabled,
                base_url,
                runtime_config
           FROM agent_runtimes
          WHERE enabled = true
          ORDER BY backend_kind, name",
    )
    .fetch_all(pool)
    .await
    .context("failed to list enabled agent runtimes")?;

    Ok(rows)
}

pub async fn get_agent_runtime(pool: &DbPool, id: &str) -> Result<Option<AgentRuntimeRow>> {
    let row = query_as::<_, AgentRuntimeRow>(
        "SELECT id,
                created_at,
                updated_at,
                name,
                backend_kind,
                enabled,
                base_url,
                runtime_config
           FROM agent_runtimes
          WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .context("failed to fetch agent runtime")?;

    Ok(row)
}

pub async fn insert_agent_runtime(pool: &DbPool, form: &CreateAgentRuntimeForm) -> Result<()> {
    let base_url = form.base_url.trim();
    sqlx::query(
        "INSERT INTO agent_runtimes (
            id,
            name,
            backend_kind,
            enabled,
            base_url,
            runtime_config
        ) VALUES ($1, $2, $3, $4, $5, '{}'::jsonb)",
    )
    .bind(form.id.trim())
    .bind(form.name.trim())
    .bind(form.backend_kind.trim())
    .bind(form.enabled())
    .bind((!base_url.is_empty()).then_some(base_url))
    .execute(pool)
    .await
    .context("failed to insert agent runtime")?;

    Ok(())
}

/// Fetch a single agent by its unique agent key.
pub async fn get_agent(pool: &DbPool, agent_key: &str) -> Result<Option<AgentDetailRow>> {
    let row = query_as::<_, AgentDetailRow>(
        "SELECT display_name,
                agents.agent_key,
                agents.enabled,
                analysis_prompt,
                trading_prompt,
                wallet_address,
                environment,
                api_key,
                api_key_last_used_at,
                agents.backend_kind,
                agents.runtime_id,
                agent_runtimes.name AS runtime_name,
                agent_runtimes.base_url AS runtime_base_url,
                agents.runtime_config,
                analysis_context_last_used_at,
                trading_context_last_used_at,
                agents.created_at,
                agents.updated_at
           FROM agents
           JOIN agent_runtimes
             ON agent_runtimes.id = agents.runtime_id
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
    let rows = query_as::<_, AgentInstrumentOptionRow>(
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

    Ok(rows)
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

/// Insert a full registry row. The caller is responsible for encrypting the
/// private key and deriving the wallet address before this call.
pub async fn insert_agent(pool: &DbPool, row: &AgentRegistryRow) -> Result<()> {
    sqlx::query(
        "INSERT INTO agents (
            agent_key,
            created_at,
            updated_at,
            enabled,
            display_name,
            analysis_prompt,
            trading_prompt,
            wallet_address,
            environment,
            api_key,
            api_key_last_used_at,
            backend_kind,
            runtime_id,
            runtime_config,
            analysis_context_last_used_at,
            trading_context_last_used_at,
            hyperliquid_private_key_ciphertext,
            hyperliquid_private_key_key_id
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)",
    )
    .bind(&row.agent_key)
    .bind(row.created_at)
    .bind(row.updated_at)
    .bind(row.enabled)
    .bind(&row.display_name)
    .bind(&row.analysis_prompt)
    .bind(&row.trading_prompt)
    .bind(&row.wallet_address)
    .bind(&row.environment)
    .bind(&row.api_key)
    .bind(row.api_key_last_used_at)
    .bind(&row.backend_kind)
    .bind(&row.runtime_id)
    .bind(&row.runtime_config)
    .bind(row.analysis_context_last_used_at)
    .bind(row.trading_context_last_used_at)
    .bind(&row.hyperliquid_private_key_ciphertext)
    .bind(&row.hyperliquid_private_key_key_id)
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

    let agent: Option<(String, String, String)> = query_as(
        "SELECT agent_key, wallet_address, environment
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

/// Update only the analysis strategy prompt for one agent.
pub async fn update_agent_analysis_prompt(
    pool: &DbPool,
    agent_key: &str,
    analysis_prompt: &str,
) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE agents
            SET analysis_prompt = $2,
                updated_at = now()
          WHERE agent_key = $1",
    )
    .bind(agent_key)
    .bind(analysis_prompt)
    .execute(pool)
    .await
    .context("failed to update agent analysis prompt")?;

    Ok(result.rows_affected() > 0)
}

/// Update only the trading strategy prompt for one agent.
pub async fn update_agent_trading_prompt(
    pool: &DbPool,
    agent_key: &str,
    trading_prompt: &str,
) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE agents
            SET trading_prompt = $2,
                updated_at = now()
          WHERE agent_key = $1",
    )
    .bind(agent_key)
    .bind(trading_prompt)
    .execute(pool)
    .await
    .context("failed to update agent trading prompt")?;

    Ok(result.rows_affected() > 0)
}

pub async fn update_agent_runtime_config(
    pool: &DbPool,
    agent_key: &str,
    runtime_config: serde_json::Value,
) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE agents
            SET runtime_config = $2,
                updated_at = now()
          WHERE agent_key = $1",
    )
    .bind(agent_key)
    .bind(runtime_config)
    .execute(pool)
    .await
    .context("failed to update agent runtime config")?;

    Ok(result.rows_affected() > 0)
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

/// Load the encrypted Hyperliquid private key + the key id used to
/// encrypt it. Used by the order submission gateway to build a
/// [`PrivateKeySigner`] for signing.
pub async fn get_agent_private_key_ciphertext(
    pool: &DbPool,
    agent_key: &str,
) -> Result<Option<(Vec<u8>, String)>> {
    let row: Option<(Vec<u8>, String)> = query_as(
        "SELECT hyperliquid_private_key_ciphertext, hyperliquid_private_key_key_id
           FROM agents
          WHERE agent_key = $1",
    )
    .bind(agent_key)
    .fetch_optional(pool)
    .await
    .context("failed to load agent private key ciphertext")?;
    Ok(row)
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

/// Deprecated support for the temporary `/api/v1/job-context` API.
pub async fn touch_job_context_last_used(
    pool: &DbPool,
    agent_key: &str,
    kind: JobContextKind,
) -> Result<()> {
    let query = match kind {
        JobContextKind::Analysis => {
            "UPDATE agents SET analysis_context_last_used_at = now() WHERE agent_key = $1"
        }
        JobContextKind::Trading => {
            "UPDATE agents SET trading_context_last_used_at = now() WHERE agent_key = $1"
        }
    };

    sqlx::query(query)
        .bind(agent_key)
        .execute(pool)
        .await
        .with_context(|| format!("failed to touch job-context check-in for agent {agent_key}"))?;

    Ok(())
}

pub async fn runtime_matches_backend(
    pool: &DbPool,
    runtime_id: &str,
    backend_kind: &str,
) -> Result<bool> {
    let row: Option<(i32,)> = query_as(
        "SELECT 1
           FROM agent_runtimes
          WHERE id = $1
            AND enabled = true
            AND backend_kind = $2",
    )
    .bind(runtime_id)
    .bind(backend_kind)
    .fetch_optional(pool)
    .await
    .context("failed to validate agent runtime/backend match")?;

    Ok(row.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    use crate::{
        agents::{
            crypto::{EncryptionKey, encrypt},
            keys::derive_wallet_address,
        },
        test_db,
    };

    fn sample_agent(key: &str) -> AgentRegistryRow {
        sample_agent_with_private_key(key, &deterministic_private_key(key))
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

    fn sample_agent_with_private_key(key: &str, private_key: &str) -> AgentRegistryRow {
        let enc = EncryptionKey::new(
            "test",
            [
                0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22,
                23, 24, 25, 26, 27, 28, 29, 30, 31,
            ],
        );
        let ciphertext = encrypt(&enc, private_key).unwrap();
        let wallet = derive_wallet_address(private_key).unwrap();
        let now = Utc::now();

        AgentRegistryRow {
            agent_key: key.to_string(),
            created_at: now,
            updated_at: now,
            enabled: true,
            display_name: format!("Test {}", key),
            analysis_prompt: "Test analysis prompt".to_string(),
            trading_prompt: "Test trading prompt".to_string(),
            wallet_address: wallet,
            environment: "live".to_string(),
            api_key: format!("vta_{}", key),
            api_key_last_used_at: None,
            backend_kind: crate::agents::model::BACKEND_KIND_OPENCODE.to_string(),
            runtime_id: "opencode-local".to_string(),
            runtime_config: serde_json::json!({}),
            analysis_context_last_used_at: None,
            trading_context_last_used_at: None,
            hyperliquid_private_key_ciphertext: ciphertext,
            hyperliquid_private_key_key_id: "test".to_string(),
        }
    }

    /// Derive a deterministic, unique private key for a test key string.
    fn deterministic_private_key(key: &str) -> String {
        use rand::rngs::StdRng;
        use rand::{Rng, SeedableRng};

        let seed = key
            .bytes()
            .fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
        let mut rng = StdRng::seed_from_u64(seed);
        let bytes: [u8; 32] = rng.r#gen();
        format!("0x{}", hex::encode(bytes))
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
    async fn list_agent_runtimes_includes_seeded_opencode_runtime() {
        let pool = test_db::pool().await;

        let runtimes = list_agent_runtimes(&pool).await.expect("list runtimes");
        let runtime = runtimes
            .iter()
            .find(|runtime| runtime.id == "opencode-local")
            .expect("seeded runtime present");
        assert_eq!(runtime.backend_kind, "opencode");
        assert_eq!(runtime.base_url.as_deref(), Some("http://localhost:14096"));
    }

    #[tokio::test]
    async fn insert_agent_runtime_and_runtime_match_round_trip() {
        let pool = test_db::pool().await;
        let id = format!("opencode-local-{}", Utc::now().timestamp_millis());
        let form = CreateAgentRuntimeForm {
            id: id.clone(),
            name: format!("OpenCode local {id}"),
            backend_kind: crate::agents::model::BACKEND_KIND_OPENCODE.to_string(),
            base_url: "http://localhost:14096".to_string(),
            enabled: Some("on".to_string()),
        };

        insert_agent_runtime(&pool, &form)
            .await
            .expect("insert agent runtime");

        let stored = get_agent_runtime(&pool, &id)
            .await
            .expect("get runtime")
            .expect("runtime present");
        assert_eq!(stored.name, form.name);
        assert_eq!(stored.base_url.as_deref(), Some("http://localhost:14096"));
        assert!(
            runtime_matches_backend(&pool, &id, crate::agents::model::BACKEND_KIND_OPENCODE)
                .await
                .expect("match runtime")
        );
        assert!(
            !runtime_matches_backend(&pool, &id, "other")
                .await
                .expect("mismatch runtime")
        );
    }

    #[tokio::test]
    async fn list_enabled_agent_runtimes_filters_disabled_rows() {
        let pool = test_db::pool().await;
        let id = format!("disabled-runtime-{}", Utc::now().timestamp_millis());
        let form = CreateAgentRuntimeForm {
            id: id.clone(),
            name: format!("Disabled runtime {id}"),
            backend_kind: crate::agents::model::BACKEND_KIND_OPENCODE.to_string(),
            base_url: "http://localhost:14096".to_string(),
            enabled: None,
        };

        insert_agent_runtime(&pool, &form)
            .await
            .expect("insert disabled runtime");

        let runtimes = list_enabled_agent_runtimes(&pool)
            .await
            .expect("list enabled runtimes");
        assert!(runtimes.iter().all(|runtime| runtime.enabled));
        assert!(runtimes.iter().all(|runtime| runtime.id != id));
    }

    #[tokio::test]
    async fn update_agent_runtime_config_persists_json() {
        let pool = test_db::pool().await;
        let key = format!("runtime-config-test-{}", Utc::now().timestamp_millis());
        let row = sample_agent(&key);
        insert_agent(&pool, &row).await.expect("insert agent");

        let updated = update_agent_runtime_config(
            &pool,
            &key,
            serde_json::json!({
                "workspace_host_path": "workspaces/agents/runtime-config-test",
                "workspace_container_path": "/workspaces/agents/runtime-config-test",
                "profile_source": "agent-runtime/workspace-template"
            }),
        )
        .await
        .expect("update runtime config");

        assert!(updated);

        let stored = get_agent(&pool, &key)
            .await
            .expect("get agent")
            .expect("agent present");
        assert_eq!(
            stored.runtime_config["workspace_container_path"],
            serde_json::json!("/workspaces/agents/runtime-config-test")
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

        // Use a different private key so the second row only conflicts on agent_key.
        let other_private_key =
            "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d";
        let mut second =
            sample_agent_with_private_key(&format!("{}-second", key), other_private_key);
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

        let before = get_agent(&pool, &key)
            .await
            .expect("fetch")
            .expect("present");
        assert!(before.api_key_last_used_at.is_none());

        touch_api_key_last_used(&pool, &row.api_key)
            .await
            .expect("touch");

        let after = get_agent(&pool, &key)
            .await
            .expect("fetch")
            .expect("present");
        assert!(after.api_key_last_used_at.is_some());
    }

    #[tokio::test]
    async fn new_agents_start_with_no_job_context_checkins() {
        let pool = test_db::pool().await;

        let key = format!("job-context-none-{}", Utc::now().timestamp_millis());
        let row = sample_agent(&key);
        insert_agent(&pool, &row).await.expect("insert agent");

        let agent = get_agent(&pool, &key)
            .await
            .expect("fetch")
            .expect("present");
        assert!(agent.analysis_context_last_used_at.is_none());
        assert!(agent.trading_context_last_used_at.is_none());
    }

    #[tokio::test]
    async fn touch_job_context_last_used_updates_only_analysis_timestamp() {
        let pool = test_db::pool().await;

        let key = format!("job-context-analysis-{}", Utc::now().timestamp_millis());
        let row = sample_agent(&key);
        insert_agent(&pool, &row).await.expect("insert agent");

        touch_job_context_last_used(&pool, &key, JobContextKind::Analysis)
            .await
            .expect("touch analysis");

        let agent = get_agent(&pool, &key)
            .await
            .expect("fetch")
            .expect("present");
        assert!(agent.analysis_context_last_used_at.is_some());
        assert!(agent.trading_context_last_used_at.is_none());
    }

    #[tokio::test]
    async fn touch_job_context_last_used_updates_only_trading_timestamp() {
        let pool = test_db::pool().await;

        let key = format!("job-context-trading-{}", Utc::now().timestamp_millis());
        let row = sample_agent(&key);
        insert_agent(&pool, &row).await.expect("insert agent");

        touch_job_context_last_used(&pool, &key, JobContextKind::Trading)
            .await
            .expect("touch trading");

        let agent = get_agent(&pool, &key)
            .await
            .expect("fetch")
            .expect("present");
        assert!(agent.analysis_context_last_used_at.is_none());
        assert!(agent.trading_context_last_used_at.is_some());
    }

    #[tokio::test]
    async fn update_agent_analysis_prompt_updates_only_analysis_and_timestamp() {
        let pool = test_db::pool().await;

        let key = format!("prompt-analysis-{}", Utc::now().timestamp_millis());
        let row = sample_agent(&key);
        insert_agent(&pool, &row).await.expect("insert agent");

        let original_trading = row.trading_prompt.clone();

        let updated = update_agent_analysis_prompt(&pool, &key, "New analysis prompt")
            .await
            .expect("update agent analysis prompt");
        assert!(updated);

        let agent = get_agent(&pool, &key)
            .await
            .expect("fetch")
            .expect("present");
        assert_eq!(agent.analysis_prompt, "New analysis prompt");
        assert_eq!(agent.trading_prompt, original_trading);
        assert!(agent.updated_at >= row.updated_at);
    }

    #[tokio::test]
    async fn update_agent_trading_prompt_updates_only_trading_and_timestamp() {
        let pool = test_db::pool().await;

        let key = format!("prompt-trading-{}", Utc::now().timestamp_millis());
        let row = sample_agent(&key);
        insert_agent(&pool, &row).await.expect("insert agent");

        let original_analysis = row.analysis_prompt.clone();

        let updated = update_agent_trading_prompt(&pool, &key, "New trading prompt")
            .await
            .expect("update agent trading prompt");
        assert!(updated);

        let agent = get_agent(&pool, &key)
            .await
            .expect("fetch")
            .expect("present");
        assert_eq!(agent.analysis_prompt, original_analysis);
        assert_eq!(agent.trading_prompt, "New trading prompt");
        assert!(agent.updated_at >= row.updated_at);
    }

    #[tokio::test]
    async fn update_agent_prompts_return_false_for_missing_agent() {
        let pool = test_db::pool().await;

        let analysis = update_agent_analysis_prompt(&pool, "does-not-exist", "a")
            .await
            .expect("update missing agent analysis");
        assert!(!analysis);

        let trading = update_agent_trading_prompt(&pool, "does-not-exist", "b")
            .await
            .expect("update missing agent trading");
        assert!(!trading);
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
        let wallet_address = agent.wallet_address.clone();
        let environment = agent.environment.clone();
        insert_agent(&pool, &agent).await.expect("insert agent");

        seed_instrument(&pool, "BTC", "perp", true).await;

        sqlx::query(
            "INSERT INTO hyperliquid.sync_state
                (account_address, environment, stream_name, status, metadata)
             VALUES ($1, $2, 'fills', 'healthy', '{}'::jsonb)",
        )
        .bind(&wallet_address)
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
        .bind(&wallet_address)
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
        .bind(&wallet_address)
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
        .bind(&wallet_address)
        .bind(&environment)
        .execute(&pool)
        .await
        .expect("insert ledger_event");

        sqlx::query(
            "INSERT INTO hyperliquid.historical_orders
                (account_address, environment, order_id, event_time, instrument_id, payload, ingest_source, inserted_at)
             VALUES ($1, $2, $3, now(), 'BTC', '{}'::jsonb, 'test', now())",
        )
        .bind(&wallet_address)
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
        .bind(&wallet_address)
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
        .bind(&wallet_address)
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
        .bind(&wallet_address)
        .bind(&environment)
        .fetch_one(&pool)
        .await
        .expect("count sync_state");
        let trade_fills_count: (i64,) = query_as(
            "SELECT COUNT(*) FROM hyperliquid.trade_fills WHERE account_address = $1 AND environment = $2",
        )
        .bind(&wallet_address)
        .bind(&environment)
        .fetch_one(&pool)
        .await
        .expect("count trade_fills");
        let funding_count: (i64,) = query_as(
            "SELECT COUNT(*) FROM hyperliquid.funding_events WHERE account_address = $1 AND environment = $2",
        )
        .bind(&wallet_address)
        .bind(&environment)
        .fetch_one(&pool)
        .await
        .expect("count funding_events");
        let ledger_count: (i64,) = query_as(
            "SELECT COUNT(*) FROM hyperliquid.ledger_events WHERE account_address = $1 AND environment = $2",
        )
        .bind(&wallet_address)
        .bind(&environment)
        .fetch_one(&pool)
        .await
        .expect("count ledger_events");
        let historical_count: (i64,) = query_as(
            "SELECT COUNT(*) FROM hyperliquid.historical_orders WHERE account_address = $1 AND environment = $2",
        )
        .bind(&wallet_address)
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
