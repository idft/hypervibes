//! Tests for backends_tests.rs
use crate::web::routes::router;
use crate::web::routes::test_support::*;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::util::ServiceExt;

#[tokio::test]
async fn backends_new_page_renders_create_form() {
    let state = test_state().await;

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/backends/new")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert!(text.contains("Create backend"));
    assert!(text.contains("name=\"id\""));
    assert!(text.contains("name=\"base_url\""));
}
#[tokio::test]
async fn backends_index_renders_seeded_runtime() {
    let state = test_state().await;

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/backends")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert!(text.contains("OpenCode local"));
    assert!(text.contains("opencode-local"));
    assert!(text.contains("http://localhost:14096"));
}
