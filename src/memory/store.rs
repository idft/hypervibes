use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sqlx::QueryBuilder;
use uuid::Uuid;

use crate::{
    db::DbPool,
    memory::model::{CreateMemory, MemoryListFilter, MemoryRecord},
};

const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 200;
const LATEST_CANDIDATE_LIMIT: i64 = 200;

fn clamp_limit(limit: Option<i64>) -> i64 {
    let raw = limit.unwrap_or(DEFAULT_LIMIT);
    if raw < 1 {
        DEFAULT_LIMIT
    } else if raw > MAX_LIMIT {
        MAX_LIMIT
    } else {
        raw
    }
}

/// Insert a new memory record and return the full row (including server-
/// generated `id` and `created_at`).
pub async fn insert_memory(
    pool: &DbPool,
    agent_key: &str,
    input: &CreateMemory,
) -> Result<MemoryRecord> {
    let id = Uuid::new_v4();
    let metadata = input.metadata_or_default();
    let timeframe = input.timeframe.as_deref();

    let row = sqlx::query_as::<_, MemoryRecord>(
        "INSERT INTO memory.records (
            id, agent_key, symbol, timeframe, memory_type, summary, content, metadata
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         RETURNING id, created_at, agent_key, symbol, timeframe, memory_type, summary, content, metadata",
    )
    .bind(id)
    .bind(agent_key)
    .bind(input.symbol.trim())
    .bind(timeframe)
    .bind(input.memory_type.trim())
    .bind(input.summary.trim())
    .bind(input.content.trim())
    .bind(&metadata)
    .fetch_one(pool)
    .await
    .context("failed to insert memory record")?;

    Ok(row)
}

/// List memories for the given agent, newest first.
///
/// Timeframe semantics (V1, see `docs/Memory.md`):
/// - If `filter.timeframe` is `Some(_)`, filter `timeframe = $x`.
/// - If `filter.timeframe` is `None`, no timeframe filter is applied;
///   rows with any timeframe value (including `NULL`) are returned.
pub async fn list_memories(
    pool: &DbPool,
    agent_key: &str,
    filter: &MemoryListFilter,
) -> Result<Vec<MemoryRecord>> {
    let limit = clamp_limit(filter.limit);

    let mut qb: QueryBuilder<sqlx::Postgres> = QueryBuilder::new(
        "SELECT id, created_at, agent_key, symbol, timeframe, memory_type, summary, content, metadata \
         FROM memory.records WHERE agent_key = ",
    );
    qb.push_bind(agent_key.to_string());

    if let Some(symbol) = &filter.symbol {
        qb.push(" AND symbol = ").push_bind(symbol.clone());
    }

    match &filter.timeframe {
        Some(value) => {
            qb.push(" AND timeframe = ").push_bind(value.clone());
        }
        None => {
            // Omitted timeframe means "any timeframe" (including NULL).
            // Per-timeframe grouping is the job of /api/v1/memories/latest.
        }
    }

    if let Some(memory_type) = &filter.memory_type {
        qb.push(" AND memory_type = ")
            .push_bind(memory_type.clone());
    }
    if let Some(since) = filter.since {
        qb.push(" AND created_at >= ").push_bind(since);
    }
    if let Some(until) = filter.until {
        qb.push(" AND created_at < ").push_bind(until);
    }

    qb.push(" ORDER BY timeframe NULLS LAST, created_at DESC LIMIT ").push_bind(limit);

    let rows = qb
        .build_query_as::<MemoryRecord>()
        .fetch_all(pool)
        .await
        .context("failed to list memory records")?;

    Ok(rows)
}

