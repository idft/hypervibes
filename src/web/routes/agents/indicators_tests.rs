use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use chrono::{TimeZone, Utc};
use rust_decimal::Decimal;
use serde_json::json;
use tower::util::ServiceExt;

use crate::{
    agents::store::replace_agent_analysis_instruments,
    indicators::{
        model::Candle,
        runtime::validate_indicator_source,
        store::{
            CreateIndicatorDefinition, IndicatorUpdateResult, NewIndicatorVersion,
            PersistedIndicatorOutput, UpdateIndicatorDefinition, claim_next_queued_run,
            create_definition_with_initial_version, enqueue_run, finish_run_succeeded,
            get_active_version, update_definition_with_new_version,
        },
    },
    web::routes::{router, test_support::*},
};

const SOURCE: &str =
    "//@version=5\nindicator(\"Test EMA\", overlay=true)\nplot(ta.ema(close, 2), \"EMA\")";

async fn seed_indicator(
    state: &std::sync::Arc<crate::web::AppState>,
) -> (String, uuid::Uuid, uuid::Uuid) {
    let (agent_key, _) = insert_test_agent(state).await.expect("insert agent");
    seed_instrument(state, "BTC", true).await;
    replace_agent_analysis_instruments(&state.db_pool, &agent_key, &["BTC".to_string()])
        .await
        .expect("select analysis instrument");
    let validated = validate_indicator_source(SOURCE, &json!({})).expect("validate source");
    let definition = create_definition_with_initial_version(
        &state.db_pool,
        &agent_key,
        &CreateIndicatorDefinition {
            name: "Test EMA".to_string(),
            description: String::new(),
            timeframes: vec!["1h".to_string()],
            enabled: true,
            instrument_ids: vec!["BTC".to_string()],
            version: NewIndicatorVersion {
                source: SOURCE.to_string(),
                compiler_version: "test".to_string(),
                metadata: json!({"indicator": validated.metadata, "inputs": validated.inputs}),
                input_values: json!({}),
                created_by_kind: "operator".to_string(),
                created_by_run_id: None,
                created_by_conversation_id: None,
            },
        },
    )
    .await
    .expect("create definition");
    let version = get_active_version(&state.db_pool, &agent_key, definition.id)
        .await
        .expect("get active version")
        .expect("active version");
    (agent_key, definition.id, version.id)
}

async fn seed_succeeded_run(
    state: &std::sync::Arc<crate::web::AppState>,
    agent_key: &str,
    definition_id: uuid::Uuid,
    version_id: uuid::Uuid,
) {
    seed_succeeded_run_with_visual_data(
        state,
        agent_key,
        definition_id,
        version_id,
        json!({"version": 1, "markers": []}),
    )
    .await;
}

async fn seed_succeeded_run_with_visual_data(
    state: &std::sync::Arc<crate::web::AppState>,
    agent_key: &str,
    definition_id: uuid::Uuid,
    version_id: uuid::Uuid,
    visual_data: serde_json::Value,
) -> (uuid::Uuid, Vec<Candle>) {
    enqueue_run(
        &state.db_pool,
        agent_key,
        definition_id,
        version_id,
        "BTC",
        "1h",
        Utc::now(),
    )
    .await
    .expect("enqueue run");
    let run = claim_next_queued_run(&state.db_pool, &[])
        .await
        .expect("claim run")
        .expect("queued run");
    let candles = [0_i64, 1, 2, 3]
        .into_iter()
        .map(|hour| Candle {
            opened_at: Utc
                .timestamp_opt(1_700_000_000 + hour * 3_600, 0)
                .single()
                .expect("deterministic candle timestamp"),
            open: Decimal::from(10),
            high: Decimal::from(12),
            low: Decimal::from(9),
            close: Decimal::from(11),
            volume: Decimal::ONE,
        })
        .collect::<Vec<_>>();
    finish_run_succeeded(
        &state.db_pool,
        run.id,
        run.claim_token.expect("claimed run token"),
        PersistedIndicatorOutput {
            candle_data: json!(&candles),
            plot_data: json!({"EMA": [10.5, null, 12.5, null]}),
            visual_data,
            latest_values: json!({"EMA": 10.5}),
            diagnostics: json!([]),
        },
    )
    .await
    .expect("finish run");
    (run.id, candles)
}

#[tokio::test]
async fn indicators_tab_renders_the_indicator_list() {
    let state = test_state().await;
    let (agent_key, _) = insert_test_agent(&state).await.expect("insert agent");

    let response = router(state)
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/indicators"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_text(response).await;
    assert!(body.contains("Indicators"));
    assert!(body.contains("New indicator"));
    assert!(body.contains(&format!("/agents/{agent_key}/indicators/new")));
    assert!(!body.contains(&format!("/agents/{agent_key}/indicators/chart")));
    assert!(!body.contains("Pine source"));
}

