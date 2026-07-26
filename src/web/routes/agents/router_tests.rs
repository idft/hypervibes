//! Tests for agents/router_tests.rs
use crate::web::routes::router;
use crate::web::routes::test_support::*;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use sqlx::query;
use tower::util::ServiceExt;
use uuid::Uuid;

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
async fn new_chat_route_redirects_to_the_latest_existing_conversation() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
    let conversation_id = Uuid::new_v4();
    query(
        "INSERT INTO agent_conversations (
             id, agent_key, opencode_session_id, channel, title, model_provider_id, model_id
         ) VALUES ($1, $2, $3, 'web', 'New conversation', 'ollama-cloud', 'glm-5.2')",
    )
    .bind(conversation_id)
    .bind(&agent_key)
    .bind(format!("ses_{conversation_id}"))
    .execute(&state.db_pool)
    .await
    .expect("insert conversation");

    let response = router(state)
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/chat/new"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response
            .headers()
            .get("location")
            .and_then(|value| value.to_str().ok()),
        Some(format!("/agents/{agent_key}/chat/{conversation_id}").as_str())
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
