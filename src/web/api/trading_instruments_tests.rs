use axum::http::{Request, StatusCode};
use serde_json::json;
use tower::util::ServiceExt;

use super::test_support::*;

async fn response_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("read response body");
    serde_json::from_slice(&bytes).expect("JSON response")
}

async fn set_instrument(
    state: &std::sync::Arc<crate::web::AppState>,
    api_key: &str,
    instrument_id: &str,
    enabled: bool,
) -> axum::response::Response {
    let (headers, body) = json_body(&json!({"enabled": enabled}));
    let mut request = Request::builder()
        .method("PUT")
        .uri(format!("/trading-instruments/{instrument_id}"))
        .header("authorization", format!("Bearer {api_key}"));
    if let Some((name, value)) = headers {
        request = request.header(name, value);
    }
    app(std::sync::Arc::clone(state))
        .oneshot(request.body(body).expect("request"))
        .await
        .expect("response")
}

#[tokio::test]
async fn trading_instruments_can_be_promoted_from_analysis_and_removed_individually() {
    let state = test_state().await;
    let (agent_key, api_key) = seed_agent(&state, "trading-instrument-write").await;
    seed_instrument(&state, "ETH", true).await;
    crate::agents::store::replace_agent_analysis_instruments(
        &state.db_pool,
        &agent_key,
        &["BTC".to_string(), "ETH".to_string()],
    )
    .await
    .expect("select analysis instruments");

    let response = set_instrument(&state, &api_key, "ETH", true).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_json(response).await, json!(["BTC", "ETH"]));

    let response = set_instrument(&state, &api_key, "BTC", false).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_json(response).await, json!(["ETH"]));

    let response = set_instrument(&state, &api_key, "ETH", false).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_json(response).await, json!([]));
}

#[tokio::test]
async fn trading_instrument_promotion_requires_an_active_analysis_selection() {
    let state = test_state().await;
    let (_agent_key, api_key) = seed_agent(&state, "trading-instrument-validation").await;
    seed_instrument(&state, "ETH", true).await;

    let response = set_instrument(&state, &api_key, "ETH", true).await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        response_json(response).await["error"],
        "trading instruments must be active instruments selected for analysis"
    );
}
