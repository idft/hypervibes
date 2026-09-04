use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sqlx::{Postgres, QueryBuilder, Transaction};
use uuid::Uuid;

use crate::{
    db::DbPool,
    memory::model::{
        CreateMemory, MEMORY_SCOPE_INSTRUMENTS, MemoryLinkRecord, MemoryListFilter, MemoryRecord,
        MemoryTimelineRecord,
    },
};

const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 200;
pub const AGENT_MEMORY_TIMELINE_PAGE_SIZE: i64 = 50;

/// Provenance stamped by the server from an authenticated run credential.
#[derive(Debug, Clone, Copy)]
pub struct MemorySourceRun {
    pub run_id: i64,
}

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
/// generated `id` and `created_at`). Instrument targets are validated against
/// the canonical instrument catalog and the owner agent's current selection
/// in the same transaction.
pub async fn insert_memory(
    pool: &DbPool,
    agent_key: &str,
    input: &CreateMemory,
    source_run: Option<MemorySourceRun>,
) -> Result<MemoryRecord> {
    input
        .validate()
        .map_err(|errors| anyhow::anyhow!(errors.join(" ")))?;
    let mut tx = pool
        .begin()
        .await
        .context("failed to begin memory insert tx")?;
    let row = insert_memory_in_tx(&mut tx, agent_key, input, source_run).await?;
    tx.commit()
        .await
        .context("failed to commit memory insert tx")?;
    Ok(row)
}

pub(crate) async fn insert_memory_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    agent_key: &str,
    input: &CreateMemory,
    source_run: Option<MemorySourceRun>,
) -> Result<MemoryRecord> {
    let id = Uuid::new_v4();
    let metadata = input.metadata_or_default();
    let timeframe = input.timeframe.as_deref();
    let source_run_id = source_run.map(|source| source.run_id);

    let row = sqlx::query_as::<_, MemoryRecord>(
        "INSERT INTO memory.records (
            id, agent_key, scope_kind, timeframe, memory_type, summary, content, metadata,
            source_run_id
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         RETURNING id, created_at, agent_key, scope_kind, timeframe, memory_type, summary, content, metadata, source_run_id",
    )
    .bind(id)
    .bind(agent_key)
    .bind(input.scope_kind.trim())
    .bind(timeframe)
    .bind(input.memory_type.trim())
    .bind(input.summary.trim())
    .bind(input.content.trim())
    .bind(&metadata)
    .bind(source_run_id)
    .fetch_one(&mut **tx)
    .await
    .context("failed to insert memory record")?;

    if input.scope_kind == MEMORY_SCOPE_INSTRUMENTS {
        for instrument_id in &input.instrument_ids {
            let selected: bool = sqlx::query_scalar(
                "SELECT EXISTS (
                     SELECT 1
                       FROM agent_instruments AS selection
                       JOIN hyperliquid.instruments AS instruments
                         ON instruments.instrument_id = selection.instrument_id
                      WHERE selection.agent_key = $1
                        AND selection.instrument_id = $2
                        AND instruments.active = true
                 )",
            )
            .bind(agent_key)
            .bind(instrument_id.trim())
            .fetch_one(&mut **tx)
            .await
            .context("failed to validate memory instrument target")?;
            anyhow::ensure!(
                selected,
                "instrument target {instrument_id} is not an active instrument selected by the owner agent"
            );
            sqlx::query(
                "INSERT INTO memory.instrument_targets (memory_id, agent_key, instrument_id)
                 VALUES ($1, $2, $3)
                 ON CONFLICT (memory_id, instrument_id) DO NOTHING",
            )
            .bind(id)
            .bind(agent_key)
            .bind(instrument_id.trim())
            .execute(&mut **tx)
            .await
            .with_context(|| {
                format!("failed to insert memory instrument target {instrument_id}")
            })?;
        }
    }

    for link in input.links_or_empty() {
        insert_memory_link_in_tx(tx, agent_key, id, &link).await?;
    }

    Ok(row)
}

