use crate::hyperliquid::live_state::LiveConnectionStatus;

use super::*;
use crate::web::templates::test_support::*;

#[test]
fn open_orders_view_sorts_by_price_descending() {
    use crate::hyperliquid::live_state::{AccountLiveState, LiveOpenOrder};
    let state = AccountLiveState {
        account_address: "0xtest".to_string(),
        environment: "live".to_string(),
        status: LiveConnectionStatus::Connected,
        open_orders: vec![
            LiveOpenOrder {
                coin: "ETH".to_string(),
                side: Some("buy".to_string()),
                limit_px: Some(rust_decimal::Decimal::new(1900, 0)),
                sz: Some(rust_decimal::Decimal::new(1, 0)),
                orig_sz: Some(rust_decimal::Decimal::new(1, 0)),
                oid: Some("1".to_string()),
                timestamp: Some(1_700_000_000_000),
                cloid: None,
                order_type: Some("limit".to_string()),
                tif: Some("Gtc".to_string()),
                reduce_only: Some(false),
                is_trigger: Some(false),
                trigger_px: None,
                trigger_condition: None,
                is_position_tpsl: Some(false),
            },
            LiveOpenOrder {
                coin: "BTC".to_string(),
                side: Some("sell".to_string()),
                limit_px: Some(rust_decimal::Decimal::new(31000, 0)),
                sz: Some(rust_decimal::Decimal::new(2, 0)),
                orig_sz: Some(rust_decimal::Decimal::new(2, 0)),
                oid: Some("2".to_string()),
                timestamp: Some(1_700_000_500_000),
                cloid: None,
                order_type: Some("limit".to_string()),
                tif: Some("Gtc".to_string()),
                reduce_only: Some(false),
                is_trigger: Some(false),
                trigger_px: None,
                trigger_condition: None,
                is_position_tpsl: Some(false),
            },
        ],
        ..Default::default()
    };
    let view = OpenOrdersView::from_live_state(state);
    assert_eq!(view.orders.len(), 2);
    assert_eq!(view.orders[0].coin, "BTC");
    assert_eq!(view.orders[1].coin, "ETH");
}

#[test]
fn open_orders_partial_renders_flags() {
    use crate::hyperliquid::live_state::{AccountLiveState, LiveOpenOrder};
    let state = AccountLiveState {
        account_address: "0xtest".to_string(),
        environment: "live".to_string(),
        status: LiveConnectionStatus::Connected,
        open_orders: vec![LiveOpenOrder {
            coin: "BTC".to_string(),
            side: Some("sell".to_string()),
            limit_px: Some(rust_decimal::Decimal::new(31000, 0)),
            sz: Some(rust_decimal::Decimal::new(2, 0)),
            orig_sz: Some(rust_decimal::Decimal::new(2, 0)),
            oid: Some("42".to_string()),
            timestamp: Some(chrono::Utc::now().timestamp_millis() as u64),
            cloid: None,
            order_type: Some("take_profit_market".to_string()),
            tif: Some("Gtc".to_string()),
            reduce_only: Some(true),
            is_trigger: Some(true),
            trigger_px: Some(rust_decimal::Decimal::new(32000, 0)),
            trigger_condition: Some("above".to_string()),
            is_position_tpsl: Some(true),
        }],
        ..Default::default()
    };
    let view = OpenOrdersView::from_live_state(state);
    assert_eq!(view.orders[0].price.color_class, "text-zinc-300");
    assert_eq!(view.orders[0].price.value, "31,000");
    let html = OpenOrdersPartialTemplate::render_view(view).unwrap();
    assert!(html.contains("BTC"));
    assert!(html.contains("sell"));
    assert!(html.contains("take_profit_market"));
    assert!(html.contains("31,000"));
    assert!(html.contains("32,000.0000"));
    assert!(html.contains("trigger"));
    assert!(html.contains("TP/SL"));
    assert!(html.contains("reduce-only"));
}

#[test]
fn open_orders_partial_renders_loading_when_no_state() {
    use crate::hyperliquid::live_state::AccountLiveState;
    let state = AccountLiveState {
        account_address: "0xtest".to_string(),
        environment: "live".to_string(),
        status: LiveConnectionStatus::Starting,
        ..Default::default()
    };
    let view = OpenOrdersView::from_live_state(state);
    let html = OpenOrdersPartialTemplate::render_view(view).unwrap();
    assert!(html.contains("Loading"));
    assert!(!html.contains("No open orders"));
}

#[test]
fn open_orders_partial_renders_empty_state() {
    use crate::hyperliquid::live_state::AccountLiveState;
    let state = AccountLiveState {
        account_address: "0xtest".to_string(),
        environment: "live".to_string(),
        status: LiveConnectionStatus::Connected,
        ..Default::default()
    };
    let view = OpenOrdersView::from_live_state(state);
    let html = OpenOrdersPartialTemplate::render_view(view).unwrap();
    assert!(html.contains("No open orders"));
}