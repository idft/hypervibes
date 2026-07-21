use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use chrono::{TimeZone, Utc};
use serde_json::Value;
use tower::util::ServiceExt;

use crate::agents::store::get_agent;

use super::test_support::*;

#[tokio::test]
async fn list_account_transactions_returns_only_the_callers_windowed_journal_events() {
    let state = test_state().await;
    let (agent_key, api_key) = seed_agent(&state, "transactions").await;
    let agent = get_agent(&state.db_pool, &agent_key)
        .await
        .expect("load agent")
        .expect("agent exists");
    let event_time = Utc.with_ymd_and_hms(2026, 7, 16, 12, 0, 0).unwrap();
    sqlx::query(
        "INSERT INTO hyperliquid.ledger_events (
             hash, account_address, environment, event_time, event_type,
             source_stream, ledger_type, usdc, details, payload,
             ingest_source, inserted_at
         ) VALUES ($1, $2, $3, $4, 'deposit', 'ledger', 'deposit', 25, '{}', '{}', 'test', $4)",
    )
    .bind("transaction-api-test-event")
    .bind(
        agent
            .trading_account_address
            .as_deref()
            .expect("trading account"),
    )
    .bind(&agent.environment)
    .bind(event_time)
    .execute(&state.db_pool)
    .await
    .expect("insert journal event");
    sqlx::query(
        "INSERT INTO hyperliquid.ledger_events (
             hash, account_address, environment, event_time, event_type,
             source_stream, ledger_type, usdc, details, payload,
             ingest_source, inserted_at
         ) VALUES ($1, $2, $3, $4, 'deposit', 'ledger', 'deposit', 30, '{}', '{}', 'test', $4)",
    )
    .bind("transaction-api-test-event-newer")
    .bind(
        agent
            .trading_account_address
            .as_deref()
            .expect("trading account"),
    )
    .bind(&agent.environment)
    .bind(Utc.with_ymd_and_hms(2026, 7, 16, 13, 0, 0).unwrap())
    .execute(&state.db_pool)
    .await
    .expect("insert newer journal event");

    let request = Request::builder()
        .uri("/account/transactions?since=2026-07-16T00:00:00Z&until=2026-07-17T00:00:00Z&event_category=ledger&limit=1&offset=1")
        .header("authorization", format!("Bearer {api_key}"))
        .body(Body::empty())
        .unwrap();
    let response = app(Arc::clone(&state)).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body.as_array().unwrap().len(), 1);
    assert_eq!(body[0]["event_category"], "ledger");
    assert_eq!(body[0]["usdc_delta"], "25.000000000000000000");
}

#[tokio::test]
async fn list_account_transactions_rejects_negative_offset() {
    let state = test_state().await;
    let (_, api_key) = seed_agent(&state, "transactions-offset").await;
    let request = Request::builder()
        .uri(
            "/account/transactions?since=2026-07-16T00:00:00Z&until=2026-07-17T00:00:00Z&offset=-1",
        )
        .header("authorization", format!("Bearer {api_key}"))
        .body(Body::empty())
        .unwrap();
    let response = app(state).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn list_account_transactions_rejects_invalid_category() {
    let state = test_state().await;
    let (_, api_key) = seed_agent(&state, "transactions-category").await;
    let request = Request::builder()
        .uri("/account/transactions?since=2026-07-16T00:00:00Z&until=2026-07-17T00:00:00Z&event_category=other")
        .header("authorization", format!("Bearer {api_key}"))
        .body(Body::empty())
        .unwrap();
    let response = app(state).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}
