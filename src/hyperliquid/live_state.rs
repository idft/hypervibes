//! In-memory live account state driven by the Hyperliquid WebSocket.
//!
//! `hypersdk` is used as an adapter for the WebSocket transport and message
//! decoding. The types defined here are the app's long-lived domain model and
//! do not leak `hypersdk` types into the web layer, DB schema, or templates.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

/// Composite key for an account tracked by the live state store.
#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountKey {
    pub account_address: String,
    pub environment: String,
}

impl AccountKey {
    pub fn new(account_address: impl Into<String>, environment: impl Into<String>) -> Self {
        Self {
            account_address: account_address.into(),
            environment: environment.into(),
        }
    }
}

/// Lifecycle status of the live WebSocket connection for a single account.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveConnectionStatus {
    #[default]
    Starting,
    StartupSyncing,
    Connecting,
    Connected,
    Reconnecting,
    Disconnected,
    Failed,
    Stopped,
}

/// Authority of one account-data subscription at the time it is consumed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveDataStatus {
    #[default]
    Loading,
    Current,
    Stale,
    Degraded,
}

impl LiveDataStatus {
    pub fn is_current(self) -> bool {
        matches!(self, Self::Current)
    }
}

/// Overall operator-facing health for one monitored account.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveAccountHealthStatus {
    #[default]
    Loading,
    Healthy,
    Degraded,
    Stale,
    Failed,
    Stopped,
}

/// Derived authority and observability data for a live account snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveAccountHealth {
    pub status: LiveAccountHealthStatus,
    pub connection_status: LiveConnectionStatus,
    pub positions: LiveDataStatus,
    pub open_orders: LiveDataStatus,
    pub balance: LiveDataStatus,
    pub last_successful_update: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
}

/// Margin/leverage summary mirroring `hypersdk::MarginSummary` for the cross
/// account (the orchestrator tracks the cross-margin account; isolated
/// positions still surface leverage fields per-position).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LiveMarginState {
    pub account_value: Option<Decimal>,
    pub total_ntl_pos: Option<Decimal>,
    pub total_raw_usd: Option<Decimal>,
    pub total_margin_used: Option<Decimal>,
    pub cross_maintenance_margin_used: Option<Decimal>,
    pub withdrawable: Option<Decimal>,
    pub updated_at: Option<DateTime<Utc>>,
}

/// A single spot balance as held in memory.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LiveSpotBalance {
    pub coin: String,
    pub total: Option<Decimal>,
    pub hold: Option<Decimal>,
    pub available: Option<Decimal>,
    pub entry_ntl: Option<Decimal>,
}

/// A single open perpetual position as held in memory.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LivePosition {
    pub coin: String,
    pub szi: Option<Decimal>,
    pub entry_px: Option<Decimal>,
    pub unrealized_pnl: Option<Decimal>,
    pub liquidation_px: Option<Decimal>,
    pub margin_used: Option<Decimal>,
    pub position_value: Option<Decimal>,
    pub return_on_equity: Option<Decimal>,
    pub leverage_type: Option<String>,
    pub leverage_value: Option<u32>,
    pub max_leverage: Option<u32>,
}

/// A single open order as held in memory.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LiveOpenOrder {
    pub coin: String,
    pub side: Option<String>,
    pub limit_px: Option<Decimal>,
    pub sz: Option<Decimal>,
    pub orig_sz: Option<Decimal>,
    pub oid: Option<String>,
    pub timestamp: Option<u64>,
    pub cloid: Option<String>,
    pub order_type: Option<String>,
    pub tif: Option<String>,
    pub reduce_only: Option<bool>,
    pub is_trigger: Option<bool>,
    pub trigger_px: Option<Decimal>,
    pub trigger_condition: Option<String>,
    pub is_position_tpsl: Option<bool>,
}

