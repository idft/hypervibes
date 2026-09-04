//! Tests for agents/coding_tests.rs
use crate::web::routes::router;
use crate::web::routes::test_support::*;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use std::fs;
use std::sync::Arc;
use tower::util::ServiceExt;

async fn seed_package(state: &Arc<crate::web::AppState>, agent_key: &str) {
    let package = state
        .opencode_workspace_config
        .host_workspaces_root
        .join("packages")
        .join(agent_key);
    fs::create_dir_all(package.join("strategies")).expect("create package");
    fs::write(
        package.join("manifest.json"),
        r#"{"schema_version": 1, "package_version": "v7", "tools": [{"id":"trend","description":"Trend target","entrypoint":"strategies/trend.py","input_kind":"ohlcv","supported_timeframes":["15m"],"minimum_candles":1,"required_arguments":["symbol","timeframe","boundary_ms","input","output"],"output_schema":"hypervibes.quantitative.v1","version":"1"}]}"#,
    )
    .expect("write manifest");
    fs::write(package.join("strategies/trend.py"), "print('trend')\n").expect("write strategy");
}

#[tokio::test]
async fn coding_page_renders_package_status_browser_and_preview() {
    let state = test_state().await;
    let app = router(Arc::clone(&state));
    let (agent_key, _) = insert_test_opencode_agent(&state)
        .await
        .expect("insert agent");
    seed_package(&state, &agent_key).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/coding"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert!(text.contains("Coding package"));
    assert!(text.contains("Valid package"));
    assert!(text.contains("version v7"));
    assert!(text.contains("strategies"));
    assert!(text.contains("strategies/trend.py"));
    assert!(!text.contains("scripts/user/strategies/trend.py"));

    let preview = router(state)
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/agents/{agent_key}/coding?file=strategies%2Ftrend.py"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(preview.status(), StatusCode::OK);
    let preview_text = response_text(preview).await;
    assert!(
        preview_text.contains("trend.py"),
        "preview page should show the path"
    );
    assert!(
        preview_text.contains("print") && preview_text.contains("trend"),
        "preview page should contain the file text: {preview_text}"
    );
}

#[tokio::test]
async fn coding_page_reports_missing_package() {
    let state = test_state().await;
    let app = router(Arc::clone(&state));
    let (agent_key, _) = insert_test_opencode_agent(&state)
        .await
        .expect("insert agent");

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/coding"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert!(text.contains("No package yet"));
}

#[tokio::test]
async fn coding_page_reports_invalid_package_with_safe_error() {
    let state = test_state().await;
    let app = router(Arc::clone(&state));
    let (agent_key, _) = insert_test_opencode_agent(&state)
        .await
        .expect("insert agent");
    let package = state
        .opencode_workspace_config
        .host_workspaces_root
        .join("packages")
        .join(&agent_key);
    fs::create_dir_all(package.join("strategies")).expect("create package");
    fs::write(package.join("strategies/trend.py"), "print('trend')\n")
        .expect("write strategy without a manifest");

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/coding"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert!(text.contains("Unavailable"));
    assert!(
        !text.contains("manifest.json is missing"),
        "inspection error detail must not leak to the page"
    );
}

#[tokio::test]
async fn coding_task_status_polls_only_active_tasks() {
    let state = test_state().await;
    let app = router(Arc::clone(&state));
    let (agent_key, _) = insert_test_opencode_agent(&state)
        .await
        .expect("insert agent");

    // No task: no polling partial.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/coding"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert!(text.contains("Coding package"));
    assert!(!text.contains("agent-coding-task-status"));
    assert!(!text.contains("hx-trigger=\"every 2s\""));

    // Seed a queued analysis-coding task; the partial renders and polls.
    sqlx::query(
        "UPDATE harness_sub_agents
            SET enabled = true,
                model_provider_id = 'test',
                model_id = 'strong'
          WHERE agent_key = $1
             AND sub_agent_kind = 'coding'",
    )
    .bind(&agent_key)
    .execute(&state.db_pool)
    .await
    .expect("configure coding event job");
    let coding_sub_agent_id: (i64,) = sqlx::query_as(
        "SELECT id FROM harness_sub_agents WHERE agent_key = $1 AND sub_agent_kind = 'coding'",
    )
    .bind(&agent_key)
    .fetch_one(&state.db_pool)
    .await
    .expect("load coding event job");
    let task_id = match crate::harness::store::insert_analysis_coding_task_and_run(
        &state.db_pool,
        crate::harness::store::AnalysisCodingTaskRequest {
            agent_key: &agent_key,
            sub_agent_id: coding_sub_agent_id.0,
            trigger_mode: crate::harness::store::CodingTriggerMode::Manual,
            request_origin: "manual",
            source_sub_agent_run_id: None,
            source_memory_id: None,
            operator_prompt: None,
            requested_mode: Some("auto"),
        },
    )
    .await
    .expect("queue coding task")
    {
        crate::harness::store::InsertAnalysisCodingTaskOutcome::Inserted { task_id, .. } => task_id,
        other => panic!("expected Inserted, got {other:?}"),
    };

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/coding"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert!(text.contains("agent-coding-task-status"));
    assert!(text.contains("hx-trigger=\"every 2s\""));

    // Mark the task terminal; information stays visible without polling.
    crate::harness::store::mark_maintenance_task_succeeded(&state.db_pool, task_id)
        .await
        .expect("mark task succeeded");
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/coding"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert!(text.contains("agent-coding-task-status"));
    assert!(!text.contains("hx-trigger=\"every 2s\""));

    // The polling endpoint itself renders the partial.
    let partial = app
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/coding/task-status"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(partial.status(), StatusCode::OK);
    let partial_text = response_text(partial).await;
    assert!(partial_text.contains("agent-coding-task-status"));
    assert!(!partial_text.contains("hx-trigger=\"every 2s\""));
}

#[tokio::test]
async fn workspace_route_is_absent() {
    let state = test_state().await;
    let app = router(Arc::clone(&state));
    let (agent_key, _) = insert_test_opencode_agent(&state)
        .await
        .expect("insert agent");

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/workspace"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
