//! Adapter conversions between `hypersdk` types and the app's domain types.
//!
//! `hypersdk` is treated strictly as an adapter: the types it exposes are
//! converted into the app's [`crate::hyperliquid::live_state`] domain types
//! (for in-memory live state) and into existing durable
//! [`crate::hyperliquid::normalize`] row types (for journal writes). No
//! `hypersdk` type is exposed to the web layer, DB schema, or templates.

use anyhow::Result;
use chrono::Utc;
use rust_decimal::Decimal;

use crate::hyperliquid::{
    account_sync::InstrumentLookupMap,
    config::AccountSyncConfig,
    live_state::{
        AccountKey, AccountLiveState, LiveMarginState, LiveOpenOrder, LivePosition, LiveSpotBalance,
    },
    normalize::{FundingEventRow, TradeFillRow},
};

/// Map a `hypersdk` [`ClearinghouseState`](hypersdk::hypercore::types::ClearinghouseState)
/// into the app's [`AccountLiveState`] margin summary, returning a fully
/// assembled live state for the account.
pub fn live_state_from_clearinghouse(
    account_key: &AccountKey,
    cross_margin_summary: &hypersdk::hypercore::types::MarginSummary,
    cross_maintenance_margin_used: Decimal,
    withdrawable: Decimal,
    asset_positions: &[hypersdk::hypercore::types::AssetPosition],
) -> AccountLiveState {
    AccountLiveState {
        account_address: account_key.account_address.clone(),
        environment: account_key.environment.clone(),
        status: crate::hyperliquid::live_state::LiveConnectionStatus::Connected,
        connected_at: None,
        updated_at: Some(Utc::now()),
        last_error: None,
        margin: Some(LiveMarginState {
            account_value: Some(cross_margin_summary.account_value),
            total_ntl_pos: Some(cross_margin_summary.total_ntl_pos),
            total_raw_usd: Some(cross_margin_summary.total_raw_usd),
            total_margin_used: Some(cross_margin_summary.total_margin_used),
            cross_maintenance_margin_used: Some(cross_maintenance_margin_used),
            withdrawable: Some(withdrawable),
            updated_at: Some(Utc::now()),
        }),
        spot_balances: Vec::new(),
        open_positions: asset_positions
            .iter()
            .map(live_position_from_asset)
            .collect(),
        open_orders: Vec::new(),
    }
}

fn live_position_from_asset(
    asset_position: &hypersdk::hypercore::types::AssetPosition,
) -> LivePosition {
    let pos = &asset_position.position;
    LivePosition {
        coin: pos.coin.clone(),
        szi: Some(pos.szi),
        entry_px: pos.entry_px,
        unrealized_pnl: Some(pos.unrealized_pnl),
        liquidation_px: pos.liquidation_px,
        margin_used: Some(pos.margin_used),
        position_value: Some(pos.position_value),
        return_on_equity: Some(pos.return_on_equity),
        leverage_type: Some(leverage_type_str(&pos.leverage.leverage_type).to_string()),
        leverage_value: Some(pos.leverage.value),
        max_leverage: Some(pos.max_leverage),
    }
}

fn leverage_type_str(leverage_type: &hypersdk::hypercore::types::LeverageType) -> &'static str {
    use hypersdk::hypercore::types::LeverageType;
    match leverage_type {
        LeverageType::Cross => "cross",
        LeverageType::Isolated => "isolated",
    }
}

/// Map a `hypersdk` [`SpotState`](hypersdk::hypercore::types::SpotState) into a
/// vector of [`LiveSpotBalance`].
pub fn live_spot_balances_from_spot_state(
    spot_state: &hypersdk::hypercore::types::SpotState,
) -> Vec<LiveSpotBalance> {
    spot_state
        .balances
        .iter()
        .map(|balance| LiveSpotBalance {
            coin: balance.coin.clone(),
            total: Some(balance.total),
            hold: Some(balance.hold),
            available: Some(balance.available()),
            entry_ntl: Some(balance.entry_ntl),
        })
        .collect()
}

