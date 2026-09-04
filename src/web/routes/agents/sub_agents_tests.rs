//! Tests for agents/sub-agents_tests.rs
use super::*;
use crate::web::routes::router;
use crate::web::routes::test_support::*;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use std::sync::{Arc, Mutex};
use tower::util::ServiceExt;

use crate::{
    agents::store::replace_agent_instruments,
    harness::model::{
        SUB_AGENT_KIND_ANALYSIS, SUB_AGENT_KIND_CODING, SUB_AGENT_KIND_REVIEW,
        SUB_AGENT_KIND_TRADING,
    },
};

struct ReviewCodingEventBackend {
    calls: Arc<Mutex<Vec<crate::harness::backend::DispatchRequest>>>,
    pool: Mutex<Option<crate::db::DbPool>>,
}

#[async_trait::async_trait]
impl crate::harness::backend::HarnessBackend for ReviewCodingEventBackend {
    async fn dispatch(
        &self,
        request: crate::harness::backend::DispatchRequest,
    ) -> anyhow::Result<crate::harness::backend::DispatchResult> {
        let pool = self
            .pool
            .lock()
            .expect("review test backend pool lock")
            .clone()
            .expect("review test backend pool configured");
        let agent_key = request.agent_key.clone();
        let run_id = request.run_id;
        let is_review = request.sub_agent_kind == SUB_AGENT_KIND_REVIEW;
        self.calls
            .lock()
            .expect("review test backend calls lock")
            .push(request);

        if is_review {
            crate::memory::insert_memory(
                &pool,
                &agent_key,
                &crate::memory::CreateMemory {
                    scope_kind: crate::memory::model::MEMORY_SCOPE_AGENT.to_string(),
                    instrument_ids: Vec::new(),
                    timeframe: None,
                    memory_type: "review".to_string(),
                    summary: "Review requests a coding improvement".to_string(),
                    content: "The analysis helper needs a targeted improvement.".to_string(),
                    metadata: Some(serde_json::json!({
                        "coding_requested": true,
                        "coding_reason": "Improve the analysis helper."
                    })),
                    links: None,
                },
                Some(crate::memory::store::MemorySourceRun { run_id }),
            )
            .await?;
        }

        Ok(crate::harness::backend::DispatchResult {
            backend_run_ref: "ses_review_coding_event".to_string(),
        })
    }
}

#[tokio::test]
async fn manual_job_run_redirects_with_warning_during_coding_promotion() {
    let state = test_state().await;
    let app = router(Arc::clone(&state));
    let (agent_key, _) = insert_test_opencode_agent(&state)
        .await
        .expect("insert agent");
    let jobs = crate::harness::store::list_agent_sub_agents(&state.db_pool, &agent_key)
        .await
        .expect("list jobs");
    let sub_agent_id = jobs
        .iter()
        .find(|row| row.sub_agent_key == "technical-15m")
        .expect("analysis job present")
        .id;
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
            task_instructions: None,
            requested_mode: Some("auto"),
        },
    )
    .await
    .expect("queue coding task")
    {
        crate::harness::store::InsertAnalysisCodingTaskOutcome::Inserted { task_id, .. } => task_id,
        other => panic!("expected Inserted, got {other:?}"),
    };
    sqlx::query(
        "UPDATE harness_maintenance_tasks
            SET phase = 'promoting', status = 'running'
          WHERE id = $1",
    )
    .bind(task_id)
    .execute(&state.db_pool)
    .await
    .expect("place coding task into promotion phase");

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/sub-agents/{sub_agent_id}/run"))
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
    assert!(location.contains("/analysis?warning="));
}

