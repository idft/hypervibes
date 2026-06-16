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

/// Full per-account live state.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AccountLiveState {
    pub account_address: String,
    pub environment: String,
    pub status: LiveConnectionStatus,
    pub connected_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub margin: Option<LiveMarginState>,
    pub spot_balances: Vec<LiveSpotBalance>,
    pub open_positions: Vec<LivePosition>,
    pub open_orders: Vec<LiveOpenOrder>,
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
    #[allow(dead_code)]
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
            if matches!(status, LiveConnectionStatus::Connected) {
                if state.connected_at.is_none() {
                    state.connected_at = Some(Utc::now());
                }
                state.last_error = None;
            }
            state.updated_at = Some(Utc::now());
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
            state.updated_at = Some(Utc::now());
        }
        self.notify(key);
    }

    /// Snapshot read of a single account. Returns `None` if not present.
    pub fn get(&self, key: &AccountKey) -> Option<AccountLiveState> {
        let guard = self.inner.read().expect("live account store poisoned");
        guard.get(key).cloned()
    }

    /// Snapshot read of all stored accounts.
    #[allow(dead_code)]
    pub fn snapshot(&self) -> Vec<AccountLiveState> {
        let guard = self.inner.read().expect("live account store poisoned");
        guard.values().cloned().collect()
    }

    /// Remove the stored entry for `key`, returning the previous value if any.
    #[allow(dead_code)]
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
        assert!(state.updated_at.is_some());
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
            state.updated_at = Some(Utc::now());
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
