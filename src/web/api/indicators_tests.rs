use std::sync::Arc;

use axum::http::{Request, StatusCode};
use serde_json::json;
use tower::util::ServiceExt;

use super::test_support::*;

const SOURCE: &str =
    "//@version=5\nindicator(\"Test EMA\", overlay=true)\nplot(ta.ema(close, 20), \"EMA\")";

async fn response_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("read response body");
    serde_json::from_slice(&bytes).expect("JSON response")
}

async fn create_indicator(state: &Arc<crate::web::AppState>, api_key: &str) -> serde_json::Value {
    let (headers, body) = json_body(&json!({
        "name": "Test EMA",
        "description": "test indicator",
        "timeframe": "1h",
        "instrument_ids": ["BTC"],
        "source": SOURCE,
        "input_values": {},
    }));
    let mut request = Request::builder()
        .method("POST")
        .uri("/indicators")
        .header("authorization", format!("Bearer {api_key}"));
    if let Some((name, value)) = headers {
        request = request.header(name, value);
    }
    let response = app(Arc::clone(state))
        .oneshot(request.body(body).expect("request"))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    response_json(response).await
}

#[tokio::test]
async fn indicators_create_and_reads_are_scoped_to_the_authenticated_agent() {
    let state = test_state().await;
    let (_first_agent, first_key) = seed_agent(&state, "indicators-first").await;
    let (_second_agent, second_key) = seed_agent(&state, "indicators-second").await;

    let created = create_indicator(&state, &first_key).await;
    let id = created["id"].as_str().expect("indicator id");
    assert_eq!(created["active_version"]["version_number"], 1);
    assert_eq!(created["instrument_ids"], json!(["BTC"]));

    let (status, foreign) =
        get_json_response(&state, &second_key, &format!("/indicators/{id}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(foreign["error"], "indicator not found");

    let (status, results) = get_json_response(
        &state,
        &first_key,
        &format!("/indicators/{id}/results?limit=101"),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(results["error"], "limit must be between 1 and 100");
}

#[tokio::test]
async fn indicators_reject_invalid_source_before_persisting_a_definition() {
    let state = test_state().await;
    let (_agent_key, api_key) = seed_agent(&state, "indicators-invalid").await;
    let (headers, body) = json_body(&json!({
        "name": "Invalid",
        "timeframe": "1h",
        "instrument_ids": ["BTC"],
        "source": "strategy(\"not allowed\")",
    }));
    let mut request = Request::builder()
        .method("POST")
        .uri("/indicators")
        .header("authorization", format!("Bearer {api_key}"));
    if let Some((name, value)) = headers {
        request = request.header(name, value);
    }
    let response = app(state)
        .oneshot(request.body(body).expect("request"))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        response_json(response).await["error"],
        "strategy( is not supported by HyperVibes indicators"
    );
}
