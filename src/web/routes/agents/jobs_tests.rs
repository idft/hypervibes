//! Tests for agents/jobs_tests.rs
use super::*;
use crate::web::routes::router;
use crate::web::routes::test_support::*;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use std::sync::{Arc, Mutex};
use tower::util::ServiceExt;

use crate::{
    agentic::model::{JOB_KIND_ANALYSIS, JOB_KIND_TRADING},
    agents::store::replace_agent_instruments,
};

#[tokio::test]
async fn manual_job_run_redirects_with_warning_during_workspace_maintenance() {
    let state = test_state().await;
    let app = router(Arc::clone(&state));
    let (agent_key, _) = insert_test_opencode_agent(&state)
        .await
        .expect("insert agent");
    let schedules = crate::agentic::store::list_agent_schedules(&state.db_pool, &agent_key)
        .await
        .expect("list schedules");
    let schedule_id = schedules
        .iter()
        .find(|row| row.job_key == "analysis-15m")
        .expect("analysis schedule present")
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
                .uri(format!("/agents/{agent_key}/jobs/{schedule_id}/run"))
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
async fn jobs_route_renders_create_job_button_and_runs_section() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/jobs"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert!(text.contains("New job"));
    assert!(text.contains(&format!("/agents/{agent_key}/jobs/new")));
    assert!(text.contains("Scheduled"));
    assert!(text.contains("Hooks"));
    assert!(text.contains("Enable all"));
    assert!(!text.contains("Disable all"));
    assert!(text.contains("Recent Runs"));
    assert!(text.contains("New hook"));
    assert!(text.contains(&format!("/agents/{agent_key}/hooks/new")));
    assert!(text.contains("Run now"));
    assert!(!text.contains("Operator prompt"));
}
#[tokio::test]
async fn jobs_route_paginates_recent_runs() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");
    let schedule_id = crate::agentic::store::list_agent_schedules(&state.db_pool, &agent_key)
        .await
        .expect("list schedules")
        .into_iter()
        .next()
        .expect("default schedule")
        .id;

    let base_time = chrono::Utc::now();
    for index in 1..=12 {
        let run_id =
            crate::agentic::store::insert_test_run(&state.db_pool, schedule_id, "succeeded")
                .await
                .expect("insert test run");
        sqlx::query(
            "UPDATE agentic_runs
                SET backend_run_ref = $1,
                    created_at = $2,
                    updated_at = $2
              WHERE id = $3",
        )
        .bind(format!("run-{index:02}"))
        .bind(base_time + chrono::Duration::seconds(index.into()))
        .bind(run_id)
        .execute(&state.db_pool)
        .await
        .expect("label test run");
    }

    let page_one = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/jobs"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(page_one.status(), StatusCode::OK);
    let page_one_text = response_text(page_one).await;
    assert!(page_one_text.contains("Showing 1-10 of 12 runs"));
    assert!(page_one_text.contains("Page 1 of 2"));
    assert!(page_one_text.contains(&format!("/agents/{agent_key}/jobs?page=2")));

    let page_two = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/jobs?page=2"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(page_two.status(), StatusCode::OK);
    let page_two_text = response_text(page_two).await;
    assert!(page_two_text.contains("Showing 11-12 of 12 runs"));
    assert!(page_two_text.contains("Page 2 of 2"));
    assert!(page_two_text.contains(&format!("/agents/{agent_key}/jobs?page=1")));
}

