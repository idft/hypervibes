use chrono::Utc;
use sqlx::{query, query_as};

use crate::agents::{keys::derive_wallet_address, model::AgentRegistryRow, store::insert_agent};
use crate::db::DbPool;

pub fn deterministic_private_key(key: &str) -> String {
    use rand::rngs::StdRng;
    use rand::{RngExt, SeedableRng};

    let seed = key
        .bytes()
        .fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
    let mut rng = StdRng::seed_from_u64(seed);
    let bytes: [u8; 32] = rng.random();
    format!("0x{}", hex::encode(bytes))
}

pub fn sample_agent(key: &str) -> AgentRegistryRow {
    let private_key = deterministic_private_key(key);
    let wallet = derive_wallet_address(&private_key).unwrap();
    let now = Utc::now();

    AgentRegistryRow {
        agent_key: key.to_string(),
        user_id: crate::test_db::test_user_id(),
        created_at: now,
        updated_at: now,
        enabled: true,
        lifecycle: crate::agents::model::AGENT_LIFECYCLE_ACTIVE.to_string(),
        display_name: format!("Test {key}"),
        trading_account_address: Some(wallet),
        environment: "live".to_string(),
        api_key: format!("vta_{key}"),
        api_key_last_used_at: None,
        runtime_config: serde_json::json!({}),
    }
}

pub async fn seed_agent_and_job(pool: &DbPool, key: &str, sub_agent_id_offset: i64) -> i64 {
    use super::insert_default_harness_sub_agents;

    insert_agent(pool, &sample_agent(key))
        .await
        .expect("insert agent");
    insert_default_harness_sub_agents(pool, key)
        .await
        .expect("insert default jobs");

    query(
        "UPDATE harness_sub_agents
            SET enabled = true,
                model_provider_id = 'anthropic',
                model_id = 'claude-sonnet-test'
           WHERE agent_key = $1
             AND sub_agent_key = $2",
    )
    .bind(key)
    .bind("technical-15m")
    .execute(pool)
    .await
    .expect("enable seeded analysis job");

    if sub_agent_id_offset > 0 {
        query(
            "UPDATE harness_sub_agents
                SET next_run_at = $2
              WHERE id = $1",
        )
        .bind(sub_agent_id_offset)
        .bind(Utc::now() + chrono::Duration::seconds(60))
        .execute(pool)
        .await
        .expect("update offset job");
    }

    let (id,): (i64,) = query_as(
        "SELECT id FROM harness_sub_agents
          WHERE agent_key = $1
            AND sub_agent_key = $2",
    )
    .bind(key)
    .bind("technical-15m")
    .fetch_one(pool)
    .await
    .expect("fetch job id");
    id
}
