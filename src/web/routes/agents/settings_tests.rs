//! Tests for agents/settings_tests.rs
use crate::web::routes::router;
use crate::web::routes::test_support::*;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use std::sync::Arc;
use tower::util::ServiceExt;

use crate::agents::store::{list_agent_instrument_ids, replace_agent_instruments};

#[tokio::test]
async fn post_reset_memories_deletes_only_that_agents_memories() {
    let state = test_state().await;
    let app = router(Arc::clone(&state));
    let (agent_key, _) = insert_test_opencode_agent(&state)
        .await
        .expect("insert agent");
    let (other_agent_key, _) = insert_test_opencode_agent(&state)
        .await
        .expect("insert other agent");
    seed_memory(&state, &agent_key, "mine", "mine content").await;
    seed_memory_with_type(&state, &agent_key, "analysis", "mine-2", "content").await;
    seed_memory(&state, &other_agent_key, "other", "other content").await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/settings/reset-memories"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let location = response
        .headers()
        .get("location")
        .and_then(|value| value.to_str().ok())
        .expect("redirect location")
        .to_string();
    assert!(location.starts_with(&format!("/agents/{agent_key}/settings?notice=")));
    let notice = urldecode(
        location
            .split("notice=")
            .nth(1)
            .unwrap_or_default()
            .as_bytes(),
    );
    assert!(notice.contains("were deleted"));

    let (mine_count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM memory.records WHERE agent_key = $1")
            .bind(&agent_key)
            .fetch_one(&state.db_pool)
            .await
            .expect("count agent memories");
    assert_eq!(mine_count, 0, "agent memories must be deleted");
    let (other_count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM memory.records WHERE agent_key = $1")
            .bind(&other_agent_key)
            .fetch_one(&state.db_pool)
            .await
            .expect("count other agent memories");
    assert_eq!(other_count, 1, "other agents' memories must be untouched");
}

#[tokio::test]
async fn post_reset_memories_reports_success_when_no_memories_exist() {
    let state = test_state().await;
    let app = router(Arc::clone(&state));
    let (agent_key, _) = insert_test_opencode_agent(&state)
        .await
        .expect("insert agent");

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/settings/reset-memories"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let location = response
        .headers()
        .get("location")
        .and_then(|value| value.to_str().ok())
        .expect("redirect location")
        .to_string();
    assert!(location.contains("notice="));
    assert!(!location.contains("were%20deleted"));
}

#[tokio::test]
async fn post_reset_memories_does_not_insert_maintenance_task() {
    let state = test_state().await;
    let app = router(Arc::clone(&state));
    let (agent_key, _) = insert_test_opencode_agent(&state)
        .await
        .expect("insert agent");
    seed_memory(&state, &agent_key, "mine", "mine content").await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/settings/reset-memories"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    let (task_count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM harness_maintenance_tasks WHERE agent_key = $1")
            .bind(&agent_key)
            .fetch_one(&state.db_pool)
            .await
            .expect("count maintenance tasks");
    assert_eq!(task_count, 0, "memory reset must not queue a task");
}

#[tokio::test]
async fn settings_page_renders_reset_memories_action_and_no_workspace_section() {
    let state = test_state().await;
    let app = router(Arc::clone(&state));
    let (agent_key, _) = insert_test_opencode_agent(&state)
        .await
        .expect("insert agent");
    seed_memory(&state, &agent_key, "mine", "mine content").await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/settings"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert!(text.contains("data-reset-memories-trigger"));
    assert!(text.contains("reset-memories-modal"));
    assert!(!text.contains("regenerate-workspace-modal"));
    assert!(!text.contains("regenerate-workspace-form"));
    assert!(!text.contains("agent-workspace-section"));
    assert!(!text.contains("workspace-maintenance-status"));
    assert!(!text.contains("Template drift"));
    assert!(!text.contains("Re-generate workspace"));

    // Deleted regeneration route must be absent.
    let gone = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/settings/regenerate-workspace"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(gone.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn agent_settings_route_renders_currency_controls() {
    let state = test_state().await;
    let (agent_key, _) = insert_test_agent(&state).await.expect("insert agent");
    seed_instrument(&state, "BTC", true).await;
    seed_instrument(&state, "ETH", true).await;
    replace_agent_instruments(&state.db_pool, &agent_key, &["BTC".to_string()])
        .await
        .expect("seed selected instruments");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/settings"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert!(text.contains("Currencies"));
    assert!(!text.contains("Select the Hyperliquid perps this agent should analyze and trade."));
    assert!(text.contains("name=\"instrument_id\""));
    assert!(text.contains("value=\"BTC\""));
    assert!(text.contains("value=\"ETH\""));
    assert!(text.contains("value=\"BTC\" checked"));
    assert!(!text.contains("Sync status"));
    assert!(!text.contains("abc123"));
    assert!(text.contains("data-select-currencies"));
    assert!(text.contains("data-currency-modal"));
    assert!(text.contains("data-apply-currencies"));
    assert!(text.contains("type=\"submit\" data-apply-currencies"));
    assert!(text.contains(">Save</button>"));
    assert!(text.contains("Select all"));
    assert!(text.contains("Select none"));
}

#[tokio::test]
async fn post_agent_instruments_updates_selection_and_redirects() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let guard = state
        ._test_db_guard
        .as_ref()
        .cloned()
        .expect("test db guard");
    let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
    seed_instrument(&state, "BTC", true).await;
    seed_instrument(&state, "ETH", true).await;

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/settings/instruments"))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("instrument_id=BTC&instrument_id=ETH"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let expected_location = format!("/agents/{agent_key}/settings");
    assert_eq!(
        response
            .headers()
            .get("location")
            .and_then(|value| value.to_str().ok()),
        Some(expected_location.as_str())
    );

    let selected = list_agent_instrument_ids(&pool, &agent_key)
        .await
        .expect("list selected instruments");
    assert_eq!(selected, vec!["BTC".to_string(), "ETH".to_string()]);
    drop(guard);
}

#[tokio::test]
async fn post_agent_instruments_without_values_clears_selection_and_redirects() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let guard = state
        ._test_db_guard
        .as_ref()
        .cloned()
        .expect("test db guard");
    let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
    seed_instrument(&state, "BTC", true).await;
    replace_agent_instruments(&pool, &agent_key, &["BTC".to_string()])
        .await
        .expect("seed selected instruments");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/settings/instruments"))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    let selected = list_agent_instrument_ids(&pool, &agent_key)
        .await
        .expect("list selected instruments");
    assert!(selected.is_empty());
    drop(guard);
}

fn urldecode(raw: &[u8]) -> String {
    let mut output = Vec::with_capacity(raw.len());
    let mut index = 0;
    while index < raw.len() {
        match raw[index] {
            b'%' if index + 2 < raw.len() => {
                let hex = std::str::from_utf8(&raw[index + 1..index + 3]).unwrap_or("");
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    output.push(byte);
                    index += 3;
                    continue;
                }
                output.push(raw[index]);
                index += 1;
            }
            b'+' => {
                output.push(b' ');
                index += 1;
            }
            byte => {
                output.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&output).into_owned()
}
