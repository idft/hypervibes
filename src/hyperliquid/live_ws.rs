//! Live Hyperliquid WebSocket loop driven by `hypersdk`.
//!
//! This module:
//!
//! - creates a mainnet `hypersdk` WebSocket based on the agent's
//!   [`HyperliquidEnvironment`];
//! - subscribes to the account-state and transaction channels needed for the
//!   orchestrator's live view;
//! - updates the in-memory [`LiveAccountStore`] on every relevant event;
//! - upserts fills, funding, and ledger events into the existing durable
//!   journal tables via [`crate::hyperliquid::account_sync`];
//! - performs one HTTP catch-up sync after a reconnect (configurable via
//!   [`LiveWsOptions`]);
//! - exits cleanly on shutdown.
//!
//! The module never exposes `hypersdk` types to the rest of the app: live
//! state is converted through [`crate::hyperliquid::live_convert`], and
//! transaction events are stored in the existing
//! [`crate::hyperliquid::normalize`] row structs.

use std::sync::Arc;

use anyhow::{Context, Result};
use futures::StreamExt;
use hypersdk::{
    Address,
    hypercore::{self, types as htypes, ws as hws},
};
use tokio::sync::watch;
use tracing::{error, info, warn};

use crate::{
    db::DbPool,
    hyperliquid::{
        account_sync::{
            InstrumentLookupMap, sync_account_once, sync_historical_orders_once,
            upsert_funding_event, upsert_trade_fill,
        },
        config::{AccountSyncConfig, HyperliquidEnvironment},
        live_convert::{
            funding_event_row_from_hypersdk_funding, live_open_orders_from_orders,
            live_spot_balances_from_spot_state, live_state_from_clearinghouse,
            trade_fill_row_from_hypersdk_fill,
        },
        live_state::{AccountKey, AccountLiveState, LiveAccountStore, LiveConnectionStatus},
        raw_http::{RawHttpConfig, RawHyperliquidHttpClient},
    },
};

/// Tunable options for the live WebSocket loop.
#[derive(Debug, Clone)]
pub struct LiveWsOptions {
    /// Whether to perform one HTTP catch-up sync after a WS reconnect.
    pub reconnect_catchup: bool,
    /// Whether to upsert WS transaction events into the durable journal
    /// tables. Live in-memory state is always updated; this flag controls
    /// whether the same events are also persisted.
    pub persist_journal: bool,
}

impl Default for LiveWsOptions {
    fn default() -> Self {
        Self {
            reconnect_catchup: true,
            persist_journal: true,
        }
    }
}