#[tokio::test]
async fn jobs_route_is_not_registered() {
    let state = test_state().await;
    let (agent_key, _) = insert_test_opencode_agent(&state)
        .await
        .expect("insert agent");

    let response = router(state)
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/jobs"))
                .body(Body::empty())
                .expect("build old jobs route request"),
        )
        .await
        .expect("request old jobs route");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn sub_agents_route_is_not_registered() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/sub-agents"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // The generic sub-agents list route was replaced by role pages.
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn analysis_route_renders_create_sub_agent_button_and_runs_section() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/analysis"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert!(text.contains("New sub-agent"));
    assert!(text.contains(&format!("/agents/{agent_key}/analysis/new")));
    assert!(text.contains("Enable all"));
    assert!(!text.contains("Disable all"));
    assert!(text.contains("Recent Runs"));
    assert!(text.contains("Run now"));
}
#[tokio::test]
async fn jobs_route_paginates_recent_runs() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");
    let sub_agent_id = crate::harness::store::list_agent_sub_agents(&state.db_pool, &agent_key)
        .await
        .expect("list jobs")
        .into_iter()
        .find(|job| job.sub_agent_kind == SUB_AGENT_KIND_ANALYSIS)
        .expect("analysis job")
        .id;

    let base_time = chrono::Utc::now();
    for index in 1..=12 {
        let run_id =
            crate::harness::store::insert_test_run(&state.db_pool, sub_agent_id, "succeeded")
                .await
                .expect("insert test run");
        sqlx::query(
            "UPDATE harness_sub_agent_runs
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
                .uri(format!("/agents/{agent_key}/analysis"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(page_one.status(), StatusCode::OK);
    let page_one_text = response_text(page_one).await;
    assert!(page_one_text.contains("Showing 1-10 of 12 runs"));
    assert!(page_one_text.contains("Page 1 of 2"));
    assert!(page_one_text.contains(&format!("/agents/{agent_key}/analysis?page=2")));

    let page_two = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/analysis?page=2"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(page_two.status(), StatusCode::OK);
    let page_two_text = response_text(page_two).await;
    assert!(page_two_text.contains("Showing 11-12 of 12 runs"));
    assert!(page_two_text.contains("Page 2 of 2"));
    assert!(page_two_text.contains(&format!("/agents/{agent_key}/analysis?page=1")));
}

#[tokio::test]
async fn analysis_page_shows_only_analysis_runs() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");
    let jobs = crate::harness::store::list_agent_sub_agents(&state.db_pool, &agent_key)
        .await
        .expect("list jobs");
    let analysis_sub_agent_id = jobs
        .iter()
        .find(|job| job.sub_agent_kind == SUB_AGENT_KIND_ANALYSIS)
        .expect("analysis job")
        .id;
    let trading_sub_agent_id = jobs
        .iter()
        .find(|job| job.sub_agent_kind == SUB_AGENT_KIND_TRADING)
        .expect("trading job")
        .id;
    let analysis_run_id =
        crate::harness::store::insert_test_run(&state.db_pool, analysis_sub_agent_id, "succeeded")
            .await
            .expect("insert analysis run");
    let trading_run_id =
        crate::harness::store::insert_test_run(&state.db_pool, trading_sub_agent_id, "succeeded")
            .await
            .expect("insert trading run");

    let response = router(state)
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/analysis"))
                .body(Body::empty())
                .expect("build analysis request"),
        )
        .await
        .expect("analysis response");

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_text(response).await;
    let runs_section = body
        .split_once("id=\"agent-recent-runs\"")
        .and_then(|(_, section)| section.split_once("</section>"))
        .map(|(section, _)| section)
        .expect("recent runs section");
    assert!(runs_section.contains(&format!("/agents/{agent_key}/runs/{analysis_run_id}")));
    assert!(!runs_section.contains(&format!("/agents/{agent_key}/runs/{trading_run_id}")));
    assert!(runs_section.contains("Showing 1-1 of 1 runs"));
}

#[tokio::test]
async fn sub_agent_detail_paginates_runs() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");
    let sub_agent_id = crate::harness::store::list_agent_sub_agents(&state.db_pool, &agent_key)
        .await
        .expect("list jobs")
        .into_iter()
        .next()
        .expect("default job")
        .id;

    for _ in 0..12 {
        crate::harness::store::insert_test_run(&state.db_pool, sub_agent_id, "succeeded")
            .await
            .expect("insert test run");
    }

    let page_one = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/sub-agents/{sub_agent_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(page_one.status(), StatusCode::OK);
    let page_one_text = response_text(page_one).await;
    assert!(page_one_text.contains("Showing 1-10 of 12 runs"));
    assert!(page_one_text.contains("Page 1 of 2"));
    assert!(page_one_text.contains(&format!(
        "/agents/{agent_key}/sub-agents/{sub_agent_id}?page=2"
    )));

    let page_two = router(state)
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/agents/{agent_key}/sub-agents/{sub_agent_id}?page=2"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(page_two.status(), StatusCode::OK);
    let page_two_text = response_text(page_two).await;
    assert!(page_two_text.contains("Showing 11-12 of 12 runs"));
    assert!(page_two_text.contains("Page 2 of 2"));
    assert!(page_two_text.contains(&format!(
        "/agents/{agent_key}/sub-agents/{sub_agent_id}?page=1"
    )));
}

#[tokio::test]
async fn recent_runs_stream_emits_initial_snapshot_and_matching_update() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let (agent_key, _) = insert_test_opencode_agent(&state)
        .await
        .expect("insert agent");
    let sub_agent_id = crate::harness::store::list_agent_sub_agents(&pool, &agent_key)
        .await
        .expect("list jobs")
        .into_iter()
        .find(|job| job.sub_agent_kind == SUB_AGENT_KIND_ANALYSIS)
        .expect("analysis job")
        .id;
    let run_id = crate::harness::store::insert_test_run(&pool, sub_agent_id, "queued")
        .await
        .expect("insert run");

    let response = router(Arc::clone(&state))
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/agents/{agent_key}/sub-agents/recent-runs/stream?page=1"
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
        "UPDATE harness_sub_agent_runs
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
    let other_sub_agent_id =
        crate::harness::store::list_agent_sub_agents(&state.db_pool, &other_agent_key)
            .await
            .expect("list other jobs")
            .first()
            .expect("other default job")
            .id;
    let other_run_id =
        crate::harness::store::insert_test_run(&state.db_pool, other_sub_agent_id, "running")
            .await
            .expect("insert other run");

    let response = router(Arc::clone(&state))
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/agents/{agent_key}/sub-agents/recent-runs/stream?page=1"
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
    let sub_agent_id = crate::harness::store::list_agent_sub_agents(&pool, &agent_key)
        .await
        .expect("list jobs")
        .into_iter()
        .find(|job| job.sub_agent_kind == SUB_AGENT_KIND_ANALYSIS)
        .expect("analysis job")
        .id;
    for _ in 0..12 {
        crate::harness::store::insert_test_run(&pool, sub_agent_id, "succeeded")
            .await
            .expect("insert run");
    }

    let response = router(Arc::clone(&state))
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/agents/{agent_key}/sub-agents/recent-runs/stream?page=2"
                ))
                .body(Body::empty())
                .expect("build stream request"),
        )
        .await
        .expect("request stream");
    let reader = tokio::spawn(read_sse_chunk(response.into_body(), 1_000));
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    crate::harness::store::insert_test_run(&pool, sub_agent_id, "queued")
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
                .uri("/agents/not-an-agent/sub-agents/recent-runs/stream")
                .body(Body::empty())
                .expect("build stream request"),
        )
        .await
        .expect("request stream");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn recent_runs_stream_ends_after_shutdown_signal() {
    let state = test_state_with_backend_and_shutdown(Arc::new(NoopHarnessBackend), true).await;
    let (agent_key, _) = insert_test_opencode_agent(&state)
        .await
        .expect("insert agent");
    let response = router(state)
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/sub-agents/recent-runs/stream"))
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
    let backend = Arc::new(RecordingHarnessBackend {
        calls: Arc::clone(&calls),
    });
    let state = test_state_with_backend(backend).await;
    let pool = state.db_pool.clone();
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");
    let jobs = crate::harness::store::list_agent_sub_agents(&pool, &agent_key)
        .await
        .expect("list jobs");
    seed_instrument(&state, "BTC", true).await;
    replace_agent_instruments(&pool, &agent_key, &["BTC".to_string()])
        .await
        .expect("seed instruments");
    let sub_agent_id = jobs
        .iter()
        .find(|row| row.sub_agent_key == "technical-15m")
        .map(|row| row.id)
        .expect("analysis job id");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/sub-agents/{sub_agent_id}/run"))
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
        assert_eq!(recorded[0].sub_agent_id, sub_agent_id);
        assert_eq!(recorded[0].agent_key, agent_key);
        assert_eq!(recorded[0].sub_agent_key, "technical-15m");
    }

    let runs = crate::harness::store::list_agent_runs(&pool, &agent_key, 10)
        .await
        .expect("list runs");
    assert!(runs.iter().any(|run| run.sub_agent_id == sub_agent_id));
}
#[tokio::test]
async fn post_review_job_run_now_triggers_coding_event_after_success() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let backend = Arc::new(ReviewCodingEventBackend {
        calls: Arc::clone(&calls),
        pool: Mutex::new(None),
    });
    let harness_backend: Arc<dyn crate::harness::backend::HarnessBackend> = backend.clone();
    let state = test_state_with_backend(harness_backend).await;
    let pool = state.db_pool.clone();
    *backend
        .pool
        .lock()
        .expect("configure review test backend pool") = Some(pool.clone());
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");
    seed_instrument(&state, "BTC", true).await;
    replace_agent_instruments(&pool, &agent_key, &["BTC".to_string()])
        .await
        .expect("seed instruments");
    let coding_sub_agent_id = crate::harness::store::list_agent_sub_agents(&pool, &agent_key)
        .await
        .expect("list jobs")
        .into_iter()
        .find(|job| job.sub_agent_kind == SUB_AGENT_KIND_CODING)
        .expect("coding job present")
        .id;
    sqlx::query(
        "UPDATE harness_sub_agents
            SET enabled = true,
                model_provider_id = 'test',
                model_id = 'strong'
          WHERE id = $1",
    )
    .bind(coding_sub_agent_id)
    .execute(&pool)
    .await
    .expect("configure coding job");
    let sub_agent_id = crate::harness::store::list_agent_sub_agents(&pool, &agent_key)
        .await
        .expect("list jobs")
        .into_iter()
        .find(|job| job.sub_agent_kind == SUB_AGENT_KIND_REVIEW)
        .map(|row| row.id)
        .expect("review job id");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/sub-agents/{sub_agent_id}/run"))
                .body(Body::empty())
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
            .is_some_and(|location| location.starts_with(&format!("/agents/{agent_key}/runs/"))),
        "review Run now should queue a dispatch"
    );

    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(1);
    loop {
        if !calls
            .lock()
            .expect("review test backend calls lock")
            .is_empty()
        {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            let run = crate::harness::store::list_agent_runs(&pool, &agent_key, 10)
                .await
                .expect("load review run")
                .into_iter()
                .find(|run| run.sub_agent_id == sub_agent_id)
                .expect("review run queued");
            panic!(
                "review dispatch was not spawned; status: {}, error: {:?}",
                run.status, run.error_summary
            );
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
    }
    let review_run_id = calls.lock().expect("review test backend calls lock")[0].run_id;
    assert!(
        crate::memory::get_review_memory_for_run(&pool, &agent_key, review_run_id)
            .await
            .expect("load review memory")
            .is_some(),
        "review dispatch should write a coding-request memory"
    );

    loop {
        if crate::harness::store::get_latest_analysis_coding_task(&pool, &agent_key)
            .await
            .expect("load queued coding task")
            .is_some()
        {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "review coding event was not queued"
        );
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
    }

    let dispatched_review_run_id = {
        let recorded = calls.lock().expect("review test backend calls lock");
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].sub_agent_kind, SUB_AGENT_KIND_REVIEW);
        recorded[0].run_id
    };

    let task = crate::harness::store::get_latest_analysis_coding_task(&pool, &agent_key)
        .await
        .expect("load queued coding task")
        .expect("coding task queued");
    assert_eq!(task.sub_agent_id, Some(coding_sub_agent_id));
    assert_eq!(task.source_sub_agent_run_id, Some(dispatched_review_run_id));
}

