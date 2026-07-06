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

    let schedules = crate::agentic::store::list_agent_schedules(&pool, &agent_key)
        .await
        .expect("list schedules");
    let schedule_id = schedules.first().expect("default schedule").id;
    let run_id = crate::agentic::store::insert_test_run(&pool, schedule_id, "running")
        .await
        .expect("insert run");
    crate::agentic::store::mark_run_succeeded(&pool, run_id, Some("ses_ui_detail"))
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
    assert!(text.contains("Scheduled for"));
    assert!(text.contains("no matching row was found yet in the"));
}
