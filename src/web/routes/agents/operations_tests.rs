//! Tests for emergency agent operations.

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use tower::util::ServiceExt;

use crate::{
    agents::store::get_agent,
    hyperliquid::orders::{
        gateway::{CancelAllSummary, CancelOutcome},
        model::OrderResult,
    },
    web::routes::{router, test_support::*},
};
use uuid::Uuid;

use super::operations::{inspect_cancel_summary, inspect_close_outcomes};

#[tokio::test]
async fn emergency_stop_stays_disabled_when_the_trading_signer_is_unavailable() {
    let state = test_state().await;
    let (agent_key, _) = insert_test_agent(&state).await.expect("insert agent");

    let response = router(Arc::clone(&state))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/emergency-stop"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let location = response
        .headers()
        .get("location")
        .and_then(|value| value.to_str().ok())
        .expect("redirect location");
    assert!(location.starts_with(&format!("/agents/{agent_key}?notice=")));

    let agent = get_agent(&state.db_pool, &agent_key)
        .await
        .expect("load agent")
        .expect("agent exists");
    assert!(
        !agent.enabled,
        "emergency stop must be durable before cleanup"
    );
}

#[test]
fn cancellation_inspection_counts_only_confirmed_cancellations() {
    let summary = CancelAllSummary {
        considered: 2,
        outcomes: vec![
            CancelOutcome {
                symbol: "BTC".to_string(),
                oid: 10,
                status: "canceled".to_string(),
                error: None,
                order_id: Some(Uuid::new_v4()),
            },
            CancelOutcome {
                symbol: "ETH".to_string(),
                oid: 20,
                status: "error".to_string(),
                error: Some("order was rejected".to_string()),
                order_id: Some(Uuid::new_v4()),
            },
        ],
    };

    let (successful, failures) = inspect_cancel_summary(&summary);

    assert_eq!(successful, 1);
    assert_eq!(failures.len(), 1);
    assert!(failures[0].contains("ETH order 20"));
    assert!(failures[0].contains("order was rejected"));
}

#[test]
fn close_inspection_rejects_unknown_and_rejected_results() {
    let outcomes = vec![
        order_result("unknown", Some("request timed out")),
        order_result("rejected", Some("insufficient margin")),
    ];

    let failures = inspect_close_outcomes("BTC", &outcomes);

    assert_eq!(failures.len(), 2);
    assert!(failures[0].contains("unknown: request timed out"));
    assert!(failures[1].contains("rejected: insufficient margin"));
}

#[test]
fn close_inspection_accepts_only_filled_results() {
    assert!(inspect_close_outcomes("BTC", &[order_result("filled", None)]).is_empty());
    assert!(!inspect_close_outcomes("BTC", &[]).is_empty());
}

fn order_result(status: &str, error: Option<&str>) -> OrderResult {
    OrderResult {
        id: Uuid::new_v4(),
        cloid: format!("0x{:032x}", 1),
        symbol: "BTC".to_string(),
        side: "sell".to_string(),
        order_kind: "market".to_string(),
        status: status.to_string(),
        exchange_oid: None,
        group_id: None,
        error: error.map(str::to_string),
    }
}