#[tokio::test]
async fn recent_runs_stream_emits_initial_snapshot_and_matching_update() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let (agent_key, _) = insert_test_opencode_agent(&state)
        .await
        .expect("insert agent");
    let schedule_id = crate::agentic::store::list_agent_schedules(&pool, &agent_key)
        .await
        .expect("list schedules")
        .first()
        .expect("default schedule")
        .id;
    let run_id = crate::agentic::store::insert_test_run(&pool, schedule_id, "queued")
        .await
        .expect("insert run");

    let response = router(Arc::clone(&state))
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/agents/{agent_key}/jobs/recent-runs/stream?page=1"
                ))
                .body(Body::empty())
                .expect("build stream request"),
        )
        .await
        .expect("request stream");
    assert_eq!(response.status(), StatusCode::OK);
    let reader = tokio::spawn(read_sse_chunk(response.into_body(), 1_000));

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    sqlx::query(
        "UPDATE agentic_runs
            SET status = 'running', started_at = now(), backend_run_ref = 'stream-session'
          WHERE id = $1",
    )
    .bind(run_id)
    .execute(&pool)
    .await
    .expect("transition run");
    state
        .run_detail_events
        .publish(crate::web::run_detail_events::RunDetailDbEvent::RunChanged { run_id });

    let body = reader.await.expect("read stream");
    assert!(body.contains("event: recent-runs"));
    assert!(body.matches("event: recent-runs").count() >= 2);
    assert!(body.contains(">running<"));
    assert!(body.contains("data-running-duration"));
    assert!(body.contains("data-started-at=\""));
}

#[tokio::test]
async fn recent_runs_stream_ignores_other_agents_and_sessions() {
    let state = test_state().await;
    let (agent_key, _) = insert_test_opencode_agent(&state)
        .await
        .expect("insert agent");
    let (other_agent_key, _) = insert_test_opencode_agent(&state)
        .await
        .expect("insert other agent");
    let other_schedule_id =
        crate::agentic::store::list_agent_schedules(&state.db_pool, &other_agent_key)
            .await
            .expect("list other schedules")
            .first()
            .expect("other default schedule")
            .id;
    let other_run_id =
        crate::agentic::store::insert_test_run(&state.db_pool, other_schedule_id, "running")
            .await
            .expect("insert other run");

    let response = router(Arc::clone(&state))
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/agents/{agent_key}/jobs/recent-runs/stream?page=1"
                ))
                .body(Body::empty())
                .expect("build stream request"),
        )
        .await
        .expect("request stream");
    let reader = tokio::spawn(read_sse_chunk(response.into_body(), 250));
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    state.run_detail_events.publish(
        crate::web::run_detail_events::RunDetailDbEvent::RunChanged {
            run_id: other_run_id,
        },
    );
    state.run_detail_events.publish(
        crate::web::run_detail_events::RunDetailDbEvent::SessionChanged {
            session_id: "ignored-session".to_string(),
        },
    );

    let body = reader.await.expect("read stream");
    assert_eq!(body.matches("event: recent-runs").count(), 1);
}

#[tokio::test]
async fn recent_runs_stream_resync_preserves_page_and_refreshes_pagination() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let (agent_key, _) = insert_test_opencode_agent(&state)
        .await
        .expect("insert agent");
    let schedule_id = crate::agentic::store::list_agent_schedules(&pool, &agent_key)
        .await
        .expect("list schedules")
        .first()
        .expect("default schedule")
        .id;
    for _ in 0..12 {
        crate::agentic::store::insert_test_run(&pool, schedule_id, "succeeded")
            .await
            .expect("insert run");
    }

    let response = router(Arc::clone(&state))
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/agents/{agent_key}/jobs/recent-runs/stream?page=2"
                ))
                .body(Body::empty())
                .expect("build stream request"),
        )
        .await
        .expect("request stream");
    let reader = tokio::spawn(read_sse_chunk(response.into_body(), 1_000));
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    crate::agentic::store::insert_test_run(&pool, schedule_id, "queued")
        .await
        .expect("insert new run");
    state
        .run_detail_events
        .publish(crate::web::run_detail_events::RunDetailDbEvent::Resync);

    let body = reader.await.expect("read stream");
    assert!(body.matches("event: recent-runs").count() >= 2);
    assert!(body.contains("Page 2 of 2"));
    assert!(body.contains("Showing 11-13 of 13 runs"));
    assert!(body.contains("sse-swap=\"recent-runs\" hx-swap=\"outerHTML\""));
    assert!(!body.contains("id=\"agent-recent-runs-stream\""));
}