/// Run the live WebSocket loop for a single account until shutdown is signalled.
///
/// This is intended to be invoked by the orchestrator once the startup HTTP
/// sync has completed. After a successful `Event::Connected` (or first
/// connection), the loop drives the `hypersdk` stream until a disconnect is
/// observed, at which point it (optionally) runs a single HTTP catch-up sync
/// to repair any gap, then continues processing incoming events.
pub async fn run_account_live_ws(
    pool: DbPool,
    config: AccountSyncConfig,
    lookup: Arc<InstrumentLookupMap>,
    raw_http: Arc<RawHyperliquidHttpClient>,
    live_store: Arc<LiveAccountStore>,
    mut shutdown_rx: watch::Receiver<bool>,
    options: LiveWsOptions,
) -> Result<()> {
    let account_key = AccountKey::new(
        config.account_address.clone(),
        config.environment.as_journal_str(),
    );

    info!(
        agent_address = %account_key.account_address,
        environment = %account_key.environment,
        "starting live WebSocket loop"
    );

    live_store.set_status(&account_key, LiveConnectionStatus::Connecting);

    let address = parse_address(&account_key.account_address)?;
    let mut ws = open_connection(config.environment);
    subscribe_account(&mut ws, address);

    let mut reconnected_after_gap = false;
    let mut stream = std::pin::pin!(ws);

    loop {
        tokio::select! {
            biased;
            changed = shutdown_rx.changed() => {
                if changed.is_ok() && *shutdown_rx.borrow() {
                    info!(
                        agent_address = %account_key.account_address,
                        "live WebSocket loop received shutdown signal"
                    );
                    live_store.set_status(&account_key, LiveConnectionStatus::Stopped);
                    return Ok(());
                }
            }
            event = stream.next() => {
                let Some(event) = event else {
                    info!(
                        agent_address = %account_key.account_address,
                        "live WebSocket stream ended"
                    );
                    live_store.set_status(&account_key, LiveConnectionStatus::Stopped);
                    return Ok(());
                };
                match event {
                    hws::Event::Connected => {
                        info!(
                            agent_address = %account_key.account_address,
                            "live WebSocket connected"
                        );
                        if reconnected_after_gap && options.reconnect_catchup {
                            if let Err(e) = run_reconnect_catchup(
                                &pool,
                                &config,
                                &lookup,
                                &raw_http,
                                &live_store,
                                &account_key,
                            ).await {
                                warn!(
                                    agent_address = %account_key.account_address,
                                    error = ?e,
                                    "live WebSocket reconnect catch-up failed"
                                );
                            }
                            reconnected_after_gap = false;
                        } else {
                            reconnected_after_gap = false;
                        }
                        live_store.set_status(&account_key, LiveConnectionStatus::Connected);
                    }
                    hws::Event::Disconnected => {
                        info!(
                            agent_address = %account_key.account_address,
                            "live WebSocket disconnected; will auto-reconnect"
                        );
                        live_store.set_status(&account_key, LiveConnectionStatus::Reconnecting);
                        reconnected_after_gap = true;
                    }
                    hws::Event::Message(message) => {
                        if let Err(e) = handle_message(
                            &pool,
                            &config,
                            &lookup,
                            &live_store,
                            &account_key,
                            message,
                            options.persist_journal,
                        ).await {
                            warn!(
                                agent_address = %account_key.account_address,
                                error = ?e,
                                "failed to process live WebSocket message"
                            );
                            live_store.record_error(&account_key, format!("{e:#}"));
                        }
                    }
                }
            }
        }
    }
}

fn open_connection(environment: HyperliquidEnvironment) -> hws::Connection {
    match environment {
        HyperliquidEnvironment::Mainnet => hypercore::mainnet_ws(),
    }
}

fn parse_address(value: &str) -> Result<Address> {
    value
        .parse::<Address>()
        .with_context(|| format!("invalid Hyperliquid account address: {value}"))
}

fn subscribe_account(ws: &mut hws::Connection, address: Address) {
    ws.subscribe(htypes::Subscription::UserFills { user: address });
    // `UserEvents` carries `Funding`, `Liquidation`, and `NonUserCancel`
    // events for the user. Hypersdk does not currently expose a dedicated
    // `UserFundings` or `UserNonFundingLedgerUpdates` WebSocket channel,
    // so funding events are harvested from `UserEvents` and the remaining
    // non-funding ledger updates continue to be captured by the HTTP
    // catch-up sync.
    ws.subscribe(htypes::Subscription::UserEvents { user: address });
    ws.subscribe(htypes::Subscription::ClearinghouseState {
        user: address,
        dex: None,
    });
    ws.subscribe(htypes::Subscription::SpotState {
        user: address,
        is_portfolio_margin: None,
    });
    ws.subscribe(htypes::Subscription::OpenOrders {
        user: address,
        dex: None,
    });
}

