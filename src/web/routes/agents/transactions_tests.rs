//! Tests for agents/transactions_tests.rs
use super::*;
use crate::web::routes::router;
use crate::web::routes::test_support::*;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::Utc;
use rust_decimal::Decimal;
use tower::util::ServiceExt;

use crate::hyperliquid::live_state::{AccountKey, AccountLiveState};

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
#[test]
fn live_cash_balance_excludes_unrealized_pnl() {
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
