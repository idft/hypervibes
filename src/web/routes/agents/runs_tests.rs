//! Tests for agents/runs_tests.rs
use std::sync::{Arc, Mutex};

use crate::web::routes::router;
use crate::web::routes::test_support::*;
use anyhow::Result;
use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::util::ServiceExt;

use crate::{
    harness::backend::{DispatchRequest, DispatchResult, HarnessBackend},
    opencode::client::SessionStatusKind,
};

struct CancelRecordingBackend {
    cancelled_sessions: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl HarnessBackend for CancelRecordingBackend {
    async fn dispatch(&self, _request: DispatchRequest) -> Result<DispatchResult> {
        Ok(DispatchResult {
            backend_run_ref: "ses_unused".to_string(),
        })
    }

    async fn abort_session(&self, _base_url: &str, session_id: &str) -> Result<bool> {
        self.cancelled_sessions
            .lock()
            .expect("lock cancelled sessions")
            .push(session_id.to_string());
        Ok(true)
    }

    async fn get_session_status(
        &self,
        _base_url: &str,
        _session_id: &str,
    ) -> Result<Option<SessionStatusKind>> {
        let cancelled = !self
            .cancelled_sessions
            .lock()
            .expect("lock cancelled sessions")
            .is_empty();
        Ok(Some(if cancelled {
            SessionStatusKind::Idle
        } else {
            SessionStatusKind::Busy
        }))
    }
}

#[tokio::test]
async fn cancel_running_run_aborts_session_and_marks_run_aborted() {
    let cancelled_sessions = Arc::new(Mutex::new(Vec::new()));
    let state = test_state_with_backend(Arc::new(CancelRecordingBackend {
        cancelled_sessions: Arc::clone(&cancelled_sessions),
    }))
    .await;
    let pool = state.db_pool.clone();
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert agent");
    let sub_agent_id = crate::harness::store::list_agent_sub_agents(&pool, &agent_key)
        .await
        .expect("list jobs")
        .first()
        .expect("default job")
        .id;
    let run_id = crate::harness::store::insert_test_run(&pool, sub_agent_id, "running")
        .await
        .expect("insert run");
    crate::harness::store::mark_run_running(&pool, run_id, Some("ses_cancel"))
        .await
        .expect("mark running");

    let detail_response = router(Arc::clone(&state))
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/runs/{run_id}"))
                .body(Body::empty())
                .expect("build detail request"),
        )
        .await
        .expect("show run detail");
    let detail_html = response_text(detail_response).await;
    assert!(detail_html.contains("Cancel run"));
    assert!(detail_html.contains(&format!("/agents/{agent_key}/runs/{run_id}/cancel")));

    let response = router(Arc::clone(&state))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/runs/{run_id}/cancel"))
                .body(Body::empty())
                .expect("build cancel request"),
        )
        .await
        .expect("cancel run");

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response
            .headers()
            .get("location")
            .and_then(|value| value.to_str().ok()),
        Some(format!("/agents/{agent_key}/runs/{run_id}").as_str())
    );
    assert_eq!(
        cancelled_sessions
            .lock()
            .expect("lock cancelled sessions")
            .as_slice(),
        ["ses_cancel"]
    );
    let run = crate::harness::store::get_run(&pool, run_id)
        .await
        .expect("fetch run")
        .expect("run present");
    assert_eq!(run.status, "aborted");
    assert_eq!(run.error_summary.as_deref(), Some("cancelled by operator"));
}

#[tokio::test]
async fn retry_failed_run_creates_a_new_queued_run() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert agent");
    let sub_agent_id = crate::harness::store::list_agent_sub_agents(&pool, &agent_key)
        .await
        .expect("list jobs")
        .iter()
        .find(|job| job.sub_agent_key == "technical-15m")
        .expect("analysis job")
        .id;
    let failed_run_id = crate::harness::store::insert_test_run(&pool, sub_agent_id, "running")
        .await
        .expect("insert run");
    crate::harness::store::mark_run_failed(&pool, failed_run_id, "provider unavailable", None)
        .await
        .expect("mark failed");

    let detail_response = router(Arc::clone(&state))
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/runs/{failed_run_id}"))
                .body(Body::empty())
                .expect("build detail request"),
        )
        .await
        .expect("show run detail");
    let detail_html = response_text(detail_response).await;
    assert!(detail_html.contains("Retry run"));
    assert!(detail_html.contains(&format!("/agents/{agent_key}/runs/{failed_run_id}/retry")));

    let response = router(Arc::clone(&state))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/runs/{failed_run_id}/retry"))
                .body(Body::empty())
                .expect("build retry request"),
        )
        .await
        .expect("retry run");

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let location = response
        .headers()
        .get("location")
        .and_then(|value| value.to_str().ok())
        .expect("retry location");
    let retry_run_id = location
        .rsplit('/')
        .next()
        .expect("retry id")
        .parse::<i64>()
        .expect("numeric retry id");
    assert_ne!(retry_run_id, failed_run_id);
    let retry = crate::harness::store::get_run(&pool, retry_run_id)
        .await
        .expect("fetch retry")
        .expect("retry exists");
    assert_eq!(retry.sub_agent_id, sub_agent_id);
    assert_eq!(retry.status, "queued");
}