#[tokio::test]
async fn post_job_run_now_rejected_after_shutdown_signal() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let backend = Arc::new(RecordingHarnessBackend {
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
    let sub_agent_id = crate::harness::store::list_agent_sub_agents(&pool, &agent_key)
        .await
        .expect("list jobs")
        .into_iter()
        .find(|row| row.sub_agent_key == "technical-15m")
        .map(|row| row.id)
        .expect("analysis job id");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/sub-agents/{sub_agent_id}/run"))
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
        location.contains("/analysis?warning="),
        "expected shutdown warning redirect, got: {location}"
    );
    assert!(
        location.contains("shutting+down") || location.contains("shutting%20down"),
        "expected shutdown warning text in redirect, got: {location}"
    );

    // The run row should have been inserted (the route claimed the
    // job) but immediately failed and never dispatched.
    let runs = crate::harness::store::list_agent_runs(&pool, &agent_key, 10)
        .await
        .expect("list runs");
    let run = runs
        .iter()
        .find(|row| row.sub_agent_id == sub_agent_id)
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
        "job Run now should not dispatch when shutdown_rx is set"
    );
}
#[tokio::test]
async fn post_review_job_run_now_skipped_does_not_trigger_coding_event() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let backend = Arc::new(RecordingHarnessBackend {
        calls: Arc::clone(&calls),
    });
    let state = test_state_with_backend(backend).await;
    let pool = state.db_pool.clone();
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");
    let coding_sub_agent_id = crate::harness::store::list_agent_sub_agents(&pool, &agent_key)
        .await
        .expect("list jobs")
        .into_iter()
        .find(|job| job.sub_agent_kind == SUB_AGENT_KIND_CODING)
        .expect("coding job present")
        .id;
    crate::harness::store::set_sub_agent_enabled(&pool, &agent_key, coding_sub_agent_id, true)
        .await
        .expect("enable coding job");
    let sub_agent_id = crate::harness::store::list_agent_sub_agents(&pool, &agent_key)
        .await
        .expect("list jobs")
        .into_iter()
        .find(|job| job.sub_agent_kind == SUB_AGENT_KIND_REVIEW)
        .map(|row| row.id)
        .expect("review job id");
    let active_run_id = crate::harness::store::insert_test_run(&pool, sub_agent_id, "running")
        .await
        .expect("insert active run");
    crate::harness::store::mark_run_running(&pool, active_run_id, Some("ses_active"))
        .await
        .expect("mark active run running");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/sub-agents/{sub_agent_id}/run"))
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
async fn post_trading_job_run_now_does_not_trigger_coding_event() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let backend = Arc::new(RecordingHarnessBackend {
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
    let coding_sub_agent_id = crate::harness::store::list_agent_sub_agents(&pool, &agent_key)
        .await
        .expect("list jobs")
        .into_iter()
        .find(|job| job.sub_agent_kind == SUB_AGENT_KIND_CODING)
        .expect("coding job present")
        .id;
    crate::harness::store::set_sub_agent_enabled(&pool, &agent_key, coding_sub_agent_id, true)
        .await
        .expect("enable coding job");
    let sub_agent_id = crate::harness::store::list_agent_sub_agents(&pool, &agent_key)
        .await
        .expect("list jobs")
        .into_iter()
        .find(|job| job.sub_agent_kind == SUB_AGENT_KIND_TRADING)
        .map(|row| row.id)
        .expect("trading job id");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/sub-agents/{sub_agent_id}/run"))
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
    assert_eq!(recorded[0].sub_agent_key, "trading-5m");
    assert_eq!(recorded[0].sub_agent_kind, SUB_AGENT_KIND_TRADING);
}
#[tokio::test]
async fn new_analysis_job_page_renders_for_opencode_agent() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/analysis/new"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert!(text.contains("Create Analysis sub-agent"));
    assert!(text.contains("name=\"sub_agent_key\""));
    assert!(!text.contains("name=\"sub_agent_kind\""));
    assert!(!text.contains("name=\"trigger_type\""));
    assert!(text.contains("name=\"timeframe\""));
    assert!(text.contains("name=\"timeout_seconds\""));
    assert!(text.contains("data-model-picker-modal"));
    assert!(text.contains("data-model-picker-submit-on-save"));
    assert!(text.contains("Prompt"));
    assert!(text.contains("Conservative swing trading research."));
}
#[tokio::test]
async fn post_analysis_job_creates_new_job_and_redirects() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/analysis"))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(
                    "sub_agent_key=technical-4h&timeframe=4h&timeout_seconds=600&model_selection=&prompt=Check+higher+timeframe+structure",
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
        Some(format!("/agents/{agent_key}/analysis").as_str())
    );

    let jobs = crate::harness::store::list_agent_sub_agents(&pool, &agent_key)
        .await
        .expect("list jobs");
    let job = jobs
        .iter()
        .find(|row| row.sub_agent_key == "technical-4h")
        .expect("custom job present");
    assert_eq!(job.sub_agent_kind, SUB_AGENT_KIND_ANALYSIS);
    assert!(!job.enabled);
    assert_eq!(job.timeframe.as_deref(), Some("4h"));
    assert_eq!(job.timeout_seconds, 600);
    assert_eq!(job.model_provider_id.as_deref(), None);
    assert_eq!(job.model_id.as_deref(), None);
    assert_eq!(
        crate::agents::strategy_prompts::get_agent_strategy_prompt(&pool, &agent_key, job.id,)
            .await
            .expect("load strategy prompt")
            .expect("strategy prompt present")
            .prompt,
        "Check higher timeframe structure"
    );
}
#[tokio::test]
async fn trading_singleton_is_created_from_its_new_page() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");

    let trading_sub_agent_id = crate::harness::store::list_agent_sub_agents(&pool, &agent_key)
        .await
        .expect("list jobs")
        .into_iter()
        .find(|job| job.sub_agent_kind == SUB_AGENT_KIND_TRADING)
        .expect("default trading singleton")
        .id;
    let deleted = crate::harness::service::delete_idle_job(
        &pool,
        &state.opencode_client,
        &state.workspace_leases,
        &state.opencode_base_url,
        &state.opencode_container_workspaces_root,
        &agent_key,
        trading_sub_agent_id,
    )
    .await
    .expect("delete default trading singleton");
    assert!(matches!(
        deleted,
        crate::harness::service::DeleteJobOutcome::Deleted
    ));

    let role_page = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/trading"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(role_page.status(), StatusCode::OK);
    assert!(
        response_text(role_page)
            .await
            .contains(&format!("/agents/{agent_key}/trading/new"))
    );

    let new_page = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/trading/new"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(new_page.status(), StatusCode::OK);
    let new_page_text = response_text(new_page).await;
    assert!(new_page_text.contains("New Trading sub-agent"));
    assert!(new_page_text.contains("Create Trading sub-agent"));
    assert!(new_page_text.contains("name=\"timeframe\""));
    assert!(new_page_text.contains("name=\"timeout_seconds\""));
    assert!(new_page_text.contains("name=\"prompt\""));
    assert!(!new_page_text.contains("data-model-picker-submit-on-save=\"true\""));

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/trading"))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(
                    "timeframe=15m&timeout_seconds=600&model_selection=&prompt=Trade+with+defined+risk",
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let jobs = crate::harness::store::list_agent_sub_agents(&pool, &agent_key)
        .await
        .expect("list jobs");
    let job = jobs
        .iter()
        .find(|row| row.sub_agent_kind == SUB_AGENT_KIND_TRADING)
        .expect("trading singleton recreated");
    assert_eq!(job.sub_agent_key, "trading-15m");
    assert_eq!(job.timeframe.as_deref(), Some("15m"));
    assert_eq!(job.timeout_seconds, 600);
    assert!(!job.enabled);
    assert_eq!(
        crate::agents::strategy_prompts::get_agent_strategy_prompt(&pool, &agent_key, job.id)
            .await
            .expect("load trading prompt")
            .expect("trading prompt present")
            .prompt,
        "Trade with defined risk"
    );

    let trading_page = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/trading"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(trading_page.status(), StatusCode::OK);
    let trading_page_text = response_text(trading_page).await;
    assert!(trading_page_text.contains("data-detail-delete-trigger"));
    assert!(trading_page_text.contains("data-model-picker-lazy"));
    assert!(trading_page_text.contains(&format!(
        "hx-get=\"/agents/{agent_key}/sub-agents/{}/model-picker\"",
        job.id
    )));
    assert!(!trading_page_text.contains("data-model-picker-lazy-result"));
    assert!(
        trading_page_text.contains(&format!("/agents/{agent_key}/sub-agents/{}/delete", job.id))
    );

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/sub-agents/{}/delete", job.id))
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
        Some(format!("/agents/{agent_key}/trading").as_str())
    );
}

