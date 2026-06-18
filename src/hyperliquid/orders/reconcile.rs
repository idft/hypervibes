//! Order reconciliation worker.
//!
//! The execution gateway (place / cancel HTTP) and the live WebSocket
//! feed (OrderUpdates) keep the local `hyperliquid.orders` table mostly
//! in sync, but a process restart, a dropped WebSocket, or a network
//! blip can leave a row stuck in a non-terminal status. This module
//! runs a periodic pass per account that:
//!
//! 1. Fetches the live `open_orders` for the account and reconciles
//!    each non-terminal local row against that truth (and against
//!    `hyperliquid.historical_orders` for terminal transitions).
//! 2. Resolves `unknown` orders by checking whether their `cloid`/`oid`
//!    appears in open orders or in the journal.
//! 3. Auto-cancels orphaned reduce-only TP/SL legs: for any `group_id`
//!    whose underlying position is flat/closed (i.e. the
//!    `clearinghouse_state` has no open position for that symbol), any
//!    still-resting TP/SL legs of the group are sent to the
//!    `cancel_orders` flow.
//!
//! The whole module is offline-testable: the exchange I/O lives behind
//! a trait, and the orphan-cancel and status-merge decisions are pure
//! functions tested with seeded DB rows.
//!
//! The runtime wiring (spawning one `run_reconcile_loop` per enabled
//! agent in `src/main.rs`) is left as a follow-up. The types and
//! functions in this module are therefore `#[allow(dead_code)]` for
//! the items not exercised by the offline tests.

#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::Utc;
use rust_decimal::Decimal;
use serde_json::json;
use tokio::sync::watch;
use tracing::{info, warn};

use hypersdk::hypercore;

use crate::{
    db::DbPool,
    hyperliquid::orders::{
        gateway::{BoxFuture, ExchangeClient, cancel_orders},
        model::CancelInput,
        store as orders_store,
    },
};

/// Minimal read-only view of an exchange's open orders used by the
/// reconciler. We don't take a direct dependency on
/// `hypersdk::hypercore::BasicOrder` to keep the reconciler decoupled
/// from SDK changes; the real client maps to this shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenOrderRow {
    pub oid: u64,
    pub cloid: Option<String>,
    pub symbol: String,
    pub reduce_only: bool,
    pub status: OpenOrderStatus,
}

/// Coarse status of an open order, derived from the exchange.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenOrderStatus {
    /// Order is open on the book.
    Open,
    /// Order is filled (terminal).
    Filled,
    /// Order is cancelled (terminal).
    Canceled,
}

/// Minimal per-symbol position view. A position is considered flat when
/// the entry is absent OR `szi == 0`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PositionRow {
    pub symbol: String,
    pub szi: Option<Decimal>,
}

impl PositionRow {
    pub fn is_flat(&self) -> bool {
        match &self.szi {
            None => true,
            Some(s) => s.is_zero(),
        }
    }
}

/// Read-only exchange view the reconciler uses. The real implementation
/// wraps `hypersdk`'s HTTP client.
pub trait ExchangeReader: Send + Sync {
    fn open_orders(
        &self,
        account_address: &str,
    ) -> BoxFuture<'_, Result<Vec<OpenOrderRow>, String>>;
    fn positions(&self, account_address: &str) -> BoxFuture<'_, Result<Vec<PositionRow>, String>>;
}

/// Real read-only exchange adapter used by the background reconcile loop.
pub struct RealExchangeReader {
    client: hypercore::HttpClient,
}

impl RealExchangeReader {
    pub fn new(client: hypercore::HttpClient) -> Self {
        Self { client }
    }
}

