//! Tests for agents/prompts_tests.rs
use crate::web::routes::router;
use crate::web::routes::test_support::*;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::util::ServiceExt;

use crate::agents::store::get_agent;

#[tokio::test]
async fn agent_prompts_route_renders_prompt_fields() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_agent_with_text(
        &state,
        "Wait for analysis confirmation first.".to_string(),
        "Trade breakouts only after confirmation.".to_string(),
    )
    .await
    .expect("insert agent");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/prompts"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert!(text.contains("Wait for analysis confirmation first."));
    assert!(text.contains("Trade breakouts only after confirmation."));
}
#[tokio::test]
async fn post_agent_analysis_prompt_updates_only_analysis() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");

    let body = "prompt=Analyze+momentum+with+market+structure.";
    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/prompts/analysis"))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let expected_location = format!("/agents/{agent_key}/prompts");
    assert_eq!(
        response
            .headers()
            .get("location")
            .and_then(|value| value.to_str().ok()),
        Some(expected_location.as_str())
    );

    let stored = get_agent(&pool, &agent_key)
        .await
        .expect("get agent")
        .expect("agent present");
    assert_eq!(
        stored.analysis_prompt,
        "Analyze momentum with market structure."
    );

    let body2 = "prompt=Original+trading+prompt.";
    let _ = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/prompts/trading"))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(body2))
                .unwrap(),
        )
        .await
        .unwrap();

    let stored = get_agent(&pool, &agent_key)
        .await
        .expect("get agent")
        .expect("agent present");
    assert_eq!(stored.trading_prompt, "Original trading prompt.");
    assert_eq!(
        stored.analysis_prompt,
        "Analyze momentum with market structure."
    );
}
#[tokio::test]
async fn post_agent_trading_prompt_updates_only_trading() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");

    let body = "prompt=Only+place+limit+orders+near+support.";
    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/prompts/trading"))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let expected_location = format!("/agents/{agent_key}/prompts");
    assert_eq!(
        response
            .headers()
            .get("location")
            .and_then(|value| value.to_str().ok()),
        Some(expected_location.as_str())
    );

    let stored = get_agent(&pool, &agent_key)
        .await
        .expect("get agent")
        .expect("agent present");
    assert_eq!(
        stored.trading_prompt,
        "Only place limit orders near support."
    );

    let body2 = "prompt=Original+analysis+prompt.";
    let _ = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/prompts/analysis"))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(body2))
                .unwrap(),
        )
        .await
        .unwrap();

    let stored = get_agent(&pool, &agent_key)
        .await
        .expect("get agent")
        .expect("agent present");
    assert_eq!(stored.analysis_prompt, "Original analysis prompt.");
    assert_eq!(
        stored.trading_prompt,
        "Only place limit orders near support."
    );
}
