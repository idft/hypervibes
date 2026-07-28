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

pub async fn seed_agent_and_job(pool: &DbPool, key: &str, job_id_offset: i64) -> i64 {
    use super::insert_default_harness_jobs;
    use super::jobs::default_analysis_job_key;

    insert_agent(pool, &sample_agent(key))
        .await
        .expect("insert agent");
    insert_default_harness_jobs(pool, key)
        .await
        .expect("insert default jobs");

    query(
        "UPDATE harness_jobs
            SET enabled = true
          WHERE agent_key = $1
            AND job_key = $2",
    )
    .bind(key)
    .bind(default_analysis_job_key())
    .execute(pool)
    .await
    .expect("enable seeded analysis job");

    if job_id_offset > 0 {
        query(
            "UPDATE harness_jobs
                SET next_run_at = $2
              WHERE id = $1",
        )
        .bind(job_id_offset)
        .bind(Utc::now() + chrono::Duration::seconds(60))
        .execute(pool)
        .await
        .expect("update offset job");
    }

    let (id,): (i64,) = query_as(
        "SELECT id FROM harness_jobs
          WHERE agent_key = $1
            AND job_key = $2",
    )
    .bind(key)
    .bind(default_analysis_job_key())
    .fetch_one(pool)
    .await
    .expect("fetch job id");
    id
}