/// Map a list of `hypersdk` [`OpenOrder`] into a vector of
/// [`LiveOpenOrder`].
pub fn live_open_orders_from_orders(
    orders: &[hypersdk::hypercore::types::OpenOrder],
) -> Vec<LiveOpenOrder> {
    orders.iter().map(live_open_order_from_order).collect()
}

fn live_open_order_from_order(order: &hypersdk::hypercore::types::OpenOrder) -> LiveOpenOrder {
    let basic = &order.basic_order;
    let order_type_str = order_type_to_str(&basic.order_type);
    let tif_str = basic.tif.as_ref().map(tif_to_str);
    LiveOpenOrder {
        coin: basic.coin.clone(),
        side: Some(side_to_str(&basic.side).to_string()),
        limit_px: Some(basic.limit_px),
        sz: Some(basic.sz),
        orig_sz: Some(basic.orig_sz),
        oid: Some(basic.oid.to_string()),
        timestamp: Some(basic.timestamp),
        cloid: basic.cloid.map(|c| c.to_string()),
        order_type: Some(order_type_str.to_string()),
        tif: tif_str.map(|s| s.to_string()),
        reduce_only: Some(basic.reduce_only),
        is_trigger: basic.is_trigger,
        trigger_px: basic.trigger_px,
        trigger_condition: basic.trigger_condition.clone(),
        is_position_tpsl: basic.is_position_tpsl,
    }
}

fn side_to_str(side: &hypersdk::hypercore::types::Side) -> &'static str {
    use hypersdk::hypercore::types::Side;
    match side {
        Side::Bid => "buy",
        Side::Ask => "sell",
    }
}

fn tif_to_str(tif: &hypersdk::hypercore::types::TimeInForce) -> &'static str {
    use hypersdk::hypercore::types::TimeInForce;
    match tif {
        TimeInForce::Gtc => "Gtc",
        TimeInForce::Ioc => "Ioc",
        TimeInForce::Alo => "Alo",
        TimeInForce::FrontendMarket => "FrontendMarket",
    }
}

fn order_type_to_str(order_type: &hypersdk::hypercore::types::OrderType) -> &'static str {
    use hypersdk::hypercore::types::OrderType;
    match order_type {
        OrderType::Limit => "limit",
        OrderType::Market => "market",
        OrderType::Trigger => "trigger",
        OrderType::StopMarket => "stop_market",
        OrderType::StopLimit => "stop_limit",
        OrderType::TakeProfitMarket => "take_profit_market",
        OrderType::TakeProfitLimit => "take_profit_limit",
    }
}

/// Convert a `hypersdk` [`Fill`](hypersdk::hypercore::types::Fill) into a
/// durable [`TradeFillRow`] for upsert into `hyperliquid.trade_fills`.
pub fn trade_fill_row_from_hypersdk_fill(
    config: &AccountSyncConfig,
    lookup: &InstrumentLookupMap,
    fill: &hypersdk::hypercore::types::Fill,
) -> Result<TradeFillRow> {
    let now = Utc::now();
    let trade_id = fill.tid.to_string();
    let oid = fill.oid.to_string();
    let direction = fill.dir.as_str().to_string();
    let side = side_to_str(&fill.side).to_string();
    let symbol_lookup = lookup.get(&fill.coin);
    let (instrument_id, symbol, base_asset) = match symbol_lookup {
        Some((iid, sym, base)) => (Some(iid.clone()), Some(sym.clone()), Some(base.clone())),
        None => (None, None, None),
    };
    let payload = serde_json::to_value(fill)?;
    Ok(TradeFillRow {
        hash: fill.hash.clone(),
        account_address: config.account_address.clone(),
        environment: config.environment.as_journal_str().to_string(),
        event_time: crate::hyperliquid::normalize::ms_to_datetime(fill.time),
        event_type: "fill".to_string(),
        source_stream: "fills".to_string(),
        instrument_id,
        asset: base_asset.or_else(|| Some(fill.coin.clone())),
        symbol,
        fee_usdc: Some(fill.fee),
        realized_pnl_usdc: Some(fill.closed_pnl),
        fill_time: crate::hyperliquid::normalize::ms_to_datetime(fill.time),
        direction,
        side,
        price: fill.px,
        size: fill.sz,
        trade_value: Some(fill.px * fill.sz),
        order_id: Some(oid),
        trade_id,
        start_position: Some(fill.start_position),
        fee: Some(fill.fee),
        fee_token: Some(fill.fee_token.clone()),
        builder_fee: None,
        crossed: Some(fill.crossed),
        tx_hash: None,
        payload,
        ingest_source: "ws".to_string(),
        inserted_at: now,
    })
}

