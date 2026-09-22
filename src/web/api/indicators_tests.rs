use std::sync::Arc;

use axum::http::{Request, StatusCode};
use serde_json::json;
use tower::util::ServiceExt;

use super::test_support::*;
use crate::indicators::store::{
    PersistedIndicatorOutput, claim_next_queued_run, finish_run_succeeded,
};

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
async fn analysis_instruments_lists_only_selected_active_targets() {
    let state = test_state().await;
    let (agent_key, api_key) = seed_agent(&state, "analysis-instruments").await;
    seed_instrument(&state, "ETH", true).await;
    select_instruments(&state, &agent_key, &["ETH"]).await;

    let (status, instruments) = get_json_response(&state, &api_key, "/analysis-instruments").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(instruments, json!(["ETH"]));
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

#[tokio::test]
async fn indicator_results_expose_raw_versioned_visual_data() {
    let state = test_state().await;
    let (agent_key, api_key) = seed_agent(&state, "indicator-visual-results").await;
    let created = create_indicator(&state, &api_key).await;
    let id = created["id"].as_str().expect("indicator id");
    let run = claim_next_queued_run(&state.db_pool, &[])
        .await
        .expect("claim run")
        .expect("queued run");
    let visual_data = json!({
        "version": 1,
        "markers": [
            {
                "kind": "plotshape", "bar_index": 0, "value": 1.0,
                "title": "Buy", "text": "BUY", "style": "triangleup",
                "location": "belowbar", "color": null, "text_color": null,
                "size": "normal", "offset": 0
            },
            {
                "kind": "plotchar", "bar_index": 0, "value": 1.0,
                "title": "Stage", "character": "1", "text": "",
                "location": "belowbar", "color": null, "text_color": null,
                "size": "normal", "offset": 0
            },
            {
                "kind": "plotarrow", "bar_index": 0, "value": -2.0,
                "title": "Momentum", "color_up": null, "color_down": null,
                "min_height": 5.0, "max_height": 100.0, "offset": 0
            }
        ]
    });
    assert!(
        finish_run_succeeded(
            &state.db_pool,
            &agent_key,
            run.id,
            PersistedIndicatorOutput {
                candle_data: json!([]),
                plot_data: json!({}),
                visual_data: visual_data.clone(),
                latest_values: json!({}),
                diagnostics: json!([]),
            },
        )
        .await
        .expect("finish run")
    );

    let (status, results) =
        get_json_response(&state, &api_key, &format!("/indicators/{id}/results")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(results[0]["visual_data"], visual_data);
    assert_eq!(results[0]["visual_data"]["markers"][0]["kind"], "plotshape");
    assert_eq!(results[0]["visual_data"]["markers"][1]["character"], "1");
    assert_eq!(results[0]["visual_data"]["markers"][2]["min_height"], 5.0);
}