#[allow(clippy::too_many_arguments)]
async fn handle_message(
    pool: &DbPool,
    config: &AccountSyncConfig,
    lookup: &InstrumentLookupMap,
    live_store: &Arc<LiveAccountStore>,
    account_key: &AccountKey,
    message: htypes::Incoming,
    persist_journal: bool,
) -> Result<()> {
    match message {
        htypes::Incoming::ClearinghouseState {
            clearinghouse_state,
            ..
        } => {
            let mut state = live_state_from_clearinghouse(
                account_key,
                &clearinghouse_state.cross_margin_summary,
                clearinghouse_state.cross_maintenance_margin_used,
                clearinghouse_state.withdrawable,
                &clearinghouse_state.asset_positions,
            );
            merge_existing_snapshot(live_store, account_key, &mut state);
            live_store.replace(account_key.clone(), state);
        }
        htypes::Incoming::SpotState { spot_state, .. } => {
            let balances = live_spot_balances_from_spot_state(&spot_state);
            live_store.upsert(account_key.clone(), |state| {
                state.spot_balances = balances.clone();
                state.updated_at = Some(chrono::Utc::now());
            });
        }
        htypes::Incoming::OpenOrders { orders, .. } => {
            let live_orders = live_open_orders_from_orders(&orders);
            live_store.upsert(account_key.clone(), |state| {
                state.open_orders = live_orders.clone();
                state.updated_at = Some(chrono::Utc::now());
            });
        }
        htypes::Incoming::UserFills {
            is_snapshot,
            user: _,
            fills,
        } => {
            if is_snapshot {
                info!(
                    agent_address = %account_key.account_address,
                    fills = fills.len(),
                    "received initial UserFills snapshot; will be repaired by reconnect catch-up if enabled"
                );
            }
            for fill in fills {
                let row = trade_fill_row_from_hypersdk_fill(config, lookup, &fill)
                    .context("failed to convert hypersdk fill")?;
                if persist_journal {
                    upsert_trade_fill(pool, row).await?;
                } else {
                    drop(row);
                }
            }
        }
        htypes::Incoming::UserEvents(event) => {
            if let Some(row) =
                user_funding_to_ledger(config, lookup, &event, &account_key.account_address)
                && persist_journal
            {
                upsert_funding_event(pool, row).await?;
            }
        }
        htypes::Incoming::OrderUpdates(_) | htypes::Incoming::UserTwapSliceFills(_) => {
            // Order-level deltas are not upserted into the durable journal in
            // this iteration; the live open-order snapshot above stays the
            // source of truth for the UI.
        }
        other => {
            // Other event types are ignored for the in-memory state. Logged
            // at debug to keep the noise low in production logs.
            tracing::debug!(
                agent_address = %account_key.account_address,
                event = ?other,
                "ignoring hypersdk incoming variant"
            );
        }
    }
    Ok(())
}

fn user_funding_to_ledger(
    config: &AccountSyncConfig,
    lookup: &InstrumentLookupMap,
    event: &htypes::UserEvent,
    _account_address: &str,
) -> Option<crate::hyperliquid::normalize::FundingEventRow> {
    use htypes::UserEvent;
    match event {
        UserEvent::Funding { funding } => {
            funding_event_row_from_hypersdk_funding(config, lookup, funding)
                .ok()
                .flatten()
        }
        UserEvent::Fills { .. }
        | UserEvent::Liquidation { .. }
        | UserEvent::NonUserCancel { .. }
        | UserEvent::Unknown(_) => None,
    }
}

fn merge_existing_snapshot(
    live_store: &Arc<LiveAccountStore>,
    account_key: &AccountKey,
    new_state: &mut AccountLiveState,
) {
    if let Some(existing) = live_store.get(account_key) {
        if new_state.margin.is_none() {
            new_state.margin = existing.margin;
        }
        if new_state.spot_balances.is_empty() {
            new_state.spot_balances = existing.spot_balances;
        }
        if new_state.open_orders.is_empty() {
            new_state.open_orders = existing.open_orders;
        }
        new_state.connected_at = existing.connected_at;
    }
}

#[allow(dead_code)]
async fn run_reconnect_catchup(
    pool: &DbPool,
    config: &AccountSyncConfig,
    lookup: &InstrumentLookupMap,
    raw_http: &RawHyperliquidHttpClient,
    live_store: &Arc<LiveAccountStore>,
    account_key: &AccountKey,
) -> Result<()> {
    info!(
        agent_address = %account_key.account_address,
        "running one-shot HTTP catch-up sync after WS reconnect"
    );
    live_store.set_status(account_key, LiveConnectionStatus::StartupSyncing);

    match sync_account_once(pool, config, raw_http, lookup).await {
        Ok(summary) => {
            for stream in summary.streams {
                if let Some(err) = &stream.error {
                    warn!(
                        agent_address = %account_key.account_address,
                        stream = %stream.stream.as_str(),
                        error = %err,
                        "reconnect catch-up stream reported error"
                    );
                }
            }
        }
        Err(e) => {
            error!(
                agent_address = %account_key.account_address,
                error = ?e,
                "reconnect catch-up sync_account_once failed"
            );
        }
    }

    if let Err(e) = sync_historical_orders_once(pool, config, raw_http, lookup).await {
        warn!(
            agent_address = %account_key.account_address,
            error = ?e,
            "reconnect catch-up historical orders sync failed"
        );
    }

    Ok(())
}

