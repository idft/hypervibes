//! Tests for agents/settings_tests.rs
use crate::web::routes::router;
use crate::web::routes::test_support::*;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use std::{fs, sync::Arc};
use tower::util::ServiceExt;

use crate::{
    agents::{
        model::slugify_agent_key,
        store::{list_agent_instrument_ids, replace_agent_instruments},
    },
    opencode::workspace::agent_workspace_host_path,
};

#[tokio::test]
async fn post_regenerate_workspace_queues_regular_maintenance_task() {
    let state = test_state().await;
    let app = router(Arc::clone(&state));
    let (agent_key, _) = insert_test_opencode_agent(&state)
        .await
        .expect("insert agent");

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/settings/regenerate-workspace"))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(""))
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
        Some(format!("/agents/{agent_key}/settings").as_str())
    );

    let task =
        crate::agentic::store::get_latest_workspace_regenerate_task(&state.db_pool, &agent_key)
            .await
            .expect("load maintenance task")
            .expect("maintenance task present");
    assert_eq!(
        task.status,
        crate::agentic::model::MAINTENANCE_STATUS_QUEUED
    );
    assert!(!task.parameter_bool("hard_reset"));
}
#[tokio::test]
async fn post_regenerate_workspace_with_hard_reset_and_memory_reset_queues_both_options() {
    let state = test_state().await;
    let app = router(Arc::clone(&state));
    let (agent_key, _) = insert_test_opencode_agent(&state)
        .await
        .expect("insert agent");

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/settings/regenerate-workspace"))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("hard_reset=on&reset_memories=on"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let task =
        crate::agentic::store::get_latest_workspace_regenerate_task(&state.db_pool, &agent_key)
            .await
            .expect("load maintenance task")
            .expect("maintenance task present");
    assert!(task.parameter_bool("hard_reset"));
    assert!(task.parameter_bool("reset_memories"));
}
#[tokio::test]
async fn post_regenerate_workspace_redirects_with_warning_when_task_already_exists() {
    let state = test_state().await;
    let app = router(Arc::clone(&state));
    let (agent_key, _) = insert_test_opencode_agent(&state)
        .await
        .expect("insert agent");
    crate::agentic::store::insert_workspace_regenerate_task(&state.db_pool, &agent_key, false, false)
        .await
        .expect("seed maintenance task");

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/settings/regenerate-workspace"))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(""))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let location = response
        .headers()
        .get("location")
        .and_then(|value| value.to_str().ok())
        .expect("redirect location");
    assert!(location.contains("workspace_warning="));
}
#[tokio::test]
async fn settings_page_and_partial_render_workspace_maintenance_status() {
    let state = test_state().await;
    let app = router(Arc::clone(&state));
    let (agent_key, _) = insert_test_opencode_agent(&state)
        .await
        .expect("insert agent");
    seed_workspace_runtime_config(&state, &agent_key).await;
    let task_id = match crate::agentic::store::insert_workspace_regenerate_task(
        &state.db_pool,
        &agent_key,
        true,
        false,
    )
    .await
    .expect("seed maintenance task")
    {
        crate::agentic::store::InsertWorkspaceMaintenanceTaskOutcome::Inserted { task_id } => {
            task_id
        }
        other => panic!("expected inserted maintenance task, got {other:?}"),
    };

    let settings_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/settings"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(settings_response.status(), StatusCode::OK);
    let settings_text = response_text(settings_response).await;
    assert!(settings_text.contains("Workspace maintenance"));
    assert!(settings_text.contains("Waiting for active jobs and sessions to finish"));
    assert!(settings_text.contains("Hard reset"));

    let partial_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/agents/{agent_key}/settings/workspace-maintenance-status"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(partial_response.status(), StatusCode::OK);
    let partial_text = response_text(partial_response).await;
    assert!(partial_text.contains("id=\"agent-workspace-section\""));
    assert!(partial_text.contains("hx-trigger=\"every 2s\""));

    assert!(
        crate::agentic::store::mark_maintenance_task_succeeded(&state.db_pool, task_id,)
            .await
            .expect("mark maintenance succeeded")
    );

    let completed_response = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/agents/{agent_key}/settings/workspace-maintenance-status"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(completed_response.status(), StatusCode::OK);
    let completed_text = response_text(completed_response).await;
    assert!(completed_text.contains("id=\"agent-workspace-section\""));
    assert!(!completed_text.contains("Workspace maintenance"));
    assert!(!completed_text.contains("hx-trigger=\"every 2s\""));
}
#[tokio::test]
async fn agent_settings_route_renders_sync_state() {
    let state = test_state().await;
    let (agent_key, wallet_address) = insert_test_agent(&state).await.expect("insert agent");
    seed_sync_state(&state, &wallet_address).await;
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
    assert!(text.contains("Select the Hyperliquid perps this agent should analyze and trade."));
    assert!(text.contains("name=\"instrument_id\""));
    assert!(text.contains("value=\"BTC\""));
    assert!(text.contains("value=\"ETH\""));
    assert!(text.contains("value=\"BTC\" checked"));
    assert!(text.contains("Sync status"));
    assert!(text.contains("fills"));
    assert!(text.contains("abc123"));
}
#[tokio::test]
async fn opencode_agent_settings_route_renders_workspace_state() {
    let state = test_state().await;
    let app = router(Arc::clone(&state));
    let timestamp = chrono::Utc::now().timestamp_millis();
    let display_name = format!("OpenCodeSettings{}", timestamp);
    let agent_key = slugify_agent_key(&display_name);
    let private_key = random_private_key();
    let body = format!(
        "display_name={}&hyperliquid_private_key={}&runtime_id=opencode-local",
        display_name, private_key
    );

    let create_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/agents")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create_response.status(), StatusCode::SEE_OTHER);

    let response = app
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
    assert!(text.contains("OpenCode workspace"));
    assert!(text.contains("Container workspace path"));
    assert!(text.contains("Profile source"));
    assert!(text.contains("Workspace .env"));
    assert!(text.contains("In sync"));
}
#[tokio::test]
async fn opencode_agent_settings_route_renders_workspace_template_drift() {
    let state = test_state().await;
    let app = router(Arc::clone(&state));
    let timestamp = chrono::Utc::now().timestamp_millis();
    let display_name = format!("OpenCodeDrift{}", timestamp);
    let agent_key = slugify_agent_key(&display_name);
    let private_key = random_private_key();
    let body = format!(
        "display_name={}&hyperliquid_private_key={}&runtime_id=opencode-local",
        display_name, private_key
    );

    let create_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/agents")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create_response.status(), StatusCode::SEE_OTHER);

    let workspace_path = agent_workspace_host_path(&state.opencode_workspace_config, &agent_key)
        .expect("workspace path");
    fs::write(workspace_path.join("AGENTS.md"), "user-modified\n").expect("modify AGENTS.md");

    let response = app
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
    assert!(text.contains("Template drift"));
    assert!(text.contains("AGENTS.md"));
    assert!(text.contains("Only files generated from the workspace template are compared."));
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
