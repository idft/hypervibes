//! Tests for agents/hooks_tests.rs
use crate::web::routes::router;
use crate::web::routes::test_support::*;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use std::sync::{Arc, Mutex};
use tower::util::ServiceExt;

use crate::{
    agentic::{model::JOB_KIND_MARKET_ANALYSIS, store::QueuedHookRun},
    agents::store::replace_agent_instruments,
};

#[tokio::test]
async fn manual_hook_run_redirects_with_warning_during_workspace_maintenance() {
    let state = test_state().await;
    let app = router(Arc::clone(&state));
    let (agent_key, _) = insert_test_opencode_agent(&state)
        .await
        .expect("insert agent");
    let hook_id = crate::agentic::store::list_agent_hooks(&state.db_pool, &agent_key)
        .await
        .expect("list hooks")
        .first()
        .expect("default hook present")
        .id;
    crate::agentic::store::insert_workspace_regenerate_task(
        &state.db_pool,
        &agent_key,
        false,
        false,
    )
    .await
    .expect("seed maintenance task");

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/hooks/{hook_id}/run"))
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
        .expect("redirect location");
    assert!(location.contains("/jobs?warning="));
}
#[tokio::test]
async fn new_hook_page_renders_for_opencode_agent() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/hooks/new"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert!(text.contains("Create market-analysis hook"));
    assert!(text.contains("analysis_batch_completed"));
    assert!(text.contains("name=\"timeout_seconds\""));
    assert!(text.contains("name=\"model_selection\""));
    assert!(text.contains("name=\"operator_prompt\""));
}
#[tokio::test]
async fn post_hook_create_inserts_hook_and_redirects() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");
    let default_hook_id = crate::agentic::store::list_agent_hooks(&pool, &agent_key)
        .await
        .expect("list hooks")
        .first()
        .expect("default hook present")
        .id;
    crate::agentic::store::delete_agent_hook(&pool, &agent_key, default_hook_id)
        .await
        .expect("delete default hook");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/hooks"))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(
                    "timeout_seconds=600&enabled=on&model_selection=&operator_prompt=Summarize+multi-timeframe+agreement",
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let hooks = crate::agentic::store::list_agent_hooks(&pool, &agent_key)
        .await
        .expect("list hooks");
    let hook = hooks.first().expect("hook present");
    assert_eq!(hook.job_key, "market-analysis");
    assert_eq!(hook.job_kind, JOB_KIND_MARKET_ANALYSIS);
}
#[tokio::test]
async fn post_hook_run_now_queues_and_dispatches_run() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let backend = Arc::new(RecordingAgenticBackend {
        calls: Arc::clone(&calls),
    });
    let state = test_state_with_backend(backend).await;
    let pool = state.db_pool.clone();
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");
    seed_instrument(&state, "BTC", true).await;
    replace_agent_instruments(&pool, &agent_key, &["BTC".to_string()])
        .await
        .expect("seed instruments");
    let hook_id = crate::agentic::store::list_agent_hooks(&pool, &agent_key)
        .await
        .expect("list hooks")
        .first()
        .expect("default hook present")
        .id;
    crate::agentic::store::set_hook_enabled(&pool, &agent_key, hook_id, true)
        .await
        .expect("enable default hook");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/hooks/{hook_id}/run"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(1);
    loop {
        if !calls.lock().unwrap().is_empty() {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "hook dispatch was not spawned"
        );
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
    }

    let recorded = calls.lock().unwrap();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].schedule_id, None);
    assert_eq!(recorded[0].hook_id, Some(hook_id));
    assert_eq!(recorded[0].job_key, "market-analysis");
}
#[tokio::test]
async fn post_hook_toggle_updates_enabled_state() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");
    let hook_id = crate::agentic::store::list_agent_hooks(&pool, &agent_key)
        .await
        .expect("list hooks")
        .first()
        .expect("default hook present")
        .id;
    crate::agentic::store::set_hook_enabled(&pool, &agent_key, hook_id, true)
        .await
        .expect("enable default hook");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/hooks/{hook_id}/toggle"))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("enabled=off"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let hook = crate::agentic::store::get_agent_hook(&pool, &agent_key, hook_id)
        .await
        .expect("get hook")
        .expect("hook present");
    assert!(!hook.enabled);
}
#[tokio::test]
async fn post_hook_delete_removes_hook() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");
    let hook_id = crate::agentic::store::list_agent_hooks(&pool, &agent_key)
        .await
        .expect("list hooks")
        .first()
        .expect("default hook present")
        .id;

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/hooks/{hook_id}/delete"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert!(
        crate::agentic::store::get_agent_hook(&pool, &agent_key, hook_id)
            .await
            .expect("get hook")
            .is_none()
    );
}
#[tokio::test]
async fn hook_detail_page_renders_hook_specific_runs() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");
    seed_instrument(&state, "BTC", true).await;
    replace_agent_instruments(&pool, &agent_key, &["BTC".to_string()])
        .await
        .expect("seed instruments");
    let hook_id = crate::agentic::store::list_agent_hooks(&pool, &agent_key)
        .await
        .expect("list hooks")
        .first()
        .expect("default hook present")
        .id;
    let run_id = match crate::agentic::store::insert_queued_hook_run(&pool, &agent_key, hook_id)
        .await
        .expect("insert hook run")
    {
        QueuedHookRun::Dispatch { run_id, .. } => run_id,
        other => panic!("expected Dispatch, got {other:?}"),
    };
    crate::agentic::store::mark_run_succeeded(&pool, run_id, Some("ses_hook_detail"))
        .await
        .expect("mark succeeded");

    let response = router(state)
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/hooks/{hook_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert!(text.contains("Hook details"));
    assert!(text.contains("analysis_batch_completed"));
    assert!(text.contains("ses_hook_detail"));
    assert!(text.contains(&format!("/agents/{agent_key}/runs/{run_id}")));
}
#[tokio::test]
async fn post_hook_timeout_updates_and_redirects() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");
    let hook_id = crate::agentic::store::list_agent_hooks(&pool, &agent_key)
        .await
        .expect("list hooks")
        .first()
        .expect("default hook present")
        .id;

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/hooks/{hook_id}/timeout"))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("timeout=25m"))
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
        Some(format!("/agents/{agent_key}/hooks/{hook_id}").as_str())
    );

    let hook = crate::agentic::store::get_agent_hook(&pool, &agent_key, hook_id)
        .await
        .expect("get hook")
        .expect("hook present");
    assert_eq!(hook.timeout_seconds, 25 * 60);
}
#[tokio::test]
async fn post_hook_timeout_invalid_value_redirects_with_error() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");
    let hook_id = crate::agentic::store::list_agent_hooks(&pool, &agent_key)
        .await
        .expect("list hooks")
        .first()
        .expect("default hook present")
        .id;

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/hooks/{hook_id}/timeout"))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("timeout=5x"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let location = response
        .headers()
        .get("location")
        .and_then(|value| value.to_str().ok())
        .expect("location header");
    assert!(location.starts_with(&format!(
        "/agents/{agent_key}/hooks/{hook_id}?timeout_error="
    )));
}
#[tokio::test]
async fn post_hook_timeout_missing_hook_returns_404() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/hooks/999999/timeout"))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("timeout=15m"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn manual_hook_run_still_dispatches_after_shutdown_signal() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let backend = Arc::new(RecordingAgenticBackend {
        calls: Arc::clone(&calls),
    });
    let state = test_state_with_backend_and_shutdown(backend, true).await;
    let pool = state.db_pool.clone();
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");
    seed_instrument(&state, "BTC", true).await;
    replace_agent_instruments(&pool, &agent_key, &["BTC".to_string()])
        .await
        .expect("seed instruments");
    let hook_id = crate::agentic::store::list_agent_hooks(&pool, &agent_key)
        .await
        .expect("list hooks")
        .first()
        .expect("default hook present")
        .id;
    crate::agentic::store::set_hook_enabled(&pool, &agent_key, hook_id, true)
        .await
        .expect("enable default hook");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/hooks/{hook_id}/run"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(1);
    loop {
        if !calls.lock().unwrap().is_empty() {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "hook dispatch was not spawned after shutdown signal"
        );
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
    }

    let recorded = calls.lock().unwrap();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].hook_id, Some(hook_id));
}
