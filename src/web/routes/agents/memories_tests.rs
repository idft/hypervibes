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
    assert!(text.contains("Types"));
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
    assert!(text.contains("HyperVibes"));
    assert!(text.contains("data-account-address=\"0x0000000000000000000000000000000000000001\""));
    assert!(text.contains("id=\"memory-detail\""));
    assert!(text.contains("Remember the breakout"));
    assert!(text.contains("<h3>Plan</h3>"));
    assert!(text.contains(&format!("href=\"/agents/{agent_key}/memories\"")));
}
#[tokio::test]
async fn agent_memory_detail_partial_renders_delete_button() {
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
    assert!(text.contains("data-detail-delete-trigger"));
    assert!(text.contains("data-delete-kind=\"memory\""));
    assert!(text.contains(&format!(
        "data-delete-action=\"/agents/{agent_key}/memories/{}/delete\"",
        memory.id
    )));
}
#[tokio::test]
async fn agents_delete_memory_removes_memory_and_redirects() {
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
                .method("POST")
                .uri(format!("/agents/{agent_key}/memories/{}/delete", memory.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response
            .headers()
            .get("location")
            .and_then(|value| value.to_str().ok()),
        Some(format!("/agents/{agent_key}/memories").as_str())
    );
    let deleted = crate::memory::get_memory(&state.db_pool, &agent_key, memory.id)
        .await
        .expect("get memory");
    assert!(deleted.is_none());
}
#[tokio::test]
async fn agents_delete_memory_returns_404_for_unknown_memory() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/agents/{agent_key}/memories/{}/delete",
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
async fn agents_delete_memory_scoped_to_owning_agent() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
    let (other_key, _other_wallet) = insert_test_agent(&state).await.expect("insert agent");
    let memory = seed_memory(
        &state,
        &agent_key,
        "Remember the breakout",
        "### Plan\n\nBTC reclaimed support.",
    )
    .await;

    // Another agent's key in the path must not delete the memory.
    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{other_key}/memories/{}/delete", memory.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let still_there = crate::memory::get_memory(&state.db_pool, &agent_key, memory.id)
        .await
        .expect("get memory");
    assert!(still_there.is_some());
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

    let event =
        render_memory_timeline_event(&state.db_pool, &agent_key, &MemoryDateFilter::default(), "")
            .await
            .expect("render event");
    let text = format!("{event:?}");

    assert!(text.contains("memories-browser"));
    assert!(text.contains("Remember the breakout"));
    let rows = list_agent_memory_timeline(&state.db_pool, &agent_key, None, None, None, None)
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

    let rows = list_agent_memory_timeline(&state.db_pool, &agent_key, None, None, None, None)
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

#[test]
fn local_day_bounds_handle_daylight_saving_transitions() {
    let spring = parse_memory_date_filter("2026-03-08", "America/New_York");
    assert_eq!(
        spring.since.expect("spring start").to_rfc3339(),
        "2026-03-08T05:00:00+00:00"
    );
    assert_eq!(
        spring.until.expect("spring end").to_rfc3339(),
        "2026-03-09T04:00:00+00:00"
    );

    let fall = parse_memory_date_filter("2026-11-01", "America/New_York");
    assert_eq!(
        fall.since.expect("fall start").to_rfc3339(),
        "2026-11-01T04:00:00+00:00"
    );
    assert_eq!(
        fall.until.expect("fall end").to_rfc3339(),
        "2026-11-02T05:00:00+00:00"
    );
}

#[test]
fn rolling_presets_use_the_current_instant_and_preserve_the_filter_in_links() {
    let now = "2026-06-20T12:00:00Z"
        .parse::<chrono::DateTime<chrono::Utc>>()
        .expect("timestamp");
    for (range, hours) in [("1h", 1), ("6h", 6), ("24h", 24)] {
        let query = AgentMemoriesQuery {
            range: range.to_string(),
            ..Default::default()
        };
        let filter = parse_memory_time_filter(&query, now);
        assert_eq!(filter.since, Some(now - chrono::Duration::hours(hours)));
        assert_eq!(filter.until, None);
        assert_eq!(
            memory_page_url("test-agent", "trend_analysis", &filter),
            format!("/agents/test-agent/memories?range={range}&memory_type=trend_analysis")
        );
        assert_eq!(
            parse_memory_time_filter(&query, now + chrono::Duration::minutes(5)).since,
            Some(now + chrono::Duration::minutes(5) - chrono::Duration::hours(hours))
        );
    }
}

#[test]
fn custom_range_includes_the_whole_end_day_across_a_dst_change() {
    let query = AgentMemoriesQuery {
        range: "custom".to_string(),
        start: "2026-03-07".to_string(),
        end: "2026-03-08".to_string(),
        tz: "America/New_York".to_string(),
        ..Default::default()
    };
    let filter = parse_memory_time_filter(&query, chrono::Utc::now());
    assert_eq!(
        filter.since.expect("start").to_rfc3339(),
        "2026-03-07T05:00:00+00:00"
    );
    assert_eq!(
        filter.until.expect("end").to_rfc3339(),
        "2026-03-09T04:00:00+00:00"
    );
    assert!(
        memory_page_url("test-agent", "", &filter)
            .contains("range=custom&start=2026-03-07&end=2026-03-08&tz=America%2FNew_York")
    );

    let backwards = AgentMemoriesQuery {
        start: query.end,
        end: query.start,
        ..query
    };
    let invalid = parse_memory_time_filter(&backwards, chrono::Utc::now());
    assert!(invalid.error_text.is_some());
    assert!(invalid.since.is_none());
}

#[tokio::test]
async fn rolling_and_custom_filters_apply_before_listing_memories() {
    let state = test_state().await;
    let (agent_key, _) = insert_test_agent(&state).await.expect("insert agent");
    let older = seed_memory_with_type(
        &state,
        &agent_key,
        "trend_analysis",
        "Earlier research",
        "Evidence",
    )
    .await;
    let recent = seed_memory_with_type(
        &state,
        &agent_key,
        "trading_decision",
        "Recent decision",
        "Audit",
    )
    .await;
    sqlx::query("UPDATE memory.records SET created_at = now() - interval '7 hours' WHERE id = $1")
        .bind(older.id)
        .execute(&state.db_pool)
        .await
        .expect("date older memory");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/memories?range=6h"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    let text = response_text(response).await;
    assert!(text.contains("Recent decision"));
    assert!(!text.contains("Earlier research"));
    assert!(
        !text.contains("trend_analysis"),
        "hide types with no records in the window"
    );
    assert!(text.contains("trading_decision"));
    assert!(text.contains("range=6h"));

    let created_at: (chrono::DateTime<chrono::Utc>,) =
        sqlx::query_as("SELECT created_at FROM memory.records WHERE id = $1")
            .bind(recent.id)
            .fetch_one(&state.db_pool)
            .await
            .expect("load created_at");
    let date = created_at.0.format("%Y-%m-%d");
    let response = router(state).oneshot(Request::builder()
        .uri(format!("/agents/{agent_key}/memories?range=custom&start={date}&end={date}&tz=UTC&memory_type=trading_decision"))
        .body(Body::empty()).expect("request")).await.expect("response");
    let text = response_text(response).await;
    assert!(text.contains("Recent decision"));
    assert!(!text.contains("Earlier research"));
    assert!(text.contains("range=custom"));
    assert!(text.contains("memory_type=trading_decision"));
}

#[tokio::test]
async fn empty_timeframe_hides_zero_count_types_but_all_restores_them() {
    let state = test_state().await;
    let (agent_key, _) = insert_test_agent(&state).await.expect("insert agent");
    let memory = seed_memory_with_type(
        &state,
        &agent_key,
        "trend_analysis",
        "Older research",
        "Evidence",
    )
    .await;
    sqlx::query("UPDATE memory.records SET created_at = now() - interval '2 hours' WHERE id = $1")
        .bind(memory.id)
        .execute(&state.db_pool)
        .await
        .expect("date memory");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/memories?range=1h"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    let text = response_text(response).await;
    assert!(text.contains("No memories in this timeframe."));
    assert!(!text.contains("trend_analysis"));
    assert!(text.contains("All memories"));

    let response = router(state)
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/memories"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    let text = response_text(response).await;
    assert!(text.contains("trend_analysis"));
    assert!(text.contains("Older research"));
}

#[tokio::test]
async fn memory_types_are_discovered_and_filtered_before_pagination() {
    let state = test_state().await;
    let (agent_key, _) = insert_test_agent(&state).await.expect("insert agent");
    let analysis = seed_memory_with_type(
        &state,
        &agent_key,
        "trend_analysis",
        "Research worth reading",
        "Evidence",
    )
    .await;
    for index in 0..55 {
        seed_memory_with_type(
            &state,
            &agent_key,
            "trading_decision",
            &format!("Decision {index}"),
            "Audit",
        )
        .await;
    }

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/agents/{agent_key}/memories?memory_type=trend_analysis"
                ))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert!(text.contains("Research worth reading"));
    assert!(text.contains("trend_analysis"));
    assert!(text.contains("trading_decision"));
    assert_eq!(
        text.matches("data-memory-timeline-item data-memory-id")
            .count(),
        1
    );
    assert!(text.contains(&format!("data-memory-detail-id=\"{}\"", analysis.id)));
    assert!(text.contains("memory_type=trend_analysis"));

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/memories"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    let text = response_text(response).await;
    assert_eq!(
        text.matches("data-memory-timeline-item data-memory-id")
            .count(),
        50
    );
    assert!(!text.contains("Research worth reading"));

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/agents/{agent_key}/memories?memory_type=trading_decision"
                ))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    let text = response_text(response).await;
    assert_eq!(
        text.matches("data-memory-timeline-item data-memory-id")
            .count(),
        50
    );
    assert!(text.contains("before_id="));
    assert!(text.contains("memory_type=trading_decision"));

    let rows = list_agent_memory_timeline(&state.db_pool, &agent_key, None, None, None, None)
        .await
        .expect("timeline");
    let cursor = &rows[49];
    let response = router(state).oneshot(Request::builder()
        .uri(format!("/agents/{agent_key}/memories/timeline?before_us={}&before_id={}&memory_type=trading_decision", cursor.created_at.timestamp_micros(), cursor.id))
        .body(Body::empty()).expect("request")).await.expect("response");
    let text = response_text(response).await;
    assert_eq!(
        text.matches("data-memory-timeline-item data-memory-id")
            .count(),
        5
    );
}

