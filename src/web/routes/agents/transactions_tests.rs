//! Tests for agents/transactions_tests.rs
use super::*;
use crate::web::routes::router;
use crate::web::routes::test_support::*;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::{DateTime, Duration, TimeZone, Utc};
use rust_decimal::Decimal;
use std::sync::Arc;
use tower::util::ServiceExt;

use crate::hyperliquid::live_state::{AccountKey, AccountLiveState};
use crate::web::AppState;

const TRANSACTIONS_PER_PAGE: usize = 25;

#[tokio::test]
async fn agent_transactions_route_renders_full_timeline() {
    let state = test_state().await;
    let (agent_key, wallet_address) = insert_test_agent(&state).await.expect("insert agent");
    seed_ledger_event(
        &state,
        &wallet_address,
        &format!("tx-route-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0)),
        Utc::now(),
        Decimal::new(42, 0),
    )
    .await;

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/transactions"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert!(text.contains("Transactions"));
    assert!(text.contains("42.0000"));
    assert!(text.contains("Showing 1-1 of 1 transactions"));
    assert!(text.contains("Page 1 of 1"));
}
#[tokio::test]
async fn agent_transactions_route_anchors_running_balance_to_live_cash_balance() {
    let state = test_state().await;
    let (agent_key, wallet_address) = insert_test_agent(&state).await.expect("insert agent");
    seed_ledger_event(
        &state,
        &wallet_address,
        &format!(
            "tx-anchor-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ),
        Utc::now(),
        Decimal::new(42, 0),
    )
    .await;

    let account_key = AccountKey::new(&wallet_address, "live");
    state.live_accounts.replace(
        account_key,
        AccountLiveState {
            account_address: wallet_address.clone(),
            environment: "live".to_string(),
            spot_balances: vec![crate::hyperliquid::live_state::LiveSpotBalance {
                coin: "USDC".to_string(),
                total: Some(Decimal::new(420017, 4)),
                available: Some(Decimal::new(420017, 4)),
                ..Default::default()
            }],
            ..Default::default()
        },
    );

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/transactions"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert!(text.contains("42.0000"));
    assert!(text.contains("42.0017"));
}
#[tokio::test]
async fn live_cash_balance_excludes_unrealized_pnl() {
    let state = AccountLiveState {
        margin: Some(crate::hyperliquid::live_state::LiveMarginState {
            account_value: Some(Decimal::new(110, 0)),
            ..Default::default()
        }),
        spot_balances: vec![crate::hyperliquid::live_state::LiveSpotBalance {
            coin: "USDC".to_string(),
            available: Some(Decimal::new(5, 0)),
            ..Default::default()
        }],
        open_positions: vec![crate::hyperliquid::live_state::LivePosition {
            unrealized_pnl: Some(Decimal::new(10, 0)),
            ..Default::default()
        }],
        ..Default::default()
    };

    assert_eq!(live_cash_balance(&state), Some(Decimal::new(105, 0)));
}

fn pagination_base_time() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
}

async fn seed_pagination_events(state: &Arc<AppState>, wallet_address: &str, count: usize) {
    let base = pagination_base_time();
    for index in 1..=count {
        let event_time = base + Duration::minutes(index as i64);
        seed_ledger_event(
            state,
            wallet_address,
            &format!("tx-page-{index:03}"),
            event_time,
            Decimal::new(1, 0),
        )
        .await;
    }
}

fn expected_time_text(minutes_offset: i64) -> String {
    let event_time = pagination_base_time() + Duration::minutes(minutes_offset);
    event_time.format("%Y-%m-%d %H:%M UTC").to_string()
}

fn format_money_like_template(value: Decimal) -> String {
    if value.is_sign_negative() {
        format!("({:.4})", value.abs())
    } else {
        format!("{:.4}", value)
    }
}

#[tokio::test]
async fn agent_transactions_route_paginates_timeline() {
    let state = test_state().await;
    let (agent_key, wallet_address) = insert_test_agent(&state).await.expect("insert agent");
    let total = TRANSACTIONS_PER_PAGE + 5;
    seed_pagination_events(&state, &wallet_address, total).await;

    let page_one = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/transactions"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(page_one.status(), StatusCode::OK);
    let page_one_text = response_text(page_one).await;
    assert!(page_one_text.contains(&format!(
        "Showing 1-{TRANSACTIONS_PER_PAGE} of {total} transactions"
    )));
    assert!(page_one_text.contains("Page 1 of 2"));
    assert!(page_one_text.contains(&expected_time_text(total as i64)));
    assert!(page_one_text.contains(&expected_time_text(6)));
    assert!(!page_one_text.contains(&expected_time_text(5)));
    assert!(!page_one_text.contains(&expected_time_text(1)));
    assert!(page_one_text.contains(&format!("/agents/{agent_key}/transactions?page=2")));

    let page_two = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/transactions?page=2"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(page_two.status(), StatusCode::OK);
    let page_two_text = response_text(page_two).await;
    assert!(page_two_text.contains(&format!(
        "Showing {first}-{last} of {total} transactions",
        first = TRANSACTIONS_PER_PAGE + 1,
        last = total
    )));
    assert!(page_two_text.contains("Page 2 of 2"));
    assert!(page_two_text.contains(&expected_time_text(5)));
    assert!(page_two_text.contains(&expected_time_text(1)));
    assert!(!page_two_text.contains(&expected_time_text(6)));
    assert!(page_two_text.contains(&format!("/agents/{agent_key}/transactions?page=1")));
}

#[tokio::test]
async fn agent_transactions_route_clamps_out_of_range_page_to_last_page() {
    let state = test_state().await;
    let (agent_key, wallet_address) = insert_test_agent(&state).await.expect("insert agent");
    let total = TRANSACTIONS_PER_PAGE + 3;
    seed_pagination_events(&state, &wallet_address, total).await;

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/transactions?page=42"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert!(text.contains("Page 2 of 2"));
    assert!(text.contains(&expected_time_text(3)));
    assert!(text.contains(&expected_time_text(1)));
    assert!(!text.contains(&expected_time_text(4)));
}

#[tokio::test]
async fn agent_transactions_route_anchors_running_balance_to_live_cash_balance_on_page_two() {
    let state = test_state().await;
    let (agent_key, wallet_address) = insert_test_agent(&state).await.expect("insert agent");
    let total = TRANSACTIONS_PER_PAGE + 5;
    seed_pagination_events(&state, &wallet_address, total).await;

    let account_key = AccountKey::new(&wallet_address, "live");
    state.live_accounts.replace(
        account_key,
        AccountLiveState {
            account_address: wallet_address.clone(),
            environment: "live".to_string(),
            spot_balances: vec![crate::hyperliquid::live_state::LiveSpotBalance {
                coin: "USDC".to_string(),
                total: Some(Decimal::new(100000, 4)),
                available: Some(Decimal::new(100000, 4)),
                ..Default::default()
            }],
            ..Default::default()
        },
    );

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/transactions?page=2"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert!(text.contains(&expected_time_text(5)));

    let total_count = total as i64;
    let live_cash = Decimal::new(100000, 4);
    let adjustment = live_cash - Decimal::new(total_count, 0);
    let anchored_for_event_5 = Decimal::new(5, 0) + adjustment;
    let anchored_for_event_1 = Decimal::new(1, 0) + adjustment;
    let formatted_5 = format_money_like_template(anchored_for_event_5);
    let formatted_1 = format_money_like_template(anchored_for_event_1);
    assert!(text.contains(&formatted_5), "expected anchored balance {formatted_5} for event 5 in {text}");
    assert!(text.contains(&formatted_1), "expected anchored balance {formatted_1} for event 1 in {text}");
}