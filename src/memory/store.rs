use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sqlx::{Postgres, QueryBuilder, Transaction};
use uuid::Uuid;

use crate::{
    db::DbPool,
    memory::model::{
        CreateMemory, MemoryLinkRecord, MemoryListFilter, MemoryRecord, MemoryTimelineRecord,
    },
};

const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 200;
const LATEST_CANDIDATE_LIMIT: i64 = 200;
pub const AGENT_MEMORY_TIMELINE_PAGE_SIZE: i64 = 50;

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
    let mut tx = pool
        .begin()
        .await
        .context("failed to begin memory insert tx")?;

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
    .fetch_one(&mut *tx)
    .await
    .context("failed to insert memory record")?;

    for link in input.links_or_empty() {
        insert_memory_link_in_tx(&mut tx, agent_key, id, &link).await?;
    }

    tx.commit()
        .await
        .context("failed to commit memory insert tx")?;

    Ok(row)
}

/// Delete a single memory, scoped to the owning `agent_key` (mirrors
/// `get_memory` scoping so existence does not leak across agents).
/// Returns `true` when a row was deleted. Linked rows in `memory.links`
/// are removed via `ON DELETE CASCADE`.
pub async fn delete_memory(pool: &DbPool, agent_key: &str, id: Uuid) -> Result<bool> {
    let result = sqlx::query("DELETE FROM memory.records WHERE id = $1 AND agent_key = $2")
        .bind(id)
        .bind(agent_key)
        .execute(pool)
        .await
        .with_context(|| format!("failed to delete memory record {id}"))?;

    Ok(result.rows_affected() > 0)
}

pub async fn delete_memories_for_agent(pool: &DbPool, agent_key: &str) -> Result<u64> {
    let result = sqlx::query("DELETE FROM memory.records WHERE agent_key = $1")
        .bind(agent_key)
        .execute(pool)
        .await
        .with_context(|| format!("failed to delete memory records for agent {agent_key}"))?;

    Ok(result.rows_affected())
}

async fn insert_memory_link_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    agent_key: &str,
    source_memory_id: Uuid,
    link: &crate::memory::model::CreateMemoryLink,
) -> Result<()> {
    let owned: (bool,) = sqlx::query_as(
        "SELECT EXISTS (
             SELECT 1 FROM memory.records WHERE id = $1 AND agent_key = $2
         )",
    )
    .bind(link.target_memory_id)
    .bind(agent_key)
    .fetch_one(&mut **tx)
    .await
    .context("failed to validate memory link ownership")?;
    if !owned.0 {
        anyhow::bail!("memory link target is not owned by agent");
    }
    let metadata = link
        .metadata
        .clone()
        .unwrap_or_else(|| serde_json::json!({}));
    sqlx::query(
        "INSERT INTO memory.links (
            agent_key,
            source_memory_id,
            target_memory_id,
            link_type,
            metadata
         ) VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (source_memory_id, target_memory_id, link_type) DO NOTHING",
    )
    .bind(agent_key)
    .bind(source_memory_id)
    .bind(link.target_memory_id)
    .bind(link.link_type.trim())
    .bind(metadata)
    .execute(&mut **tx)
    .await
    .with_context(|| {
        format!(
            "failed to insert memory link from {source_memory_id} to {}",
            link.target_memory_id
        )
    })?;
    Ok(())
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

    qb.push(" ORDER BY timeframe NULLS LAST, created_at DESC LIMIT ")
        .push_bind(limit);

    let rows = qb
        .build_query_as::<MemoryRecord>()
        .fetch_all(pool)
        .await
        .context("failed to list memory records")?;

    Ok(rows)
}