#[tokio::test]
async fn recent_runs_stream_returns_not_found_for_unknown_agent() {
    let state = test_state().await;
    let response = router(state)
        .oneshot(
            Request::builder()
                .uri("/agents/not-an-agent/jobs/recent-runs/stream")
                .body(Body::empty())
                .expect("build stream request"),
        )
        .await
        .expect("request stream");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn recent_runs_stream_ends_after_shutdown_signal() {
    let state = test_state_with_backend_and_shutdown(Arc::new(NoopAgenticBackend), true).await;
    let (agent_key, _) = insert_test_opencode_agent(&state)
        .await
        .expect("insert agent");
    let response = router(state)
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/jobs/recent-runs/stream"))
                .body(Body::empty())
                .expect("build stream request"),
        )
        .await
        .expect("request stream");

    let body = read_sse_chunk(response.into_body(), 250).await;
    assert_eq!(body.matches("event: recent-runs").count(), 1);
}
#[tokio::test]
async fn post_job_run_now_queues_and_dispatches_run() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let backend = Arc::new(RecordingAgenticBackend {
        calls: Arc::clone(&calls),
    });
    let state = test_state_with_backend(backend).await;
    let pool = state.db_pool.clone();
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");
    let schedules = crate::agentic::store::list_agent_schedules(&pool, &agent_key)
        .await
        .expect("list schedules");
    seed_instrument(&state, "BTC", true).await;
    replace_agent_instruments(&pool, &agent_key, &["BTC".to_string()])
        .await
        .expect("seed instruments");
    let schedule_id = schedules
        .iter()
        .find(|row| row.job_key == "analysis-15m")
        .map(|row| row.id)
        .expect("analysis schedule id");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/jobs/{schedule_id}/run"))
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
        Some(format!("/agents/{agent_key}/runs/1").as_str())
    );

    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(1);
    loop {
        if !calls.lock().unwrap().is_empty() {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "dispatch was not spawned"
        );
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
    }

    {
        let recorded = calls.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].schedule_id, Some(schedule_id));
        assert_eq!(recorded[0].agent_key, agent_key);
        assert_eq!(recorded[0].job_key, "analysis-15m");
    }

    let runs = crate::agentic::store::list_agent_runs(&pool, &agent_key, 10)
        .await
        .expect("list runs");
    assert!(runs.iter().any(|run| run.schedule_id == Some(schedule_id)));
}
#[tokio::test]
async fn post_analysis_job_run_now_triggers_market_analysis_hook_after_success() {
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
    let schedule_id = crate::agentic::store::list_agent_schedules(&pool, &agent_key)
        .await
        .expect("list schedules")
        .into_iter()
        .find(|row| row.job_key == "analysis-15m")
        .map(|row| row.id)
        .expect("analysis schedule id");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/jobs/{schedule_id}/run"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(1);
    loop {
        if calls.lock().unwrap().len() >= 2 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "analysis hook dispatch was not spawned"
        );
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
    }

    let recorded = calls.lock().unwrap();
    let job_keys: Vec<&str> = recorded
        .iter()
        .map(|request| request.job_key.as_str())
        .collect();
    assert!(job_keys.contains(&"analysis-15m"));
    assert!(job_keys.contains(&"market-analysis"));
}

