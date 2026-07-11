use crate::hyperliquid::live_state::LiveConnectionStatus;

use super::*;
use crate::web::templates::test_support::*;
use chrono::Utc;
use rust_decimal::Decimal;

#[test]
fn account_balance_partial_renders_loading_state_when_value_missing() {
    let view = AccountBalanceView {
        total_balance: None,
        total_u_pnl: AnimatedNumber::for_pnl(rust_decimal::Decimal::ZERO),
    };
    let html = AccountBalancePartialTemplate::render_view(view).unwrap();
    assert!(html.contains("Balance"));
    assert!(html.contains("Loading"));
    assert!(!html.contains("USDC"));
}

#[test]
fn account_balance_partial_renders_value_with_status() {
    let view = sample_account_balance_view();
    let html = AccountBalancePartialTemplate::render_view(view).unwrap();
    assert!(html.contains("Balance"));
    assert!(html.contains("Unrealized"));
    assert!(html.contains("232.6800"));
    assert!(html.contains("USDC"));
}

#[test]
fn balance_view_sums_perps_and_spot_available() {
    use crate::hyperliquid::live_state::{AccountLiveState, LiveMarginState, LiveSpotBalance};
    let state = AccountLiveState {
        account_address: "0xtest".to_string(),
        environment: "live".to_string(),
        status: LiveConnectionStatus::Connected,
        margin: Some(LiveMarginState {
            account_value: Some(rust_decimal::Decimal::new(50_0700, 4)),
            withdrawable: Some(rust_decimal::Decimal::new(420, 4)),
            ..Default::default()
        }),
        spot_balances: vec![LiveSpotBalance {
            coin: "USDC".to_string(),
            total: Some(rust_decimal::Decimal::new(232_6700, 4)),
            hold: Some(rust_decimal::Decimal::new(50_0600, 4)),
            available: Some(rust_decimal::Decimal::new(182_6100, 4)),
            ..Default::default()
        }],
        ..Default::default()
    };
    let view = AccountBalanceView::from_live_state(state);
    // perps (50.07) + spot available (182.61) = 232.68
    assert_eq!(
        view.total_balance,
        Some(rust_decimal::Decimal::new(232_6800, 4))
    );
    assert_eq!(view.total_u_pnl.value, "-");
}

#[test]
fn balance_view_uses_perps_only_when_no_spot_state() {
    use crate::hyperliquid::live_state::{AccountLiveState, LiveMarginState};
    let state = AccountLiveState {
        account_address: "0xtest".to_string(),
        environment: "live".to_string(),
        status: LiveConnectionStatus::Connected,
        margin: Some(LiveMarginState {
            account_value: Some(rust_decimal::Decimal::new(1000, 0)),
            withdrawable: Some(rust_decimal::Decimal::new(900, 0)),
            ..Default::default()
        }),
        ..Default::default()
    };
    let view = AccountBalanceView::from_live_state(state);
    assert_eq!(
        view.total_balance,
        Some(rust_decimal::Decimal::new(1000, 0))
    );
    assert_eq!(view.total_u_pnl.value, "-");
}

#[test]
fn balance_view_uses_spot_total_when_no_perps_margin_state() {
    use crate::hyperliquid::live_state::{AccountLiveState, LiveSpotBalance};
    let state = AccountLiveState {
        account_address: "0xtest".to_string(),
        environment: "live".to_string(),
        status: LiveConnectionStatus::Connected,
        spot_balances: vec![LiveSpotBalance {
            coin: "USDC".to_string(),
            total: Some(rust_decimal::Decimal::new(500, 0)),
            hold: Some(rust_decimal::Decimal::new(50, 0)),
            available: Some(rust_decimal::Decimal::new(450, 0)),
            ..Default::default()
        }],
        ..Default::default()
    };
    let view = AccountBalanceView::from_live_state(state);
    // No margin snapshot yet -> use spot USDC available.
    assert_eq!(view.total_balance, Some(rust_decimal::Decimal::new(450, 0)));
    assert_eq!(view.total_u_pnl.value, "-");
}

#[test]
fn balance_view_sums_total_upnl_from_open_positions() {
    use crate::hyperliquid::live_state::{AccountLiveState, LivePosition};
    let state = AccountLiveState {
        account_address: "0xtest".to_string(),
        environment: "live".to_string(),
        status: LiveConnectionStatus::Connected,
        open_positions: vec![
            LivePosition {
                unrealized_pnl: Some(rust_decimal::Decimal::new(125, 0)),
                ..Default::default()
            },
            LivePosition {
                unrealized_pnl: Some(rust_decimal::Decimal::new(-25, 0)),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let view = AccountBalanceView::from_live_state(state);
    assert_eq!(view.total_u_pnl.value, "100.0000");
    assert_eq!(view.total_u_pnl.color_class, "text-emerald-400");
}

#[test]
fn sparkline_from_series_empty_when_no_points() {
    let view = SparklineView::from_series("24h", &[], 240, 48);
    assert!(view.is_empty);
    assert_eq!(view.label, "24h");
    assert!(view.polyline.is_empty());
    assert_eq!(view.change.value, "-");
    assert_eq!(view.change.color_class, "text-zinc-500");
}

#[test]
fn sparkline_from_series_empty_when_single_point() {
    let now = Utc::now();
    let view = SparklineView::from_series("24h", &[bp(now, Decimal::new(100, 0))], 240, 48);
    assert!(view.is_empty);
    assert!(view.polyline.is_empty());
    // Change is undefined for a single point: dash cell.
    assert_eq!(view.change.value, "-");
}

#[test]
fn sparkline_from_series_builds_polyline_and_change() {
    let now = Utc::now();
    let points = vec![
        bp(now - chrono::Duration::hours(3), Decimal::new(100, 0)),
        bp(now - chrono::Duration::hours(2), Decimal::new(150, 0)),
        bp(now - chrono::Duration::hours(1), Decimal::new(120, 0)),
    ];
    let view = SparklineView::from_series("24h", &points, 240, 48);
    assert!(!view.is_empty);
    // last - first = 120 - 100 = 20 → emerald, + prefix.
    assert_eq!(view.change.value, "+20.0000");
    assert_eq!(view.change.color_class, "text-emerald-400");

    // Polyline: one "x,y" pair per point, space-separated.
    let coords: Vec<&str> = view.polyline.split_whitespace().collect();
    assert_eq!(coords.len(), 3);
    for c in &coords {
        assert!(c.contains(','));
    }
}

#[test]
fn sparkline_from_series_change_sign_and_color() {
    let now = Utc::now();
    // First larger than last: change should render in red, parens.
    let points = vec![
        bp(now - chrono::Duration::hours(2), Decimal::new(200, 0)),
        bp(now - chrono::Duration::hours(1), Decimal::new(50, 0)),
    ];
    let view = SparklineView::from_series("30d", &points, 240, 48);
    assert_eq!(view.change.value, "(150.0000)");
    assert_eq!(view.change.color_class, "text-red-400");
}
