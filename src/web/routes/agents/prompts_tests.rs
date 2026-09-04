//! Tests for the sub-agent strategy prompt endpoints owned by the agent
//! routes: singleton edit pages render prompt editors and the sub-agent prompt update,
//! rollback, and review-prompt-update handlers manage revisions.
use crate::web::routes::router;
use crate::web::routes::test_support::*;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::util::ServiceExt;

use crate::agents::strategy_prompts::get_agent_strategy_prompt;

async fn sub_agent_id_for(
    state: &std::sync::Arc<crate::web::AppState>,
    agent_key: &str,
    sub_agent_kind: &str,
) -> i64 {
    sqlx::query_scalar(
        "SELECT id FROM harness_sub_agents WHERE agent_key = $1 AND sub_agent_kind = $2",
    )
    .bind(agent_key)
    .bind(sub_agent_kind)
    .fetch_one(&state.db_pool)
    .await
    .expect("load sub-agent id")
}

#[tokio::test]
async fn singleton_edit_pages_render_prompt_editors() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_agent_with_text(
        &state,
        "Wait for analysis confirmation first.".to_string(),
        "Trade breakouts only after confirmation.".to_string(),
    )
    .await
    .expect("insert agent");

    for role in ["trading", "review"] {
        let sub_agent_id = sub_agent_id_for(&state, &agent_key, role).await;

        let response = router(state.clone())
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{agent_key}/{role}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let detail_text = response_text(response).await;
        assert!(detail_text.contains(&format!("/agents/{agent_key}/{role}/edit")));
        assert!(detail_text.contains("Recent Runs"));
        assert!(!detail_text.contains("Save prompt"));
        assert!(!detail_text.contains("Preview Prompt"));

        let response = router(state.clone())
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{agent_key}/{role}/edit"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let text = response_text(response).await;
        let editor_action = format!("/agents/{agent_key}/sub-agents/{sub_agent_id}/prompt");
        assert!(
            text.contains(&editor_action),
            "missing prompt form action {editor_action} for {role}"
        );
        assert!(
            text.contains("Save prompt"),
            "missing save button for {role}"
        );
        assert!(text.contains("name=\"base_revision_id\""));
    }

    // The Analysis list page links to per-job detail pages instead of an
    // inline editor.
    let analysis_id = sub_agent_id_for(&state, &agent_key, "analysis").await;
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
    assert!(text.contains(&format!("/agents/{agent_key}/sub-agents/{analysis_id}")));
}

#[tokio::test]
async fn post_analysis_prompt_updates_only_analysis_target() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
    let analysis_id = sub_agent_id_for(&state, &agent_key, "analysis").await;
    let trading_id = sub_agent_id_for(&state, &agent_key, "trading").await;

    let base_revision = get_agent_strategy_prompt(&pool, &agent_key, analysis_id)
        .await
        .expect("get analysis prompt")
        .expect("analysis prompt present")
        .revision_id;

    let body =
        format!("base_revision_id={base_revision}&prompt=Analyze+momentum+with+market+structure.");
    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/agents/{agent_key}/sub-agents/{analysis_id}/prompt"
                ))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let expected_location = format!("/agents/{agent_key}/sub-agents/{analysis_id}");
    assert_eq!(
        response
            .headers()
            .get("location")
            .and_then(|value| value.to_str().ok()),
        Some(expected_location.as_str())
    );

    let stored = get_agent_strategy_prompt(&pool, &agent_key, analysis_id)
        .await
        .expect("get analysis prompt")
        .expect("analysis prompt present");
    assert_eq!(stored.prompt, "Analyze momentum with market structure.");
    assert_ne!(stored.revision_id, base_revision);

    let trading = get_agent_strategy_prompt(&pool, &agent_key, trading_id)
        .await
        .expect("get trading prompt")
        .expect("trading prompt present");
    assert_ne!(
        trading.prompt, "Analyze momentum with market structure.",
        "analysis prompt update must not touch the trading prompt"
    );
}

#[tokio::test]
async fn post_trading_prompt_updates_only_trading_target() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
    let analysis_id = sub_agent_id_for(&state, &agent_key, "analysis").await;
    let trading_id = sub_agent_id_for(&state, &agent_key, "trading").await;

    let base_revision = get_agent_strategy_prompt(&pool, &agent_key, trading_id)
        .await
        .expect("get trading prompt")
        .expect("trading prompt present")
        .revision_id;

    let body =
        format!("base_revision_id={base_revision}&prompt=Only+place+limit+orders+near+support.");
    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/agents/{agent_key}/sub-agents/{trading_id}/prompt"
                ))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let expected_location = format!("/agents/{agent_key}/trading/edit");
    assert_eq!(
        response
            .headers()
            .get("location")
            .and_then(|value| value.to_str().ok()),
        Some(expected_location.as_str())
    );

    let stored = get_agent_strategy_prompt(&pool, &agent_key, trading_id)
        .await
        .expect("get trading prompt")
        .expect("trading prompt present");
    assert_eq!(stored.prompt, "Only place limit orders near support.");

    let analysis = get_agent_strategy_prompt(&pool, &agent_key, analysis_id)
        .await
        .expect("get analysis prompt")
        .expect("analysis prompt present");
    assert_ne!(
        analysis.prompt, "Only place limit orders near support.",
        "trading prompt update must not touch the analysis prompt"
    );
}

#[tokio::test]
async fn post_prompt_with_stale_base_revision_redirects_with_error() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
    let analysis_id = sub_agent_id_for(&state, &agent_key, "analysis").await;

    let body = "base_revision_id=999999&prompt=Stale+revision+attempt.".to_string();
    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/agents/{agent_key}/sub-agents/{analysis_id}/prompt"
                ))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(body))
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
        location.contains("prompt_error="),
        "stale revision should redirect with a prompt error, got: {location}"
    );

    let stored = get_agent_strategy_prompt(&state.db_pool, &agent_key, analysis_id)
        .await
        .expect("get analysis prompt")
        .expect("analysis prompt present");
    assert_ne!(stored.prompt, "Stale revision attempt.");
}

#[tokio::test]
async fn post_prompt_rollback_restores_prior_revision() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
    let analysis_id = sub_agent_id_for(&state, &agent_key, "analysis").await;
    let original = get_agent_strategy_prompt(&pool, &agent_key, analysis_id)
        .await
        .expect("get analysis prompt")
        .expect("analysis prompt present")
        .prompt;

    // Create one new revision, then roll it back.
    let base_revision = get_agent_strategy_prompt(&pool, &agent_key, analysis_id)
        .await
        .expect("get analysis prompt")
        .expect("analysis prompt present")
        .revision_id;
    let update_body =
        format!("base_revision_id={base_revision}&prompt=Temporary+revision+to+roll+back.");
    let _ = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/agents/{agent_key}/sub-agents/{analysis_id}/prompt"
                ))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(update_body))
                .unwrap(),
        )
        .await
        .unwrap();
    let new_revision = get_agent_strategy_prompt(&pool, &agent_key, analysis_id)
        .await
        .expect("get analysis prompt")
        .expect("analysis prompt present")
        .revision_id;

    let rollback_body = format!("revision_id={base_revision}");
    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/agents/{agent_key}/sub-agents/{analysis_id}/prompt/rollback"
                ))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(rollback_body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let stored = get_agent_strategy_prompt(&pool, &agent_key, analysis_id)
        .await
        .expect("get analysis prompt")
        .expect("analysis prompt present");
    assert_eq!(stored.prompt, original);
    assert_ne!(stored.revision_id, new_revision);
}
