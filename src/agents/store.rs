use anyhow::{Context, Result};
use sqlx::query_as;

use crate::{
    agents::model::{AgentDetailRow, AgentListRow, AgentRegistryRow},
    db::DbPool,
};

/// List all agents ordered by creation time, newest first.
pub async fn list_agents(pool: &DbPool) -> Result<Vec<AgentListRow>> {
    let rows = query_as::<_, AgentListRow>(
        "SELECT display_name,
                agent_key,
                enabled,
                wallet_address,
                api_key,
                api_key_last_used_at
           FROM agents.registry
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
                prompt,
                wallet_address,
                api_key,
                api_key_last_used_at,
                created_at,
                updated_at
           FROM agents.registry
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
        "INSERT INTO agents.registry (
            agent_key,
            created_at,
            updated_at,
            enabled,
            display_name,
            prompt,
            wallet_address,
            api_key,
            api_key_last_used_at,
            hyperliquid_private_key_ciphertext,
            hyperliquid_private_key_key_id
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(&row.agent_key)
    .bind(row.created_at)
    .bind(row.updated_at)
    .bind(row.enabled)
    .bind(&row.display_name)
    .bind(&row.prompt)
    .bind(&row.wallet_address)
    .bind(&row.api_key)
    .bind(row.api_key_last_used_at)
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
    let result = sqlx::query("DELETE FROM agents.registry WHERE agent_key = $1")
        .bind(agent_key)
        .execute(pool)
        .await
        .context("failed to delete agent")?;

    Ok(result.rows_affected() > 0)
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
        db::{connect, migrate},
    };

    fn db_url() -> Option<String> {
        std::env::var("DATABASE_URL").ok()
    }

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
            prompt: "Test prompt".to_string(),
            wallet_address: wallet,
            api_key: format!("vta_{}", key),
            api_key_last_used_at: None,
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
        let Some(database_url) = db_url() else {
            eprintln!("DATABASE_URL not set; skipping integration test");
            return;
        };

        let pool = connect(&database_url).await.expect("connect to database");
        migrate(&pool).await.expect("run migrations");

        let key = format!("list-test-{}", Utc::now().timestamp_millis());
        let row = sample_agent(&key);
        insert_agent(&pool, &row).await.expect("insert agent");

        let agents = list_agents(&pool).await.expect("list agents");
        assert!(agents.iter().any(|a| a.agent_key == key));
    }

    #[tokio::test]
    async fn delete_agent_removes_row() {
        let Some(database_url) = db_url() else {
            eprintln!("DATABASE_URL not set; skipping integration test");
            return;
        };

        let pool = connect(&database_url).await.expect("connect to database");
        migrate(&pool).await.expect("run migrations");

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
        let Some(database_url) = db_url() else {
            eprintln!("DATABASE_URL not set; skipping integration test");
            return;
        };

        let pool = connect(&database_url).await.expect("connect to database");
        migrate(&pool).await.expect("run migrations");

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
            matches!(db_err, Some(Some(ref c)) if c.contains("agent_key") || c.contains("registry_pkey")),
            "expected unique violation on agent_key, got {:?}",
            db_err
        );
    }
}
