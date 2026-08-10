use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::{DateTime, Duration, Utc};
use serde_json::json;
use tower::util::ServiceExt;
use uuid::Uuid;

use crate::web::ui_events::UiEvent;

use super::test_support::*;
#[tokio::test]
async fn post_memories_creates_record() {
    let state = test_state().await;
    let mut ui_events = state.ui_events.subscribe();

    let (agent_key, api_key) = seed_agent(&state, "create").await;

    let body = serde_json::json!({
        "symbol": "BTC",
        "timeframe": "1h",
        "memory_type": "plan",
        "summary": "buy pullback",
        "content": "BTC reclaimed the prior breakout level",
        "metadata": { "confidence": 0.72 }
    });
    let (headers, body) = json_body(&body);
    let mut builder = Request::builder()
        .method("POST")
        .uri("/memories")
        .header("authorization", format!("Bearer {api_key}"));
    if let Some((k, v)) = headers {
        builder = builder.header(k, v);
    }
    let response = app(Arc::clone(&state))
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let ct = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        ct.starts_with("application/json"),
        "expected application/json content-type, got {ct}"
    );

    let body_bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(body["symbol"], "BTC");
    assert_eq!(body["timeframe"], "1h");
    assert_eq!(body["summary"], "buy pullback");
    assert_eq!(body["metadata"]["confidence"], 0.72);
    assert!(body["id"].is_string());
    // `plan` rows have no implicit expiration, so `expires_at` is null.
    assert!(body["expires_at"].is_null());

    let event = ui_events.recv().await.expect("memory UI event");
    assert_eq!(
        event,
        UiEvent::MemoryCreated {
            agent_key,
            memory_id: Uuid::parse_str(body["id"].as_str().unwrap()).unwrap(),
        }
    );
}

#[tokio::test]
async fn post_memories_response_includes_expires_at_from_valid_for_seconds() {
    let state = test_state().await;
    let (_agent_key, api_key) = seed_agent(&state, "create-exp").await;

    let body = serde_json::json!({
        "symbol": "BTC",
        "timeframe": "15m",
        "memory_type": "analysis",
        "summary": "btc analysis",
        "content": "body",
        "metadata": { "valid_for_seconds": 600 }
    });
    let (headers, body) = json_body(&body);
    let mut builder = Request::builder()
        .method("POST")
        .uri("/memories")
        .header("authorization", format!("Bearer {api_key}"));
    if let Some((k, v)) = headers {
        builder = builder.header(k, v);
    }
    let response = app(Arc::clone(&state))
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body_bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    let expires_at = body["expires_at"]
        .as_str()
        .expect("expires_at should be present for analysis with valid_for_seconds");
    let created_at: DateTime<Utc> =
        DateTime::parse_from_rfc3339(body["created_at"].as_str().unwrap())
            .unwrap()
            .with_timezone(&Utc);
    let expires_at_parsed: DateTime<Utc> = DateTime::parse_from_rfc3339(expires_at)
        .unwrap()
        .with_timezone(&Utc);
    let delta = (expires_at_parsed - created_at).num_seconds();
    assert_eq!(
        delta, 600,
        "expires_at must equal created_at + valid_for_seconds"
    );
}

#[tokio::test]
async fn post_memories_response_uses_analysis_default_when_no_valid_for_seconds() {
    let state = test_state().await;
    let (_agent_key, api_key) = seed_agent(&state, "create-default-exp").await;

    let body = serde_json::json!({
        "symbol": "BTC",
        "timeframe": "15m",
        "memory_type": "analysis",
        "summary": "btc analysis no meta",
        "content": "body"
    });
    let (headers, body) = json_body(&body);
    let mut builder = Request::builder()
        .method("POST")
        .uri("/memories")
        .header("authorization", format!("Bearer {api_key}"));
    if let Some((k, v)) = headers {
        builder = builder.header(k, v);
    }
    let response = app(Arc::clone(&state))
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body_bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    // 15m analysis default is 30m (2x the 15m schedule interval), so
    // expires_at = created_at + 30m.
    let expires_at = body["expires_at"]
        .as_str()
        .expect("analysis with no valid_for_seconds still has a default expires_at");
    let created_at: DateTime<Utc> =
        DateTime::parse_from_rfc3339(body["created_at"].as_str().unwrap())
            .unwrap()
            .with_timezone(&Utc);
    let expires_at_parsed: DateTime<Utc> = DateTime::parse_from_rfc3339(expires_at)
        .unwrap()
        .with_timezone(&Utc);
    let delta = (expires_at_parsed - created_at).num_minutes();
    assert_eq!(
        delta, 30,
        "15m analysis default validity is 30m (2x schedule interval)"
    );
}