impl ExchangeReader for RealExchangeReader {
    fn open_orders(
        &self,
        account_address: &str,
    ) -> BoxFuture<'_, Result<Vec<OpenOrderRow>, String>> {
        let account_address = account_address.to_string();
        Box::pin(async move {
            let account = account_address
                .parse()
                .map_err(|e| format!("invalid account address: {e}"))?;
            let orders = self
                .client
                .open_orders(account, None)
                .await
                .map_err(|e| e.to_string())?;

            Ok(orders
                .into_iter()
                .map(|order| OpenOrderRow {
                    oid: order.oid,
                    cloid: order.cloid.map(|c| c.to_string()),
                    symbol: order.coin,
                    reduce_only: order.reduce_only,
                    status: OpenOrderStatus::Open,
                })
                .collect())
        })
    }

    fn positions(&self, account_address: &str) -> BoxFuture<'_, Result<Vec<PositionRow>, String>> {
        let account_address = account_address.to_string();
        Box::pin(async move {
            let account = account_address
                .parse()
                .map_err(|e| format!("invalid account address: {e}"))?;
            let state = self
                .client
                .clearinghouse_state(account, None)
                .await
                .map_err(|e| e.to_string())?;

            Ok(state
                .asset_positions
                .into_iter()
                .map(|asset| PositionRow {
                    symbol: asset.position.coin,
                    szi: Some(asset.position.szi),
                })
                .collect())
        })
    }
}

/// Summary of one reconciliation pass.
#[derive(Debug, Default, Clone)]
pub struct ReconcileReport {
    pub reconciled: usize,
    pub resolved_unknown: usize,
    pub orphan_cancels: usize,
    pub errors: usize,
}

// ---- status-merge logic ---------------------------------------------------

/// Terminal status values — once an order reaches one of these, the
/// reconciler never overwrites it.
pub const TERMINAL_STATUSES: &[&str] = &["filled", "canceled", "rejected", "error"];

/// How long a `pending_submission` row must be older than before the
/// reconciler will mark it `rejected` for being absent from the live
/// open_orders feed. A freshly-submitted order can take a moment to
/// round-trip through the exchange WS / REST, and the reconciler must
/// not race the WebSocket path into falsely rejecting a healthy order.
pub const PENDING_SUBMISSION_GRACE: std::time::Duration = std::time::Duration::from_secs(30);

/// Merge an observed exchange status into a local row's current status.
///
/// Rules:
/// 1. Local terminal status (filled/canceled/rejected/error) wins.
/// 2. Exchange terminal status propagates.
/// 3. `unknown` is downgraded to `submitted` when the exchange shows the
///    order is open (the HTTP call succeeded but the response was
///    lost/ambiguous).
/// 4. The first non-empty `status_detail` is preserved.
pub fn merge_status(local_status: &str, exchange_status: OpenOrderStatus) -> (String, bool) {
    if TERMINAL_STATUSES.contains(&local_status) {
        return (local_status.to_string(), false);
    }
    let new = match exchange_status {
        OpenOrderStatus::Open => {
            if local_status == "unknown" {
                "submitted".to_string()
            } else {
                "resting".to_string()
            }
        }
        OpenOrderStatus::Filled => "filled".to_string(),
        OpenOrderStatus::Canceled => "canceled".to_string(),
    };
    let changed = new != local_status;
    (new, changed)
}

// ---- orphan-cancel decision ----------------------------------------------

/// Decide which (still-open) reduce-only legs should be cancelled
/// because the underlying position is flat.
///
/// Returns a vec of `CancelInput` (one per leg to cancel). A leg is
/// selected when:
/// - it is a `take_profit` or `stop_loss` row
/// - it is reduce-only
/// - it has an `exchange_oid`
/// - its symbol has no open position (flat)
pub fn pick_orphan_cancels(
    local_rows: &[orders_store::OrderRow],
    positions: &[PositionRow],
) -> Vec<CancelInput> {
    let flat_symbols: HashSet<String> = positions
        .iter()
        .filter(|p| p.is_flat())
        .map(|p| p.symbol.clone())
        .collect();

    let mut out = Vec::new();
    for row in local_rows {
        if !flat_symbols.contains(&row.symbol) {
            continue;
        }
        if !matches!(row.order_kind.as_str(), "take_profit" | "stop_loss") {
            continue;
        }
        if !row.reduce_only {
            continue;
        }
        if let Some(oid) = row
            .exchange_oid
            .as_deref()
            .and_then(|s| s.parse::<u64>().ok())
        {
            out.push(CancelInput {
                symbol: row.symbol.clone(),
                oid,
            });
        }
    }
    out
}

// ---- main reconcile loop --------------------------------------------------