/// Delete a single memory, scoped to the owning `agent_key` (mirrors
/// `get_memory` scoping so existence does not leak across agents).
/// Returns `true` when a row was deleted. Linked rows in `memory.links`
/// and instrument targets are removed via `ON DELETE CASCADE`.
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
pub async fn list_memories(
    pool: &DbPool,
    agent_key: &str,
    filter: &MemoryListFilter,
) -> Result<Vec<MemoryRecord>> {
    let limit = clamp_limit(filter.limit);

    let mut qb: QueryBuilder<sqlx::Postgres> = QueryBuilder::new(
        "SELECT records.id, records.created_at, records.agent_key, records.scope_kind, \
         records.timeframe, records.memory_type, records.summary, records.content, records.metadata, records.source_run_id \
         FROM memory.records AS records WHERE records.agent_key = ",
    );
    qb.push_bind(agent_key.to_string());

    if let Some(scope_kind) = &filter.scope_kind {
        qb.push(" AND records.scope_kind = ")
            .push_bind(scope_kind.clone());
    }

    if let Some(instrument_id) = &filter.instrument_id {
        qb.push(
            " AND records.scope_kind = 'instruments' AND EXISTS (SELECT 1 FROM memory.instrument_targets AS targets WHERE targets.memory_id = records.id AND targets.instrument_id = ",
        )
        .push_bind(instrument_id.clone())
        .push(")");
    }

    match &filter.timeframe {
        Some(value) => {
            qb.push(" AND records.timeframe = ")
                .push_bind(value.clone());
        }
        None => {
            // Omitted timeframe means "any timeframe" (including NULL).
        }
    }

    if let Some(memory_type) = &filter.memory_type {
        qb.push(" AND records.memory_type = ")
            .push_bind(memory_type.clone());
    }
    if let Some(since) = filter.since {
        qb.push(" AND records.created_at >= ").push_bind(since);
    }
    if let Some(until) = filter.until {
        qb.push(" AND records.created_at < ").push_bind(until);
    }

    qb.push(" ORDER BY records.created_at DESC, records.id DESC LIMIT ")
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
        "SELECT id, created_at, scope_kind, timeframe, memory_type, summary \
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
        "SELECT id, created_at, agent_key, scope_kind, timeframe, memory_type, summary, content, metadata, source_run_id
           FROM memory.records
          WHERE agent_key = $1
            AND memory_type = $2
          ORDER BY created_at DESC, id DESC
          LIMIT 1",
    )
    .bind(agent_key)
    .bind(memory_type)
    .fetch_optional(pool)
    .await
    .context("failed to fetch latest agent memory by type")?;

    Ok(row)
}

/// Latest `trading_decision` memory overall, newest first.
pub async fn get_latest_trading_decision(
    pool: &DbPool,
    agent_key: &str,
) -> Result<Option<MemoryRecord>> {
    get_latest_agent_memory_by_type(pool, agent_key, "trading_decision").await
}

pub async fn get_review_memory_for_run(
    pool: &DbPool,
    agent_key: &str,
    run_id: i64,
) -> Result<Option<MemoryRecord>> {
    let rows = sqlx::query_as::<_, MemoryRecord>(
        "SELECT id, created_at, agent_key, scope_kind, timeframe, memory_type,
                summary, content, metadata, source_run_id
           FROM memory.records
          WHERE agent_key = $1
            AND scope_kind = 'agent'
            AND memory_type = 'review'
            AND source_run_id = $2
          ORDER BY created_at DESC, id DESC
          LIMIT 2",
    )
    .bind(agent_key)
    .bind(run_id)
    .fetch_all(pool)
    .await
    .context("failed to fetch review memory for run")?;
    if rows.len() > 1 {
        anyhow::bail!("multiple review memories match harness run {run_id}");
    }
    Ok(rows.into_iter().next())
}