/// Agent-facing snapshot of live account state. Built from the in-memory
/// `AccountLiveState` and used as the payload in OpenCode trading prompts.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LiveAgentSnapshot {
    pub account_address: String,
    pub environment: String,
    pub account_data_available: bool,
    pub account_data_stale: bool,
    pub account_data_as_of: Option<DateTime<Utc>>,
    pub account_data_status: LiveAccountHealthStatus,
    pub account_data_error: Option<String>,
    pub positions_data_status: LiveDataStatus,
    pub orders_data_status: LiveDataStatus,
    pub balance_data_status: LiveDataStatus,
    pub total_equity_usd: Option<Decimal>,
    pub available_to_trade_usd: Option<Decimal>,
    pub margin_used_usd: Option<Decimal>,
    pub unrealized_pnl_usd: Option<Decimal>,
    pub open_positions: Vec<LivePosition>,
    pub open_orders: Vec<LiveOpenOrder>,
}

impl LiveAgentSnapshot {
    pub fn to_markdown(&self) -> String {
        let available = self.account_data_available;
        let stale = self.account_data_stale;
        let as_of = self
            .account_data_as_of
            .map(|ts| ts.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true))
            .unwrap_or_default();

        let mut body = format!(
            "- Account: {}\n- Environment: {}\n- Available: {}\n- Stale: {}\n- Status: {:?}\n- As of: {}\n",
            self.account_address,
            self.environment,
            available,
            stale,
            self.account_data_status,
            as_of,
        );
        if let Some(error) = &self.account_data_error {
            body.push_str(&format!("- Monitoring error: {error}\n"));
        }

        body.push_str(&format!(
            "- Total equity USD: {}\n- Available to trade USD: {}\n- Margin used USD: {}\n- Unrealized PnL USD: {}\n",
            self.total_equity_usd.map(|v| v.to_string()).unwrap_or_else(|| "-".to_string()),
            self.available_to_trade_usd.map(|v| v.to_string()).unwrap_or_else(|| "-".to_string()),
            self.margin_used_usd.map(|v| v.to_string()).unwrap_or_else(|| "-".to_string()),
            self.unrealized_pnl_usd.map(|v| v.to_string()).unwrap_or_else(|| "-".to_string()),
        ));

        body.push_str("\n### Open positions\n");
        if !self.positions_data_status.is_current() {
            body.push_str("Unavailable: live positions data is not authoritative.\n");
        } else if self.open_positions.is_empty() {
            body.push_str("None\n");
        } else {
            for position in &self.open_positions {
                body.push_str(&format!(
                    "- {}: size {}, unrealized_pnl {}\n",
                    position.coin,
                    position
                        .szi
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "-".to_string()),
                    position
                        .unrealized_pnl
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "-".to_string()),
                ));
            }
        }

        body.push_str("\n### Open orders\n");
        if !self.orders_data_status.is_current() {
            body.push_str("Unavailable: live open-orders data is not authoritative.\n");
        } else if self.open_orders.is_empty() {
            body.push_str("None\n");
        } else {
            for order in &self.open_orders {
                body.push_str(&format!(
                    "- {} {} {} @ {}\n",
                    order.side.as_deref().unwrap_or("-"),
                    order
                        .sz
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "-".to_string()),
                    order.coin,
                    order
                        .limit_px
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "-".to_string()),
                ));
            }
        }

        body
    }
}

/// Maximum age for live account data to be considered fresh enough for
/// trading prompts. Data older than this is treated as stale/unavailable.
pub const ACCOUNT_DATA_MAX_AGE: chrono::Duration = chrono::Duration::minutes(2);

/// Assets treated as collateral when computing account equity and
/// available-to-trade balances.
const COLLATERAL_ASSETS: &[&str] = &["USDC", "USDE", "USDT0", "USDH"];