/// List all memories for the given agent, newest first.
pub async fn list_agent_memories(
    pool: &DbPool,
    agent_key: &str,
    since: Option<DateTime<Utc>>,
    until: Option<DateTime<Utc>>,
) -> Result<Vec<MemoryRecord>> {
    let mut qb: QueryBuilder<sqlx::Postgres> = QueryBuilder::new(
        "SELECT id, created_at, agent_key, symbol, timeframe, memory_type, summary, content, metadata \
         FROM memory.records WHERE agent_key = ",
    );
    qb.push_bind(agent_key.to_string());

    if let Some(since) = since {
        qb.push(" AND created_at >= ").push_bind(since);
    }
    if let Some(until) = until {
        qb.push(" AND created_at < ").push_bind(until);
    }

    qb.push(" ORDER BY created_at DESC");

    let rows = qb
        .build_query_as::<MemoryRecord>()
        .fetch_all(pool)
        .await
        .context("failed to list agent memory records")?;

    Ok(rows)
}

/// Fetch the latest memory for an agent and memory type.
pub async fn get_latest_agent_memory_by_type(
    pool: &DbPool,
    agent_key: &str,
    memory_type: &str,
) -> Result<Option<MemoryRecord>> {
    let row = sqlx::query_as::<_, MemoryRecord>(
        "SELECT id, created_at, agent_key, symbol, timeframe, memory_type, summary, content, metadata
           FROM memory.records
          WHERE agent_key = $1
            AND memory_type = $2
          ORDER BY created_at DESC
          LIMIT 1",
    )
    .bind(agent_key)
    .bind(memory_type)
    .fetch_optional(pool)
    .await
    .context("failed to fetch latest agent memory by type")?;

    Ok(row)
}

/// List newest-first candidates for the latest-per-timeframe endpoint.
pub async fn list_latest_memory_candidates(
    pool: &DbPool,
    agent_key: &str,
    symbol: &str,
    memory_type: &str,
) -> Result<Vec<MemoryRecord>> {
    let rows = sqlx::query_as::<_, MemoryRecord>(
        "SELECT id, created_at, agent_key, symbol, timeframe, memory_type, summary, content, metadata
           FROM memory.records
          WHERE agent_key = $1
            AND symbol = $2
            AND memory_type = $3
            AND timeframe IS NOT NULL
          ORDER BY created_at DESC, timeframe ASC, id DESC
           LIMIT $4",
    )
    .bind(agent_key)
    .bind(symbol)
    .bind(memory_type)
    .bind(LATEST_CANDIDATE_LIMIT)
    .fetch_all(pool)
    .await
    .context("failed to list latest memory candidates")?;

    Ok(rows)
}