pub async fn list_memory_instrument_targets(
    pool: &DbPool,
    agent_key: &str,
    memory_id: Uuid,
) -> Result<Vec<String>> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT instrument_id FROM memory.instrument_targets WHERE agent_key = $1 AND memory_id = $2 ORDER BY instrument_id",
    )
    .bind(agent_key)
    .bind(memory_id)
    .fetch_all(pool)
    .await
    .with_context(|| format!("failed to list instrument targets for memory {memory_id}"))?;
    Ok(rows.into_iter().map(|row| row.0).collect())
}

pub async fn list_latest_memory_candidates(
    pool: &DbPool,
    agent_key: &str,
    instrument_id: &str,
    memory_type: &str,
) -> Result<Vec<MemoryRecord>> {
    let rows = sqlx::query_as::<_, MemoryRecord>(
        "SELECT records.id, records.created_at, records.agent_key, records.scope_kind, records.timeframe, records.memory_type, records.summary, records.content, records.metadata, records.source_run_id
           FROM memory.records AS records
          WHERE records.agent_key = $1
            AND records.memory_type = $3
            AND records.scope_kind = 'instruments'
            AND EXISTS (
                SELECT 1 FROM memory.instrument_targets AS targets
                 WHERE targets.memory_id = records.id AND targets.instrument_id = $2
            )
            AND records.timeframe IS NOT NULL
          ORDER BY records.created_at DESC, records.id DESC
           LIMIT $4",
    )
    .bind(agent_key)
    .bind(instrument_id)
    .bind(memory_type)
    .bind(MAX_LIMIT)
    .fetch_all(pool)
    .await
    .context("failed to list latest memory candidates")?;

    Ok(rows)
}

