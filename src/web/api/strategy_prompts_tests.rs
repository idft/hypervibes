use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use serde_json::json;
use tower::util::ServiceExt;

use super::test_support::*;

async fn response_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("read response body");
    serde_json::from_slice(&bytes).expect("JSON response")
}

async fn default_sub_agent_id(
    state: &Arc<crate::web::AppState>,
    agent_key: &str,
    kind: &str,
) -> i64 {
    sqlx::query_scalar(
        "SELECT id FROM harness_sub_agents WHERE agent_key = $1 AND sub_agent_kind = $2",
    )
    .bind(agent_key)
    .bind(kind)
    .fetch_one(&state.db_pool)
    .await
    .expect("load default sub-agent")
}

#[tokio::test]
async fn strategy_prompts_list_returns_only_safe_prompt_fields() {
    let state = test_state().await;
    let (_agent_key, api_key) = seed_agent(&state, "prompts-list").await;

    let response = app(Arc::clone(&state))
        .oneshot(
            Request::builder()
                .uri("/strategy-prompts")
                .header("authorization", format!("Bearer {api_key}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    let prompts = body.as_array().expect("prompt list");
    assert_eq!(prompts.len(), 5);
    for prompt in prompts {
        assert_eq!(prompt.as_object().expect("prompt object").len(), 5);
        assert!(prompt["revision_id"].is_i64());
        assert!(prompt["target_sub_agent_id"].is_i64());
        assert!(prompt["target_sub_agent_key"].is_string());
        assert!(prompt["prompt"].is_string());
        assert!(prompt["updated_at"].is_string());
        assert!(prompt.get("agent_key").is_none());
    }
}

#[tokio::test]
async fn strategy_prompt_get_and_update_are_scoped_to_authenticated_agent() {
    let state = test_state().await;
    let (first_agent_key, first_api_key) = seed_agent(&state, "prompts-first").await;
    let (second_agent_key, second_api_key) = seed_agent(&state, "prompts-second").await;
    let first_trading_id = default_sub_agent_id(&state, &first_agent_key, "trading").await;
    let second_trading_id = default_sub_agent_id(&state, &second_agent_key, "trading").await;

    let body = json!({"prompt": "  Trade only liquid breakouts.  "});
    let (headers, body) = json_body(&body);
    let mut request = Request::builder()
        .method("PUT")
        .uri(format!("/strategy-prompts/{first_trading_id}"))
        .header("authorization", format!("Bearer {first_api_key}"));
    if let Some((name, value)) = headers {
        request = request.header(name, value);
    }
    let response = app(Arc::clone(&state))
        .oneshot(request.body(body).expect("request"))
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["target_sub_agent_id"], first_trading_id);
    assert_eq!(body["target_sub_agent_key"], "trading-5m");
    assert_eq!(body["prompt"], "Trade only liquid breakouts.");
    assert!(body["updated_at"].is_string());
    assert!(body["revision_id"].is_i64());
    assert_eq!(body.as_object().expect("prompt object").len(), 5);

    let (status, second_prompt) = get_json_response(
        &state,
        &second_api_key,
        &format!("/strategy-prompts/{second_trading_id}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_ne!(second_prompt["prompt"], "Trade only liquid breakouts.");
}

#[tokio::test]
async fn strategy_prompt_update_allows_an_empty_trimmed_prompt() {
    let state = test_state().await;
    let (agent_key, api_key) = seed_agent(&state, "prompts-empty").await;
    let analysis_id = default_sub_agent_id(&state, &agent_key, "analysis").await;
    let (headers, body) = json_body(&json!({"prompt": "   "}));
    let mut request = Request::builder()
        .method("PUT")
        .uri(format!("/strategy-prompts/{analysis_id}"))
        .header("authorization", format!("Bearer {api_key}"));
    if let Some((name, value)) = headers {
        request = request.header(name, value);
    }
    let response = app(state)
        .oneshot(request.body(body).expect("request"))
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_json(response).await["prompt"], "");
}

#[tokio::test]
async fn strategy_prompt_rejects_invalid_sub_agent_id_and_unknown_input() {
    let state = test_state().await;
    let (agent_key, api_key) = seed_agent(&state, "prompts-invalid").await;
    let analysis_id = default_sub_agent_id(&state, &agent_key, "analysis").await;

    let response = app(Arc::clone(&state))
        .oneshot(
            Request::builder()
                .uri("/strategy-prompts/not-an-id")
                .header("authorization", format!("Bearer {api_key}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let (headers, body) = json_body(&json!({"prompt": "updated", "agent_key": "other"}));
    let mut request = Request::builder()
        .method("PUT")
        .uri(format!("/strategy-prompts/{analysis_id}"))
        .header("authorization", format!("Bearer {api_key}"));
    if let Some((name, value)) = headers {
        request = request.header(name, value);
    }
    let response = app(state)
        .oneshot(request.body(body).expect("request"))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}