impl LiveAgentSnapshot {
    /// Build a prompt-ready snapshot from an in-memory account state.
    ///
    /// Carries authority metadata with the snapshot so consumers never mistake
    /// unavailable data for an empty account.
    pub fn from_state(state: &AccountLiveState) -> Self {
        let health = account_live_health(state);
        let spot_balances = &state.spot_balances;

        let collateral_total: Decimal = COLLATERAL_ASSETS
            .iter()
            .map(|asset| {
                spot_balances
                    .iter()
                    .filter(|balance| balance.coin == *asset)
                    .filter_map(|balance| balance.total)
                    .sum::<Decimal>()
            })
            .sum();

        let collateral_available: Decimal = COLLATERAL_ASSETS
            .iter()
            .map(|asset| {
                spot_balances
                    .iter()
                    .filter(|balance| balance.coin == *asset)
                    .filter_map(|balance| balance.available)
                    .sum::<Decimal>()
            })
            .sum();

        let margin_withdrawable = state
            .margin
            .as_ref()
            .and_then(|margin| margin.withdrawable)
            .unwrap_or(Decimal::ZERO);
        let margin_account_value = state
            .margin
            .as_ref()
            .and_then(|margin| margin.account_value)
            .unwrap_or(Decimal::ZERO);
        let margin_used_usd = state
            .margin
            .as_ref()
            .and_then(|margin| margin.total_margin_used)
            .unwrap_or(Decimal::ZERO);

        let unrealized_pnl_usd: Decimal = state
            .open_positions
            .iter()
            .filter_map(|position| position.unrealized_pnl)
            .sum();

        let total_equity_usd =
            if collateral_total > Decimal::ZERO || margin_account_value > Decimal::ZERO {
                Some(collateral_total.max(margin_account_value))
            } else {
                None
            };
        let available_to_trade_usd =
            if collateral_available > Decimal::ZERO || margin_withdrawable > Decimal::ZERO {
                Some(collateral_available.max(margin_withdrawable))
            } else {
                None
            };

        Self {
            account_address: state.account_address.clone(),
            environment: state.environment.clone(),
            account_data_available: health.status == LiveAccountHealthStatus::Healthy,
            account_data_stale: health.status != LiveAccountHealthStatus::Healthy,
            account_data_as_of: state.account_data_as_of(),
            account_data_status: health.status,
            account_data_error: health.last_error,
            positions_data_status: health.positions,
            orders_data_status: health.open_orders,
            balance_data_status: health.balance,
            total_equity_usd,
            available_to_trade_usd,
            margin_used_usd: if margin_used_usd > Decimal::ZERO {
                Some(margin_used_usd)
            } else {
                None
            },
            unrealized_pnl_usd: if unrealized_pnl_usd != Decimal::ZERO {
                Some(unrealized_pnl_usd)
            } else {
                None
            },
            open_positions: state.open_positions.clone(),
            open_orders: state.open_orders.clone(),
        }
    }
}

/// Build a `LiveAgentSnapshot` from the in-memory live account store. If no
/// fresh state is available, a stale placeholder is returned so prompts can
/// still reason about the missing data.
pub fn live_agent_snapshot_for_dispatch(
    account_address: impl Into<String>,
    environment: impl Into<String>,
    store: &LiveAccountStore,
) -> LiveAgentSnapshot {
    let key = AccountKey::new(account_address, environment);
    let Some(state) = store.get(&key) else {
        return LiveAgentSnapshot {
            account_address: key.account_address,
            environment: key.environment,
            account_data_available: false,
            account_data_stale: true,
            ..Default::default()
        };
    };

    LiveAgentSnapshot::from_state(&state)
}

/// Full per-account live state.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AccountLiveState {
    pub account_address: String,
    pub environment: String,
    pub status: LiveConnectionStatus,
    pub connected_at: Option<DateTime<Utc>>,
    /// Last successful clearinghouse snapshot. This authorizes positions.
    pub clearinghouse_updated_at: Option<DateTime<Utc>>,
    /// Last successful open-orders snapshot.
    pub open_orders_updated_at: Option<DateTime<Utc>>,
    /// Last successful spot-state snapshot. This, with clearinghouse state,
    /// authorizes the unified-account balance.
    pub spot_updated_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub margin: Option<LiveMarginState>,
    pub spot_balances: Vec<LiveSpotBalance>,
    pub open_positions: Vec<LivePosition>,
    pub open_orders: Vec<LiveOpenOrder>,
}

impl AccountLiveState {
    pub fn account_data_as_of(&self) -> Option<DateTime<Utc>> {
        [
            self.clearinghouse_updated_at,
            self.open_orders_updated_at,
            self.spot_updated_at,
        ]
        .into_iter()
        .flatten()
        .min()
    }
}

pub fn account_live_health(state: &AccountLiveState) -> LiveAccountHealth {
    account_live_health_at(state, Utc::now())
}

