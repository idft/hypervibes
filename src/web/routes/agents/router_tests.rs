//! Tests for agents/router_tests.rs
use crate::web::routes::router;
use crate::web::routes::test_support::*;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::util::ServiceExt;

#[tokio::test]
async fn agent_chat_route_renders_empty_state() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/chat"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["content-type"],
        "text/html; charset=utf-8"
    );
    let body = response_text(response).await;
    assert!(body.contains("Start a conversation"));
    assert!(body.contains("id=\"agent-show-tab-content\""));
}

#[tokio::test]
async fn new_chat_route_renders_model_selection_even_with_existing_conversations() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");

    let response = router(state)
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/chat/new"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response_text(response)
            .await
            .contains("Start a conversation")
    );
}
#[tokio::test]
async fn unknown_agent_subroute_returns_404() {
    let state = test_state().await;

    let response = router(state)
        .oneshot(
            Request::builder()
                .uri("/agents/does-not-exist-12345/settings")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