#[tokio::test]
async fn post_schedule_run_now_rejected_after_shutdown_signal() {
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
    let schedule_id = crate::agentic::store::list_agent_schedules(&pool, &agent_key)
        .await
        .expect("list schedules")
        .into_iter()
        .find(|row| row.job_key == "analysis-15m")
        .map(|row| row.id)
        .expect("analysis schedule id");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/jobs/{schedule_id}/run"))
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
    assert!(
        location.contains("/jobs?warning="),
        "expected shutdown warning redirect, got: {location}"
    );
    assert!(
        location.contains("shutting+down") || location.contains("shutting%20down"),
        "expected shutdown warning text in redirect, got: {location}"
    );

    // The run row should have been inserted (the route claimed the
    // schedule) but immediately failed and never dispatched.
    let runs = crate::agentic::store::list_agent_runs(&pool, &agent_key, 10)
        .await
        .expect("list runs");
    let run = runs
        .iter()
        .find(|row| row.schedule_id == Some(schedule_id))
        .expect("a run row was inserted");
    assert_eq!(run.status, "failed");
    assert!(
        run.error_summary
            .as_deref()
            .unwrap_or("")
            .contains("shutting down"),
        "error_summary should mention shutdown, got: {:?}",
        run.error_summary
    );

    // No backend call should have been recorded.
    assert!(
        calls.lock().unwrap().is_empty(),
        "schedule Run now should not dispatch when shutdown_rx is set"
    );
}
#[tokio::test]
async fn post_analysis_job_run_now_skipped_does_not_trigger_market_analysis_hook() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let backend = Arc::new(RecordingAgenticBackend {
        calls: Arc::clone(&calls),
    });
    let state = test_state_with_backend(backend).await;
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
    let schedule_id = crate::agentic::store::list_agent_schedules(&pool, &agent_key)
        .await
        .expect("list schedules")
        .into_iter()
        .find(|row| row.job_key == "analysis-15m")
        .map(|row| row.id)
        .expect("analysis schedule id");
    let active_run_id = crate::agentic::store::insert_test_run(&pool, schedule_id, "running")
        .await
        .expect("insert active run");
    crate::agentic::store::mark_run_running(&pool, active_run_id, Some("ses_active"))
        .await
        .expect("mark active run running");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/jobs/{schedule_id}/run"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    assert!(calls.lock().unwrap().is_empty());
}
#[tokio::test]
async fn post_trading_job_run_now_does_not_trigger_market_analysis_hook() {
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
    let schedule_id = crate::agentic::store::list_agent_schedules(&pool, &agent_key)
        .await
        .expect("list schedules")
        .into_iter()
        .find(|row| row.job_key == "trading-1m")
        .map(|row| row.id)
        .expect("trading schedule id");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/jobs/{schedule_id}/run"))
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
            "trading dispatch was not spawned"
        );
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
    }

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let recorded = calls.lock().unwrap();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].job_key, "trading-1m");
    assert_eq!(recorded[0].job_kind, JOB_KIND_TRADING);
}
#[tokio::test]
async fn new_job_page_renders_for_opencode_agent() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/jobs/new"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert!(text.contains("Create job"));
    assert!(!text.contains("name=\"job_key\""));
    assert!(text.contains("name=\"job_kind\""));
    assert!(text.contains("name=\"timeframe\""));
    assert!(text.contains("name=\"timeout_seconds\""));
}
#[tokio::test]
async fn post_job_creates_new_schedule_and_redirects() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/jobs"))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(
                    "job_kind=analysis&timeframe=4h&timeout_seconds=600&model_selection=&operator_prompt=Check+higher+timeframe+structure",
                ))
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
        Some(format!("/agents/{agent_key}/jobs").as_str())
    );

    let schedules = crate::agentic::store::list_agent_schedules(&pool, &agent_key)
        .await
        .expect("list schedules");
    let schedule = schedules
        .iter()
        .find(|row| row.job_key == "analysis-4h")
        .expect("custom schedule present");
    assert_eq!(schedule.job_kind, JOB_KIND_ANALYSIS);
    assert!(!schedule.enabled);
    assert_eq!(schedule.timeframe, "4h");
    assert_eq!(schedule.timeout_seconds, 600);
    assert_eq!(schedule.model_provider_id.as_deref(), None);
    assert_eq!(schedule.model_id.as_deref(), None);
    assert_eq!(schedule.operator_prompt, "Check higher timeframe structure");
}
#[tokio::test]
async fn post_toggle_all_jobs_updates_schedules_and_hooks() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/jobs/toggle-all"))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("enabled=on"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert!(
        crate::agentic::store::list_agent_schedules(&pool, &agent_key)
            .await
            .expect("list schedules")
            .iter()
            .all(|row| row.enabled)
    );
    // Bulk-enable must NOT enable the autonomous analysis-coding
    // hook; the operator must enable it explicitly with a pinned model.
    let hooks = crate::agentic::store::list_agent_hooks(&pool, &agent_key)
        .await
        .expect("list hooks");
    for hook in &hooks {
        if hook.job_kind == crate::agentic::model::JOB_KIND_ANALYSIS_CODING {
            assert!(
                !hook.enabled,
                "coding hook {} must remain disabled after bulk enable",
                hook.id
            );
        } else {
            assert!(
                hook.enabled,
                "non-coding hook {} ({}) should be enabled after bulk enable",
                hook.id, hook.job_kind
            );
        }
    }

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/jobs/toggle-all"))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("enabled=off"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert!(
        crate::agentic::store::list_agent_schedules(&pool, &agent_key)
            .await
            .expect("list schedules")
            .iter()
            .all(|row| !row.enabled)
    );
    assert!(
        crate::agentic::store::list_agent_hooks(&pool, &agent_key)
            .await
            .expect("list hooks")
            .iter()
            .all(|row| !row.enabled)
    );
}
#[tokio::test]
async fn post_job_with_duplicate_job_kind_timeframe_returns_validation_error() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/jobs"))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(
                    "job_kind=analysis&timeframe=15m&timeout_seconds=600&model_selection=",
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let text = response_text(response).await;
    assert!(text.contains("A job with this kind and timeframe already exists for this agent."));
}
#[tokio::test]
async fn job_detail_page_renders_job_specific_runs() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");

    let schedules = crate::agentic::store::list_agent_schedules(&pool, &agent_key)
        .await
        .expect("list schedules");
    let schedule_id = schedules.first().expect("default schedule").id;
    let run_id = crate::agentic::store::insert_test_run(&pool, schedule_id, "running")
        .await
        .expect("insert run");
    crate::agentic::store::mark_run_succeeded(&pool, run_id, Some("ses_job_detail"))
        .await
        .expect("mark succeeded");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/jobs/{schedule_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert!(text.contains("Run now"));
    assert!(text.contains(&format!("/agents/{agent_key}/runs/{run_id}")));
    assert!(text.contains(&format!("/agents/{agent_key}/jobs/{schedule_id}/timeframe")));
    assert!(text.contains("data-detail-delete-trigger"));
    assert!(text.contains(&format!("/agents/{agent_key}/jobs/{schedule_id}/delete")));
    assert!(text.contains("cursor-pointer"));
    assert!(text.contains("data-model-picker-mode=\"modal\""));
    assert!(!text.contains("data-model-picker-lazy-open data-model-picker-url"));

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/agents/{agent_key}/jobs/{schedule_id}/model-picker"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert!(text.contains("data-model-picker-mode=\"modal\""));
    assert!(text.contains("Could not load configured OpenCode models"));
}
#[tokio::test]
async fn post_job_model_htmx_updates_without_redirect() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");
    let schedule_id = crate::agentic::store::list_agent_schedules(&pool, &agent_key)
        .await
        .expect("list schedules")
        .first()
        .expect("default schedule present")
        .id;

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/jobs/{schedule_id}/model"))
                .header("content-type", "application/x-www-form-urlencoded")
                .header("HX-Request", "true")
                .body(Body::from("model_selection="))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let schedule = crate::agentic::store::get_agent_schedule(&pool, &agent_key, schedule_id)
        .await
        .expect("get schedule")
        .expect("schedule present");
    assert!(schedule.model_provider_id.is_none());
    assert!(schedule.model_id.is_none());
}
#[tokio::test]
async fn post_job_model_without_htmx_redirects_to_detail() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");
    let schedule_id = crate::agentic::store::list_agent_schedules(&state.db_pool, &agent_key)
        .await
        .expect("list schedules")
        .first()
        .expect("default schedule present")
        .id;

    let response = router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/jobs/{schedule_id}/model"))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("model_selection="))
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
        Some(format!("/agents/{agent_key}/jobs/{schedule_id}").as_str())
    );
}
#[tokio::test]
async fn post_job_timeout_updates_and_redirects() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");
    let schedule_id = crate::agentic::store::list_agent_schedules(&pool, &agent_key)
        .await
        .expect("list schedules")
        .first()
        .expect("default schedule present")
        .id;

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/jobs/{schedule_id}/timeout"))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("timeout=20m"))
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
        Some(format!("/agents/{agent_key}/jobs/{schedule_id}").as_str())
    );

    let schedule = crate::agentic::store::get_agent_schedule(&pool, &agent_key, schedule_id)
        .await
        .expect("get schedule")
        .expect("schedule present");
    assert_eq!(schedule.timeout_seconds, 20 * 60);
}
#[tokio::test]
async fn post_job_timeframe_reanchors_schedule_and_regenerates_job_key() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");
    let schedule = crate::agentic::store::list_agent_schedules(&pool, &agent_key)
        .await
        .expect("list schedules")
        .into_iter()
        .find(|schedule| schedule.job_key == "analysis-15m")
        .expect("analysis schedule present");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/agents/{agent_key}/jobs/{}/timeframe",
                    schedule.id
                ))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("timeframe=4h"))
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
        Some(format!("/agents/{agent_key}/jobs/{}", schedule.id).as_str())
    );

    let updated = crate::agentic::store::get_agent_schedule(&pool, &agent_key, schedule.id)
        .await
        .expect("get schedule")
        .expect("schedule present");
    assert_eq!(updated.timeframe, "4h");
    assert_eq!(updated.job_key, "analysis-4h");
    assert!(updated.next_run_at > chrono::Utc::now());
    assert_eq!(
        (updated.next_run_at.timestamp()
            - i64::from(crate::agentic::timeframe::DEFAULT_TRIGGER_DELAY_SECONDS))
            % (4 * 60 * 60),
        0
    );
}
#[tokio::test]
async fn post_job_timeframe_invalid_value_redirects_with_error() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");
    let schedule_id = crate::agentic::store::list_agent_schedules(&pool, &agent_key)
        .await
        .expect("list schedules")
        .first()
        .expect("default schedule present")
        .id;

    let response = router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/jobs/{schedule_id}/timeframe"))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("timeframe=15s"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert!(
        response
            .headers()
            .get("location")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|location| location.starts_with(&format!(
                "/agents/{agent_key}/jobs/{schedule_id}?timeframe_error="
            )))
    );
}
#[tokio::test]
async fn post_job_timeout_accepts_humanized_and_composite_inputs() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");
    let schedule_id = crate::agentic::store::list_agent_schedules(&pool, &agent_key)
        .await
        .expect("list schedules")
        .first()
        .expect("default schedule present")
        .id;

    for (raw, expected_seconds) in [("1h 30m", 90 * 60), ("90", 90), ("45s", 45)] {
        let response = router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/agents/{agent_key}/jobs/{schedule_id}/timeout"))
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(format!("timeout={}", urlencode(raw))))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER, "input: {raw}");

        let schedule = crate::agentic::store::get_agent_schedule(&pool, &agent_key, schedule_id)
            .await
            .expect("get schedule")
            .expect("schedule present");
        assert_eq!(schedule.timeout_seconds, expected_seconds, "input: {raw}");
    }
}
#[tokio::test]
async fn post_job_timeout_invalid_value_redirects_with_error() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");
    let schedule_id = crate::agentic::store::list_agent_schedules(&pool, &agent_key)
        .await
        .expect("list schedules")
        .first()
        .expect("default schedule present")
        .id;

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/jobs/{schedule_id}/timeout"))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("timeout=not-a-time"))
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
        "/agents/{agent_key}/jobs/{schedule_id}?timeout_error="
    )));

    let schedule = crate::agentic::store::get_agent_schedule(&pool, &agent_key, schedule_id)
        .await
        .expect("get schedule")
        .expect("schedule present");
    assert_ne!(schedule.timeout_seconds, 0);
}
#[tokio::test]
async fn post_job_timeout_missing_schedule_returns_404() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/jobs/999999/timeout"))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("timeout=15m"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