pub fn account_live_health_at(state: &AccountLiveState, now: DateTime<Utc>) -> LiveAccountHealth {
    let positions = stream_health(state.clearinghouse_updated_at, state.status, now);
    let open_orders = stream_health(state.open_orders_updated_at, state.status, now);
    let balance = if positions.is_current() {
        stream_health(state.spot_updated_at, state.status, now)
    } else {
        positions
    };
    let status = match state.status {
        LiveConnectionStatus::Failed => LiveAccountHealthStatus::Failed,
        LiveConnectionStatus::Stopped => LiveAccountHealthStatus::Stopped,
        LiveConnectionStatus::Starting
        | LiveConnectionStatus::StartupSyncing
        | LiveConnectionStatus::Connecting => LiveAccountHealthStatus::Loading,
        LiveConnectionStatus::Disconnected | LiveConnectionStatus::Reconnecting => {
            LiveAccountHealthStatus::Degraded
        }
        LiveConnectionStatus::Connected => {
            if [positions, open_orders, balance]
                .into_iter()
                .all(LiveDataStatus::is_current)
            {
                LiveAccountHealthStatus::Healthy
            } else if [positions, open_orders, balance]
                .into_iter()
                .any(|status| status == LiveDataStatus::Stale)
            {
                LiveAccountHealthStatus::Stale
            } else {
                LiveAccountHealthStatus::Loading
            }
        }
    };

    LiveAccountHealth {
        status,
        connection_status: state.status,
        positions,
        open_orders,
        balance,
        last_successful_update: [
            state.clearinghouse_updated_at,
            state.open_orders_updated_at,
            state.spot_updated_at,
        ]
        .into_iter()
        .flatten()
        .max(),
        last_error: state.last_error.clone(),
    }
}

fn stream_health(
    updated_at: Option<DateTime<Utc>>,
    connection_status: LiveConnectionStatus,
    now: DateTime<Utc>,
) -> LiveDataStatus {
    if matches!(
        connection_status,
        LiveConnectionStatus::Disconnected
            | LiveConnectionStatus::Reconnecting
            | LiveConnectionStatus::Failed
            | LiveConnectionStatus::Stopped
    ) {
        return LiveDataStatus::Degraded;
    }
    let Some(updated_at) = updated_at else {
        return LiveDataStatus::Loading;
    };
    if now - updated_at > ACCOUNT_DATA_MAX_AGE {
        return LiveDataStatus::Stale;
    }
    if connection_status != LiveConnectionStatus::Connected {
        return LiveDataStatus::Degraded;
    }
    LiveDataStatus::Current
}

/// Capacity of the per-store broadcast channel used to notify subscribers
/// (e.g. the web SSE endpoint) of account-state changes.
const NOTIFY_CAPACITY: usize = 64;

/// Thread-safe store for live account state.
///
/// The store keeps one [`AccountLiveState`] per [`AccountKey`] and exposes
/// snapshot reads so the web layer never blocks behind a write. WebSocket
/// handlers replace the per-account snapshot atomically under a write lock.
///
/// Every mutation also publishes the affected [`AccountKey`] on a
/// `tokio::sync::broadcast` channel so subscribers can be notified of
/// changes without having to poll the store.
#[derive(Debug, Clone)]
pub struct LiveAccountStore {
    inner: Arc<RwLock<HashMap<AccountKey, AccountLiveState>>>,
    notifier: Arc<broadcast::Sender<AccountKey>>,
}

impl Default for LiveAccountStore {
    fn default() -> Self {
        Self::new()
    }
}