#[tokio::test]
async fn local_date_filter_applies_to_memories_and_type_counts() {
    let state = test_state().await;
    let (agent_key, _) = insert_test_agent(&state).await.expect("insert agent");
    let previous = seed_memory_with_type(
        &state,
        &agent_key,
        "trend_analysis",
        "Previous local day",
        "Evidence",
    )
    .await;
    let today = seed_memory_with_type(
        &state,
        &agent_key,
        "trading_decision",
        "Selected local day",
        "Audit",
    )
    .await;
    for (id, instant) in [
        (previous.id, "2026-06-20T06:59:59Z"),
        (today.id, "2026-06-20T07:00:00Z"),
    ] {
        sqlx::query("UPDATE memory.records SET created_at = $1 WHERE id = $2")
            .bind(
                instant
                    .parse::<chrono::DateTime<chrono::Utc>>()
                    .expect("timestamp"),
            )
            .bind(id)
            .execute(&state.db_pool)
            .await
            .expect("set memory date");
    }

    let response = router(state)
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/agents/{agent_key}/memories?date=2026-06-20&tz=America%2FLos_Angeles"
                ))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert!(text.contains("Selected local day"));
    assert!(!text.contains("Previous local day"));
    assert!(
        !text.contains("trend_analysis"),
        "hide types without memories on the selected day"
    );
    assert!(text.contains("tz=America%2FLos_Angeles"));
    assert_eq!(
        text.matches("data-memory-timeline-item data-memory-id")
            .count(),
        1
    );
}