/// List one page of lightweight timeline rows for the operator UI, newest first.
pub async fn list_agent_memory_timeline(
    pool: &DbPool,
    agent_key: &str,
    since: Option<DateTime<Utc>>,
    until: Option<DateTime<Utc>>,
    before: Option<(DateTime<Utc>, Uuid)>,
) -> Result<Vec<MemoryTimelineRecord>> {
    let mut qb: QueryBuilder<sqlx::Postgres> = QueryBuilder::new(
        "SELECT id, created_at, symbol, timeframe, memory_type, summary \
         FROM memory.records WHERE agent_key = ",
    );
    qb.push_bind(agent_key.to_string());

    if let Some(since) = since {
        qb.push(" AND created_at >= ").push_bind(since);
    }
    if let Some(until) = until {
        qb.push(" AND created_at < ").push_bind(until);
    }
    if let Some((created_at, id)) = before {
        qb.push(" AND (created_at, id) < (")
            .push_bind(created_at)
            .push(", ")
            .push_bind(id)
            .push(")");
    }

    qb.push(" ORDER BY created_at DESC, id DESC LIMIT ")
        .push_bind(AGENT_MEMORY_TIMELINE_PAGE_SIZE + 1);

    let rows = qb
        .build_query_as::<MemoryTimelineRecord>()
        .fetch_all(pool)
        .await
        .context("failed to list agent memory timeline")?;

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

pub async fn get_daily_review_memory_for_run(
    pool: &DbPool,
    agent_key: &str,
    run_id: i64,
) -> Result<Option<MemoryRecord>> {
    let rows = sqlx::query_as::<_, MemoryRecord>(
        "SELECT id, created_at, agent_key, symbol, timeframe, memory_type,
                summary, content, metadata
           FROM memory.records
          WHERE agent_key = $1
            AND symbol = '__agent__'
            AND memory_type = 'daily_review'
            AND metadata->>'source_agentic_run_id' = $2
          ORDER BY created_at DESC, id DESC
          LIMIT 2",
    )
    .bind(agent_key)
    .bind(run_id.to_string())
    .fetch_all(pool)
    .await
    .context("failed to fetch daily review memory for run")?;
    if rows.len() > 1 {
        anyhow::bail!("multiple daily review memories match agentic run {run_id}");
    }
    Ok(rows.into_iter().next())
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

pub async fn list_memory_links_from(
    pool: &DbPool,
    agent_key: &str,
    source_memory_id: Uuid,
) -> Result<Vec<MemoryLinkRecord>> {
    sqlx::query_as::<_, MemoryLinkRecord>(
        "SELECT id, agent_key, source_memory_id, target_memory_id, link_type, metadata, created_at
           FROM memory.links
          WHERE agent_key = $1
            AND source_memory_id = $2
          ORDER BY created_at ASC, id ASC",
    )
    .bind(agent_key)
    .bind(source_memory_id)
    .fetch_all(pool)
    .await
    .context("failed to list outgoing memory links")
}

pub async fn list_memory_links_to(
    pool: &DbPool,
    agent_key: &str,
    target_memory_id: Uuid,
) -> Result<Vec<MemoryLinkRecord>> {
    sqlx::query_as::<_, MemoryLinkRecord>(
        "SELECT id, agent_key, source_memory_id, target_memory_id, link_type, metadata, created_at
           FROM memory.links
          WHERE agent_key = $1
            AND target_memory_id = $2
          ORDER BY created_at ASC, id ASC",
    )
    .bind(agent_key)
    .bind(target_memory_id)
    .fetch_all(pool)
    .await
    .context("failed to list incoming memory links")
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::{
        agents::{keys::derive_wallet_address, model::AgentRegistryRow, store::insert_agent},
        db::{connect, migrate},
    };

    fn db_url() -> Option<String> {
        std::env::var("DATABASE_URL").ok()
    }

    fn deterministic_private_key(key: &str) -> String {
        use rand::rngs::StdRng;
        use rand::{RngExt, SeedableRng};

        let seed = key
            .bytes()
            .fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
        let mut rng = StdRng::seed_from_u64(seed);
        let bytes: [u8; 32] = rng.random();
        format!("0x{}", hex::encode(bytes))
    }

    fn sample_agent(key: &str) -> AgentRegistryRow {
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
            display_name: format!("Test {}", key),
            trading_account_address: Some(wallet),
            environment: "live".to_string(),
            api_key: format!("vta_{}", key),
            api_key_last_used_at: None,
            runtime_config: serde_json::json!({}),
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
            links: None,
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