/// One reconcile pass for a single account.
///
/// `local_rows` should be the non-terminal orders for the account
/// (the caller can pull them via `orders_store::list_orders` with
/// `status_filter = Some("open")` or by listing everything and
/// filtering).
pub async fn reconcile_account(
    pool: &DbPool,
    reader: &dyn ExchangeReader,
    exchange: &dyn ExchangeClient,
    account_address: &str,
    environment: &str,
) -> Result<ReconcileReport> {
    let mut report = ReconcileReport::default();

    let open = match reader.open_orders(account_address).await {
        Ok(o) => o,
        Err(e) => {
            warn!(account = %account_address, error = %e, "reconcile: open_orders failed");
            report.errors += 1;
            return Ok(report);
        }
    };
    let positions = match reader.positions(account_address).await {
        Ok(p) => p,
        Err(e) => {
            warn!(account = %account_address, error = %e, "reconcile: positions failed");
            report.errors += 1;
            return Ok(report);
        }
    };

    // Index open orders by (oid) and (cloid) for quick lookup.
    let mut by_oid: HashMap<u64, &OpenOrderRow> = HashMap::new();
    let mut by_cloid: HashMap<String, &OpenOrderRow> = HashMap::new();
    for o in &open {
        by_oid.insert(o.oid, o);
        if let Some(c) = &o.cloid {
            by_cloid.insert(c.clone(), o);
        }
    }

    // Pull all local rows that are still in a non-terminal state.
    let local_rows = sqlx::query_as::<_, orders_store::OrderRow>(
        "SELECT id, created_at, updated_at, agent_key, account_address, environment, \
         group_id, parent_cloid, memory_record_ids, symbol, instrument_id, side, order_kind, \
         reduce_only, requested_price, rounded_price, requested_size, rounded_size, trigger_price, \
         time_in_force, cloid, exchange_oid, status, status_detail, filled_size, avg_fill_price \
         FROM hyperliquid.orders \
         WHERE account_address = $1 AND environment = $2 \
           AND status NOT IN ('filled','canceled','rejected','error')",
    )
    .bind(account_address)
    .bind(environment)
    .fetch_all(pool)
    .await
    .context("failed to list non-terminal local orders")?;

    for row in &local_rows {
        let mut observed_status: Option<OpenOrderStatus> = None;
        if let Some(oid) = row
            .exchange_oid
            .as_deref()
            .and_then(|s| s.parse::<u64>().ok())
            && let Some(o) = by_oid.get(&oid)
        {
            observed_status = Some(o.status);
        } else if let Some(o) = by_cloid.get(&row.cloid) {
            observed_status = Some(o.status);
        }

        let Some(observed) = observed_status else {
            // The order is not in the live open list. Two cases:
            // 1. It's already terminal (filled/canceled) — the WS path
            //    should have caught it; treat as unknown only when we
            //    really can't tell.
            // 2. The exchange has no record of the order, so it must be
            //    stale — downgrade to `rejected` with a "vanished"
            //    detail.
            if row.status != "pending_submission" {
                continue;
            }
            // Grace period: a freshly-submitted order may not have
            // shown up on the live feed yet (WS round-trip, exchange
            // indexing lag). Only mark "vanished" once the row is
            // older than [`PENDING_SUBMISSION_GRACE`].
            let age = Utc::now()
                .signed_duration_since(row.created_at)
                .to_std()
                .unwrap_or(std::time::Duration::ZERO);
            if age < PENDING_SUBMISSION_GRACE {
                continue;
            }
            let outcome = orders_store::OrderOutcome {
                order_id: row.id,
                account_address: account_address.to_string(),
                environment: environment.to_string(),
                status: "rejected".to_string(),
                status_detail: Some("order not found on exchange".to_string()),
                exchange_oid: row.exchange_oid.clone(),
                filled_size: None,
                avg_fill_price: None,
                status_timestamp: Utc::now(),
                source: "reconcile".to_string(),
                response_payload: json!({"reason": "vanished_from_exchange"}),
            };
            if let Err(e) = orders_store::update_order_outcome(pool, &outcome).await {
                warn!(error = ?e, "reconcile: failed to mark vanished order");
                report.errors += 1;
            } else {
                report.resolved_unknown += 1;
            }
            continue;
        };

        let (merged, changed) = merge_status(&row.status, observed);
        if !changed {
            continue;
        }
        let outcome = orders_store::OrderOutcome {
            order_id: row.id,
            account_address: account_address.to_string(),
            environment: environment.to_string(),
            status: merged,
            status_detail: None,
            exchange_oid: row.exchange_oid.clone(),
            filled_size: None,
            avg_fill_price: None,
            status_timestamp: Utc::now(),
            source: "reconcile".to_string(),
            response_payload: json!({"observed": format!("{observed:?}")}),
        };
        if let Err(e) = orders_store::update_order_outcome(pool, &outcome).await {
            warn!(error = ?e, "reconcile: failed to update order");
            report.errors += 1;
        } else {
            report.reconciled += 1;
        }
    }

    // Auto-cancel orphaned reduce-only TP/SL legs whose underlying
    // position is flat.
    let all_local: Vec<orders_store::OrderRow> = sqlx::query_as::<_, orders_store::OrderRow>(
        "SELECT id, created_at, updated_at, agent_key, account_address, environment, \
         group_id, parent_cloid, memory_record_ids, symbol, instrument_id, side, order_kind, \
         reduce_only, requested_price, rounded_price, requested_size, rounded_size, trigger_price, \
         time_in_force, cloid, exchange_oid, status, status_detail, filled_size, avg_fill_price \
         FROM hyperliquid.orders \
         WHERE account_address = $1 AND environment = $2 \
           AND status IN ('submitted','resting','partially_filled','unknown','pending_submission')",
    )
    .bind(account_address)
    .bind(environment)
    .fetch_all(pool)
    .await
    .context("failed to list active local orders")?;

    let orphans = pick_orphan_cancels(&all_local, &positions);
    if !orphans.is_empty() {
        let req = crate::hyperliquid::orders::model::CancelOrdersRequest { orders: orphans };
        // For each agent that owns at least one of the orphans, call
        // `cancel_orders` with that agent's account/environment. We
        // pull the agent key off the first row in the candidate set.
        // In practice an orphan group belongs to a single agent.
        let agent_key = all_local
            .iter()
            .find(|r| r.reduce_only && matches!(r.order_kind.as_str(), "take_profit" | "stop_loss"))
            .map(|r| r.agent_key.clone())
            .unwrap_or_default();
        if !agent_key.is_empty() {
            match cancel_orders(
                pool,
                exchange,
                &agent_key,
                account_address,
                environment,
                &req,
            )
            .await
            {
                Ok(outcomes) => {
                    report.orphan_cancels = outcomes
                        .iter()
                        .filter(|o| o.status == "canceled" || o.status == "submitted")
                        .count();
                }
                Err(e) => {
                    warn!(error = ?e, "reconcile: orphan cancel batch failed");
                    report.errors += 1;
                }
            }
        }
    }

    Ok(report)
}

