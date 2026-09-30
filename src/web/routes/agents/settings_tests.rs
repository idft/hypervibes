//! Tests for agents/settings_tests.rs
use crate::web::routes::router;
use crate::web::routes::test_support::*;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use std::sync::Arc;
use tower::util::ServiceExt;

use crate::agents::store::{
    list_agent_analysis_instrument_ids, list_agent_trading_instrument_ids,
    replace_agent_analysis_instruments, replace_agent_trading_instruments,
};

#[tokio::test]
async fn chat_permission_defaults_save_and_render_selected_values() {
    let state = test_state().await;
    let (agent_key, _) = insert_test_agent(&state).await.expect("insert agent");
    let app = router(Arc::clone(&state));
    let response = app.clone().oneshot(
        Request::builder().method("POST")
            .uri(format!("/agents/{agent_key}/settings/chat-permissions"))
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("orders_policy=allow&memory_writes_policy=deny&notifications_policy=confirm&journal_writes_policy=allow&indicator_writes_policy=deny&strategy_prompt_writes_policy=allow&csrf_token=test-token"))
            .expect("request"),
    ).await.expect("save defaults");
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let location = response
        .headers()
        .get("location")
        .expect("redirect")
        .to_str()
        .expect("redirect text");
    assert!(location.starts_with(&format!("/agents/{agent_key}/settings?notice=")));
    let defaults = crate::agent_conversations::store::get_agent_chat_policy_defaults(
        &state.db_pool,
        &agent_key,
    )
    .await
    .expect("saved defaults");
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/settings"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("render settings");
    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert!(text.contains("Chat permission defaults"));
    assert!(text.contains("Existing conversations keep their current permissions."));
    for (group, policy) in [
        ("orders", "allow"),
        ("memory_writes", "deny"),
        ("notifications", "confirm"),
        ("journal_writes", "allow"),
        ("indicator_writes", "deny"),
        ("strategy_prompt_writes", "allow"),
    ] {
        assert!(
            defaults
                .iter()
                .any(|row| row.tool_group == group && row.policy == policy)
        );
        let select = text
            .split(&format!("name=\"{group}_policy\""))
            .nth(1)
            .expect("permission select")
            .split("</select>")
            .next()
            .expect("select contents");
        assert!(select.contains(&format!("value=\"{policy}\" selected")));
    }
}

#[tokio::test]
async fn chat_permission_defaults_reject_invalid_incomplete_and_duplicate_forms() {
    let state = test_state().await;
    let (agent_key, _) = insert_test_agent(&state).await.expect("insert agent");
    let app = router(Arc::clone(&state));
    let valid = "orders_policy=allow&memory_writes_policy=allow&notifications_policy=allow&journal_writes_policy=allow&indicator_writes_policy=allow&strategy_prompt_writes_policy=allow";
    for body in [
        valid.replace("orders_policy=allow", "orders_policy=invalid"),
        "orders_policy=allow".to_string(),
        valid.replace("orders_policy", "unknown_policy"),
        valid.replace("orders_policy", "memory_writes_policy"),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/agents/{agent_key}/settings/chat-permissions"))
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .expect("request"),
            )
            .await
            .expect("invalid form response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
    let defaults = crate::agent_conversations::store::get_agent_chat_policy_defaults(
        &state.db_pool,
        &agent_key,
    )
    .await
    .expect("unchanged defaults");
    for (group, policy) in crate::agent_conversations::model::DEFAULT_TOOL_POLICIES {
        assert!(
            defaults
                .iter()
                .any(|row| row.tool_group == group && row.policy == policy)
        );
    }
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/agents/missing/settings/chat-permissions")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(valid))
                .expect("request"),
        )
        .await
        .expect("missing agent response");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

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
    replace_agent_analysis_instruments(&state.db_pool, &agent_key, &["BTC".to_string()])
        .await
        .expect("seed analysis instruments");
    replace_agent_trading_instruments(&state.db_pool, &agent_key, &["ETH".to_string()])
        .await
        .expect("seed trading instruments");

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
    assert!(text.contains("Analysis instruments"));
    assert!(text.contains("Trading instruments"));
    assert!(text.contains("name=\"analysis_instrument_id\""));
    assert!(text.contains("name=\"trading_instrument_id\""));
    assert!(text.contains("/settings/analysis-instruments"));
    assert!(text.contains("/settings/trading-instruments"));
    assert!(text.contains("value=\"BTC\""));
    assert!(text.contains("value=\"ETH\""));
    assert!(text.contains("value=\"BTC\" checked"));
    assert!(text.contains("value=\"ETH\" checked"));
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
async fn post_agent_trading_instruments_updates_selection_and_redirects() {
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
                .uri(format!("/agents/{agent_key}/settings/trading-instruments"))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(
                    "trading_instrument_id=BTC&trading_instrument_id=ETH",
                ))
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

    let selected = list_agent_trading_instrument_ids(&pool, &agent_key)
        .await
        .expect("list selected instruments");
    assert_eq!(selected, vec!["BTC".to_string(), "ETH".to_string()]);
    drop(guard);
}

#[tokio::test]
async fn post_agent_trading_instruments_without_values_clears_selection_and_redirects() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let guard = state
        ._test_db_guard
        .as_ref()
        .cloned()
        .expect("test db guard");
    let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
    seed_instrument(&state, "BTC", true).await;
    replace_agent_trading_instruments(&pool, &agent_key, &["BTC".to_string()])
        .await
        .expect("seed selected instruments");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/settings/trading-instruments"))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    let selected = list_agent_trading_instrument_ids(&pool, &agent_key)
        .await
        .expect("list selected instruments");
    assert!(selected.is_empty());
    drop(guard);
}

#[tokio::test]
async fn post_agent_analysis_instruments_does_not_change_trading_allowlist() {
    let state = test_state().await;
    let (agent_key, _) = insert_test_agent(&state).await.expect("insert agent");
    seed_instrument(&state, "BTC", true).await;
    seed_instrument(&state, "ETH", true).await;
    replace_agent_trading_instruments(&state.db_pool, &agent_key, &["BTC".to_string()])
        .await
        .expect("seed trading instruments");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/settings/analysis-instruments"))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("analysis_instrument_id=ETH"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        list_agent_analysis_instrument_ids(&state.db_pool, &agent_key)
            .await
            .expect("list analysis instruments"),
        vec!["ETH".to_string()]
    );
    assert_eq!(
        list_agent_trading_instrument_ids(&state.db_pool, &agent_key)
            .await
            .expect("list trading instruments"),
        vec!["BTC".to_string()]
    );
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