/// Fetch a single memory by id, scoped to the caller's `agent_key`.
///
/// Returning `None` covers both "not found" and "not owned" — the caller
/// cannot distinguish, so existence does not leak across agents.
pub async fn get_memory(pool: &DbPool, agent_key: &str, id: Uuid) -> Result<Option<MemoryRecord>> {
    let row = sqlx::query_as::<_, MemoryRecord>(
        "SELECT id, created_at, agent_key, symbol, timeframe, memory_type, summary, content, metadata
           FROM memory.records
          WHERE id = $1 AND agent_key = $2",
    )
    .bind(id)
    .bind(agent_key)
    .fetch_optional(pool)
    .await
    .context("failed to fetch memory record")?;

    Ok(row)
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::{
        agents::{
            crypto::{EncryptionKey, encrypt},
            keys::derive_wallet_address,
            model::AgentRegistryRow,
            store::insert_agent,
        },
        db::{connect, migrate},
    };

    fn db_url() -> Option<String> {
        std::env::var("DATABASE_URL").ok()
    }

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

    fn sample_agent(key: &str) -> AgentRegistryRow {
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
            display_name: format!("Test {}", key),
            analysis_prompt: String::new(),
            trading_prompt: String::new(),
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

    async fn seed_agent(pool: &DbPool, key: &str) {
        let row = sample_agent(key);
        insert_agent(pool, &row).await.expect("insert agent");
    }

    fn create(symbol: &str, tf: Option<&str>, summary: &str) -> CreateMemory {
        CreateMemory {
            symbol: symbol.to_string(),
            timeframe: tf.map(str::to_string),
            memory_type: "observation".to_string(),
            summary: summary.to_string(),
            content: format!("body for {summary}"),
            metadata: Some(serde_json::json!({ "k": "v" })),
        }
    }

    #[tokio::test]
    async fn insert_and_read_back_round_trip() {
        let Some(database_url) = db_url() else {
            eprintln!("DATABASE_URL not set; skipping integration test");
            return;
        };

        let pool = connect(&database_url).await.expect("connect");
        migrate(&pool).await.expect("migrate");

        let key = format!("mem-rt-{}", Utc::now().timestamp_millis());
        seed_agent(&pool, &key).await;

        let inserted = insert_memory(&pool, &key, &create("BTC", Some("1h"), "first"))
            .await
            .expect("insert");
        assert_eq!(inserted.symbol, "BTC");
        assert_eq!(inserted.timeframe.as_deref(), Some("1h"));
        assert_eq!(inserted.summary, "first");
        assert_eq!(inserted.metadata, serde_json::json!({ "k": "v" }));

        // timeframe = None is allowed and round-trips as NULL.
        let general = insert_memory(
            &pool,
            &key,
            &CreateMemory {
                timeframe: None,
                ..create("BTC", None, "general")
            },
        )
        .await
        .expect("insert general");
        assert!(general.timeframe.is_none());

        let fetched = get_memory(&pool, &key, inserted.id)
            .await
            .expect("fetch")
            .expect("present");
        assert_eq!(fetched.id, inserted.id);
        assert_eq!(fetched.summary, "first");
    }

    #[tokio::test]
    async fn list_memories_filters_by_timeframe_value_and_null() {
        let Some(database_url) = db_url() else {
            eprintln!("DATABASE_URL not set; skipping integration test");
            return;
        };

        let pool = connect(&database_url).await.expect("connect");
        migrate(&pool).await.expect("migrate");

        let key = format!("mem-ls-{}", Utc::now().timestamp_millis());
        seed_agent(&pool, &key).await;

        insert_memory(&pool, &key, &create("BTC", Some("1h"), "h1"))
            .await
            .unwrap();
        insert_memory(&pool, &key, &create("BTC", Some("1h"), "h2"))
            .await
            .unwrap();
        insert_memory(&pool, &key, &create("BTC", Some("15m"), "m1"))
            .await
            .unwrap();
        insert_memory(
            &pool,
            &key,
            &CreateMemory {
                timeframe: None,
                ..create("BTC", None, "general")
            },
        )
        .await
        .unwrap();

        // Timeframe = Some("1h") => only the 1h rows, DESC.
        let rows = list_memories(
            &pool,
            &key,
            &MemoryListFilter {
                symbol: Some("BTC".into()),
                timeframe: Some("1h".into()),
                limit: Some(10),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.timeframe.as_deref() == Some("1h")));
        assert!(rows[0].created_at >= rows[1].created_at);

        // Timeframe = None => every timeframe (including NULL) in DESC order.
        let rows = list_memories(
            &pool,
            &key,
            &MemoryListFilter {
                symbol: Some("BTC".into()),
                timeframe: None,
                limit: Some(10),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(rows.len(), 4);
        assert!(
            rows.iter().any(|r| r.timeframe.is_none()),
            "NULL-timeframe row still in the set"
        );
    }

    #[tokio::test]
    async fn list_memories_filters_by_symbol_memory_type_and_window() {
        let Some(database_url) = db_url() else {
            eprintln!("DATABASE_URL not set; skipping integration test");
            return;
        };

        let pool = connect(&database_url).await.expect("connect");
        migrate(&pool).await.expect("migrate");

        let key = format!("mem-flt-{}", Utc::now().timestamp_millis());
        seed_agent(&pool, &key).await;

        insert_memory(
            &pool,
            &key,
            &CreateMemory {
                symbol: "BTC".into(),
                memory_type: "plan".into(),
                ..create("BTC", Some("1h"), "btc-plan")
            },
        )
        .await
        .unwrap();
        insert_memory(
            &pool,
            &key,
            &CreateMemory {
                symbol: "ETH".into(),
                memory_type: "plan".into(),
                ..create("ETH", Some("1h"), "eth-plan")
            },
        )
        .await
        .unwrap();
        insert_memory(
            &pool,
            &key,
            &CreateMemory {
                symbol: "BTC".into(),
                memory_type: "observation".into(),
                ..create("BTC", Some("1h"), "btc-obs")
            },
        )
        .await
        .unwrap();

        // symbol filter
        let rows = list_memories(
            &pool,
            &key,
            &MemoryListFilter {
                symbol: Some("BTC".into()),
                timeframe: Some("1h".into()),
                limit: Some(10),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(rows.iter().all(|r| r.symbol == "BTC"));
        assert_eq!(rows.len(), 2);

        // memory_type filter
        let rows = list_memories(
            &pool,
            &key,
            &MemoryListFilter {
                symbol: Some("BTC".into()),
                timeframe: Some("1h".into()),
                memory_type: Some("plan".into()),
                limit: Some(10),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].memory_type, "plan");

        // since/until window: a window that excludes all rows returns nothing.
        let rows = list_memories(
            &pool,
            &key,
            &MemoryListFilter {
                symbol: Some("BTC".into()),
                timeframe: Some("1h".into()),
                since: Some(Utc::now() + chrono::Duration::seconds(60)),
                limit: Some(10),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn list_memories_clamps_limit() {
        let Some(database_url) = db_url() else {
            eprintln!("DATABASE_URL not set; skipping integration test");
            return;
        };

        let pool = connect(&database_url).await.expect("connect");
        migrate(&pool).await.expect("migrate");

        let key = format!("mem-lim-{}", Utc::now().timestamp_millis());
        seed_agent(&pool, &key).await;

        for i in 0..5 {
            insert_memory(
                &pool,
                &key,
                &CreateMemory {
                    summary: format!("row {i}"),
                    ..create("BTC", Some("1h"), &format!("row {i}"))
                },
            )
            .await
            .unwrap();
        }

        // limit=0 must be clamped to the default (50), so all 5 come back.
        let rows = list_memories(
            &pool,
            &key,
            &MemoryListFilter {
                symbol: Some("BTC".into()),
                timeframe: Some("1h".into()),
                limit: Some(0),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(rows.len(), 5);

        // limit=99999 must be clamped to MAX (200) — still 5 here.
        let rows = list_memories(
            &pool,
            &key,
            &MemoryListFilter {
                symbol: Some("BTC".into()),
                timeframe: Some("1h".into()),
                limit: Some(99_999),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(rows.len(), 5);
    }

    #[tokio::test]
    async fn get_memory_returns_none_for_other_agent() {
        let Some(database_url) = db_url() else {
            eprintln!("DATABASE_URL not set; skipping integration test");
            return;
        };

        let pool = connect(&database_url).await.expect("connect");
        migrate(&pool).await.expect("migrate");

        let owner = format!("mem-own-{}", Utc::now().timestamp_millis());
        let other = format!("mem-oth-{}", Utc::now().timestamp_millis());
        seed_agent(&pool, &owner).await;
        seed_agent(&pool, &other).await;

        let row = insert_memory(&pool, &owner, &create("BTC", Some("1h"), "owned"))
            .await
            .unwrap();

        // Owner can fetch it.
        let fetched = get_memory(&pool, &owner, row.id).await.unwrap();
        assert!(fetched.is_some());

        // Other agent cannot.
        let fetched = get_memory(&pool, &other, row.id).await.unwrap();
        assert!(fetched.is_none());
    }
}