#[tokio::test]
async fn post_memories_without_auth_returns_401_json() {
    let state = test_state().await;

    let body = serde_json::json!({
        "symbol": "BTC",
        "memory_type": "plan",
        "summary": "x",
        "content": "y"
    });
    let (headers, body) = json_body(&body);
    let mut builder = Request::builder().method("POST").uri("/memories");
    if let Some((k, v)) = headers {
        builder = builder.header(k, v);
    }
    let response = app(state)
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body_bytes = axum::body::to_bytes(response.into_body(), 16 * 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert!(body["error"].is_string());
}

#[tokio::test]
async fn post_memories_with_invalid_bearer_returns_401_json() {
    let state = test_state().await;

    let body = serde_json::json!({
        "symbol": "BTC",
        "memory_type": "plan",
        "summary": "x",
        "content": "y"
    });
    let (headers, body) = json_body(&body);
    let mut builder = Request::builder()
        .method("POST")
        .uri("/memories")
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

#[tokio::test]
async fn post_memories_empty_field_returns_422_json() {
    let state = test_state().await;

    let (_agent_key, api_key) = seed_agent(&state, "val").await;

    let body = serde_json::json!({
        "symbol": "BTC",
        "memory_type": "plan",
        "summary": "  ",
        "content": "y"
    });
    let (headers, body) = json_body(&body);
    let mut builder = Request::builder()
        .method("POST")
        .uri("/memories")
        .header("authorization", format!("Bearer {api_key}"));
    if let Some((k, v)) = headers {
        builder = builder.header(k, v);
    }
    let response = app(state)
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body_bytes = axum::body::to_bytes(response.into_body(), 16 * 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert!(body["error"].as_str().unwrap().contains("summary"));
}

#[tokio::test]
async fn post_market_analysis_with_timeframe_returns_422() {
    let state = test_state().await;
    let (_agent_key, api_key) = seed_agent(&state, "market-analysis-timeframe").await;

    let body = serde_json::json!({
        "symbol": "BTC",
        "timeframe": "__omit__",
        "memory_type": "market_analysis",
        "summary": "invalid market analysis",
        "content": "timeframe must be absent"
    });
    let (headers, body) = json_body(&body);
    let mut builder = Request::builder()
        .method("POST")
        .uri("/memories")
        .header("authorization", format!("Bearer {api_key}"));
    if let Some((key, value)) = headers {
        builder = builder.header(key, value);
    }
    let response = app(state)
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = axum::body::to_bytes(response.into_body(), 16 * 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        body["error"]
            .as_str()
            .expect("validation error")
            .contains("market_analysis memories must omit timeframe")
    );
}

#[tokio::test]
async fn post_memories_non_object_metadata_returns_422() {
    let state = test_state().await;

    let (_agent_key, api_key) = seed_agent(&state, "meta").await;

    let body = serde_json::json!({
        "symbol": "BTC",
        "memory_type": "plan",
        "summary": "x",
        "content": "y",
        "metadata": [1, 2, 3]
    });
    let (headers, body) = json_body(&body);
    let mut builder = Request::builder()
        .method("POST")
        .uri("/memories")
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
async fn auth_touches_api_key_last_used_at() {
    let state = test_state().await;

    let (agent_key, api_key) = seed_agent(&state, "touch").await;

    let body = serde_json::json!({
        "symbol": "BTC",
        "memory_type": "plan",
        "summary": "x",
        "content": "y"
    });
    let (headers, body) = json_body(&body);
    let mut builder = Request::builder()
        .method("POST")
        .uri("/memories")
        .header("authorization", format!("Bearer {api_key}"));
    if let Some((k, v)) = headers {
        builder = builder.header(k, v);
    }
    let response = app(Arc::clone(&state))
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    let (last_used_at,): (Option<chrono::DateTime<chrono::Utc>>,) =
        sqlx::query_as("SELECT api_key_last_used_at FROM agents WHERE agent_key = $1")
            .bind(agent_key)
            .fetch_one(&state.db_pool)
            .await
            .unwrap();
    assert!(last_used_at.is_some());
}

#[tokio::test]
async fn list_memories_filters_and_orders_desc() {
    let state = test_state().await;

    let (_agent_key, api_key) = seed_agent(&state, "list").await;

    for summary in ["a", "b", "c"] {
        let body = serde_json::json!({
            "symbol": "BTC",
            "timeframe": "1h",
            "memory_type": "plan",
            "summary": summary,
            "content": format!("body {summary}"),
        });
        let (headers, body) = json_body(&body);
        let mut builder = Request::builder()
            .method("POST")
            .uri("/memories")
            .header("authorization", format!("Bearer {api_key}"));
        if let Some((k, v)) = headers {
            builder = builder.header(k, v);
        }
        let response = app(Arc::clone(&state))
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
    }

    // Also insert a general (NULL timeframe) memory.
    let body = serde_json::json!({
        "symbol": "BTC",
        "memory_type": "plan",
        "summary": "general",
        "content": "general"
    });
    let (headers, body) = json_body(&body);
    let mut builder = Request::builder()
        .method("POST")
        .uri("/memories")
        .header("authorization", format!("Bearer {api_key}"));
    if let Some((k, v)) = headers {
        builder = builder.header(k, v);
    }
    let response = app(Arc::clone(&state))
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    // List the 1h timeframe: should return the 3 plan memories in DESC order.
    let request = Request::builder()
        .uri("/memories?symbol=BTC&timeframe=1h&limit=10")
        .header("authorization", format!("Bearer {api_key}"))
        .body(Body::empty())
        .unwrap();
    let response = app(Arc::clone(&state)).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    let rows: Vec<serde_json::Value> = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(rows.len(), 3);
    let summaries: Vec<&str> = rows
        .iter()
        .map(|r| r["summary"].as_str().unwrap())
        .collect();
    assert_eq!(summaries, vec!["c", "b", "a"]);

    // No timeframe => all timeframes (including NULL) in newest-first order.
    let request = Request::builder()
        .uri("/memories?symbol=BTC")
        .header("authorization", format!("Bearer {api_key}"))
        .body(Body::empty())
        .unwrap();
    let response = app(Arc::clone(&state)).oneshot(request).await.unwrap();
    let body_bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    let rows: Vec<serde_json::Value> = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(rows.len(), 4, "omitted timeframe returns all timeframes");
    let summaries: Vec<&str> = rows
        .iter()
        .map(|r| r["summary"].as_str().unwrap())
        .collect();
    assert_eq!(summaries, vec!["general", "c", "b", "a"]);
    let first = rows.first().expect("at least one row");
    assert!(
        first["timeframe"].is_null(),
        "newest NULL-timeframe row remains in the set"
    );

    // Scope check: another agent must not see any of these rows.
    let (_other_key, other_api_key) = seed_agent(&state, "other").await;
    let request = Request::builder()
        .uri("/memories?symbol=BTC&timeframe=1h")
        .header("authorization", format!("Bearer {other_api_key}"))
        .body(Body::empty())
        .unwrap();
    let response = app(Arc::clone(&state)).oneshot(request).await.unwrap();
    let body_bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    let rows: Vec<serde_json::Value> = serde_json::from_slice(&body_bytes).unwrap();
    assert!(rows.is_empty());
}

#[tokio::test]
async fn list_memories_returns_fresh_null_timeframe_market_analysis() {
    let state = test_state().await;
    let (_agent_key, api_key) = seed_agent(&state, "market-analysis-list").await;

    let body = serde_json::json!({
        "symbol": "BTC",
        "memory_type": "market_analysis",
        "summary": "wait for confirmation",
        "content": "No new exposure.",
        "metadata": { "valid_for_seconds": 1800 }
    });
    let (headers, body) = json_body(&body);
    let mut builder = Request::builder()
        .method("POST")
        .uri("/memories")
        .header("authorization", format!("Bearer {api_key}"));
    if let Some((key, value)) = headers {
        builder = builder.header(key, value);
    }
    let response = app(Arc::clone(&state))
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    let (status, body) = get_json_response(
        &state,
        &api_key,
        "/memories?symbol=BTC&memory_type=market_analysis&limit=1",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let rows = body.as_array().expect("array response");
    assert_eq!(rows.len(), 1);
    assert!(rows[0]["timeframe"].is_null());
    assert_eq!(rows[0]["summary"], "wait for confirmation");
    assert!(rows[0]["expires_at"].is_string());
}

#[tokio::test]
async fn list_memories_orders_before_filtering_expired_legacy_timeframe_rows() {
    let state = test_state().await;
    let (agent_key, api_key) = seed_agent(&state, "market-analysis-legacy").await;
    let now = Utc::now();

    insert_memory_at(
        &state,
        &agent_key,
        now - Duration::hours(2),
        "BTC",
        Some("__omit__"),
        "market_analysis",
        "expired legacy row",
        json!({ "valid_for_seconds": 1800 }),
    )
    .await;
    insert_memory_at(
        &state,
        &agent_key,
        now - Duration::minutes(1),
        "BTC",
        None,
        "market_analysis",
        "fresh handoff",
        json!({ "valid_for_seconds": 1800 }),
    )
    .await;

    let (status, body) = get_json_response(
        &state,
        &api_key,
        "/memories?symbol=BTC&memory_type=market_analysis&limit=1",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let rows = body.as_array().expect("array response");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["summary"], "fresh handoff");
    assert!(rows[0]["timeframe"].is_null());
}

#[tokio::test]
async fn list_memories_hides_expired_by_default_and_include_expired_returns_them() {
    let state = test_state().await;
    let (agent_key, api_key) = seed_agent(&state, "list-expired").await;
    let now = Utc::now();

    // Fresh row: 5m old, well within the 30m default 15m analysis
    // validity window. Must show up in both queries.
    insert_memory_at(
        &state,
        &agent_key,
        now - Duration::minutes(5),
        "BTC",
        Some("15m"),
        "analysis",
        "fresh",
        json!({}),
    )
    .await;
    // Expired row: 40m old, beyond the 30m default validity window.
    // Must be hidden by default and visible only with include_expired.
    insert_memory_at(
        &state,
        &agent_key,
        now - Duration::minutes(40),
        "BTC",
        Some("15m"),
        "analysis",
        "expired",
        json!({}),
    )
    .await;

    // Default: expired row is hidden, only the fresh one is returned.
    let (status, body) = get_json_response(
        &state,
        &api_key,
        "/memories?symbol=BTC&memory_type=analysis",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let rows = body.as_array().expect("array response");
    assert_eq!(rows.len(), 1, "expired row hidden by default");
    assert_eq!(rows[0]["summary"], "fresh");

    // Explicit include_expired=false matches the default.
    let (status, body) = get_json_response(
        &state,
        &api_key,
        "/memories?symbol=BTC&memory_type=analysis&include_expired=false",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let rows = body.as_array().expect("array response");
    assert_eq!(rows.len(), 1, "include_expired=false matches default");
    assert_eq!(rows[0]["summary"], "fresh");

    // include_expired=true returns both rows, newest first.
    let (status, body) = get_json_response(
        &state,
        &api_key,
        "/memories?symbol=BTC&memory_type=analysis&include_expired=true",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let rows = body.as_array().expect("array response");
    assert_eq!(rows.len(), 2, "include_expired=true returns expired rows");
    let summaries: Vec<&str> = rows
        .iter()
        .map(|r| r["summary"].as_str().unwrap())
        .collect();
    assert_eq!(summaries, vec!["fresh", "expired"]);
}

#[tokio::test]
async fn list_memories_include_expired_keeps_explicit_valid_for_seconds() {
    let state = test_state().await;
    let (_agent_key, api_key) = seed_agent(&state, "list-exp-explicit").await;
    let now = Utc::now();

    // Explicit valid_for_seconds=600 (10m) on a row that's 20m old.
    // The default analysis validity window (30m) would consider it
    // fresh, but the explicit valid_for_seconds wins, so it's expired.
    let body = serde_json::json!({
        "symbol": "BTC",
        "timeframe": "15m",
        "memory_type": "analysis",
        "summary": "explicit-short",
        "content": "body",
        "metadata": { "valid_for_seconds": 600 }
    });
    let (headers, body) = json_body(&body);
    let mut builder = Request::builder()
        .method("POST")
        .uri("/memories")
        .header("authorization", format!("Bearer {api_key}"));
    if let Some((k, v)) = headers {
        builder = builder.header(k, v);
    }
    let response = app(Arc::clone(&state))
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    // Backdate the row directly so the explicit 10m validity has lapsed.
    sqlx::query("UPDATE memory.records SET created_at = $1")
        .bind(now - Duration::minutes(20))
        .execute(&state.db_pool)
        .await
        .unwrap();

    // Default: hidden.
    let (status, body) = get_json_response(
        &state,
        &api_key,
        "/memories?symbol=BTC&memory_type=analysis",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body.as_array().expect("array").len(),
        0,
        "explicit valid_for_seconds still hides expired row by default"
    );

    // include_expired=true: visible.
    let (status, body) = get_json_response(
        &state,
        &api_key,
        "/memories?symbol=BTC&memory_type=analysis&include_expired=true",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_array().expect("array").len(), 1);
}

#[tokio::test]
async fn latest_memories_returns_latest_valid_per_timeframe() {
    let state = test_state().await;
    let (agent_key, api_key) = seed_agent(&state, "latest-per-tf").await;
    let now = Utc::now();

    insert_memory_at(
        &state,
        &agent_key,
        now - Duration::minutes(12),
        "BTC",
        Some("15m"),
        "analysis",
        "15m-old",
        json!({}),
    )
    .await;
    insert_memory_at(
        &state,
        &agent_key,
        now - Duration::minutes(5),
        "BTC",
        Some("15m"),
        "analysis",
        "15m-new",
        json!({}),
    )
    .await;
    insert_memory_at(
        &state,
        &agent_key,
        now - Duration::minutes(30),
        "BTC",
        Some("1h"),
        "analysis",
        "1h-new",
        json!({}),
    )
    .await;
    insert_memory_at(
        &state,
        &agent_key,
        now - Duration::hours(4),
        "BTC",
        Some("1d"),
        "analysis",
        "1d-new",
        json!({}),
    )
    .await;

    let (status, body) = latest_memories_response(
        &state,
        &api_key,
        "/memories/latest?symbol=BTC&memory_type=analysis",
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let rows = body.as_array().expect("array response");
    assert_eq!(rows.len(), 3);
    let summaries: Vec<&str> = rows
        .iter()
        .map(|row| row["summary"].as_str().unwrap())
        .collect();
    assert_eq!(summaries, vec!["15m-new", "1h-new", "1d-new"]);
    assert!(rows.iter().all(|row| row["expires_at"].is_string()));
}

#[tokio::test]
async fn latest_memories_applies_limit_after_grouping() {
    let state = test_state().await;
    let (agent_key, api_key) = seed_agent(&state, "latest-limit").await;
    let now = Utc::now();

    insert_memory_at(
        &state,
        &agent_key,
        now - Duration::minutes(12),
        "BTC",
        Some("15m"),
        "analysis",
        "15m-old",
        json!({}),
    )
    .await;
    insert_memory_at(
        &state,
        &agent_key,
        now - Duration::minutes(5),
        "BTC",
        Some("15m"),
        "analysis",
        "15m-new",
        json!({}),
    )
    .await;
    insert_memory_at(
        &state,
        &agent_key,
        now - Duration::minutes(30),
        "BTC",
        Some("1h"),
        "analysis",
        "1h-new",
        json!({}),
    )
    .await;
    insert_memory_at(
        &state,
        &agent_key,
        now - Duration::hours(4),
        "BTC",
        Some("1d"),
        "analysis",
        "1d-new",
        json!({}),
    )
    .await;

    let (status, body) = latest_memories_response(
        &state,
        &api_key,
        "/memories/latest?symbol=BTC&memory_type=analysis&limit=2",
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let rows = body.as_array().expect("array response");
    assert_eq!(rows.len(), 2);
    let summaries: Vec<&str> = rows
        .iter()
        .map(|row| row["summary"].as_str().unwrap())
        .collect();
    assert_eq!(summaries, vec!["15m-new", "1h-new"]);
}

#[tokio::test]
async fn latest_memories_excludes_stale_analysis() {
    let state = test_state().await;
    let (agent_key, api_key) = seed_agent(&state, "latest-stale").await;
    let now = Utc::now();

    insert_memory_at(
        &state,
        &agent_key,
        now - Duration::minutes(1),
        "BTC",
        Some("15m"),
        "analysis",
        "stale",
        json!({ "stale_after": (now - Duration::seconds(1)).to_rfc3339() }),
    )
    .await;

    let (status, body) = latest_memories_response(
        &state,
        &api_key,
        "/memories/latest?symbol=BTC&memory_type=analysis",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!([]));
}

#[tokio::test]
async fn latest_memories_uses_analysis_timeframe_defaults() {
    let state = test_state().await;
    let (agent_key, api_key) = seed_agent(&state, "latest-defaults").await;
    let now = Utc::now();

    // 1d analysis default is 48h; insert 49h old to make sure it
    // crosses the default staleness boundary.
    insert_memory_at(
        &state,
        &agent_key,
        now - Duration::hours(49),
        "BTC",
        Some("1d"),
        "analysis",
        "expired-by-default",
        json!({}),
    )
    .await;

    let (status, body) = latest_memories_response(
        &state,
        &api_key,
        "/memories/latest?symbol=BTC&memory_type=analysis",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!([]));
}

#[tokio::test]
async fn latest_memories_filters_by_memory_type() {
    let state = test_state().await;
    let (agent_key, api_key) = seed_agent(&state, "latest-type").await;
    let now = Utc::now();

    insert_memory_at(
        &state,
        &agent_key,
        now - Duration::minutes(4),
        "BTC",
        Some("15m"),
        "analysis",
        "analysis-row",
        json!({}),
    )
    .await;
    insert_memory_at(
        &state,
        &agent_key,
        now - Duration::minutes(1),
        "BTC",
        Some("15m"),
        "reflection",
        "reflection-row",
        json!({}),
    )
    .await;

    let (status, body) = latest_memories_response(
        &state,
        &api_key,
        "/memories/latest?symbol=BTC&memory_type=analysis",
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let rows = body.as_array().expect("array response");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["summary"], "analysis-row");
    assert_eq!(rows[0]["memory_type"], "analysis");
}

#[tokio::test]
async fn latest_memories_excludes_null_timeframe() {
    let state = test_state().await;
    let (agent_key, api_key) = seed_agent(&state, "latest-null-tf").await;

    insert_memory_at(
        &state,
        &agent_key,
        Utc::now(),
        "BTC",
        None,
        "analysis",
        "general",
        json!({}),
    )
    .await;

    let (status, body) = latest_memories_response(
        &state,
        &api_key,
        "/memories/latest?symbol=BTC&memory_type=analysis",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!([]));
}

#[tokio::test]
async fn latest_memories_requires_symbol_and_memory_type() {
    let state = test_state().await;
    let (_agent_key, api_key) = seed_agent(&state, "latest-validate").await;

    let (status, body) = latest_memories_response(&state, &api_key, "/memories/latest").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("symbol is required")
    );
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("memory_type is required")
    );

    let (status, body) = latest_memories_response(
        &state,
        &api_key,
        "/memories/latest?symbol=%20%20&memory_type=%20%20",
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("symbol is required")
    );
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("memory_type is required")
    );
}

#[tokio::test]
async fn latest_memories_rejects_invalid_limit() {
    let state = test_state().await;
    let (_agent_key, api_key) = seed_agent(&state, "latest-bad-limit").await;

    for uri in [
        "/memories/latest?symbol=BTC&memory_type=analysis&limit=0",
        "/memories/latest?symbol=BTC&memory_type=analysis&limit=-1",
        "/memories/latest?symbol=BTC&memory_type=analysis&limit=abc",
    ] {
        let (status, body) = latest_memories_response(&state, &api_key, uri).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "limit must be an integer >= 1");
    }
}

#[tokio::test]
async fn latest_memories_is_scoped_to_authenticated_agent() {
    let state = test_state().await;
    let (agent_a_key, agent_a_api_key) = seed_agent(&state, "latest-scope-a").await;
    let (agent_b_key, _agent_b_api_key) = seed_agent(&state, "latest-scope-b").await;
    let now = Utc::now();

    insert_memory_at(
        &state,
        &agent_a_key,
        now - Duration::minutes(2),
        "BTC",
        Some("15m"),
        "analysis",
        "agent-a",
        json!({}),
    )
    .await;
    insert_memory_at(
        &state,
        &agent_b_key,
        now - Duration::minutes(1),
        "BTC",
        Some("15m"),
        "analysis",
        "agent-b",
        json!({}),
    )
    .await;

    let (status, body) = latest_memories_response(
        &state,
        &agent_a_api_key,
        "/memories/latest?symbol=BTC&memory_type=analysis",
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let rows = body.as_array().expect("array response");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["summary"], "agent-a");
    assert_eq!(rows[0]["agent_key"], agent_a_key);
}

#[tokio::test]
async fn get_memory_by_id_returns_200_or_404() {
    let state = test_state().await;

    let (_agent_key, api_key) = seed_agent(&state, "get").await;
    let (_other_key, other_api_key) = seed_agent(&state, "get-other").await;

    let body = serde_json::json!({
        "symbol": "BTC",
        "timeframe": "1h",
        "memory_type": "plan",
        "summary": "x",
        "content": "y"
    });
    let (headers, body) = json_body(&body);
    let mut builder = Request::builder()
        .method("POST")
        .uri("/memories")
        .header("authorization", format!("Bearer {api_key}"));
    if let Some((k, v)) = headers {
        builder = builder.header(k, v);
    }
    let response = app(Arc::clone(&state))
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body_bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    let created: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    let id = created["id"].as_str().unwrap().to_string();

    // Owner can fetch.
    let request = Request::builder()
        .uri(format!("/memories/{id}"))
        .header("authorization", format!("Bearer {api_key}"))
        .body(Body::empty())
        .unwrap();
    let response = app(Arc::clone(&state)).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    let fetched: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(fetched["id"], created["id"]);

    // Another agent gets 404 (do not leak existence).
    let request = Request::builder()
        .uri(format!("/memories/{id}"))
        .header("authorization", format!("Bearer {other_api_key}"))
        .body(Body::empty())
        .unwrap();
    let response = app(Arc::clone(&state)).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    // Invalid UUID -> 404.
    let request = Request::builder()
        .uri("/memories/not-a-uuid")
        .header("authorization", format!("Bearer {api_key}"))
        .body(Body::empty())
        .unwrap();
    let response = app(Arc::clone(&state)).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
