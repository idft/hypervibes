//! Tests for agents/show_tests.rs
use crate::web::routes::router;
use crate::web::routes::test_support::*;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use std::fs;
use tower::util::ServiceExt;

#[tokio::test]
async fn agent_positions_route_renders_latest_trade_execution_summary_under_open_orders() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
    seed_memory_with_type(
        &state,
        &agent_key,
        "trade_execution",
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
        "market_analysis",
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
    assert!(text.contains(">Analysis<"));
    assert!(text.contains("BTC bullish continuation above 67k"));
    assert!(!text.contains("Older plan"));
    assert!(
        text.contains(&format!("/agents/{agent_key}/memories/{}", analysis.id)),
        "analysis summary should link to the memory detail page"
    );
}

#[tokio::test]
async fn selected_agent_page_renders_workspace_template_drift_warning() {
    let state = test_state().await;
    let (agent_key, _) = insert_test_opencode_agent(&state)
        .await
        .expect("insert agent");
    generate_test_agent_workspace(&state, &agent_key).await;
    let workspace_path = crate::opencode::workspace::agent_workspace_host_path(
        &state.opencode_workspace_config,
        &agent_key,
    )
    .expect("workspace path");
    let clean_response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(clean_response.status(), StatusCode::OK);
    assert!(
        !response_text(clean_response)
            .await
            .contains("data-navbar-workspace-template-drift")
    );
    fs::write(workspace_path.join("AGENTS.md"), "user-modified\n").expect("modify AGENTS.md");

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
    let text = response_text(response).await;
    assert!(text.contains("data-navbar-workspace-template-drift"));
    assert!(text.contains(&format!("href=\"/agents/{agent_key}/settings\"")));
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