#[tokio::test]
async fn review_singleton_is_created_from_its_new_page() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let (agent_key, _) = insert_test_opencode_agent(&state)
        .await
        .expect("insert agent");
    let review_sub_agent_id = crate::harness::store::list_agent_sub_agents(&pool, &agent_key)
        .await
        .expect("list jobs")
        .into_iter()
        .find(|job| job.sub_agent_kind == SUB_AGENT_KIND_REVIEW)
        .expect("default review singleton")
        .id;
    crate::harness::service::delete_idle_job(
        &pool,
        &state.opencode_client,
        &state.workspace_leases,
        &state.opencode_base_url,
        &state.opencode_container_workspaces_root,
        &agent_key,
        review_sub_agent_id,
    )
    .await
    .expect("delete default review singleton");

    let new_page = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/review/new"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(new_page.status(), StatusCode::OK);
    let new_page_text = response_text(new_page).await;
    assert!(new_page_text.contains("New Review sub-agent"));
    assert!(new_page_text.contains("Create Review sub-agent"));
    assert!(new_page_text.contains("name=\"timeframe\""));

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/review"))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(
                    "timeframe=4h&timeout_seconds=1200&model_selection=&prompt=Review+the+trading+outcomes",
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
        Some(format!("/agents/{agent_key}/review").as_str())
    );
    let job =
        crate::harness::store::get_singleton_sub_agent(&pool, &agent_key, SUB_AGENT_KIND_REVIEW)
            .await
            .expect("load review singleton")
            .expect("review singleton created");
    assert_eq!(job.sub_agent_key, "review-4h");
    assert_eq!(job.timeframe.as_deref(), Some("4h"));
    assert_eq!(job.timeout_seconds, 1200);
    assert!(!job.enabled);
    assert_eq!(
        crate::agents::strategy_prompts::get_agent_strategy_prompt(&pool, &agent_key, job.id)
            .await
            .expect("load review prompt")
            .expect("review prompt present")
            .prompt,
        "Review the trading outcomes"
    );
}
#[tokio::test]
async fn post_toggle_all_jobs_updates_all_jobs() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/sub-agents/toggle-all"))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("enabled=on"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let jobs = crate::harness::store::list_agent_sub_agents(&pool, &agent_key)
        .await
        .expect("list jobs");
    for job in &jobs {
        if job.sub_agent_kind == crate::harness::model::SUB_AGENT_KIND_CODING {
            assert!(
                !job.enabled,
                "coding job {} must remain disabled after bulk enable",
                job.id
            );
        } else {
            assert!(
                job.enabled,
                "non-coding job {} ({}) should be enabled after bulk enable",
                job.id, job.sub_agent_kind
            );
        }
    }

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/sub-agents/toggle-all"))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("enabled=off"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert!(
        crate::harness::store::list_agent_sub_agents(&pool, &agent_key)
            .await
            .expect("list jobs")
            .iter()
            .all(|row| !row.enabled)
    );
}
#[tokio::test]
async fn post_sub_agent_with_duplicate_type_timeframe_returns_validation_error() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/analysis"))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(
                    "sub_agent_key=technical-15m&timeframe=4h&timeout_seconds=600&model_selection=&prompt=Use+the+default+research+strategy",
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let text = response_text(response).await;
    assert!(text.contains("A sub-agent with this key already exists for this agent."));
}
#[tokio::test]
async fn job_detail_page_renders_job_specific_runs() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");

    let jobs = crate::harness::store::list_agent_sub_agents(&pool, &agent_key)
        .await
        .expect("list jobs");
    let sub_agent_id = jobs
        .iter()
        .find(|job| job.sub_agent_key == "technical-15m")
        .expect("analysis candle job")
        .id;
    let run_id = crate::harness::store::insert_test_run(&pool, sub_agent_id, "running")
        .await
        .expect("insert run");
    crate::harness::store::mark_run_succeeded(&pool, run_id, Some("ses_job_detail"))
        .await
        .expect("mark succeeded");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/sub-agents/{sub_agent_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert!(text.contains("Run now"));
    assert!(text.contains(&format!("/agents/{agent_key}/runs/{run_id}")));
    assert!(text.contains(&format!(
        "/agents/{agent_key}/sub-agents/{sub_agent_id}/timeframe"
    )));
    assert!(text.contains("At 15m candle close"));
    assert!(!text.contains(">Timeframe</p>"));
    assert!(text.contains("data-detail-delete-trigger"));
    assert!(text.contains(&format!(
        "/agents/{agent_key}/sub-agents/{sub_agent_id}/delete"
    )));
    assert!(text.contains("cursor-pointer"));
    assert!(text.contains("data-model-picker-modal"));
    assert!(!text.contains("data-model-picker-lazy-open data-model-picker-url"));
    assert!(text.contains("Prompt"));
    assert!(text.contains("Preview Prompt"));
    assert!(text.contains(&format!(
        "/agents/{agent_key}/sub-agents/{sub_agent_id}/prompt"
    )));

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/agents/{agent_key}/sub-agents/{sub_agent_id}/model-picker"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert!(text.contains("data-model-picker-modal"));
    assert!(text.contains("Could not load configured OpenCode models"));
}
#[tokio::test]
async fn post_job_prompt_updates_strategy_prompt() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");
    let sub_agent_id = crate::harness::store::list_agent_sub_agents(&pool, &agent_key)
        .await
        .expect("list jobs")
        .into_iter()
        .find(|job| job.sub_agent_kind == SUB_AGENT_KIND_ANALYSIS)
        .expect("default job present")
        .id;
    let base_revision =
        crate::agents::strategy_prompts::get_agent_strategy_prompt(&pool, &agent_key, sub_agent_id)
            .await
            .expect("load strategy prompt")
            .expect("strategy prompt present")
            .revision_id;
    let url = format!("/agents/{agent_key}/sub-agents/{sub_agent_id}/prompt");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&url)
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(format!(
                    "base_revision_id={base_revision}&prompt=++Focus+on+BTC++"
                )))
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
        Some(format!("/agents/{agent_key}/sub-agents/{sub_agent_id}").as_str())
    );
    assert_eq!(
        crate::agents::strategy_prompts::get_agent_strategy_prompt(&pool, &agent_key, sub_agent_id)
            .await
            .expect("load strategy prompt")
            .expect("strategy prompt present")
            .prompt,
        "Focus on BTC"
    );

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&url)
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(format!(
                    "base_revision_id={base_revision}&prompt=+++"
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let prompt =
        crate::agents::strategy_prompts::get_agent_strategy_prompt(&pool, &agent_key, sub_agent_id)
            .await
            .expect("load strategy prompt")
            .expect("strategy prompt present")
            .prompt;
    assert!(!prompt.is_empty());
}
#[tokio::test]
async fn post_job_model_htmx_updates_without_redirect() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");
    let sub_agent_id = crate::harness::store::list_agent_sub_agents(&pool, &agent_key)
        .await
        .expect("list jobs")
        .first()
        .expect("default job present")
        .id;
    crate::harness::store::set_sub_agent_model_with_variant(
        &pool,
        &agent_key,
        sub_agent_id,
        Some("anthropic"),
        Some("claude-sonnet-4"),
        Some("high"),
    )
    .await
    .expect("set job model");
    assert!(
        crate::harness::store::set_sub_agent_enabled(&pool, &agent_key, sub_agent_id, true)
            .await
            .expect("enable modeled job")
    );

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/agents/{agent_key}/sub-agents/{sub_agent_id}/model"
                ))
                .header("content-type", "application/x-www-form-urlencoded")
                .header("HX-Request", "true")
                .body(Body::from("model_selection="))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        response
            .headers()
            .get("hx-redirect")
            .and_then(|value| value.to_str().ok()),
        Some(format!("/agents/{agent_key}/sub-agents/{sub_agent_id}").as_str())
    );
    let job = crate::harness::store::get_agent_sub_agent(&pool, &agent_key, sub_agent_id)
        .await
        .expect("get job")
        .expect("job present");
    assert!(job.model_provider_id.is_none());
    assert!(job.model_id.is_none());
    assert!(job.model_variant.is_none());
    assert!(!job.enabled);
}

