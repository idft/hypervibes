use std::sync::Arc;

use axum::http::{Request, StatusCode};
use serde_json::json;
use tower::util::ServiceExt;

use super::test_support::*;

#[tokio::test]
async fn coding_report_is_scoped_to_the_authenticated_agent() {
    let state = test_state().await;
    let (agent_key, api_key) = seed_agent(&state, "coding-report").await;
    let (_other_agent, other_key) = seed_agent(&state, "coding-other").await;
    sqlx::query(
        "INSERT INTO agentic_job_hooks
            (agent_key, job_key, job_kind, hook_event, enabled, timeout_seconds, operator_prompt)
         VALUES ($1, 'analysis-coding', 'analysis_coding', 'daily_review_completed', false, 1800, '')",
    )
    .bind(&agent_key)
    .execute(&state.db_pool)
    .await
    .unwrap();
    let hook_id = crate::agentic::store::list_agent_hooks(&state.db_pool, &agent_key)
        .await
        .unwrap()
        .into_iter()
        .find(|hook| hook.job_kind == "analysis_coding")
        .expect("coding hook")
        .id;
    sqlx::query("UPDATE agentic_job_hooks SET model_provider_id = 'test', model_id = 'strong' WHERE id = $1")
        .bind(hook_id)
        .execute(&state.db_pool)
        .await
        .unwrap();
    let queued = crate::agentic::store::insert_analysis_coding_task_and_run(
        &state.db_pool,
        &agent_key,
        hook_id,
        crate::agentic::store::CodingTriggerMode::Manual,
        None,
        None,
        None,
        Some("auto"),
    )
    .await
    .unwrap();
    let task_id = match queued {
        crate::agentic::store::InsertAnalysisCodingTaskOutcome::Inserted { task_id, .. } => task_id,
        crate::agentic::store::InsertAnalysisCodingTaskOutcome::AlreadyQueued => {
            panic!("unexpected duplicate")
        }
    };
    let body = json!({
        "task_id": task_id,
        "schema_version": 1,
        "outcome": "no_change",
        "summary": "No safe change",
        "rationale": "The current implementation is adequate",
        "changed_paths": [],
        "evidence_memory_ids": [],
        "validation_notes": "Not run"
    });
    let (_, body) = json_body(&body);
    let response = app(Arc::clone(&state))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/coding/report")
                .header("authorization", format!("Bearer {other_key}"))
                .header("content-type", "application/json")
                .body(body)
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let body = json!({
        "task_id": task_id,
        "schema_version": 1,
        "outcome": "changed",
        "summary": "Changed analyzer",
        "rationale": "Added quantitative measurements",
        "changed_paths": ["scripts/user/analyze.py"],
        "evidence_memory_ids": [],
        "validation_notes": "Passed"
    });
    let (_, body) = json_body(&body);
    let response = app(Arc::clone(&state))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/coding/report")
                .header("authorization", format!("Bearer {api_key}"))
                .header("content-type", "application/json")
                .body(body)
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let body = json!({
        "task_id": task_id,
        "schema_version": 1,
        "outcome": "no_change",
        "summary": "No safe change",
        "rationale": "The current implementation is adequate",
        "changed_paths": [],
        "evidence_memory_ids": [],
        "validation_notes": "Not run"
    });
    let (_, body) = json_body(&body);
    let response = app(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/coding/report")
                .header("authorization", format!("Bearer {api_key}"))
                .header("content-type", "application/json")
                .body(body)
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
}