impl LiveAccountStore {
    pub fn new() -> Self {
        let (tx, _rx) = broadcast::channel(NOTIFY_CAPACITY);
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
            notifier: Arc::new(tx),
        }
    }

    /// Subscribe to notifications emitted whenever the store is mutated.
    ///
    /// The returned [`broadcast::Receiver`] yields the [`AccountKey`] of
    /// every account whose state changed. Subscribers can read the
    /// corresponding snapshot via [`Self::get`].
    pub fn subscribe(&self) -> broadcast::Receiver<AccountKey> {
        self.notifier.subscribe()
    }

    /// Publish a notification for `key` to all current subscribers.
    ///
    /// A send with zero receivers is a no-op; we silently drop the error so
    /// producers do not need to special-case "nobody is listening".
    fn notify(&self, key: &AccountKey) {
        let _ = self.notifier.send(key.clone());
    }

    /// Build the initial empty state for an account, returning a clone of the
    /// new state. If a state already exists, the existing state is returned
    /// unchanged.
    #[cfg(test)]
    pub fn ensure_account(&self, key: &AccountKey) -> AccountLiveState {
        {
            let guard = self.inner.read().expect("live account store poisoned");
            if let Some(existing) = guard.get(key).cloned() {
                return existing;
            }
        }

        let mut guard = self.inner.write().expect("live account store poisoned");
        guard
            .entry(key.clone())
            .or_insert_with(|| AccountLiveState {
                account_address: key.account_address.clone(),
                environment: key.environment.clone(),
                status: LiveConnectionStatus::Starting,
                ..Default::default()
            })
            .clone()
    }

    /// Atomically replace the stored state for `key`.
    pub fn replace(&self, key: AccountKey, state: AccountLiveState) {
        {
            let mut guard = self.inner.write().expect("live account store poisoned");
            guard.insert(key.clone(), state);
        }
        self.notify(&key);
    }

    /// Update the stored state for `key` in place if it already exists, otherwise
    /// store a fresh `Starting` state. Returns the updated state.
    pub fn upsert<F>(&self, key: AccountKey, mutate: F) -> AccountLiveState
    where
        F: FnOnce(&mut AccountLiveState),
    {
        let updated = {
            let mut guard = self.inner.write().expect("live account store poisoned");
            let state = guard
                .entry(key.clone())
                .or_insert_with(|| AccountLiveState {
                    account_address: key.account_address.clone(),
                    environment: key.environment.clone(),
                    status: LiveConnectionStatus::Starting,
                    ..Default::default()
                });
            mutate(state);
            state.clone()
        };
        self.notify(&key);
        updated
    }

    /// Set the connection status for an account, creating the state on demand.
    pub fn set_status(&self, key: &AccountKey, status: LiveConnectionStatus) -> AccountLiveState {
        let updated = {
            let mut guard = self.inner.write().expect("live account store poisoned");
            let state = guard
                .entry(key.clone())
                .or_insert_with(|| AccountLiveState {
                    account_address: key.account_address.clone(),
                    environment: key.environment.clone(),
                    status,
                    ..Default::default()
                });
            state.status = status;
            if matches!(status, LiveConnectionStatus::Connected) && state.connected_at.is_none() {
                state.connected_at = Some(Utc::now());
            }
            state.clone()
        };
        self.notify(key);
        updated
    }

    /// Record the most recent error for an account without changing status.
    pub fn record_error(&self, key: &AccountKey, message: impl Into<String>) {
        {
            let mut guard = self.inner.write().expect("live account store poisoned");
            let state = guard
                .entry(key.clone())
                .or_insert_with(|| AccountLiveState {
                    account_address: key.account_address.clone(),
                    environment: key.environment.clone(),
                    ..Default::default()
                });
            state.last_error = Some(message.into());
        }
        self.notify(key);
    }

    /// Snapshot read of a single account. Returns `None` if not present.
    pub fn get(&self, key: &AccountKey) -> Option<AccountLiveState> {
        let guard = self.inner.read().expect("live account store poisoned");
        guard.get(key).cloned()
    }

    /// Snapshot read of all stored accounts.
    #[cfg(test)]
    pub fn snapshot(&self) -> Vec<AccountLiveState> {
        let guard = self.inner.read().expect("live account store poisoned");
        guard.values().cloned().collect()
    }

    /// Remove the stored entry for `key`, returning the previous value if any.
    pub fn remove(&self, key: &AccountKey) -> Option<AccountLiveState> {
        let mut guard = self.inner.write().expect("live account store poisoned");
        guard.remove(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(addr: &str) -> AccountKey {
        AccountKey::new(addr, "live")
    }

    #[test]
    fn ensure_account_creates_starting_state() {
        let store = LiveAccountStore::new();
        let k = key("0xabc");
        let state = store.ensure_account(&k);
        assert_eq!(state.account_address, "0xabc");
        assert_eq!(state.environment, "live");
        assert_eq!(state.status, LiveConnectionStatus::Starting);

        let state2 = store.ensure_account(&k);
        assert_eq!(state, state2);
    }

    #[test]
    fn set_status_creates_and_updates_state() {
        let store = LiveAccountStore::new();
        let k = key("0xdef");
        let state = store.set_status(&k, LiveConnectionStatus::Connected);
        assert_eq!(state.status, LiveConnectionStatus::Connected);
        assert!(state.connected_at.is_some());
        assert!(state.connected_at.is_some());
    }

    #[test]
    fn status_transitions_preserve_account_keys() {
        let store = LiveAccountStore::new();
        let k = key("0x1");
        store.set_status(&k, LiveConnectionStatus::StartupSyncing);
        store.set_status(&k, LiveConnectionStatus::Connecting);
        let final_state = store.set_status(&k, LiveConnectionStatus::Connected);
        assert_eq!(final_state.account_address, "0x1");
        assert_eq!(final_state.environment, "live");
        assert_eq!(final_state.status, LiveConnectionStatus::Connected);
        assert!(final_state.connected_at.is_some());
    }

    #[test]
    fn replace_isolates_accounts_by_key() {
        let store = LiveAccountStore::new();
        let a = key("0xa");
        let b = key("0xb");
        store.replace(
            a.clone(),
            AccountLiveState {
                account_address: a.account_address.clone(),
                environment: a.environment.clone(),
                status: LiveConnectionStatus::Connected,
                margin: Some(LiveMarginState {
                    account_value: Some(Decimal::new(100, 0)),
                    ..Default::default()
                }),
                ..Default::default()
            },
        );
        store.replace(
            b.clone(),
            AccountLiveState {
                account_address: b.account_address.clone(),
                environment: b.environment.clone(),
                status: LiveConnectionStatus::Failed,
                last_error: Some("boom".to_string()),
                ..Default::default()
            },
        );

        let snap_a = store.get(&a).expect("account a present");
        let snap_b = store.get(&b).expect("account b present");
        assert_eq!(snap_a.status, LiveConnectionStatus::Connected);
        assert_eq!(
            snap_a.margin.as_ref().and_then(|m| m.account_value),
            Some(Decimal::new(100, 0))
        );
        assert_eq!(snap_b.status, LiveConnectionStatus::Failed);
        assert_eq!(snap_b.last_error.as_deref(), Some("boom"));
    }

    #[test]
    fn record_error_does_not_change_status() {
        let store = LiveAccountStore::new();
        let k = key("0xerr");
        store.set_status(&k, LiveConnectionStatus::Connected);
        store.record_error(&k, "oops");
        let snap = store.get(&k).expect("state present");
        assert_eq!(snap.status, LiveConnectionStatus::Connected);
        assert_eq!(snap.last_error.as_deref(), Some("oops"));
    }

    #[test]
    fn health_requires_each_subscription_before_reporting_a_current_account() {
        let now = Utc::now();
        let mut state = AccountLiveState {
            status: LiveConnectionStatus::Connected,
            clearinghouse_updated_at: Some(now),
            ..Default::default()
        };

        let health = account_live_health_at(&state, now);
        assert_eq!(health.positions, LiveDataStatus::Current);
        assert_eq!(health.open_orders, LiveDataStatus::Loading);
        assert_eq!(health.balance, LiveDataStatus::Loading);
        assert_eq!(health.status, LiveAccountHealthStatus::Loading);

        state.open_orders_updated_at = Some(now);
        state.spot_updated_at = Some(now);
        let health = account_live_health_at(&state, now);
        assert_eq!(health.status, LiveAccountHealthStatus::Healthy);
    }

    #[test]
    fn health_marks_recent_data_degraded_when_monitoring_is_not_connected() {
        let now = Utc::now();
        let state = AccountLiveState {
            status: LiveConnectionStatus::Failed,
            clearinghouse_updated_at: Some(now),
            open_orders_updated_at: Some(now),
            spot_updated_at: Some(now),
            ..Default::default()
        };

        let health = account_live_health_at(&state, now);
        assert_eq!(health.status, LiveAccountHealthStatus::Failed);
        assert_eq!(health.positions, LiveDataStatus::Degraded);
        assert_eq!(health.open_orders, LiveDataStatus::Degraded);
    }

    #[test]
    fn health_marks_old_connected_data_stale() {
        let now = Utc::now();
        let state = AccountLiveState {
            status: LiveConnectionStatus::Connected,
            clearinghouse_updated_at: Some(
                now - ACCOUNT_DATA_MAX_AGE - chrono::Duration::seconds(1),
            ),
            open_orders_updated_at: Some(now),
            spot_updated_at: Some(now),
            ..Default::default()
        };

        let health = account_live_health_at(&state, now);
        assert_eq!(health.positions, LiveDataStatus::Stale);
        assert_eq!(health.balance, LiveDataStatus::Stale);
        assert_eq!(health.status, LiveAccountHealthStatus::Stale);
    }

    #[test]
    fn snapshot_returns_all_stored_accounts() {
        let store = LiveAccountStore::new();
        store.set_status(&key("0x1"), LiveConnectionStatus::Connected);
        store.set_status(&key("0x2"), LiveConnectionStatus::Reconnecting);
        let snap = store.snapshot();
        assert_eq!(snap.len(), 2);
    }

    #[test]
    fn remove_drops_account() {
        let store = LiveAccountStore::new();
        let k = key("0xgone");
        store.set_status(&k, LiveConnectionStatus::Connected);
        assert!(store.get(&k).is_some());
        let removed = store.remove(&k).expect("present");
        assert_eq!(removed.status, LiveConnectionStatus::Connected);
        assert!(store.get(&k).is_none());
    }

    #[test]
    fn upsert_mutates_existing_state() {
        let store = LiveAccountStore::new();
        let k = key("0xup");
        store.upsert(k.clone(), |state| {
            state.status = LiveConnectionStatus::Connected;
            state.spot_balances.push(LiveSpotBalance {
                coin: "USDC".to_string(),
                total: Some(Decimal::new(50, 0)),
                ..Default::default()
            });
        });
        let snap = store.get(&k).expect("present");
        assert_eq!(snap.status, LiveConnectionStatus::Connected);
        assert_eq!(snap.spot_balances.len(), 1);
        assert_eq!(snap.spot_balances[0].coin, "USDC");
    }

    #[tokio::test]
    async fn mutations_notify_subscribers_with_account_key() {
        let store = LiveAccountStore::new();
        let mut rx = store.subscribe();
        let k = key("0xnotify");

        store.set_status(&k, LiveConnectionStatus::Connecting);
        let notified = rx.recv().await.expect("first notification");
        assert_eq!(notified, k);

        store.replace(
            k.clone(),
            AccountLiveState {
                account_address: k.account_address.clone(),
                environment: k.environment.clone(),
                status: LiveConnectionStatus::Connected,
                margin: Some(LiveMarginState {
                    account_value: Some(Decimal::new(42, 0)),
                    ..Default::default()
                }),
                ..Default::default()
            },
        );
        let notified = rx.recv().await.expect("second notification");
        assert_eq!(notified, k);

        store.upsert(k.clone(), |state| {
            state.spot_balances.push(LiveSpotBalance {
                coin: "USDC".to_string(),
                ..Default::default()
            });
        });
        let notified = rx.recv().await.expect("third notification");
        assert_eq!(notified, k);

        store.record_error(&k, "boom");
        let notified = rx.recv().await.expect("fourth notification");
        assert_eq!(notified, k);
    }

    #[tokio::test]
    async fn notify_is_noop_when_no_subscribers() {
        let store = LiveAccountStore::new();
        let k = key("0xnobody");
        // Should not panic even though nobody is subscribed.
        store.set_status(&k, LiveConnectionStatus::Connected);
        store.upsert(k.clone(), |state| {
            state.clearinghouse_updated_at = Some(Utc::now());
        });
        store.record_error(&k, "ignore me");
        store.replace(
            k.clone(),
            AccountLiveState {
                account_address: k.account_address.clone(),
                environment: k.environment.clone(),
                status: LiveConnectionStatus::Stopped,
                ..Default::default()
            },
        );
        assert_eq!(
            store.get(&k).expect("present").status,
            LiveConnectionStatus::Stopped
        );
    }
}