/// One fresh research output selected per `(analysis producer, memory_type,
/// scope)` for the Trading context query. Includes agent-scoped records and
/// records targeting the requested instrument.
pub async fn get_trading_context_evidence(
    pool: &DbPool,
    agent_key: &str,
    instrument_id: &str,
    now: DateTime<Utc>,
) -> Result<Vec<TradingContextEvidence>> {
    let rows: Vec<TradingContextEvidenceRow> = sqlx::query_as(
        "SELECT DISTINCT ON (runs.sub_agent_id, records.memory_type, records.scope_kind)
                records.id, records.created_at, records.memory_type, records.summary,
                records.content, records.metadata, records.scope_kind, records.timeframe,
                records.source_run_id,
                runs.sub_agent_key AS producer_sub_agent_key,
                jobs.enabled AS producer_enabled
           FROM memory.records AS records
           LEFT JOIN harness_sub_agent_runs AS runs
             ON runs.id = records.source_run_id AND runs.agent_key = records.agent_key
           LEFT JOIN harness_sub_agents AS jobs
              ON jobs.id = runs.sub_agent_id AND jobs.agent_key = runs.agent_key
           WHERE records.agent_key = $1
             AND runs.sub_agent_kind = 'analysis'
            AND records.memory_type NOT IN ('trading_decision', 'review', 'agent_learnings')
            AND (
                records.scope_kind = 'agent'
                OR EXISTS (
                    SELECT 1 FROM memory.instrument_targets AS targets
                     WHERE targets.memory_id = records.id AND targets.instrument_id = $2
                )
            )
           ORDER BY runs.sub_agent_id, records.memory_type, records.scope_kind,
                    records.created_at DESC, records.id DESC",
    )
    .bind(agent_key)
    .bind(instrument_id)
    .fetch_all(pool)
    .await
    .context("failed to load trading context evidence")?;

    let mut evidence = Vec::new();
    for row in rows {
        let expires_at = crate::memory::memory_expires_at(&MemoryRecord {
            id: row.id,
            created_at: row.created_at,
            agent_key: agent_key.to_string(),
            scope_kind: row.scope_kind.clone(),
            timeframe: row.timeframe.clone(),
            memory_type: row.memory_type.clone(),
            summary: row.summary.clone(),
            content: row.content.clone(),
            metadata: row.metadata.clone(),
            source_run_id: row.source_run_id,
        });
        let targets = if row.scope_kind == MEMORY_SCOPE_INSTRUMENTS {
            list_memory_instrument_targets(pool, agent_key, row.id).await?
        } else {
            Vec::new()
        };
        let producer_enabled = row.producer_enabled;
        let status = match expires_at {
            Some(expires_at) if expires_at <= now => EvidenceStatus::Stale,
            _ if producer_enabled == Some(false) => EvidenceStatus::Disabled,
            _ => EvidenceStatus::Fresh,
        };
        evidence.push(TradingContextEvidence {
            memory_id: row.id,
            created_at: row.created_at,
            memory_type: row.memory_type,
            producer_sub_agent_key: row.producer_sub_agent_key,
            scope_kind: row.scope_kind,
            instrument_targets: targets,
            timeframe: row.timeframe,
            summary: row.summary,
            content: row.content,
            metadata: row.metadata,
            expires_at,
            status,
        });
    }
    Ok(evidence)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum EvidenceStatus {
    Fresh,
    Stale,
    Disabled,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct TradingContextEvidenceRow {
    id: Uuid,
    created_at: DateTime<Utc>,
    memory_type: String,
    summary: String,
    content: String,
    metadata: serde_json::Value,
    scope_kind: String,
    timeframe: Option<String>,
    source_run_id: Option<i64>,
    producer_sub_agent_key: Option<String>,
    producer_enabled: Option<bool>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TradingContextEvidence {
    pub memory_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub memory_type: String,
    pub producer_sub_agent_key: Option<String>,
    pub scope_kind: String,
    pub instrument_targets: Vec<String>,
    pub timeframe: Option<String>,
    pub summary: String,
    pub content: String,
    pub metadata: serde_json::Value,
    pub expires_at: Option<DateTime<Utc>>,
    pub status: EvidenceStatus,
}

/// Fetch a single memory by id, scoped to the caller's `agent_key`.
///
/// Returning `None` covers both "not found" and "not owned" — the caller
/// cannot distinguish, so existence does not leak across agents.
pub async fn get_memory(pool: &DbPool, agent_key: &str, id: Uuid) -> Result<Option<MemoryRecord>> {
    let row = sqlx::query_as::<_, MemoryRecord>(
        "SELECT id, created_at, agent_key, scope_kind, timeframe, memory_type, summary, content, metadata, source_run_id
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
    use crate::memory::model::MEMORY_SCOPE_AGENT;
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

    fn create(scope_kind: &str, instrument_ids: Vec<&str>, summary: &str) -> CreateMemory {
        CreateMemory {
            scope_kind: scope_kind.to_string(),
            instrument_ids: instrument_ids.into_iter().map(str::to_string).collect(),
            timeframe: None,
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
        crate::harness::store::insert_default_harness_sub_agents(&pool, &key)
            .await
            .expect("insert default jobs");

        let inserted = insert_memory(
            &pool,
            &key,
            &create(MEMORY_SCOPE_AGENT, Vec::new(), "first"),
            None,
        )
        .await
        .expect("insert");
        assert_eq!(inserted.scope_kind, MEMORY_SCOPE_AGENT);
        assert_eq!(inserted.memory_type, "observation");
        assert_eq!(inserted.summary, "first");
        assert_eq!(inserted.metadata, serde_json::json!({ "k": "v" }));

        let fetched = get_memory(&pool, &key, inserted.id)
            .await
            .expect("fetch")
            .expect("present");
        assert_eq!(fetched.id, inserted.id);
        assert_eq!(fetched.summary, "first");
    }

    #[tokio::test]
    async fn agent_scope_rejects_instrument_ids() {
        let input = create(MEMORY_SCOPE_AGENT, vec!["BTC"], "bad");
        assert!(input.validate().is_err());
    }

    #[tokio::test]
    async fn instruments_scope_requires_unique_targets() {
        assert!(
            create(MEMORY_SCOPE_INSTRUMENTS, Vec::new(), "none")
                .validate()
                .is_err()
        );
        assert!(
            create(MEMORY_SCOPE_INSTRUMENTS, vec!["BTC", "BTC"], "dupes")
                .validate()
                .is_err()
        );
        assert!(
            create(MEMORY_SCOPE_INSTRUMENTS, vec!["BTC", "ETH"], "multi")
                .validate()
                .is_ok()
        );
    }

    #[tokio::test]
    async fn list_memories_filters_by_scope_and_instrument() {
        let Some(database_url) = db_url() else {
            eprintln!("DATABASE_URL not set; skipping integration test");
            return;
        };

        let pool = connect(&database_url).await.expect("connect");
        migrate(&pool).await.expect("migrate");

        let key = format!("mem-flt-{}", Utc::now().timestamp_millis());
        seed_agent(&pool, &key).await;
        seed_instrument(&pool, &key, "BTC").await;

        insert_memory(
            &pool,
            &key,
            &create(MEMORY_SCOPE_AGENT, Vec::new(), "agent-scope"),
            None,
        )
        .await
        .expect("insert agent scope");
        insert_memory(
            &pool,
            &key,
            &create(MEMORY_SCOPE_INSTRUMENTS, vec!["BTC"], "btc"),
            None,
        )
        .await
        .expect("insert btc target");

        let rows = list_memories(
            &pool,
            &key,
            &MemoryListFilter {
                instrument_id: Some("BTC".into()),
                limit: Some(10),
                ..Default::default()
            },
        )
        .await
        .expect("list by instrument");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].summary, "btc");

        let rows = list_memories(
            &pool,
            &key,
            &MemoryListFilter {
                scope_kind: Some(MEMORY_SCOPE_AGENT.into()),
                limit: Some(10),
                ..Default::default()
            },
        )
        .await
        .expect("list by scope");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].summary, "agent-scope");
    }

    async fn seed_instrument(pool: &DbPool, agent_key: &str, instrument_id: &str) {
        sqlx::query(
            "INSERT INTO hyperliquid.instruments (
                instrument_id, name, market_type, base_asset, quote_asset,
                price_decimals, size_decimals, lot_size, active
             ) VALUES ($1, $1, 'perp', $1, 'USDC', 5, 5, 0.001, true)
             ON CONFLICT (instrument_id) DO NOTHING",
        )
        .bind(instrument_id)
        .execute(pool)
        .await
        .expect("seed instrument");
        sqlx::query(
            "INSERT INTO agent_instruments (agent_key, instrument_id) VALUES ($1, $2)
             ON CONFLICT (agent_key, instrument_id) DO NOTHING",
        )
        .bind(agent_key)
        .bind(instrument_id)
        .execute(pool)
        .await
        .expect("select instrument for agent");
    }

    #[tokio::test]
    async fn insert_rejects_unselected_instrument_target() {
        let Some(database_url) = db_url() else {
            eprintln!("DATABASE_URL not set; skipping integration test");
            return;
        };

        let pool = connect(&database_url).await.expect("connect");
        migrate(&pool).await.expect("migrate");

        let key = format!("mem-tgt-{}", Utc::now().timestamp_millis());
        seed_agent(&pool, &key).await;
        // Instrument exists in the catalog but is not selected by the agent.
        sqlx::query(
            "INSERT INTO hyperliquid.instruments (
                instrument_id, name, market_type, base_asset, quote_asset,
                price_decimals, size_decimals, lot_size, active
             ) VALUES ('DOGE', 'DOGE', 'perp', 'DOGE', 'USDC', 5, 5, 0.001, true)
             ON CONFLICT (instrument_id) DO NOTHING",
        )
        .execute(&pool)
        .await
        .expect("seed instrument");

        let error = insert_memory(
            &pool,
            &key,
            &create(MEMORY_SCOPE_INSTRUMENTS, vec!["DOGE"], "bad"),
            None,
        )
        .await
        .expect_err("unselected target must fail");
        assert!(
            error
                .to_string()
                .contains("not an active instrument selected"),
            "unexpected error: {error:#}"
        );
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

        let row = insert_memory(
            &pool,
            &owner,
            &create(MEMORY_SCOPE_AGENT, Vec::new(), "owned"),
            None,
        )
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