#[tokio::test]
async fn new_indicator_page_renders_the_editor_and_target_selector() {
    let state = test_state().await;
    let (agent_key, _) = insert_test_agent(&state).await.expect("insert agent");

    let response = router(state)
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/indicators/new"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_text(response).await;
    assert!(body.contains("Pine source"));
    assert!(body.contains("Select indicator instruments"));
    assert!(body.contains("data-agent-instrument-selector=\"indicator\""));
    assert!(body.contains("data-indicator-timeframe-add"));
    assert!(body.contains("The same input values apply independently"));
}

#[tokio::test]
async fn indicator_detail_renders_a_chart_for_each_target() {
    let state = test_state().await;
    let (agent_key, definition_id, version_id) = seed_indicator(&state).await;
    seed_succeeded_run(&state, &agent_key, definition_id, version_id).await;

    let response = router(state)
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/indicators/{definition_id}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_text(response).await;
    assert!(body.contains("Test EMA"));
    assert!(body.contains("Status:"));
    assert!(body.contains(&format!("data-indicator-id=\"{definition_id}\"")));
    assert!(body.contains("data-instrument-id=\"BTC\""));
    assert!(body.contains("data-indicator-timeframe-selector"));
    assert!(body.contains("data-timeframe=\"1h\""));
    assert!(!body.contains("Pine source"));
}

#[tokio::test]
async fn chart_data_requires_a_current_indicator_target() {
    let state = test_state().await;
    let (agent_key, definition_id, _) = seed_indicator(&state).await;

    let response = router(state)
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/indicators/chart-data?indicator_id={definition_id}&instrument_id=ETH&timeframe=1h"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn chart_data_clamps_bars_and_omits_null_plot_points() {
    let state = test_state().await;
    let (agent_key, definition_id, version_id) = seed_indicator(&state).await;
    seed_succeeded_run(&state, &agent_key, definition_id, version_id).await;

    let response = router(state)
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/indicators/chart-data?indicator_id={definition_id}&instrument_id=BTC&timeframe=1h&bars=0"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&response_text(response).await).expect("JSON");
    assert_eq!(body["indicator"]["overlay"], true);
    assert_eq!(body["run"]["version"], 1);
    assert_eq!(body["candles"].as_array().expect("candles").len(), 1);
    assert!(
        body["plots"][0]["values"]
            .as_array()
            .expect("plot values")
            .is_empty()
    );
    assert_eq!(body["markers"], json!([]));
}

#[tokio::test]
async fn chart_data_resolves_marker_offsets_prices_directions_and_window() {
    let state = test_state().await;
    let (agent_key, definition_id, version_id) = seed_indicator(&state).await;
    let (_, candles) = seed_succeeded_run_with_visual_data(
        &state,
        &agent_key,
        definition_id,
        version_id,
        json!({
            "version": 1,
            "markers": [
                {
                    "kind": "plotshape", "bar_index": 0, "value": 1.0,
                    "title": "Buy", "text": "BUY", "style": "triangleup",
                    "location": "belowbar", "color": {"red": 0, "green": 128, "blue": 0, "transparency": 0},
                    "text_color": null, "size": "large", "offset": 1
                },
                {
                    "kind": "plotchar", "bar_index": 3, "value": 0.0,
                    "title": "Stage", "character": "2", "text": "",
                    "location": "absolute", "color": null, "text_color": null,
                    "size": "small", "offset": -1
                },
                {
                    "kind": "plotarrow", "bar_index": 1, "value": 2.5,
                    "title": "Up", "color_up": {"red": 0, "green": 128, "blue": 0, "transparency": 0},
                    "color_down": null, "min_height": 5.0, "max_height": 100.0, "offset": 0
                },
                {
                    "kind": "plotarrow", "bar_index": 2, "value": -3.0,
                    "title": "Down", "color_up": null,
                    "color_down": {"red": 255, "green": 0, "blue": 0, "transparency": 10},
                    "min_height": 5.0, "max_height": 100.0, "offset": 0
                },
                {
                    "kind": "plotshape", "bar_index": 0, "value": 1.0,
                    "title": "Outside history", "text": "", "style": "circle",
                    "location": "abovebar", "color": null, "text_color": null,
                    "size": "normal", "offset": -1
                },
                {
                    "kind": "plotchar", "bar_index": 0, "value": 1.0,
                    "title": "Outside window", "character": "1", "text": "",
                    "location": "belowbar", "color": null, "text_color": null,
                    "size": "normal", "offset": 0
                }
            ]
        }),
    )
    .await;

    let response = router(state)
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/indicators/chart-data?indicator_id={definition_id}&instrument_id=BTC&timeframe=1h&bars=3"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&response_text(response).await).expect("JSON");
    let markers = body["markers"].as_array().expect("markers");
    assert_eq!(markers.len(), 4);
    assert_eq!(markers[0]["kind"], "plotshape");
    assert_eq!(markers[0]["time"], candles[1].opened_at.timestamp());
    assert_eq!(markers[1]["kind"], "plotarrow");
    assert_eq!(markers[1]["direction"], "up");
    assert_eq!(markers[1]["color"]["green"], 128);
    assert_eq!(markers[2]["kind"], "plotchar");
    assert_eq!(markers[2]["time"], candles[2].opened_at.timestamp());
    assert_eq!(markers[2]["price"], 0.0);
    assert_eq!(markers[3]["kind"], "plotarrow");
    assert_eq!(markers[3]["direction"], "down");
    assert_eq!(markers[3]["color"]["red"], 255);
    assert_eq!(body["candles"].as_array().expect("candles").len(), 3);
}

