//! DB access for the orders gateway: `hyperliquid.orders` and
//! `hyperliquid.order_events`. All agent-facing queries are scoped by
//! `agent_key`. The WebSocket task and the (future) reconciler use
//! account-scoped lookups.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::Value;
use sqlx::Postgres;
use uuid::Uuid;

use crate::db::DbPool;

/// One row from `hyperliquid.orders`, minus the large JSONB payloads.
///
/// Payloads (`request_payload`, `response_payload`) are accessed via
/// dedicated helpers below when needed.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct OrderRow {
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
    pub agent_key: String,
    pub group_id: Option<Uuid>,
    pub memory_record_ids: Value,
    pub attribution_source: String,
    pub symbol: String,
    pub side: String,
    pub order_kind: String,
    pub reduce_only: bool,
    pub cloid: String,
    pub exchange_oid: Option<String>,
    pub status: String,
    pub filled_size: Option<Decimal>,
    pub avg_fill_price: Option<Decimal>,
}

/// Input for [`insert_order`]. Built up by the gateway as it processes
/// one `PlaceOrderInput` (entry + TP/SL legs).
#[derive(Debug, Clone)]
pub struct NewOrder {
    pub id: Uuid,
    pub agent_key: String,
    pub account_address: String,
    pub environment: String,
    pub group_id: Option<Uuid>,
    pub parent_cloid: Option<String>,
    pub memory_record_ids: Value,
    pub attribution_source: String,
    pub symbol: String,
    pub instrument_id: Option<String>,
    pub side: String,
    /// `"limit" | "market" | "take_profit" | "stop_loss"`.
    pub order_kind: String,
    pub reduce_only: bool,
    pub requested_price: Option<Decimal>,
    pub rounded_price: Option<Decimal>,
    pub requested_size: Decimal,
    pub rounded_size: Option<Decimal>,
    pub trigger_price: Option<Decimal>,
    pub time_in_force: Option<String>,
    pub cloid: String,
    pub status: String,
    pub status_detail: Option<String>,
    pub request_payload: Value,
}

/// Input for [`append_order_event`].
#[derive(Debug, Clone)]
pub struct OrderEventInsert {
    pub id: Uuid,
    pub order_id: Uuid,
    pub account_address: String,
    pub environment: String,
    pub status: String,
    pub status_detail: Option<String>,
    pub filled_size: Option<Decimal>,
    pub avg_fill_price: Option<Decimal>,
    pub status_timestamp: DateTime<Utc>,
    /// `"http_response" | "ws_order_update" | "reconcile" | "historical_reconcile"`.
    pub source: String,
    pub payload: Value,
}

const SELECT_ORDER: &str = "SELECT id, created_at, agent_key, group_id, memory_record_ids, attribution_source, \
     symbol, side, order_kind, reduce_only, cloid, exchange_oid, status, filled_size, avg_fill_price \
     FROM hyperliquid.orders";

/// Insert a new `hyperliquid.orders` row.
pub async fn insert_order(pool: &DbPool, new: &NewOrder) -> Result<()> {
    sqlx::query(
        "INSERT INTO hyperliquid.orders (
            id, created_at, updated_at, agent_key, account_address, environment,
            group_id, parent_cloid, memory_record_ids, attribution_source, symbol, instrument_id,
            side, order_kind, reduce_only, requested_price, rounded_price,
            requested_size, rounded_size, trigger_price, time_in_force, cloid,
            status, status_detail, request_payload
         ) VALUES (
            $1, now(), now(), $2, $3, $4,
            $5, $6, $7, $8, $9, $10,
            $11, $12, $13, $14, $15,
            $16, $17, $18, $19, $20,
            $21, $22, $23
         )",
    )
    .bind(new.id)
    .bind(&new.agent_key)
    .bind(&new.account_address)
    .bind(&new.environment)
    .bind(new.group_id)
    .bind(new.parent_cloid.as_deref())
    .bind(&new.memory_record_ids)
    .bind(&new.attribution_source)
    .bind(&new.symbol)
    .bind(new.instrument_id.as_deref())
    .bind(&new.side)
    .bind(&new.order_kind)
    .bind(new.reduce_only)
    .bind(new.requested_price)
    .bind(new.rounded_price)
    .bind(new.requested_size)
    .bind(new.rounded_size)
    .bind(new.trigger_price)
    .bind(new.time_in_force.as_deref())
    .bind(&new.cloid)
    .bind(&new.status)
    .bind(new.status_detail.as_deref())
    .bind(&new.request_payload)
    .execute(pool)
    .await
    .context("failed to insert order")?;
    Ok(())
}

