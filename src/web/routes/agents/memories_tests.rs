//! Tests for agents/memories_tests.rs
use super::*;
use crate::web::routes::router;
use crate::web::routes::test_support::*;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::util::ServiceExt;

use crate::memory::list_agent_memory_timeline;

#[tokio::test]
async fn agent_memories_route_renders_saved_memories() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
    let memory = seed_memory(
        &state,
        &agent_key,
        "Remember the breakout",
        "### Plan\n\nBTC reclaimed support.",
    )
    .await;

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/memories"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert!(text.contains("Memories"));
    assert!(text.contains("Timeline"));
    assert!(text.contains("Remember the breakout"));
    assert!(text.contains("<h3>Plan</h3>"));
    assert!(text.contains("BTC reclaimed support."));
    assert!(text.contains(&format!("/agents/{agent_key}/memories/{}", memory.id)));
}
#[tokio::test]
async fn agent_memory_detail_route_renders_partial_for_htmx() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
    let memory = seed_memory(
        &state,
        &agent_key,
        "Remember the breakout",
        "### Plan\n\nBTC reclaimed support.",
    )
    .await;

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/memories/{}", memory.id))
                .header("HX-Request", "true")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert!(text.contains("id=\"memory-detail\""));
    assert!(text.contains("Remember the breakout"));
    assert!(text.contains("<h3>Plan</h3>"));
    // Bare partial — no base layout, no nav, no back link.
    assert!(!text.contains("<!DOCTYPE html>"));
    assert!(!text.contains("Back to"));
}
#[tokio::test]
async fn agent_memory_detail_route_renders_full_page_for_direct_navigation() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
    let memory = seed_memory(
        &state,
        &agent_key,
        "Remember the breakout",
        "### Plan\n\nBTC reclaimed support.",
    )
    .await;

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/memories/{}", memory.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    // Full styled page so direct navigation doesn't render unstyled HTML.
    assert!(text.contains("<!DOCTYPE html>"));
    assert!(text.contains("/static/dist/app.css"));
    assert!(text.contains("Vibetrading"));
    assert!(text.contains("id=\"memory-detail\""));
    assert!(text.contains("Remember the breakout"));
    assert!(text.contains("<h3>Plan</h3>"));
    assert!(text.contains("Agent sections"));
    assert!(text.contains(&format!("href=\"/agents/{agent_key}/memories\"")));
}
#[tokio::test]
async fn agent_memory_detail_route_returns_404_for_unknown_memory() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/agents/{agent_key}/memories/{}",
                    uuid::Uuid::new_v4()
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
#[tokio::test]
async fn memory_timeline_event_renders_refresh_without_selected_card() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
    seed_memory(
        &state,
        &agent_key,
        "Remember the breakout",
        "### Plan\n\nBTC reclaimed support.",
    )
    .await;

    let event = render_memory_timeline_event(
        &state.db_pool,
        &agent_key,
        None,
        None,
        None,
        String::new(),
    )
        .await
        .expect("render event");
    let text = format!("{event:?}");

    assert!(text.contains("memories-timeline"));
    assert!(text.contains("Remember the breakout"));
    let rows = list_agent_memory_timeline(&state.db_pool, &agent_key, None, None, None)
        .await
        .expect("list memories");
    let timeline = crate::web::templates::build_memory_timeline_for_sse(&agent_key, &rows);
    assert!(timeline.iter().all(|item| !item.selected));
}

#[tokio::test]
async fn agent_memories_route_pages_timeline_and_loads_older_rows() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
    for index in 0..55 {
        seed_memory(
            &state,
            &agent_key,
            &format!("Memory {index}"),
            "Timeline pagination content.",
        )
        .await;
    }

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/memories"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert_eq!(
        text.matches("data-memory-timeline-item data-memory-id")
            .count(),
        50
    );
    assert!(text.contains("data-memory-timeline-loader"));

    let rows = list_agent_memory_timeline(&state.db_pool, &agent_key, None, None, None)
        .await
        .expect("list first timeline page");
    let cursor = &rows[49];
    let response = router(state)
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/agents/{agent_key}/memories/timeline?before_us={}&before_id={}",
                    cursor.created_at.timestamp_micros(),
                    cursor.id
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert_eq!(
        text.matches("data-memory-timeline-item data-memory-id")
            .count(),
        5
    );
    assert!(!text.contains("data-memory-timeline-loader"));
}