#[tokio::test]
async fn chart_data_accepts_historical_null_visual_data() {
    let state = test_state().await;
    let (agent_key, definition_id, version_id) = seed_indicator(&state).await;
    let (run_id, _) = seed_succeeded_run_with_visual_data(
        &state,
        &agent_key,
        definition_id,
        version_id,
        json!({"version": 1, "markers": []}),
    )
    .await;
    sqlx::query("UPDATE agent_indicator_runs SET visual_data = NULL WHERE id = $1")
        .bind(run_id)
        .execute(&state.db_pool)
        .await
        .expect("clear visual data");

    let response = router(state)
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/indicators/chart-data?indicator_id={definition_id}&instrument_id=BTC&timeframe=1h"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&response_text(response).await).expect("JSON");
    assert_eq!(body["markers"], json!([]));
}

#[tokio::test]
async fn chart_data_rejects_unsupported_visual_data_versions() {
    let state = test_state().await;
    let (agent_key, definition_id, version_id) = seed_indicator(&state).await;
    seed_succeeded_run_with_visual_data(
        &state,
        &agent_key,
        definition_id,
        version_id,
        json!({"version": 2, "markers": []}),
    )
    .await;

    let response = router(state)
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/indicators/chart-data?indicator_id={definition_id}&instrument_id=BTC&timeframe=1h"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn create_indicator_redirects_with_form_validation_errors() {
    let state = test_state().await;
    let (agent_key, _) = insert_test_agent(&state).await.expect("insert agent");

    let response = router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/indicators"))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(
                    "name=&timeframe=1h&source=indicator%28%22Test%22%29",
                ))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert!(
        response
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .expect("location")
            .contains("indicator name must not be blank")
    );
}

#[tokio::test]
async fn update_indicator_redirects_when_the_active_version_is_stale() {
    let state = test_state().await;
    let (agent_key, definition_id, version_id) = seed_indicator(&state).await;
    let validated = validate_indicator_source(SOURCE, &json!({})).expect("validate source");
    let result = update_definition_with_new_version(
        &state.db_pool,
        &agent_key,
        definition_id,
        &UpdateIndicatorDefinition {
            expected_active_version_id: version_id,
            name: "Test EMA".to_string(),
            description: String::new(),
            timeframes: vec!["1h".to_string()],
            enabled: true,
            instrument_ids: vec!["BTC".to_string()],
            version: NewIndicatorVersion {
                source: SOURCE.to_string(),
                compiler_version: "test".to_string(),
                metadata: json!({"indicator": validated.metadata, "inputs": validated.inputs}),
                input_values: json!({}),
                created_by_kind: "operator".to_string(),
                created_by_run_id: None,
                created_by_conversation_id: None,
            },
        },
    )
    .await
    .expect("advance version");
    assert_eq!(result, IndicatorUpdateResult::Updated);

    let response = router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/indicators/{definition_id}"))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(format!(
                    "expected_version_id={version_id}&name=Test+EMA&timeframe=1h&instrument_id=BTC&source=%2F%2F%40version%3D5%0Aindicator%28%22Test+EMA%22%2Coverlay%3Dtrue%29%0Aplot%28ta.ema%28close%2C2%29%2C%22EMA%22%29"
                )))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert!(
        response
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .expect("location")
            .contains("Indicator+changed")
    );
}