/// Append one row to `hyperliquid.order_events`.
///
/// The unique constraint `(order_id, status, status_timestamp, source)` makes
/// this idempotent: a duplicate insert is silently ignored.
#[cfg(test)]
pub async fn append_order_event(pool: &DbPool, ev: &OrderEventInsert) -> Result<()> {
    sqlx::query(
        "INSERT INTO hyperliquid.order_events (
            id, order_id, account_address, environment,
            status, status_detail, filled_size, avg_fill_price,
            status_timestamp, source, payload, inserted_at
         ) VALUES (
            $1, $2, $3, $4,
            $5, $6, $7, $8,
            $9, $10, $11, now()
         )
         ON CONFLICT (order_id, status, status_timestamp, source) DO NOTHING",
    )
    .bind(ev.id)
    .bind(ev.order_id)
    .bind(&ev.account_address)
    .bind(&ev.environment)
    .bind(&ev.status)
    .bind(ev.status_detail.as_deref())
    .bind(ev.filled_size)
    .bind(ev.avg_fill_price)
    .bind(ev.status_timestamp)
    .bind(&ev.source)
    .bind(&ev.payload)
    .execute(pool)
    .await
    .context("failed to append order event")?;
    Ok(())
}

/// One row from `hyperliquid.order_events`.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct OrderEventRow {
    pub id: Uuid,
    pub order_id: Uuid,
    pub account_address: String,
    pub environment: String,
    pub status: String,
    pub status_detail: Option<String>,
    pub filled_size: Option<Decimal>,
    pub avg_fill_price: Option<Decimal>,
    pub status_timestamp: DateTime<Utc>,
    pub source: String,
    pub payload: Value,
    pub inserted_at: DateTime<Utc>,
}

/// Outcome of a place / cancel step. The gateway passes this to
/// [`update_order_outcome`] to bump the order row and append the matching
/// `order_events` row in one transaction.
#[derive(Debug, Clone)]
pub struct OrderOutcome {
    pub order_id: Uuid,
    pub account_address: String,
    pub environment: String,
    pub status: String,
    pub status_detail: Option<String>,
    pub exchange_oid: Option<String>,
    pub filled_size: Option<Decimal>,
    pub avg_fill_price: Option<Decimal>,
    pub status_timestamp: DateTime<Utc>,
    pub source: String,
    pub response_payload: Value,
}

#[derive(sqlx::FromRow)]
struct CurrentOrderState {
    status: String,
    status_timestamp: Option<DateTime<Utc>>,
}

/// Bump the latest state of an order AND append a matching `order_events`
/// row, in a single transaction. All events are retained, but the current row
/// changes only through a valid state transition. Exchange-timestamped WS and
/// historical events cannot be superseded by older events or local HTTP/snapshot
/// observations. The event dedup constraint makes repeated updates safe.
pub async fn update_order_outcome(pool: &DbPool, outcome: &OrderOutcome) -> Result<()> {
    let mut tx = pool.begin().await.context("failed to begin tx")?;

    let current = sqlx::query_as::<_, CurrentOrderState>(
        "SELECT status, status_timestamp
           FROM hyperliquid.orders
          WHERE id = $1
          FOR UPDATE",
    )
    .bind(outcome.order_id)
    .fetch_optional(&mut *tx)
    .await
    .context("failed to load current order state")?
    .context("order outcome references a missing order")?;

    let authoritative = is_authoritative_source(&outcome.source);
    if should_apply_outcome(&current, outcome, authoritative) {
        sqlx::query::<Postgres>(
            "UPDATE hyperliquid.orders
                SET status = $2,
                    status_detail = $3,
                    exchange_oid = COALESCE(exchange_oid, $4),
                    filled_size = CASE
                        WHEN $5 IS NULL THEN filled_size
                        WHEN filled_size IS NULL THEN $5
                        ELSE GREATEST(filled_size, $5)
                    END,
                    avg_fill_price = COALESCE($6, avg_fill_price),
                    response_payload = $7,
                    status_timestamp = CASE WHEN $8 THEN $9 ELSE status_timestamp END,
                    updated_at = now()
              WHERE id = $1",
        )
        .bind(outcome.order_id)
        .bind(&outcome.status)
        .bind(outcome.status_detail.as_deref())
        .bind(outcome.exchange_oid.as_deref())
        .bind(outcome.filled_size)
        .bind(outcome.avg_fill_price)
        .bind(&outcome.response_payload)
        .bind(authoritative)
        .bind(outcome.status_timestamp)
        .execute(&mut *tx)
        .await
        .context("failed to update order outcome")?;
    }

    let event = OrderEventInsert {
        id: Uuid::new_v4(),
        order_id: outcome.order_id,
        account_address: outcome.account_address.clone(),
        environment: outcome.environment.clone(),
        status: outcome.status.clone(),
        status_detail: outcome.status_detail.clone(),
        filled_size: outcome.filled_size,
        avg_fill_price: outcome.avg_fill_price,
        status_timestamp: outcome.status_timestamp,
        source: outcome.source.clone(),
        payload: outcome.response_payload.clone(),
    };
    append_order_event_in_tx(&mut tx, &event).await?;

    tx.commit()
        .await
        .context("failed to commit order outcome tx")?;
    Ok(())
}