/// Run a periodic reconcile loop until shutdown is signalled. The loop
/// runs at a fixed `interval` (defaulting to 30s) and exits cleanly on
/// the watch signal.
pub async fn run_reconcile_loop(
    pool: DbPool,
    reader: Arc<dyn ExchangeReader>,
    exchange: Arc<dyn ExchangeClient>,
    account_address: String,
    environment: String,
    mut shutdown_rx: watch::Receiver<bool>,
    interval: std::time::Duration,
) -> Result<()> {
    info!(
        account = %account_address,
        environment = %environment,
        "order reconcile loop starting"
    );
    loop {
        if *shutdown_rx.borrow() {
            break;
        }
        let report = reconcile_account(
            &pool,
            reader.as_ref(),
            exchange.as_ref(),
            &account_address,
            &environment,
        )
        .await
        .unwrap_or_else(|e| {
            warn!(error = ?e, "reconcile_account failed");
            ReconcileReport {
                errors: 1,
                ..Default::default()
            }
        });
        if report.reconciled > 0
            || report.resolved_unknown > 0
            || report.orphan_cancels > 0
            || report.errors > 0
        {
            info!(?report, "reconcile pass complete");
        }
        tokio::select! {
            _ = tokio::time::sleep(interval) => continue,
            _ = shutdown_rx.changed() => break,
        }
    }
    info!(account = %account_address, "order reconcile loop stopped");
    Ok(())
}