/// Build a raw HTTP client suitable for catch-up sync.
///
/// This is exposed so the orchestrator can construct the client once and
/// share it between the startup sync and the live WebSocket loop.
#[allow(dead_code)]
pub fn build_raw_http_client(
    environment: HyperliquidEnvironment,
    account_address: String,
) -> RawHyperliquidHttpClient {
    RawHyperliquidHttpClient::new(RawHttpConfig {
        environment,
        account_address,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hyperliquid::config::HyperliquidEnvironment;
    use crate::hyperliquid::live_state::{LiveAccountStore, LiveConnectionStatus};

    #[test]
    fn parse_address_accepts_lowercase_hex() {
        let addr = parse_address("0x8f0bb61c41988b44f623a0b5390fd2b52838d20e").expect("parses");
        let expected: Address = "0x8f0bb61c41988b44f623a0b5390fd2b52838d20e"
            .parse()
            .expect("parses");
        assert_eq!(addr, expected);
    }

    #[test]
    fn parse_address_rejects_garbage() {
        let err = parse_address("not-an-address").expect_err("must fail");
        assert!(format!("{err:#}").contains("invalid Hyperliquid account address"));
    }

    #[tokio::test]
    async fn open_connection_routes_to_mainnet() {
        // The constructors don't open a connection until first polled; we
        // just need to make sure the routing function returns something for
        // the supported environment.
        let _ = open_connection(HyperliquidEnvironment::Mainnet);
    }

    #[test]
    fn merge_existing_snapshot_preserves_unset_fields() {
        let store = Arc::new(LiveAccountStore::new());
        let key = AccountKey::new("0xabc", "live");
        store.replace(
            key.clone(),
            AccountLiveState {
                account_address: key.account_address.clone(),
                environment: key.environment.clone(),
                status: LiveConnectionStatus::Connected,
                connected_at: Some(chrono::Utc::now()),
                spot_balances: vec![crate::hyperliquid::live_state::LiveSpotBalance {
                    coin: "USDC".to_string(),
                    total: Some(rust_decimal::Decimal::new(1, 0)),
                    ..Default::default()
                }],
                open_orders: vec![crate::hyperliquid::live_state::LiveOpenOrder {
                    coin: "BTC".to_string(),
                    ..Default::default()
                }],
                ..Default::default()
            },
        );
        let mut new_state = AccountLiveState {
            account_address: key.account_address.clone(),
            environment: key.environment.clone(),
            status: LiveConnectionStatus::Connected,
            margin: Some(crate::hyperliquid::live_state::LiveMarginState {
                account_value: Some(rust_decimal::Decimal::new(42, 0)),
                ..Default::default()
            }),
            ..Default::default()
        };
        merge_existing_snapshot(&store, &key, &mut new_state);
        assert_eq!(new_state.spot_balances.len(), 1);
        assert_eq!(new_state.open_orders.len(), 1);
        assert!(new_state.connected_at.is_some());
        assert_eq!(
            new_state.margin.as_ref().and_then(|m| m.account_value),
            Some(rust_decimal::Decimal::new(42, 0))
        );
    }

    #[test]
    fn user_funding_to_ledger_returns_none_for_fills() {
        let config = AccountSyncConfig {
            account_address: "0xtest".to_string(),
            environment: HyperliquidEnvironment::Mainnet,
            history_start_ms: 0,
            overlap_ms: 0,
        };
        let lookup: InstrumentLookupMap = std::collections::HashMap::new();
        let event = htypes::UserEvent::Fills { fills: vec![] };
        let result = user_funding_to_ledger(&config, &lookup, &event, "0xtest");
        assert!(result.is_none());
    }

    #[test]
    fn address_from_str_works() {
        // Sanity check: alloy's Address::from_str parses lowercase hex.
        let _: Address = "0x8f0bb61c41988b44f623a0b5390fd2b52838d20e"
            .parse()
            .expect("valid hex address");
    }
}