fn is_authoritative_source(source: &str) -> bool {
    matches!(source, "ws_order_update" | "historical_reconcile")
}

fn should_apply_outcome(
    current: &CurrentOrderState,
    outcome: &OrderOutcome,
    authoritative: bool,
) -> bool {
    if !is_valid_transition(&current.status, &outcome.status) {
        return false;
    }

    if !authoritative {
        return current.status_timestamp.is_none();
    }

    current
        .status_timestamp
        .is_none_or(|timestamp| outcome.status_timestamp >= timestamp)
}

fn is_valid_transition(current: &str, next: &str) -> bool {
    if current == next {
        return true;
    }

    match current {
        "pending_submission" => matches!(
            next,
            "submitted" | "resting" | "partially_filled" | "filled" | "rejected" | "unknown"
        ),
        "unknown" => matches!(
            next,
            "submitted" | "resting" | "partially_filled" | "filled" | "canceled" | "rejected"
        ),
        "submitted" => matches!(
            next,
            "resting" | "partially_filled" | "filled" | "canceled" | "rejected"
        ),
        "resting" => matches!(
            next,
            "partially_filled" | "filled" | "canceled" | "rejected"
        ),
        "partially_filled" => matches!(next, "filled" | "canceled"),
        "filled" | "canceled" | "rejected" | "error" => false,
        _ => false,
    }
}

async fn append_order_event_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    ev: &OrderEventInsert,
) -> Result<()> {
    sqlx::query::<Postgres>(
        "INSERT INTO hyperliquid.order_events (
            id, order_id, account_address, environment,
            status, status_detail, filled_size, avg_fill_price,
            status_timestamp, source, payload, inserted_at
         ) VALUES (
            $1, $2, $3, $4,
            $5, $6, $7, $8,
            $9, $10, $11, now()
         )
         ON CONFLICT (order_id, status, status_timestamp, source) DO NOTHING",
    )
    .bind(ev.id)
    .bind(ev.order_id)
    .bind(&ev.account_address)
    .bind(&ev.environment)
    .bind(&ev.status)
    .bind(ev.status_detail.as_deref())
    .bind(ev.filled_size)
    .bind(ev.avg_fill_price)
    .bind(ev.status_timestamp)
    .bind(&ev.source)
    .bind(&ev.payload)
    .execute(&mut **tx)
    .await
    .context("failed to append order event in tx")?;
    Ok(())
}

/// "open" status filter shorthand: the three states a live order can be
/// in before it reaches a terminal state.
pub const OPEN_STATUSES: &[&str] = &["submitted", "resting", "partially_filled"];