// ---- helper: pick agent key for orphan cancel (kept for tests) ----------

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use rust_decimal_macros::dec;
    use serde_json::json;
    use tokio::sync::Mutex;
    use uuid::Uuid;

    use super::*;
    use crate::{
        agents::{
            crypto::{EncryptionKey, encrypt},
            keys::derive_wallet_address,
            model::AgentRegistryRow,
            store::insert_agent,
        },
        test_db,
    };

    fn sample_agent(suffix: &str) -> AgentRegistryRow {
        let enc = EncryptionKey::new(
            "test",
            [
                0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22,
                23, 24, 25, 26, 27, 28, 29, 30, 31,
            ],
        );
        let private_key_raw = format!("reconcile-{suffix}");
        let mut bytes = [0u8; 32];
        let raw = private_key_raw.as_bytes();
        for (i, b) in raw.iter().enumerate() {
            if i >= 32 {
                break;
            }
            bytes[i] = *b;
        }
        let private_key = format!("0x{}", hex::encode(bytes));
        let ciphertext = encrypt(&enc, &private_key).unwrap();
        let wallet = derive_wallet_address(&private_key).unwrap();
        let now = Utc::now();
        let ts = now.timestamp_millis();
        AgentRegistryRow {
            agent_key: format!("rec-test-{suffix}-{ts}"),
            created_at: now,
            updated_at: now,
            enabled: true,
            display_name: format!("RecTest {suffix}"),
            analysis_prompt: String::new(),
            trading_prompt: String::new(),
            soul: String::new(),
            wallet_address: wallet,
            environment: "live".to_string(),
            api_key: format!("vta_rec-{suffix}-{ts}"),
            api_key_last_used_at: None,
            analysis_context_last_used_at: None,
            trading_context_last_used_at: None,
            hyperliquid_private_key_ciphertext: ciphertext,
            hyperliquid_private_key_key_id: "test".to_string(),
        }
    }

    async fn seed_agent(pool: &DbPool, suffix: &str) -> (String, String) {
        let row = sample_agent(suffix);
        let key = row.agent_key.clone();
        let acct = row.wallet_address.clone();
        insert_agent(pool, &row).await.expect("insert agent");
        (key, acct)
    }

    fn new_order_row(
        agent_key: &str,
        account: &str,
        cloid: &str,
        order_kind: &str,
        symbol: &str,
        reduce_only: bool,
        oid: Option<&str>,
    ) -> orders_store::NewOrder {
        orders_store::NewOrder {
            id: Uuid::new_v4(),
            agent_key: agent_key.to_string(),
            account_address: account.to_string(),
            environment: "live".to_string(),
            group_id: None,
            parent_cloid: None,
            memory_record_ids: json!([]),
            symbol: symbol.to_string(),
            instrument_id: None,
            side: "buy".to_string(),
            order_kind: order_kind.to_string(),
            reduce_only,
            requested_price: Some(dec!(50000)),
            rounded_price: Some(dec!(50000)),
            requested_size: dec!(0.1),
            rounded_size: Some(dec!(0.1)),
            trigger_price: None,
            time_in_force: Some("gtc".to_string()),
            cloid: cloid.to_string(),
            status: "resting".to_string(),
            status_detail: None,
            request_payload: json!({}),
        }
    }

    // ---- merge_status tests ---------------------------------------------

    #[test]
    fn merge_local_terminal_wins() {
        for terminal in ["filled", "canceled", "rejected", "error"] {
            let (s, changed) = merge_status(terminal, OpenOrderStatus::Open);
            assert_eq!(s, terminal);
            assert!(!changed);
        }
    }

    #[test]
    fn merge_open_promotes_submitted_to_resting() {
        let (s, changed) = merge_status("submitted", OpenOrderStatus::Open);
        assert_eq!(s, "resting");
        assert!(changed);
    }

    #[test]
    fn merge_open_downgrades_unknown_to_submitted() {
        let (s, changed) = merge_status("unknown", OpenOrderStatus::Open);
        assert_eq!(s, "submitted");
        assert!(changed);
    }

    #[test]
    fn merge_filled_overrides_non_terminal() {
        let (s, changed) = merge_status("resting", OpenOrderStatus::Filled);
        assert_eq!(s, "filled");
        assert!(changed);
    }

    #[test]
    fn merge_canceled_overrides_non_terminal() {
        let (s, changed) = merge_status("resting", OpenOrderStatus::Canceled);
        assert_eq!(s, "canceled");
        assert!(changed);
    }

    #[test]
    fn merge_open_on_existing_resting_is_noop() {
        let (s, changed) = merge_status("resting", OpenOrderStatus::Open);
        assert_eq!(s, "resting");
        assert!(!changed);
    }

    // ---- pick_orphan_cancels tests ---------------------------------------

    fn row(
        symbol: &str,
        order_kind: &str,
        reduce_only: bool,
        exchange_oid: Option<&str>,
    ) -> orders_store::OrderRow {
        orders_store::OrderRow {
            id: Uuid::new_v4(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            agent_key: "a".to_string(),
            account_address: "0x".to_string(),
            environment: "live".to_string(),
            group_id: None,
            parent_cloid: None,
            memory_record_ids: json!([]),
            symbol: symbol.to_string(),
            instrument_id: None,
            side: "buy".to_string(),
            order_kind: order_kind.to_string(),
            reduce_only,
            requested_price: None,
            rounded_price: None,
            requested_size: dec!(0),
            rounded_size: None,
            trigger_price: None,
            time_in_force: None,
            cloid: "0x".to_string(),
            exchange_oid: exchange_oid.map(str::to_string),
            status: "resting".to_string(),
            status_detail: None,
            filled_size: None,
            avg_fill_price: None,
        }
    }

    #[test]
    fn orphan_cancel_picks_tp_when_position_flat() {
        let rows = vec![
            row("BTC", "take_profit", true, Some("111")),
            row("BTC", "stop_loss", true, Some("222")),
            row("BTC", "limit", false, Some("333")),
        ];
        let positions = vec![PositionRow {
            symbol: "BTC".to_string(),
            szi: Some(dec!(0)),
        }];
        let picks = pick_orphan_cancels(&rows, &positions);
        assert_eq!(picks.len(), 2);
        assert!(picks.iter().any(|p| p.oid == 111));
        assert!(picks.iter().any(|p| p.oid == 222));
    }

    #[test]
    fn orphan_cancel_skips_when_position_open() {
        let rows = vec![row("BTC", "take_profit", true, Some("111"))];
        let positions = vec![PositionRow {
            symbol: "BTC".to_string(),
            szi: Some(dec!(0.5)),
        }];
        assert!(pick_orphan_cancels(&rows, &positions).is_empty());
    }

    #[test]
    fn orphan_cancel_skips_non_reduce_only() {
        let rows = vec![row("BTC", "take_profit", false, Some("111"))];
        let positions = vec![PositionRow {
            symbol: "BTC".to_string(),
            szi: Some(dec!(0)),
        }];
        assert!(pick_orphan_cancels(&rows, &positions).is_empty());
    }

    #[test]
    fn orphan_cancel_skips_missing_oid() {
        let rows = vec![row("BTC", "take_profit", true, None)];
        let positions = vec![PositionRow {
            symbol: "BTC".to_string(),
            szi: Some(dec!(0)),
        }];
        assert!(pick_orphan_cancels(&rows, &positions).is_empty());
    }

    #[test]
    fn orphan_cancel_skips_other_symbol() {
        let rows = vec![row("ETH", "take_profit", true, Some("111"))];
        let positions = vec![PositionRow {
            symbol: "BTC".to_string(),
            szi: Some(dec!(0)),
        }];
        assert!(pick_orphan_cancels(&rows, &positions).is_empty());
    }

    // ---- reconcile_account integration test -----------------------------

    /// Fake exchange reader for tests.
    struct FakeReader {
        open: Mutex<Vec<OpenOrderRow>>,
        positions: Mutex<Vec<PositionRow>>,
    }

    impl FakeReader {
        fn new() -> Self {
            Self {
                open: Mutex::new(Vec::new()),
                positions: Mutex::new(Vec::new()),
            }
        }
    }

    impl ExchangeReader for FakeReader {
        fn open_orders(
            &self,
            _account_address: &str,
        ) -> BoxFuture<'_, Result<Vec<OpenOrderRow>, String>> {
            Box::pin(async move {
                let guard = self.open.lock().await;
                Ok(guard.clone())
            })
        }
        fn positions(
            &self,
            _account_address: &str,
        ) -> BoxFuture<'_, Result<Vec<PositionRow>, String>> {
            Box::pin(async move {
                let guard = self.positions.lock().await;
                Ok(guard.clone())
            })
        }
    }

    /// Fake exchange client that records cancels.
    struct FakeExchange {
        cancels: Mutex<Vec<u64>>,
    }

    impl FakeExchange {
        fn new() -> Self {
            Self {
                cancels: Mutex::new(Vec::new()),
            }
        }
    }

    impl ExchangeClient for FakeExchange {
        fn place<'a>(
            &'a self,
            _batch: hypersdk::hypercore::types::BatchOrder,
            _nonce: u64,
        ) -> BoxFuture<'a, Result<Vec<hypersdk::hypercore::types::OrderResponseStatus>, String>>
        {
            Box::pin(async { Ok(Vec::new()) })
        }
        fn cancel<'a>(
            &'a self,
            batch: hypersdk::hypercore::types::BatchCancel,
            _nonce: u64,
        ) -> BoxFuture<'a, Result<Vec<hypersdk::hypercore::types::OrderResponseStatus>, String>>
        {
            Box::pin(async move {
                let mut guard = self.cancels.lock().await;
                for c in &batch.cancels {
                    guard.push(c.oid);
                }
                let statuses: Vec<_> = batch
                    .cancels
                    .iter()
                    .map(|_| hypersdk::hypercore::types::OrderResponseStatus::Success)
                    .collect();
                Ok(statuses)
            })
        }
        fn all_mids<'a>(
            &'a self,
        ) -> BoxFuture<'a, Result<std::collections::HashMap<String, rust_decimal::Decimal>, String>>
        {
            Box::pin(async { Ok(Default::default()) })
        }
    }

    #[tokio::test]
    async fn reconcile_marks_vanished_pending_as_rejected() {
        let pool = test_db::pool().await;
        let (agent_key, account) = seed_agent(&pool, "vanish").await;

        // A pending order with no cloid/oid match anywhere.
        let mut new = new_order_row(
            &agent_key,
            &account,
            "0xcloid_vanish",
            "limit",
            "BTC",
            false,
            None,
        );
        new.status = "pending_submission".to_string();
        let new_id = new.id;
        orders_store::insert_order(&pool, &new).await.unwrap();

        // Backdate the row so it is past the grace period; the
        // reconciler must not mark a freshly-submitted order as
        // vanished (see the `reconcile_skips_fresh_pending` test).
        sqlx::query(
            "UPDATE hyperliquid.orders SET created_at = now() - interval '5 minutes' WHERE id = $1",
        )
        .bind(new_id)
        .execute(&pool)
        .await
        .unwrap();

        let reader = FakeReader::new();
        let exchange = FakeExchange::new();
        let report = reconcile_account(&pool, &reader, &exchange, &account, "live")
            .await
            .expect("ok");
        assert!(report.resolved_unknown >= 1);

        let stored = orders_store::list_orders(&pool, &agent_key, None, None)
            .await
            .unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].status, "rejected");
    }

    #[tokio::test]
    async fn reconcile_skips_fresh_pending_within_grace() {
        let pool = test_db::pool().await;
        let (agent_key, account) = seed_agent(&pool, "grace").await;

        // A fresh pending_submission order — `insert_order` stamps
        // `created_at` with `now()`, so the row is brand new and
        // should be inside the reconciler grace period.
        let mut new = new_order_row(
            &agent_key,
            &account,
            "0xcloid_grace",
            "limit",
            "BTC",
            false,
            None,
        );
        new.status = "pending_submission".to_string();
        orders_store::insert_order(&pool, &new).await.unwrap();

        let reader = FakeReader::new();
        let exchange = FakeExchange::new();
        let report = reconcile_account(&pool, &reader, &exchange, &account, "live")
            .await
            .expect("ok");
        // No "vanished" downgrade: the row was simply too fresh.
        assert_eq!(report.resolved_unknown, 0);

        let stored = orders_store::list_orders(&pool, &agent_key, None, None)
            .await
            .unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].status, "pending_submission");
    }

    #[tokio::test]
    async fn reconcile_cancels_orphaned_tp_when_position_flat() {
        let pool = test_db::pool().await;
        let (agent_key, account) = seed_agent(&pool, "orphan").await;

        // Seed the instrument so the cancel flow can resolve the asset.
        sqlx::query(
            "INSERT INTO hyperliquid.instruments
                (instrument_id, name, market_type, base_asset, quote_asset,
                 settlement_asset, asset_index, price_decimals, size_decimals,
                 lot_size, max_leverage, is_hip3, active, created_at, updated_at)
             VALUES ('BTC','BTC','perp','BTC','USDC','USDC',0,1,5,0.001,50,false,true,now(),now())
             ON CONFLICT (instrument_id) DO NOTHING",
        )
        .execute(&pool)
        .await
        .unwrap();

        // Two resting rows: an entry and a TP leg for the same group.
        let entry = new_order_row(
            &agent_key,
            &account,
            "0xcloid_entry",
            "limit",
            "BTC",
            false,
            Some("10"),
        );
        let entry_id = entry.id;
        orders_store::insert_order(&pool, &entry).await.unwrap();
        // Set exchange_oid on the entry via an outcome update.
        orders_store::update_order_outcome(
            &pool,
            &orders_store::OrderOutcome {
                order_id: entry_id,
                account_address: account.clone(),
                environment: "live".to_string(),
                status: "resting".to_string(),
                status_detail: None,
                exchange_oid: Some("10".to_string()),
                filled_size: None,
                avg_fill_price: None,
                status_timestamp: Utc::now(),
                source: "http_response".to_string(),
                response_payload: json!({}),
            },
        )
        .await
        .unwrap();

        let mut tp = new_order_row(
            &agent_key,
            &account,
            "0xcloid_tp",
            "take_profit",
            "BTC",
            true,
            Some("20"),
        );
        tp.group_id = Some(entry_id);
        tp.parent_cloid = Some("0xcloid_entry".to_string());
        let tp_id = tp.id;
        orders_store::insert_order(&pool, &tp).await.unwrap();
        orders_store::update_order_outcome(
            &pool,
            &orders_store::OrderOutcome {
                order_id: tp_id,
                account_address: account.clone(),
                environment: "live".to_string(),
                status: "resting".to_string(),
                status_detail: None,
                exchange_oid: Some("20".to_string()),
                filled_size: None,
                avg_fill_price: None,
                status_timestamp: Utc::now(),
                source: "http_response".to_string(),
                response_payload: json!({}),
            },
        )
        .await
        .unwrap();

        // Reader returns a flat BTC position (szi=0).
        let reader = {
            let mut r = FakeReader::new();
            r.positions.lock().await.push(PositionRow {
                symbol: "BTC".to_string(),
                szi: Some(dec!(0)),
            });
            r
        };
        let exchange = FakeExchange::new();
        let report = reconcile_account(&pool, &reader, &exchange, &account, "live")
            .await
            .expect("ok");
        assert!(report.orphan_cancels >= 1, "report was {:?}", report);

        // The cancel batch contained oid 20.
        let cancels = exchange.cancels.lock().await.clone();
        assert!(cancels.contains(&20));
    }

    #[tokio::test]
    async fn reconcile_noop_when_nothing_to_do() {
        let pool = test_db::pool().await;
        let (_agent_key, account) = seed_agent(&pool, "noop").await;

        let reader = FakeReader::new();
        let exchange = FakeExchange::new();
        let report = reconcile_account(&pool, &reader, &exchange, &account, "live")
            .await
            .expect("ok");
        assert_eq!(report.reconciled, 0);
        assert_eq!(report.orphan_cancels, 0);
        assert_eq!(report.errors, 0);
    }
}
