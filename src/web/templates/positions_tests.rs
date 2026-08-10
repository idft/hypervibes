use crate::hyperliquid::live_state::LiveConnectionStatus;
use crate::hyperliquid::market_data::{MarketDataStore, MarketPrice};
use std::collections::HashMap;

use super::*;
use chrono::Utc;

#[test]
fn open_positions_view_filters_zero_szi_and_sign_based_side() {
    use crate::hyperliquid::live_state::{AccountLiveState, LivePosition};
    let state = AccountLiveState {
        account_address: "0xtest".to_string(),
        environment: "live".to_string(),
        status: LiveConnectionStatus::Connected,
        clearinghouse_updated_at: Some(Utc::now()),
        open_positions: vec![
            LivePosition {
                coin: "BTC".to_string(),
                szi: Some(rust_decimal::Decimal::new(1, 0)),
                entry_px: Some(rust_decimal::Decimal::new(30000, 0)),
                liquidation_px: None,
                margin_used: Some(rust_decimal::Decimal::new(600, 0)),
                position_value: Some(rust_decimal::Decimal::new(30000, 0)),
                unrealized_pnl: Some(rust_decimal::Decimal::new(100, 0)),
                return_on_equity: Some(rust_decimal::Decimal::new(5, 1)),
                leverage_type: Some("cross".to_string()),
                leverage_value: Some(5),
                max_leverage: Some(50),
            },
            LivePosition {
                coin: "ETH".to_string(),
                szi: Some(rust_decimal::Decimal::new(-3, 0)),
                entry_px: Some(rust_decimal::Decimal::new(2000, 0)),
                liquidation_px: Some(rust_decimal::Decimal::new(2500, 0)),
                margin_used: Some(rust_decimal::Decimal::new(200, 0)),
                position_value: Some(rust_decimal::Decimal::new(6000, 0)),
                unrealized_pnl: Some(rust_decimal::Decimal::new(-150, 0)),
                return_on_equity: Some(rust_decimal::Decimal::new(-7, 1)),
                leverage_type: Some("isolated".to_string()),
                leverage_value: Some(3),
                max_leverage: Some(50),
            },
            LivePosition {
                coin: "DUST".to_string(),
                szi: Some(rust_decimal::Decimal::new(0, 0)),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let view = OpenPositionsView::from_live_state(state);
    assert_eq!(view.positions.len(), 2);
    // Sorted by |szi| desc: ETH (3) before BTC (1).
    assert_eq!(view.positions[0].coin, "ETH");
    assert_eq!(view.positions[0].side, "short");
    assert_eq!(view.positions[1].coin, "BTC");
    assert_eq!(view.positions[1].side, "long");
    assert_eq!(view.positions[0].entry_px.value, "2,000");
    assert_eq!(view.positions[0].mark_px_or_value, "6,000");
    assert_eq!(view.positions[0].entry_px.color_class, "text-zinc-300");
    assert_eq!(
        view.positions[0].liquidation_px.color_class,
        "text-zinc-300"
    );
    assert_eq!(view.positions[0].liquidation_px.value, "2,500");
    assert_eq!(view.positions[0].margin_used.color_class, "text-zinc-300");
    assert_eq!(view.positions[0].margin_used.value, "200");
}

#[test]
fn open_positions_view_handles_no_positions() {
    use crate::hyperliquid::live_state::AccountLiveState;
    let state = AccountLiveState {
        account_address: "0xtest".to_string(),
        environment: "live".to_string(),
        status: LiveConnectionStatus::Connected,
        clearinghouse_updated_at: Some(Utc::now()),
        ..Default::default()
    };
    let view = OpenPositionsView::from_live_state(state);
    assert!(!view.is_loading);
    assert!(view.positions.is_empty());
}

#[test]
fn open_positions_view_includes_configured_coins_without_live_positions() {
    use crate::hyperliquid::live_state::{AccountLiveState, LivePosition};

    let state = AccountLiveState {
        account_address: "0xtest".to_string(),
        environment: "live".to_string(),
        status: LiveConnectionStatus::Connected,
        clearinghouse_updated_at: Some(Utc::now()),
        open_positions: vec![LivePosition {
            coin: "BTC".to_string(),
            szi: Some(rust_decimal::Decimal::new(1, 0)),
            position_value: Some(rust_decimal::Decimal::new(30_000, 0)),
            unrealized_pnl: Some(rust_decimal::Decimal::new(250, 0)),
            margin_used: Some(rust_decimal::Decimal::new(600, 0)),
            ..Default::default()
        }],
        ..Default::default()
    };

    let view = OpenPositionsView::from_live_state_with_configured_coins(
        state,
        &["ETH".to_string(), "BTC".to_string()],
    );

    assert_eq!(view.positions.len(), 2);
    assert_eq!(view.positions[0].coin, "ETH");
    assert!(!view.positions[0].has_position);
    assert_eq!(view.positions[0].side, "No position");
    assert_eq!(
        view.positions[0].market_url,
        "https://app.hyperliquid.xyz/trade/ETH"
    );
    assert_eq!(view.positions[1].coin, "BTC");
    assert!(view.positions[1].has_position);
}

#[test]
fn open_positions_view_has_no_state_when_only_starting() {
    use crate::hyperliquid::live_state::AccountLiveState;
    let state = AccountLiveState {
        account_address: "0xtest".to_string(),
        environment: "live".to_string(),
        status: LiveConnectionStatus::Starting,
        ..Default::default()
    };
    let view = OpenPositionsView::from_live_state(state);
    assert!(view.is_loading);
}

#[test]
fn open_positions_partial_renders_loading_when_no_state() {
    use crate::hyperliquid::live_state::AccountLiveState;
    let state = AccountLiveState {
        account_address: "0xtest".to_string(),
        environment: "live".to_string(),
        status: LiveConnectionStatus::Starting,
        ..Default::default()
    };
    let view = OpenPositionsView::from_live_state(state);
    let html = OpenPositionsPartialTemplate::render_view(view).unwrap();
    assert!(html.contains("Loading"));
    assert!(!html.contains("No open positions"));
}

#[test]
fn open_positions_partial_renders_empty_state() {
    use crate::hyperliquid::live_state::AccountLiveState;
    let state = AccountLiveState {
        account_address: "0xtest".to_string(),
        environment: "live".to_string(),
        status: LiveConnectionStatus::Connected,
        clearinghouse_updated_at: Some(Utc::now()),
        ..Default::default()
    };
    let view = OpenPositionsView::from_live_state(state);
    let html = OpenPositionsPartialTemplate::render_view(view).unwrap();
    assert!(html.contains("No open positions"));
}

#[test]
fn open_positions_partial_does_not_render_empty_state_when_monitoring_failed() {
    use crate::hyperliquid::live_state::AccountLiveState;
    let state = AccountLiveState {
        account_address: "0xtest".to_string(),
        environment: "live".to_string(),
        status: LiveConnectionStatus::Failed,
        ..Default::default()
    };
    let html = OpenPositionsPartialTemplate::render_view(OpenPositionsView::from_live_state(state))
        .expect("render positions");
    assert!(html.contains("unavailable"));
    assert!(!html.contains("No open positions"));
}

#[test]
fn open_positions_partial_renders_configured_placeholder_rows_and_market_links() {
    use crate::hyperliquid::live_state::AccountLiveState;

    let state = AccountLiveState {
        account_address: "0xtest".to_string(),
        environment: "live".to_string(),
        status: LiveConnectionStatus::Connected,
        clearinghouse_updated_at: Some(Utc::now()),
        ..Default::default()
    };

    let view =
        OpenPositionsView::from_live_state_with_configured_coins(state, &["BTC".to_string()]);
    let html = OpenPositionsPartialTemplate::render_view(view).unwrap();

    assert!(html.contains("No position"));
    assert!(html.contains("https://app.hyperliquid.xyz/trade/BTC"));
    assert!(html.contains("target=\"_blank\""));
    assert!(html.contains("rel=\"noopener noreferrer\""));
    assert!(!html.contains("No open positions"));
}

#[test]
fn open_positions_partial_renders_market_price_and_sparkline_for_placeholder_row() {
    use crate::hyperliquid::live_state::AccountLiveState;

    let market_data = MarketDataStore::new();
    market_data.replace_for_test(HashMap::from([(
        "BTC".to_string(),
        MarketPrice {
            current: Some(rust_decimal::Decimal::new(65_000, 0)),
            prices_24h: vec![
                rust_decimal::Decimal::new(63_000, 0),
                rust_decimal::Decimal::new(64_000, 0),
            ],
        },
    )]));
    let state = AccountLiveState {
        account_address: "0xtest".to_string(),
        environment: "live".to_string(),
        status: LiveConnectionStatus::Connected,
        clearinghouse_updated_at: Some(Utc::now()),
        ..Default::default()
    };

    let view = OpenPositionsView::from_live_state_with_configured_coins_and_market_data(
        state,
        &["BTC".to_string()],
        &market_data.snapshot(),
    );
    let html = OpenPositionsPartialTemplate::render_view(view).expect("render positions");

    assert!(html.contains(">Price<"));
    assert!(html.contains(">24h<"));
    assert!(html.contains("65,000.0000"));
    assert!(html.contains("BTC price over the last 24 hours"));
    assert!(html.contains("<polyline"));
}

#[test]
fn open_positions_partial_renders_rows_and_pills() {
    use crate::hyperliquid::live_state::{AccountLiveState, LivePosition};
    let state = AccountLiveState {
        account_address: "0xtest".to_string(),
        environment: "live".to_string(),
        status: LiveConnectionStatus::Connected,
        clearinghouse_updated_at: Some(Utc::now()),
        open_positions: vec![
            LivePosition {
                coin: "BTC".to_string(),
                szi: Some(rust_decimal::Decimal::new(1, 0)),
                entry_px: Some(rust_decimal::Decimal::new(30000, 0)),
                liquidation_px: Some(rust_decimal::Decimal::new(25000, 0)),
                margin_used: Some(rust_decimal::Decimal::new(6000, 0)),
                position_value: Some(rust_decimal::Decimal::new(30000, 0)),
                unrealized_pnl: Some(rust_decimal::Decimal::new(1500, 0)),
                return_on_equity: Some(rust_decimal::Decimal::new(2500, 2)),
                leverage_type: Some("cross".to_string()),
                leverage_value: Some(5),
                max_leverage: Some(50),
            },
            LivePosition {
                coin: "ETH".to_string(),
                szi: Some(rust_decimal::Decimal::new(-2, 0)),
                entry_px: Some(rust_decimal::Decimal::new(2000, 0)),
                liquidation_px: None,
                margin_used: Some(rust_decimal::Decimal::new(400, 0)),
                position_value: Some(rust_decimal::Decimal::new(4000, 0)),
                unrealized_pnl: Some(rust_decimal::Decimal::new(-100, 0)),
                return_on_equity: Some(rust_decimal::Decimal::new(-2500, 2)),
                leverage_type: Some("isolated".to_string()),
                leverage_value: Some(10),
                max_leverage: Some(50),
            },
        ],
        ..Default::default()
    };
    let view = OpenPositionsView::from_live_state(state);
    let html = OpenPositionsPartialTemplate::render_view(view).unwrap();
    assert!(html.contains("Notional"));
    assert!(!html.contains(">Coin<"));
    assert!(!html.contains(">Side<"));
    assert!(!html.contains("Mark / NTL"));
    assert!(html.contains("BTC"));
    assert!(html.contains("ETH"));
    assert!(html.contains("long"));
    assert!(html.contains("short"));
    assert!(html.contains("+25.00%"));
    assert!(html.contains("-25.00%"));
    assert!(html.contains("1.0000"));
    assert!(html.contains("2.0000"));
    assert!(html.contains("30,000"));
    assert!(html.contains("4,000"));
    assert!(html.contains("(100.0000)"));
    assert!(html.contains("25,000"));
    assert!(html.contains("6,000"));
}

#[test]
fn format_signed_percent_works() {
    assert_eq!(
        format_signed_percent(rust_decimal::Decimal::new(243, 2), 2),
        "+2.43%"
    );
    assert_eq!(
        format_signed_percent(rust_decimal::Decimal::new(-110, 2), 2),
        "-1.10%"
    );
    assert_eq!(
        format_signed_percent(rust_decimal::Decimal::new(0, 0), 2),
        "+0.00%"
    );
}
