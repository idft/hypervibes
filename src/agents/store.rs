use anyhow::{Context, Result};
use sqlx::query_as;

use crate::{
    agents::model::{AgentDetailRow, AgentListRow, AgentRegistryRow},
    db::DbPool,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobContextKind {
    Analysis,
    Trading,
}

/// List all agents ordered by creation time, newest first.
pub async fn list_agents(pool: &DbPool) -> Result<Vec<AgentListRow>> {
    let rows = query_as::<_, AgentListRow>(
        "SELECT display_name,
                agent_key,
                enabled,
                wallet_address,
                environment,
                api_key,
                api_key_last_used_at
           FROM agents
          ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await
    .context("failed to list agents")?;

    Ok(rows)
}

/// Fetch a single agent by its unique agent key.
pub async fn get_agent(pool: &DbPool, agent_key: &str) -> Result<Option<AgentDetailRow>> {
    let row = query_as::<_, AgentDetailRow>(
        "SELECT display_name,
                agent_key,
                enabled,
                analysis_prompt,
                trading_prompt,
                wallet_address,
                environment,
                api_key,
                api_key_last_used_at,
                analysis_context_last_used_at,
                trading_context_last_used_at,
                created_at,
                updated_at
           FROM agents
          WHERE agent_key = $1",
    )
    .bind(agent_key)
    .fetch_optional(pool)
    .await
    .context("failed to fetch agent")?;

    Ok(row)
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
            analysis_context_last_used_at,
            trading_context_last_used_at,
            hyperliquid_private_key_ciphertext,
            hyperliquid_private_key_key_id
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)",
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
    let result = sqlx::query("DELETE FROM agents WHERE agent_key = $1")
        .bind(agent_key)
        .execute(pool)
        .await
        .context("failed to delete agent")?;

    Ok(result.rows_affected() > 0)
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

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
}