/// Fetch orders for the given agent, newest first. `status_filter`:
/// - `None`: all statuses
/// - `Some("open")`: only the open states (see [`OPEN_STATUSES`])
/// - `Some("pending")`: only `pending_submission`
/// - `Some(other)`: exact match
pub async fn list_orders(
    pool: &DbPool,
    agent_key: &str,
    status_filter: Option<&str>,
    symbol_filter: Option<&str>,
    since: Option<DateTime<Utc>>,
    until: Option<DateTime<Utc>>,
    limit: Option<i64>,
) -> Result<Vec<OrderRow>> {
    let mut qb: sqlx::QueryBuilder<sqlx::Postgres> = sqlx::QueryBuilder::new(SELECT_ORDER);
    qb.push(" WHERE agent_key = ")
        .push_bind(agent_key.to_string());

    match status_filter {
        Some("open") => {
            qb.push(" AND status IN (");
            let mut sep = qb.separated(", ");
            for s in OPEN_STATUSES {
                sep.push_bind(s.to_string());
            }
            qb.push(")");
        }
        Some("pending") => {
            qb.push(" AND status = ")
                .push_bind("pending_submission".to_string());
        }
        Some(other) => {
            qb.push(" AND status = ").push_bind(other.to_string());
        }
        None => {}
    }

    if let Some(symbol) = symbol_filter {
        qb.push(" AND symbol = ").push_bind(symbol.to_string());
    }

    if let Some(since) = since {
        qb.push(" AND created_at >= ").push_bind(since);
    }

    if let Some(until) = until {
        qb.push(" AND created_at < ").push_bind(until);
    }

    qb.push(" ORDER BY created_at DESC LIMIT ")
        .push_bind(limit.unwrap_or(500).clamp(1, 500));

    let rows = qb
        .build_query_as::<OrderRow>()
        .fetch_all(pool)
        .await
        .context("failed to list orders")?;
    Ok(rows)
}

/// Fetch a single order by id, scoped to the caller's `agent_key`.
///
/// Returns `None` for both "not found" and "not owned" so existence does
/// not leak across agents.
pub async fn get_order(pool: &DbPool, agent_key: &str, id: Uuid) -> Result<Option<OrderRow>> {
    let row = sqlx::query_as::<_, OrderRow>(sqlx::AssertSqlSafe(format!(
        "{SELECT_ORDER} WHERE id = $1 AND agent_key = $2"
    )))
    .bind(id)
    .bind(agent_key)
    .fetch_optional(pool)
    .await
    .context("failed to fetch order")?;
    Ok(row)
}

/// Fetch the lifecycle events for one order, oldest first.
pub async fn list_events(pool: &DbPool, order_id: Uuid) -> Result<Vec<OrderEventRow>> {
    let rows = sqlx::query_as::<_, OrderEventRow>(
        "SELECT id, order_id, account_address, environment, status, status_detail,
                filled_size, avg_fill_price, status_timestamp, source, payload, inserted_at
           FROM hyperliquid.order_events
          WHERE order_id = $1
          ORDER BY status_timestamp ASC, inserted_at ASC",
    )
    .bind(order_id)
    .fetch_all(pool)
    .await
    .context("failed to list order events")?;
    Ok(rows)
}

/// Find an order by its `cloid`. Account-scoped (not agent-scoped) because
/// the live WebSocket task is per-account.
pub async fn find_order_by_cloid(
    pool: &DbPool,
    account_address: &str,
    environment: &str,
    cloid: &str,
) -> Result<Option<OrderRow>> {
    let row = sqlx::query_as::<_, OrderRow>(sqlx::AssertSqlSafe(format!(
        "{SELECT_ORDER} WHERE account_address = $1 AND environment = $2 AND cloid = $3"
    )))
    .bind(account_address)
    .bind(environment)
    .bind(cloid)
    .fetch_optional(pool)
    .await
    .context("failed to find order by cloid")?;
    Ok(row)
}

