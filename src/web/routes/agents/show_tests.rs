//! Tests for agents/show_tests.rs
use crate::web::routes::router;
use crate::web::routes::test_support::*;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::util::ServiceExt;

#[tokio::test]
async fn agent_positions_route_renders_latest_trade_execution_summary_under_open_orders() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
    seed_memory_with_type(
        &state,
        &agent_key,
        "trading_decision",
        "Scaled out into strength",
        "Took profit on the upper band.",
    )
    .await;
    seed_memory_with_type(
        &state,
        &agent_key,
        "plan",
        "Older plan",
        "Wait for reclaim.",
    )
    .await;

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert!(text.contains("Open orders"));
    assert!(text.contains("Scaled out into strength"));
    assert!(!text.contains("Older plan"));
}
#[tokio::test]
async fn agent_positions_route_renders_latest_analysis_summary_under_open_orders() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
    let analysis = seed_memory_with_type(
        &state,
        &agent_key,
        "analysis",
        "BTC bullish continuation above 67k",
        "## Thesis\nReclaimed intraday support.",
    )
    .await;
    seed_memory_with_type(
        &state,
        &agent_key,
        "plan",
        "Older plan",
        "Wait for reclaim.",
    )
    .await;

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert!(!text.contains("BTC bullish continuation above 67k"));
    assert!(!text.contains("Older plan"));
    assert!(
        !text.contains(&format!("/agents/{agent_key}/memories/{}", analysis.id)),
        "analysis evidence belongs in the Analysis role, not the positions summary"
    );
}

#[tokio::test]
async fn selected_agent_page_omits_workspace_template_drift_warning() {
    let state = test_state().await;
    let (agent_key, _) = insert_test_opencode_agent(&state)
        .await
        .expect("insert agent");

    let response = router(state)
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        !response_text(response)
            .await
            .contains("data-navbar-workspace-template-drift")
    );
}
#[tokio::test]
async fn agent_positions_route_renders_empty_analysis_section_when_no_market_analysis_memory() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert!(text.contains(">Analysis<"));
    // Empty placeholder — no link to any memory.
    assert!(!text.contains(&format!("/agents/{agent_key}/memories/")));
}
