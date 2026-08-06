use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::Utc;
use rust_decimal_macros::dec;
use serde_json::json;
use tower::util::ServiceExt;

use crate::{
    agents::store::get_agent,
    hyperliquid::live_state::{AccountKey, AccountLiveState, LiveMarginState},
};

use super::test_support::*;
#[tokio::test]
async fn get_account_returns_200_with_unavailable_data_when_orchestrator_has_no_snapshot() {
    let state = test_state().await;

    let (agent_key, api_key) = seed_agent(&state, "acct-empty").await;

    let request = Request::builder()
        .uri("/account")
        .header("authorization", format!("Bearer {api_key}"))
        .body(Body::empty())
        .unwrap();
    let response = app(Arc::clone(&state)).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(body["agent_key"], serde_json::Value::from(agent_key));
    assert_eq!(body["account_data"]["available"], false);
    assert_eq!(body["account_data"]["stale"], true);
    assert!(body["account_data"]["as_of"].is_null());
    assert!(body["balance"].is_null());
    assert_eq!(body["open_positions"], json!([]));
    assert_eq!(body["open_orders"], json!([]));
    assert!(body.get("connected").is_none());
    assert!(body.get("state").is_none());
}

#[tokio::test]
async fn get_account_returns_fresh_account_contract_after_live_state_seeded() {
    let state = test_state().await;

    let (agent_key, api_key) = seed_agent(&state, "acct-live").await;
    let row = get_agent(&state.db_pool, &agent_key)
        .await
        .unwrap()
        .expect("present");
    let trading_account = row
        .trading_account_address
        .as_deref()
        .expect("trading account");
    let key = AccountKey::new(trading_account, &row.environment);
    state.live_accounts.replace(
        key,
        AccountLiveState {
            account_address: trading_account.to_string(),
            environment: row.environment.clone(),
            status: crate::hyperliquid::live_state::LiveConnectionStatus::Connected,
            clearinghouse_updated_at: Some(Utc::now()),
            open_orders_updated_at: Some(Utc::now()),
            spot_updated_at: Some(Utc::now()),
            margin: Some(LiveMarginState {
                account_value: Some(dec!(100)),
                withdrawable: Some(dec!(75)),
                total_margin_used: Some(dec!(25)),
                ..Default::default()
            }),
            ..Default::default()
        },
    );

    let request = Request::builder()
        .uri("/account")
        .header("authorization", format!("Bearer {api_key}"))
        .body(Body::empty())
        .unwrap();
    let response = app(Arc::clone(&state)).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(body["agent_key"], serde_json::Value::from(agent_key));
    assert_eq!(
        body["account_address"],
        serde_json::Value::from(trading_account)
    );
    assert_eq!(
        body["environment"],
        serde_json::Value::from(row.environment)
    );
    assert_eq!(body["account_data"]["available"], true);
    assert_eq!(body["account_data"]["stale"], false);
    assert!(body["account_data"]["as_of"].is_string());
    assert_eq!(body["balance"]["exchange"], "hyperliquid");
    assert_eq!(body["balance"]["model"], "unified_cross_margin");
    assert_eq!(body["balance"]["available_to_trade_usd"], "75");
    assert_eq!(body["open_positions"], json!([]));
    assert_eq!(body["open_orders"], json!([]));
    assert!(body.get("connected").is_none());
    assert!(body.get("state").is_none());
}

#[tokio::test]
async fn get_account_without_auth_returns_401_json() {
    let state = test_state().await;

    let request = Request::builder()
        .uri("/account")
        .body(Body::empty())
        .unwrap();
    let response = app(Arc::clone(&state)).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body_bytes = axum::body::to_bytes(response.into_body(), 16 * 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert!(body["error"].is_string());
}

#[tokio::test]
async fn get_account_with_invalid_bearer_returns_401_json() {
    let state = test_state().await;

    let request = Request::builder()
        .uri("/account")
        .header("authorization", "Bearer vta_does-not-exist")
        .body(Body::empty())
        .unwrap();
    let response = app(Arc::clone(&state)).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
