use std::sync::Arc;

use axum::http::{Request, StatusCode};
use serde_json::json;
use tower::util::ServiceExt;
use uuid::Uuid;

use super::test_support::*;
use crate::web::ui_events::UiEvent;

#[tokio::test]
async fn post_notifications_creates_queued_row() {
    let state = test_state().await;
    let mut ui_events = state.ui_events.subscribe();
    let (agent_key, api_key) = seed_agent(&state, "notifications").await;

    let body = json!({
        "title": "Position opened",
        "body": "BTC/USDC long 0.1",
        "severity": "info"
    });
    let (headers, body) = json_body(&body);
    let mut builder = Request::builder()
        .method("POST")
        .uri("/notifications")
        .header("authorization", format!("Bearer {api_key}"));
    if let Some((k, v)) = headers {
        builder = builder.header(k, v);
    }
    let response = app(Arc::clone(&state))
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body_bytes = axum::body::to_bytes(response.into_body(), 16 * 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    let id: Uuid = body["id"].as_str().expect("id string").parse().unwrap();

    let event = ui_events.recv().await.expect("notification UI event");
    assert_eq!(
        event,
        UiEvent::NotificationQueued {
            agent_key: agent_key.clone(),
            notification_id: id,
        }
    );

    let (status,): (String,) = sqlx::query_as("SELECT status FROM notifications WHERE id = $1")
        .bind(id)
        .fetch_one(&state.db_pool)
        .await
        .unwrap();
    assert_eq!(status, "queued");
}

#[tokio::test]
async fn post_notifications_rejects_blank_title_with_422() {
    let state = test_state().await;
    let (_agent_key, api_key) = seed_agent(&state, "notifications-blank").await;

    let body = json!({
        "title": "  ",
        "body": "body",
        "severity": "info"
    });
    let (headers, body) = json_body(&body);
    let mut builder = Request::builder()
        .method("POST")
        .uri("/notifications")
        .header("authorization", format!("Bearer {api_key}"));
    if let Some((k, v)) = headers {
        builder = builder.header(k, v);
    }
    let response = app(state)
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn post_notifications_rejects_unknown_severity_with_422() {
    let state = test_state().await;
    let (_agent_key, api_key) = seed_agent(&state, "notifications-severity").await;

    let body = json!({
        "title": "title",
        "body": "body",
        "severity": "critical"
    });
    let (headers, body) = json_body(&body);
    let mut builder = Request::builder()
        .method("POST")
        .uri("/notifications")
        .header("authorization", format!("Bearer {api_key}"));
    if let Some((k, v)) = headers {
        builder = builder.header(k, v);
    }
    let response = app(state)
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn post_notifications_without_auth_returns_401() {
    let state = test_state().await;
    let body = json!({
        "title": "title",
        "body": "body",
        "severity": "info"
    });
    let (headers, body) = json_body(&body);
    let mut builder = Request::builder()
        .method("POST")
        .uri("/notifications")
        .header("authorization", "Bearer vta_does-not-exist");
    if let Some((k, v)) = headers {
        builder = builder.header(k, v);
    }
    let response = app(state)
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