#[tokio::test]
async fn post_invalid_job_model_htmx_redirects_with_an_error() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");
    let sub_agent_id = crate::harness::store::list_agent_sub_agents(&state.db_pool, &agent_key)
        .await
        .expect("list jobs")
        .first()
        .expect("default job present")
        .id;

    let response = router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/agents/{agent_key}/sub-agents/{sub_agent_id}/model"
                ))
                .header("content-type", "application/x-www-form-urlencoded")
                .header("HX-Request", "true")
                .body(Body::from("model_selection=invalid"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let location = response
        .headers()
        .get("hx-redirect")
        .and_then(|value| value.to_str().ok())
        .expect("model error redirect");
    assert!(location.starts_with(&format!(
        "/agents/{agent_key}/sub-agents/{sub_agent_id}?model_error="
    )));
}
#[tokio::test]
async fn post_job_model_without_htmx_redirects_to_detail() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");
    let sub_agent_id = crate::harness::store::list_agent_sub_agents(&state.db_pool, &agent_key)
        .await
        .expect("list jobs")
        .first()
        .expect("default job present")
        .id;

    let response = router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/agents/{agent_key}/sub-agents/{sub_agent_id}/model"
                ))
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
        Some(format!("/agents/{agent_key}/sub-agents/{sub_agent_id}").as_str())
    );
}
#[tokio::test]
async fn post_job_timeout_updates_and_redirects() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");
    let sub_agent_id = crate::harness::store::list_agent_sub_agents(&pool, &agent_key)
        .await
        .expect("list jobs")
        .first()
        .expect("default job present")
        .id;

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/agents/{agent_key}/sub-agents/{sub_agent_id}/timeout"
                ))
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
        Some(format!("/agents/{agent_key}/sub-agents/{sub_agent_id}").as_str())
    );

    let job = crate::harness::store::get_agent_sub_agent(&pool, &agent_key, sub_agent_id)
        .await
        .expect("get job")
        .expect("job present");
    assert_eq!(job.timeout_seconds, 20 * 60);
}
#[tokio::test]
async fn post_job_timeframe_keeps_durable_analysis_key() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");
    let job = crate::harness::store::list_agent_sub_agents(&pool, &agent_key)
        .await
        .expect("list jobs")
        .into_iter()
        .find(|job| job.sub_agent_key == "technical-15m")
        .expect("analysis job present");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/agents/{agent_key}/sub-agents/{}/timeframe",
                    job.id
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
        Some(format!("/agents/{agent_key}/sub-agents/{}", job.id).as_str())
    );

    let updated = crate::harness::store::get_agent_sub_agent(&pool, &agent_key, job.id)
        .await
        .expect("get job")
        .expect("job present");
    assert_eq!(updated.timeframe.as_deref(), Some("4h"));
    // Analysis keys are durable user identities; a timeframe change must
    // not regenerate them.
    assert_eq!(updated.sub_agent_key, "technical-15m");
    let next_run_at = updated.next_run_at.expect("candle job has next run time");
    assert!(next_run_at > chrono::Utc::now());
    assert_eq!(
        (next_run_at.timestamp()
            - i64::from(crate::harness::timeframe::DEFAULT_TRIGGER_DELAY_SECONDS))
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
    let sub_agent_id = crate::harness::store::list_agent_sub_agents(&pool, &agent_key)
        .await
        .expect("list jobs")
        .first()
        .expect("default job present")
        .id;

    let response = router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/agents/{agent_key}/sub-agents/{sub_agent_id}/timeframe"
                ))
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
                "/agents/{agent_key}/sub-agents/{sub_agent_id}?timeframe_error="
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
    let sub_agent_id = crate::harness::store::list_agent_sub_agents(&pool, &agent_key)
        .await
        .expect("list jobs")
        .first()
        .expect("default job present")
        .id;

    for (raw, expected_seconds) in [("1h 30m", 90 * 60), ("90", 90), ("45s", 45)] {
        let response = router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/agents/{agent_key}/sub-agents/{sub_agent_id}/timeout"
                    ))
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(format!("timeout={}", urlencode(raw))))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER, "input: {raw}");

        let job = crate::harness::store::get_agent_sub_agent(&pool, &agent_key, sub_agent_id)
            .await
            .expect("get job")
            .expect("job present");
        assert_eq!(job.timeout_seconds, expected_seconds, "input: {raw}");
    }
}
#[tokio::test]
async fn post_job_timeout_invalid_value_redirects_with_error() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");
    let sub_agent_id = crate::harness::store::list_agent_sub_agents(&pool, &agent_key)
        .await
        .expect("list jobs")
        .first()
        .expect("default job present")
        .id;

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/agents/{agent_key}/sub-agents/{sub_agent_id}/timeout"
                ))
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
        "/agents/{agent_key}/sub-agents/{sub_agent_id}?timeout_error="
    )));

    let job = crate::harness::store::get_agent_sub_agent(&pool, &agent_key, sub_agent_id)
        .await
        .expect("get job")
        .expect("job present");
    assert_ne!(job.timeout_seconds, 0);
}
#[tokio::test]
async fn post_job_timeout_missing_job_returns_404() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/sub-agents/999999/timeout"))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("timeout=15m"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