#[tokio::test]
async fn run_detail_page_handles_missing_opencode_session_mirror() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");

    let jobs = crate::harness::store::list_agent_sub_agents(&pool, &agent_key)
        .await
        .expect("list jobs");
    let sub_agent_id = jobs.first().expect("default job").id;
    let run_id = crate::harness::store::insert_test_run(&pool, sub_agent_id, "running")
        .await
        .expect("insert run");
    crate::harness::store::mark_run_succeeded(&pool, run_id, Some("ses_ui_detail"))
        .await
        .expect("mark succeeded");

    let response = router(state)
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/runs/{run_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert!(text.contains("backend session id"));
    assert!(text.contains("no matching row was found yet in the"));
}

#[tokio::test]
async fn run_detail_surfaces_session_error_only_in_top_error_panel() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");
    let sub_agent_id = crate::harness::store::list_agent_sub_agents(&pool, &agent_key)
        .await
        .expect("list jobs")
        .first()
        .expect("default job")
        .id;
    let run_id = crate::harness::store::insert_test_run(&pool, sub_agent_id, "running")
        .await
        .expect("insert run");
    crate::harness::store::mark_run_running(&pool, run_id, Some("ses_error_detail"))
        .await
        .expect("mark running");
    crate::harness::store::mark_run_failed(&pool, run_id, "model request failed", None)
        .await
        .expect("mark failed");
    sqlx::query("INSERT INTO opencode.sessions (id) VALUES ('ses_error_detail')")
        .execute(&pool)
        .await
        .expect("insert session");
    sqlx::query(
        "INSERT INTO opencode.session_errors (session_id, error_type, error_message)
         VALUES ('ses_error_detail', 'APIError', 'Model requires explicit opt-in')",
    )
    .execute(&pool)
    .await
    .expect("insert session error");

    let response = router(state)
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/runs/{run_id}"))
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("show run detail");

    let text = response_text(response).await;
    assert!(text.contains(">Error</h2>"));
    assert!(text.contains("APIError: Model requires explicit opt-in"));
    assert!(!text.contains("Session errors"));
}

#[tokio::test]
async fn run_detail_stream_emits_summary_and_transcript_snapshots() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");
    let sub_agent_id = crate::harness::store::list_agent_sub_agents(&pool, &agent_key)
        .await
        .expect("list jobs")
        .first()
        .expect("default job")
        .id;
    let run_id = crate::harness::store::insert_test_run(&pool, sub_agent_id, "running")
        .await
        .expect("insert run");
    crate::harness::store::mark_run_succeeded(&pool, run_id, Some("ses_stream_detail"))
        .await
        .expect("mark succeeded");
    sqlx::query("INSERT INTO opencode.sessions (id) VALUES ('ses_stream_detail')")
        .execute(&pool)
        .await
        .expect("insert session");
    sqlx::query(
        "INSERT INTO opencode.messages (id, session_id, role, text)
         VALUES ('msg_stream_detail', 'ses_stream_detail', 'assistant', 'stream message')",
    )
    .execute(&pool)
    .await
    .expect("insert message");

    let response = router(state)
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/runs/{run_id}/stream"))
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("request stream");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .map(|value| value.starts_with("text/event-stream")),
        Some(true)
    );
    let body = read_sse_chunk(response.into_body(), 250).await;
    assert!(body.contains("event: run-summary"));
    assert!(body.contains("event: run-transcript"));
    assert!(body.contains("stream message"));
}

#[tokio::test]
async fn run_detail_stream_rerenders_after_matching_session_event() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");
    let sub_agent_id = crate::harness::store::list_agent_sub_agents(&pool, &agent_key)
        .await
        .expect("list jobs")
        .first()
        .expect("default job")
        .id;
    let run_id = crate::harness::store::insert_test_run(&pool, sub_agent_id, "running")
        .await
        .expect("insert run");
    crate::harness::store::mark_run_succeeded(&pool, run_id, Some("ses_stream_update"))
        .await
        .expect("mark succeeded");
    sqlx::query("INSERT INTO opencode.sessions (id) VALUES ('ses_stream_update')")
        .execute(&pool)
        .await
        .expect("insert session");
    sqlx::query(
        "INSERT INTO opencode.messages (id, session_id, role, text)
         VALUES ('msg_stream_update', 'ses_stream_update', 'assistant', 'before update')",
    )
    .execute(&pool)
    .await
    .expect("insert message");

    let response = router(std::sync::Arc::clone(&state))
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/runs/{run_id}/stream"))
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("request stream");
    assert_eq!(response.status(), StatusCode::OK);

    let reader = tokio::spawn(read_sse_chunk(response.into_body(), 1_000));
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    sqlx::query(
        "UPDATE opencode.messages SET text = 'after update' WHERE id = 'msg_stream_update'",
    )
    .execute(&pool)
    .await
    .expect("update message");
    state.run_detail_events.publish(
        crate::web::run_detail_events::RunDetailDbEvent::SessionChanged {
            session_id: "ses_stream_update".to_string(),
        },
    );

    let body = reader.await.expect("read stream");
    assert!(body.contains("after update"));
}

#[tokio::test]
async fn unknown_run_detail_stream_returns_not_found() {
    let state = test_state().await;
    let response = router(state)
        .oneshot(
            Request::builder()
                .uri("/agents/not-an-agent/runs/999999/stream")
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("request stream");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