/// Convert a `hypersdk` `UserFunding` event into a durable
/// [`FundingEventRow`] for upsert into `hyperliquid.funding_events`.
pub fn funding_event_row_from_hypersdk_funding(
    config: &AccountSyncConfig,
    lookup: &InstrumentLookupMap,
    funding: &hypersdk::hypercore::types::UserFunding,
) -> Result<Option<FundingEventRow>> {
    let Some((instrument_id, symbol, base_asset)) = lookup
        .get(&funding.coin)
        .map(|(iid, sym, base)| (iid.clone(), Some(sym.clone()), Some(base.clone())))
    else {
        return Ok(None);
    };

    let now = Utc::now();
    let payload = serde_json::to_value(funding)?;

    Ok(Some(FundingEventRow {
        account_address: config.account_address.clone(),
        environment: config.environment.as_journal_str().to_string(),
        instrument_id,
        event_time: crate::hyperliquid::normalize::ms_to_datetime(funding.time),
        event_type: "funding".to_string(),
        source_stream: "funding".to_string(),
        asset: base_asset.or_else(|| Some(funding.coin.clone())),
        symbol,
        fee_usdc: None,
        realized_pnl_usdc: Some(funding.usdc),
        usdc: funding.usdc,
        position_size: Some(funding.szi),
        funding_rate: Some(funding.funding_rate),
        hash: None,
        payload,
        ingest_source: "ws".to_string(),
        inserted_at: now,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hypersdk::hypercore::types::{
        AssetPosition, BasicOrder, ClearinghouseState, CumulativeFunding, Fill, FillDirection,
        Leverage, LeverageType, MarginSummary, OpenOrder, OrderType, PositionData, PositionType,
        Side, SpotState, TimeInForce, UserBalance,
    };
    use rust_decimal::Decimal;
    use std::collections::HashMap;

    fn d(s: &str) -> Decimal {
        s.parse().expect("decimal")
    }

    fn instrument_lookup() -> InstrumentLookupMap {
        let mut map = HashMap::new();
        map.insert(
            "BTC".to_string(),
            (
                "hyperliquid:BTC".to_string(),
                "BTC".to_string(),
                "BTC".to_string(),
            ),
        );
        map
    }

    fn sample_fill() -> Fill {
        Fill {
            coin: "BTC".to_string(),
            px: d("30000"),
            sz: d("0.5"),
            side: Side::Bid,
            time: 1_700_000_000_000,
            start_position: d("0"),
            dir: FillDirection::OpenLong,
            closed_pnl: d("0"),
            hash: "0xfillhash".to_string(),
            oid: 42,
            crossed: true,
            fee: d("1.5"),
            tid: 7,
            cloid: None,
            fee_token: "USDC".to_string(),
            liquidation: None,
            builder_fee: None,
        }
    }

    fn config_for() -> AccountSyncConfig {
        AccountSyncConfig {
            account_address: "0xtest".to_string(),
            environment: crate::hyperliquid::config::HyperliquidEnvironment::Mainnet,
            history_start_ms: 0,
            overlap_ms: 0,
        }
    }

    #[test]
    fn trade_fill_row_conversion_preserves_fields() {
        let lookup = instrument_lookup();
        let config = config_for();
        let fill = sample_fill();
        let row = trade_fill_row_from_hypersdk_fill(&config, &lookup, &fill).expect("row");

        assert_eq!(row.hash, "0xfillhash");
        assert_eq!(row.account_address, "0xtest");
        assert_eq!(row.environment, "live");
        assert_eq!(row.symbol.as_deref(), Some("BTC"));
        assert_eq!(row.asset.as_deref(), Some("BTC"));
        assert_eq!(row.price, d("30000"));
        assert_eq!(row.size, d("0.5"));
        assert_eq!(row.side, "buy");
        assert_eq!(row.direction, "Open Long");
        assert_eq!(row.order_id.as_deref(), Some("42"));
        assert_eq!(row.trade_id, "7");
        assert_eq!(row.fee_usdc, Some(d("1.5")));
        assert_eq!(row.realized_pnl_usdc, Some(d("0")));
        assert_eq!(row.ingest_source, "ws");
        assert_eq!(row.trade_value, Some(d("15000")));
    }

    #[test]
    fn trade_fill_row_uses_coin_when_unknown() {
        let lookup = instrument_lookup();
        let config = config_for();
        let mut fill = sample_fill();
        fill.coin = "UNKNOWN".to_string();
        let row = trade_fill_row_from_hypersdk_fill(&config, &lookup, &fill).expect("row");
        assert!(row.instrument_id.is_none());
        assert_eq!(row.asset.as_deref(), Some("UNKNOWN"));
    }

    #[test]
    fn funding_event_row_skips_unknown_coin() {
        let lookup = instrument_lookup();
        let config = config_for();
        let funding = hypersdk::hypercore::types::UserFunding {
            time: 1_700_000_000_000,
            coin: "BTC".to_string(),
            usdc: d("2.5"),
            szi: d("0.5"),
            funding_rate: d("0.0001"),
        };
        let row = funding_event_row_from_hypersdk_funding(&config, &lookup, &funding)
            .expect("ok")
            .expect("present");
        assert_eq!(row.instrument_id, "hyperliquid:BTC");
        assert_eq!(row.usdc, d("2.5"));
        assert_eq!(row.ingest_source, "ws");

        let mut unknown = funding.clone();
        unknown.coin = "UNKNOWN".to_string();
        let row = funding_event_row_from_hypersdk_funding(&config, &lookup, &unknown).expect("ok");
        assert!(row.is_none());
    }

    #[test]
    fn live_spot_balances_from_spot_state_test() {
        let spot = SpotState {
            balances: vec![UserBalance {
                coin: "USDC".to_string(),
                token: Some(0),
                hold: d("10"),
                total: d("110"),
                entry_ntl: d("0"),
            }],
        };
        let balances = live_spot_balances_from_spot_state(&spot);
        assert_eq!(balances.len(), 1);
        assert_eq!(balances[0].coin, "USDC");
        assert_eq!(balances[0].total, Some(d("110")));
        assert_eq!(balances[0].hold, Some(d("10")));
        assert_eq!(balances[0].available, Some(d("100")));
    }

    #[test]
    fn live_state_from_clearinghouse_preserves_margin_and_positions() {
        let key = AccountKey::new("0xtest", "live");
        let clearinghouse = ClearinghouseState {
            margin_summary: MarginSummary {
                account_value: d("0"),
                total_ntl_pos: d("0"),
                total_raw_usd: d("0"),
                total_margin_used: d("0"),
            },
            cross_margin_summary: MarginSummary {
                account_value: d("1000"),
                total_ntl_pos: d("500"),
                total_raw_usd: d("1000"),
                total_margin_used: d("50"),
            },
            cross_maintenance_margin_used: d("25"),
            withdrawable: d("900"),
            asset_positions: vec![AssetPosition {
                position_type: PositionType::OneWay,
                position: PositionData {
                    coin: "BTC".to_string(),
                    szi: d("0.1"),
                    leverage: Leverage {
                        leverage_type: LeverageType::Cross,
                        value: 5,
                        raw_usd: None,
                    },
                    entry_px: Some(d("30000")),
                    position_value: d("3000"),
                    unrealized_pnl: d("10"),
                    return_on_equity: d("0.02"),
                    liquidation_px: Some(d("25000")),
                    margin_used: d("600"),
                    max_leverage: 50,
                    cum_funding: CumulativeFunding {
                        all_time: d("0"),
                        since_open: d("0"),
                        since_change: d("0"),
                    },
                },
            }],
            time: 0,
        };
        let state = live_state_from_clearinghouse(
            &key,
            &clearinghouse.cross_margin_summary,
            clearinghouse.cross_maintenance_margin_used,
            clearinghouse.withdrawable,
            &clearinghouse.asset_positions,
        );
        let margin = state.margin.expect("margin set");
        assert_eq!(margin.account_value, Some(d("1000")));
        assert_eq!(margin.cross_maintenance_margin_used, Some(d("25")));
        assert_eq!(margin.withdrawable, Some(d("900")));
        assert_eq!(state.open_positions.len(), 1);
        let pos = &state.open_positions[0];
        assert_eq!(pos.coin, "BTC");
        assert_eq!(pos.leverage_type.as_deref(), Some("cross"));
        assert_eq!(pos.leverage_value, Some(5));
        assert_eq!(pos.max_leverage, Some(50));
        assert_eq!(pos.entry_px, Some(d("30000")));
    }

    #[test]
    fn live_open_order_from_hypersdk_basic_order() {
        let basic = BasicOrder {
            timestamp: 1_700_000_000_000,
            coin: "ETH".to_string(),
            side: Side::Ask,
            limit_px: d("2000"),
            sz: d("1.5"),
            oid: 99,
            orig_sz: d("1.5"),
            cloid: None,
            order_type: OrderType::Limit,
            tif: Some(TimeInForce::Gtc),
            reduce_only: false,
            is_trigger: Some(false),
            trigger_px: None,
            trigger_condition: None,
            is_position_tpsl: Some(false),
        };
        let order = OpenOrder {
            basic_order: basic,
            trigger_condition: String::new(),
            is_trigger: false,
            trigger_px: d("0"),
            children: Vec::new(),
            is_position_tpsl: false,
        };
        let live = live_open_orders_from_orders(&[order]);
        assert_eq!(live.len(), 1);
        let lo = &live[0];
        assert_eq!(lo.coin, "ETH");
        assert_eq!(lo.side.as_deref(), Some("sell"));
        assert_eq!(lo.limit_px, Some(d("2000")));
        assert_eq!(lo.orig_sz, Some(d("1.5")));
        assert_eq!(lo.oid.as_deref(), Some("99"));
        assert_eq!(lo.tif.as_deref(), Some("Gtc"));
        assert_eq!(lo.reduce_only, Some(false));
    }

    #[test]
    fn side_to_str_and_lift_coverage() {
        assert_eq!(side_to_str(&Side::Bid), "buy");
        assert_eq!(side_to_str(&Side::Ask), "sell");
        assert_eq!(leverage_type_str(&LeverageType::Cross), "cross");
        assert_eq!(leverage_type_str(&LeverageType::Isolated), "isolated");
        assert_eq!(order_type_to_str(&OrderType::Limit), "limit");
        assert_eq!(order_type_to_str(&OrderType::Market), "market");
        assert_eq!(order_type_to_str(&OrderType::Trigger), "trigger");
        assert_eq!(tif_to_str(&TimeInForce::Gtc), "Gtc");
        assert_eq!(tif_to_str(&TimeInForce::Ioc), "Ioc");
        assert_eq!(tif_to_str(&TimeInForce::Alo), "Alo");

        let k = AccountKey::new("0xaddr", "live");
        assert_eq!(k.account_address, "0xaddr");
        assert_eq!(k.environment, "live");
    }
}