/// Find an order by its exchange-assigned `oid`. Account-scoped (not
/// agent-scoped) because the live WebSocket task is per-account.
pub async fn find_order_by_oid(
    pool: &DbPool,
    account_address: &str,
    environment: &str,
    exchange_oid: &str,
) -> Result<Option<OrderRow>> {
    let row = sqlx::query_as::<_, OrderRow>(sqlx::AssertSqlSafe(format!(
        "{SELECT_ORDER} WHERE account_address = $1 AND environment = $2 AND exchange_oid = $3"
    )))
    .bind(account_address)
    .bind(environment)
    .bind(exchange_oid)
    .fetch_optional(pool)
    .await
    .context("failed to find order by oid")?;
    Ok(row)
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use rust_decimal_macros::dec;
    use serde_json::json;

    use super::*;
    use crate::{
        agents::{keys::derive_wallet_address, model::AgentRegistryRow, store::insert_agent},
        test_db,
    };

    fn sample_agent(suffix: &str) -> AgentRegistryRow {
        let private_key = format!(
            "0x{}",
            hex::encode(format!("deterministic-{suffix}").as_bytes())
        );
        let private_key_padded = if private_key.len() < 66 {
            format!("{private_key}{}", "0".repeat(66 - private_key.len()))
        } else {
            private_key
        };
        let private_key_trimmed = if private_key_padded.len() > 66 {
            private_key_padded[..66].to_string()
        } else {
            private_key_padded
        };
        let wallet = derive_wallet_address(&private_key_trimmed).unwrap();
        let now = Utc::now();
        let ts = now.timestamp_millis();
        AgentRegistryRow {
            agent_key: format!("ord-test-{suffix}-{ts}"),
            user_id: crate::test_db::test_user_id(),
            created_at: now,
            updated_at: now,
            enabled: true,
            lifecycle: crate::agents::model::AGENT_LIFECYCLE_ACTIVE.to_string(),
            display_name: format!("OrderTest {suffix}"),
            trading_account_address: Some(wallet),
            environment: "live".to_string(),
            api_key: format!("vta_ord-{suffix}-{ts}"),
            api_key_last_used_at: None,
            runtime_config: serde_json::json!({}),
        }
    }

    fn new_order(agent_key: &str, account_address: &str, cloid: &str) -> NewOrder {
        NewOrder {
            id: Uuid::new_v4(),
            agent_key: agent_key.to_string(),
            account_address: account_address.to_string(),
            environment: "live".to_string(),
            group_id: None,
            parent_cloid: None,
            memory_record_ids: json!([]),
            attribution_source: "agent".to_string(),
            symbol: "BTC".to_string(),
            // instrument_id None skips the FK to hyperliquid.instruments
            // (the test DB starts empty). Production rows always set it.
            instrument_id: None,
            side: "buy".to_string(),
            order_kind: "limit".to_string(),
            reduce_only: false,
            requested_price: Some(dec!(50000)),
            rounded_price: Some(dec!(50000)),
            requested_size: dec!(0.1),
            rounded_size: Some(dec!(0.1)),
            trigger_price: None,
            time_in_force: Some("gtc".to_string()),
            cloid: cloid.to_string(),
            status: "pending_submission".to_string(),
            status_detail: None,
            request_payload: json!({"symbol":"BTC"}),
        }
    }

    async fn seed(pool: &DbPool, suffix: &str) -> (String, String) {
        let row = sample_agent(suffix);
        let key = row.agent_key.clone();
        let acct = row
            .trading_account_address
            .clone()
            .expect("trading account");
        insert_agent(pool, &row).await.expect("insert agent");
        (key, acct)
    }

    #[tokio::test]
    async fn insert_and_get_round_trip() {
        let pool = test_db::pool().await;
        let (agent_key, account) = seed(&pool, "rt").await;

        let new = new_order(&agent_key, &account, "0xcloid_rt_1");
        let id = new.id;
        insert_order(&pool, &new).await.expect("insert");

        let fetched = get_order(&pool, &agent_key, id)
            .await
            .expect("get")
            .expect("present");
        assert_eq!(fetched.cloid, "0xcloid_rt_1");
        assert_eq!(fetched.symbol, "BTC");
        assert_eq!(fetched.status, "pending_submission");
        let (requested_size,): (Decimal,) =
            sqlx::query_as("SELECT requested_size FROM hyperliquid.orders WHERE id = $1")
                .bind(id)
                .fetch_one(&pool)
                .await
                .expect("fetch requested size");
        assert_eq!(requested_size, dec!(0.1));
    }

    #[tokio::test]
    async fn list_orders_is_scoped_by_agent() {
        let pool = test_db::pool().await;
        let (a_key, a_acct) = seed(&pool, "list-a").await;
        let (b_key, b_acct) = seed(&pool, "list-b").await;

        let mut a = new_order(&a_key, &a_acct, "0xcloid_list_a");
        a.symbol = "BTC".to_string();
        insert_order(&pool, &a).await.unwrap();

        let mut b = new_order(&b_key, &b_acct, "0xcloid_list_b");
        b.symbol = "ETH".to_string();
        insert_order(&pool, &b).await.unwrap();

        // Agent A sees only its BTC order.
        let a_rows = list_orders(&pool, &a_key, None, None, None, None, None)
            .await
            .unwrap();
        assert!(a_rows.iter().all(|r| r.agent_key == a_key));
        assert!(a_rows.iter().any(|r| r.symbol == "BTC"));

        // Symbol filter further narrows it.
        let a_btc = list_orders(&pool, &a_key, None, Some("BTC"), None, None, None)
            .await
            .unwrap();
        assert!(a_btc.iter().all(|r| r.symbol == "BTC"));

        let a_eth = list_orders(&pool, &a_key, None, Some("ETH"), None, None, None)
            .await
            .unwrap();
        assert!(a_eth.is_empty());

        // Agent B sees only its ETH order.
        let b_rows = list_orders(&pool, &b_key, None, None, None, None, None)
            .await
            .unwrap();
        assert!(b_rows.iter().all(|r| r.agent_key == b_key));
        assert!(b_rows.iter().any(|r| r.symbol == "ETH"));
    }

    #[tokio::test]
    async fn list_orders_open_status_filter() {
        let pool = test_db::pool().await;
        let (agent_key, account) = seed(&pool, "open").await;

        let mut resting = new_order(&agent_key, &account, "0xcloid_rest");
        resting.status = "resting".to_string();
        insert_order(&pool, &resting).await.unwrap();

        let mut filled = new_order(&agent_key, &account, "0xcloid_filled");
        filled.status = "filled".to_string();
        insert_order(&pool, &filled).await.unwrap();

        let open = list_orders(&pool, &agent_key, Some("open"), None, None, None, None)
            .await
            .unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].cloid, "0xcloid_rest");
    }

    #[tokio::test]
    async fn get_order_returns_none_for_other_agent() {
        let pool = test_db::pool().await;
        let (a_key, a_acct) = seed(&pool, "own-a").await;
        let (b_key, _b_acct) = seed(&pool, "own-b").await;

        let new = new_order(&a_key, &a_acct, "0xcloid_own");
        let id = new.id;
        insert_order(&pool, &new).await.unwrap();

        // Owner can fetch.
        let fetched = get_order(&pool, &a_key, id).await.unwrap();
        assert!(fetched.is_some());

        // Other agent cannot.
        let fetched = get_order(&pool, &b_key, id).await.unwrap();
        assert!(fetched.is_none());
    }

    #[tokio::test]
    async fn append_order_event_dedups_on_conflict() {
        let pool = test_db::pool().await;
        let (agent_key, account) = seed(&pool, "ev").await;

        let new = new_order(&agent_key, &account, "0xcloid_ev");
        let id = new.id;
        insert_order(&pool, &new).await.unwrap();

        // Fixed timestamp at second precision so it round-trips exactly
        // through Postgres TIMESTAMPTZ.
        let ts: DateTime<Utc> = DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let ev1 = OrderEventInsert {
            id: Uuid::new_v4(),
            order_id: id,
            account_address: account.clone(),
            environment: "live".to_string(),
            status: "submitted".to_string(),
            status_detail: None,
            filled_size: None,
            avg_fill_price: None,
            status_timestamp: ts,
            source: "http_response".to_string(),
            payload: json!({"a": 1}),
        };
        append_order_event(&pool, &ev1).await.expect("insert 1");
        append_order_event(&pool, &ev1)
            .await
            .expect("insert 2 (dup)");

        let events = list_events(&pool, id).await.unwrap();
        let matching: Vec<_> = events
            .iter()
            .filter(|e| {
                e.status == "submitted" && e.source == "http_response" && e.status_timestamp == ts
            })
            .collect();
        assert_eq!(
            matching.len(),
            1,
            "duplicate (order_id,status,ts,source) must dedup"
        );
    }

    #[tokio::test]
    async fn update_order_outcome_bumps_status_and_appends_event() {
        let pool = test_db::pool().await;
        let (agent_key, account) = seed(&pool, "upd").await;

        let new = new_order(&agent_key, &account, "0xcloid_upd");
        let id = new.id;
        insert_order(&pool, &new).await.unwrap();

        let outcome = OrderOutcome {
            order_id: id,
            account_address: account.clone(),
            environment: "live".to_string(),
            status: "resting".to_string(),
            status_detail: None,
            exchange_oid: Some("12345".to_string()),
            filled_size: None,
            avg_fill_price: None,
            status_timestamp: Utc::now(),
            source: "http_response".to_string(),
            response_payload: json!({"resting_oid": 12345}),
        };
        update_order_outcome(&pool, &outcome)
            .await
            .expect("update outcome");

        let row = get_order(&pool, &agent_key, id)
            .await
            .unwrap()
            .expect("present");
        assert_eq!(row.status, "resting");
        assert_eq!(row.exchange_oid.as_deref(), Some("12345"));

        let events = list_events(&pool, id).await.unwrap();
        let http_event: Vec<_> = events
            .iter()
            .filter(|e| e.source == "http_response" && e.status == "resting")
            .collect();
        assert_eq!(http_event.len(), 1);
    }

    #[tokio::test]
    async fn late_http_response_cannot_overwrite_websocket_fill() {
        let pool = test_db::pool().await;
        let (agent_key, account) = seed(&pool, "late-http").await;
        let new = new_order(&agent_key, &account, "0xcloid_late_http");
        let id = new.id;
        insert_order(&pool, &new).await.unwrap();

        let filled_at = DateTime::parse_from_rfc3339("2025-01-01T00:00:10Z")
            .unwrap()
            .with_timezone(&Utc);
        update_order_outcome(
            &pool,
            &OrderOutcome {
                order_id: id,
                account_address: account.clone(),
                environment: "live".to_string(),
                status: "filled".to_string(),
                status_detail: None,
                exchange_oid: Some("12345".to_string()),
                filled_size: Some(dec!(0.1)),
                avg_fill_price: Some(dec!(50000)),
                status_timestamp: filled_at,
                source: "ws_order_update".to_string(),
                response_payload: json!({"status": "filled"}),
            },
        )
        .await
        .unwrap();

        update_order_outcome(
            &pool,
            &OrderOutcome {
                order_id: id,
                account_address: account.clone(),
                environment: "live".to_string(),
                status: "submitted".to_string(),
                status_detail: None,
                exchange_oid: None,
                filled_size: None,
                avg_fill_price: None,
                status_timestamp: Utc::now(),
                source: "http_response".to_string(),
                response_payload: json!({"status": "success"}),
            },
        )
        .await
        .unwrap();

        let row = get_order(&pool, &agent_key, id).await.unwrap().unwrap();
        assert_eq!(row.status, "filled");
        assert_eq!(row.filled_size, Some(dec!(0.1)));

        let events = list_events(&pool, id).await.unwrap();
        assert!(
            events
                .iter()
                .any(|event| { event.source == "http_response" && event.status == "submitted" })
        );
    }

    #[test]
    fn older_authoritative_event_cannot_advance_current_state() {
        let current_timestamp = DateTime::parse_from_rfc3339("2025-01-01T00:00:10Z")
            .unwrap()
            .with_timezone(&Utc);
        let older_timestamp = DateTime::parse_from_rfc3339("2025-01-01T00:00:09Z")
            .unwrap()
            .with_timezone(&Utc);
        let current = CurrentOrderState {
            status: "resting".to_string(),
            status_timestamp: Some(current_timestamp),
        };
        let outcome = OrderOutcome {
            order_id: Uuid::new_v4(),
            account_address: "0xaccount".to_string(),
            environment: "live".to_string(),
            status: "filled".to_string(),
            status_detail: None,
            exchange_oid: Some("12345".to_string()),
            filled_size: Some(dec!(0.1)),
            avg_fill_price: Some(dec!(50000)),
            status_timestamp: older_timestamp,
            source: "ws_order_update".to_string(),
            response_payload: json!({}),
        };

        assert!(!should_apply_outcome(&current, &outcome, true));
    }

    #[tokio::test]
    async fn find_by_cloid_and_oid() {
        let pool = test_db::pool().await;
        let (agent_key, account) = seed(&pool, "find").await;

        let new = new_order(&agent_key, &account, "0xcloid_find");
        let id = new.id;
        insert_order(&pool, &new).await.unwrap();

        // First try by cloid (no oid yet).
        let by_cloid = find_order_by_cloid(&pool, &account, "live", "0xcloid_find")
            .await
            .unwrap();
        assert!(by_cloid.is_some());

        // Now bump oid.
        let outcome = OrderOutcome {
            order_id: id,
            account_address: account.clone(),
            environment: "live".to_string(),
            status: "resting".to_string(),
            status_detail: None,
            exchange_oid: Some("999".to_string()),
            filled_size: None,
            avg_fill_price: None,
            status_timestamp: Utc::now(),
            source: "http_response".to_string(),
            response_payload: json!({}),
        };
        update_order_outcome(&pool, &outcome).await.unwrap();

        let by_oid = find_order_by_oid(&pool, &account, "live", "999")
            .await
            .unwrap();
        assert!(by_oid.is_some());
    }
}
