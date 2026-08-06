//! Tests for agents/runs_tests.rs
use crate::web::routes::router;
use crate::web::routes::test_support::*;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::util::ServiceExt;

#[tokio::test]
async fn run_detail_page_handles_missing_opencode_session_mirror() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");

    let jobs = crate::harness::store::list_agent_jobs(&pool, &agent_key)
        .await
        .expect("list jobs");
    let job_id = jobs.first().expect("default job").id;
    let run_id = crate::harness::store::insert_test_run(&pool, job_id, "running")
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
async fn run_detail_stream_emits_summary_and_transcript_snapshots() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
        .await
        .expect("insert opencode agent");
    let job_id = crate::harness::store::list_agent_jobs(&pool, &agent_key)
        .await
        .expect("list jobs")
        .first()
        .expect("default job")
        .id;
    let run_id = crate::harness::store::insert_test_run(&pool, job_id, "running")
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
    let job_id = crate::harness::store::list_agent_jobs(&pool, &agent_key)
        .await
        .expect("list jobs")
        .first()
        .expect("default job")
        .id;
    let run_id = crate::harness::store::insert_test_run(&pool, job_id, "running")
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
