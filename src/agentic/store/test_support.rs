use chrono::Utc;
use sqlx::{query, query_as};

use crate::{
    agents::{
        crypto::{EncryptionKey, encrypt},
        keys::derive_wallet_address,
        model::{AgentRegistryRow, BACKEND_KIND_OPENCODE},
        store::insert_agent,
    },
    test_db,
};
use crate::db::DbPool;

pub fn deterministic_private_key(key: &str) -> String {
    use rand::rngs::StdRng;
    use rand::{Rng, SeedableRng};

    let seed = key
        .bytes()
        .fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
    let mut rng = StdRng::seed_from_u64(seed);
    let bytes: [u8; 32] = rng.r#gen();
    format!("0x{}", hex::encode(bytes))
}

pub fn sample_agent(key: &str) -> AgentRegistryRow {
    let enc = EncryptionKey::new(
        "test",
        [
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22,
            23, 24, 25, 26, 27, 28, 29, 30, 31,
        ],
    );
    let private_key = deterministic_private_key(key);
    let ciphertext = encrypt(&enc, &private_key).unwrap();
    let wallet = derive_wallet_address(&private_key).unwrap();
    let now = Utc::now();

    AgentRegistryRow {
        agent_key: key.to_string(),
        created_at: now,
        updated_at: now,
        enabled: true,
        display_name: format!("Test {key}"),
        analysis_prompt: String::new(),
        trading_prompt: String::new(),
        wallet_address: wallet,
        environment: "live".to_string(),
        api_key: format!("vta_{key}"),
        api_key_last_used_at: None,
        backend_kind: BACKEND_KIND_OPENCODE.to_string(),
        runtime_id: "opencode-local".to_string(),
        runtime_config: serde_json::json!({}),
        hyperliquid_private_key_ciphertext: ciphertext,
        hyperliquid_private_key_key_id: "test".to_string(),
    }
}

pub async fn seed_agent_and_schedule(pool: &DbPool, key: &str, schedule_id_offset: i64) -> i64 {
    use super::schedules::default_analysis_job_key;
    use super::insert_default_opencode_schedules;

    insert_agent(pool, &sample_agent(key))
        .await
        .expect("insert agent");
    insert_default_opencode_schedules(pool, key)
        .await
        .expect("insert default schedules");

    query(
        "UPDATE agentic_job_schedules
            SET enabled = true
          WHERE agent_key = $1
            AND job_key = $2",
    )
    .bind(key)
    .bind(default_analysis_job_key())
    .execute(pool)
    .await
    .expect("enable seeded analysis schedule");

    if schedule_id_offset > 0 {
        query(
            "UPDATE agentic_job_schedules
                SET next_run_at = $2
              WHERE id = $1",
        )
        .bind(schedule_id_offset)
        .bind(Utc::now() + chrono::Duration::seconds(60))
        .execute(pool)
        .await
        .expect("update offset schedule");
    }

    let (id,): (i64,) = query_as(
        "SELECT id FROM agentic_job_schedules
          WHERE agent_key = $1
            AND job_key = $2",
    )
    .bind(key)
    .bind(default_analysis_job_key())
    .fetch_one(pool)
    .await
    .expect("fetch schedule id");
    id
}

pub async fn insert_test_opencode_session(pool: &DbPool, session_id: &str, status: &str) {
    query("INSERT INTO opencode.sessions (id, status, updated_at) VALUES ($1, $2, now())")
        .bind(session_id)
        .bind(status)
        .execute(pool)
        .await
        .expect("insert opencode session");
}

pub async fn insert_test_opencode_command(pool: &DbPool, session_id: &str) {
    query(
        "INSERT INTO opencode.commands (session_id, command_name, command_args) VALUES ($1, 'vibetrading-trading', '')",
    )
    .bind(session_id)
    .execute(pool)
    .await
    .expect("insert opencode command");
}