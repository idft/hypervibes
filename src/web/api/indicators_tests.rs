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
    create_named_indicator(state, api_key, "Test EMA").await
}

async fn create_named_indicator(
    state: &Arc<crate::web::AppState>,
    api_key: &str,
    name: &str,
) -> serde_json::Value {
    let (headers, body) = json_body(&json!({
        "name": name,
        "description": "test indicator",
        "timeframes": ["1h"],
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
    assert_eq!(created["timeframes"], json!(["1h"]));

    let (status, foreign) =
        get_json_response(&state, &second_key, &format!("/indicators/{id}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(foreign["error"], "indicator not found");

    let (status, results) = get_json_response(
        &state,
        &first_key,
        &format!("/indicators/{id}/results?timeframe=1h&limit=101"),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(results["error"], "limit must be between 1 and 100");
}

#[tokio::test]
async fn indicator_create_normalizes_timeframes_and_enqueues_the_target_cross_product() {
    let state = test_state().await;
    let (_agent_key, api_key) = seed_agent(&state, "indicators-multi-timeframe").await;
    let (headers, body) = json_body(&json!({
        "name": "Multi timeframe",
        "timeframes": [" 4h ", "15m", "1h"],
        "instrument_ids": ["BTC"],
        "source": SOURCE,
    }));
    let mut request = Request::builder()
        .method("POST")
        .uri("/indicators")
        .header("authorization", format!("Bearer {api_key}"));
    if let Some((name, value)) = headers {
        request = request.header(name, value);
    }
    let response = app(Arc::clone(&state))
        .oneshot(request.body(body).expect("request"))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let created = response_json(response).await;
    assert_eq!(created["timeframes"], json!(["15m", "1h", "4h"]));
    let definition_id = uuid::Uuid::parse_str(created["id"].as_str().expect("definition id"))
        .expect("definition UUID");
    let queued: Vec<(String, String)> = sqlx::query_as(
        "SELECT instrument_id, timeframe FROM agent_indicator_runs WHERE indicator_definition_id = $1 ORDER BY timeframe",
    )
    .bind(definition_id)
    .fetch_all(&state.db_pool)
    .await
    .expect("load immediate runs");
    assert_eq!(queued.len(), 3);
    assert_eq!(
        queued.into_iter().map(|row| row.1).collect::<Vec<_>>(),
        ["15m", "1h", "4h"]
    );
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
        "timeframes": ["1h"],
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
    let (_agent_key, api_key) = seed_agent(&state, "indicator-visual-results").await;
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
            run.id,
            run.claim_token.expect("claimed run token"),
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

    let (status, results) = get_json_response(
        &state,
        &api_key,
        &format!("/indicators/{id}/results?timeframe=1h"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(results[0]["visual_data"], visual_data);
    assert_eq!(results[0]["visual_data"]["markers"][0]["kind"], "plotshape");
    assert_eq!(results[0]["visual_data"]["markers"][1]["character"], "1");
    assert_eq!(results[0]["visual_data"]["markers"][2]["min_height"], 5.0);
    assert!(results[0].get("claim_token").is_none());
    assert!(results[0].get("lease_expires_at").is_none());
    assert!(results[0].get("next_attempt_at").is_none());

    let (status, exact) = get_json_response(
        &state,
        &api_key,
        &format!("/indicators/{id}/results?timeframe=1h&run_id={}", run.id),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(exact.as_array().expect("exact result array").len(), 1);
    assert_eq!(exact[0]["id"], run.id.to_string());
}

#[tokio::test]
async fn indicators_reject_the_removed_singular_timeframe_contract() {
    let state = test_state().await;
    let (_agent_key, api_key) = seed_agent(&state, "indicators-singular-timeframe").await;
    let (headers, body) = json_body(&json!({
        "name": "Old contract",
        "timeframe": "1h",
        "instrument_ids": ["BTC"],
        "source": SOURCE,
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
}

#[tokio::test]
async fn indicator_result_instrument_filter_and_limit_apply_in_sql() {
    let state = test_state().await;
    let (_agent_key, api_key) = seed_agent(&state, "indicator-sql-filter").await;
    seed_instrument(&state, "ETH", true).await;
    let created = create_indicator(&state, &api_key).await;
    let definition_id = uuid::Uuid::parse_str(created["id"].as_str().expect("definition id"))
        .expect("definition UUID");
    let version_id = uuid::Uuid::parse_str(
        created["active_version"]["id"]
            .as_str()
            .expect("version id"),
    )
    .expect("version UUID");
    let eth_run_id: uuid::Uuid = sqlx::query_scalar(
        "UPDATE agent_indicator_runs SET instrument_id = 'ETH', scheduled_for = now() - interval '1 day' WHERE indicator_definition_id = $1 RETURNING id",
    )
    .bind(definition_id)
    .fetch_one(&state.db_pool)
    .await
    .expect("make historical ETH run");
    for offset in 1..=100_i64 {
        sqlx::query(
            "INSERT INTO agent_indicator_runs (id, agent_key, indicator_definition_id, indicator_version_id, instrument_id, timeframe, scheduled_for, status) SELECT $1, agent_key, $2, $3, 'BTC', '1h', now() + make_interval(secs => $4), 'queued' FROM agent_indicator_definitions WHERE id = $2",
        )
        .bind(uuid::Uuid::new_v4())
        .bind(definition_id)
        .bind(version_id)
        .bind(offset as f64)
        .execute(&state.db_pool)
        .await
        .expect("insert newer BTC run");
    }

    let (status, results) = get_json_response(
        &state,
        &api_key,
        &format!("/indicators/{definition_id}/results?timeframe=1h&instrument_id=ETH&limit=1"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(results.as_array().expect("results").len(), 1);
    assert_eq!(results[0]["id"], eth_run_id.to_string());
}

#[tokio::test]
async fn analysis_run_credential_reads_only_its_frozen_exact_dependencies() {
    let state = test_state().await;
    let (agent_key, api_key) = seed_agent(&state, "indicator-run-credential").await;
    let visible = create_named_indicator(&state, &api_key, "Visible dependency").await;
    let timed_out = create_named_indicator(&state, &api_key, "Timed out dependency").await;
    let visible_definition =
        uuid::Uuid::parse_str(visible["id"].as_str().expect("visible id")).expect("visible UUID");
    let timed_out_definition =
        uuid::Uuid::parse_str(timed_out["id"].as_str().expect("timed out id"))
            .expect("timed out UUID");
    let analysis_sub_agent_id: i64 = sqlx::query_scalar(
        "SELECT id FROM harness_sub_agents WHERE agent_key = $1 AND sub_agent_kind = 'analysis' ORDER BY id LIMIT 1",
    )
    .bind(&agent_key)
    .fetch_one(&state.db_pool)
    .await
    .expect("load Analysis sub-agent");
    let boundary =
        crate::harness::timeframe::canonical_boundary_at_or_before(chrono::Utc::now(), "1h")
            .expect("canonical boundary");
    let credential_run_id = crate::harness::store::insert_test_run(
        &state.db_pool,
        analysis_sub_agent_id,
        crate::harness::model::RUN_STATUS_RUNNING,
    )
    .await
    .expect("insert credential run");
    crate::harness::store::prepare_indicator_dependencies(
        &state.db_pool,
        credential_run_id,
        &agent_key,
        &["BTC".to_string()],
        boundary,
        std::time::Duration::from_secs(30),
    )
    .await
    .expect("prepare frozen dependencies");
    let dependencies: Vec<(uuid::Uuid, uuid::Uuid)> = sqlx::query_as(
        "SELECT runs.indicator_definition_id, runs.id FROM harness_run_indicator_dependencies dependencies JOIN agent_indicator_runs runs ON runs.id = dependencies.indicator_run_id WHERE dependencies.analysis_run_id = $1",
    )
    .bind(credential_run_id)
    .fetch_all(&state.db_pool)
    .await
    .expect("load dependency runs");
    let visible_run_id = dependencies
        .iter()
        .find(|(definition_id, _)| *definition_id == visible_definition)
        .map(|(_, run_id)| *run_id)
        .expect("visible dependency run");
    let timed_out_run_id = dependencies
        .iter()
        .find(|(definition_id, _)| *definition_id == timed_out_definition)
        .map(|(_, run_id)| *run_id)
        .expect("timed out dependency run");
    sqlx::query("UPDATE agent_indicator_runs SET status = 'succeeded', candle_data = '[]'::jsonb, plot_data = '{}'::jsonb, visual_data = '{\"version\":1,\"markers\":[]}'::jsonb, latest_values = '{}'::jsonb, finished_at = now() WHERE id = $1")
        .bind(visible_run_id)
        .execute(&state.db_pool)
        .await
        .expect("complete visible dependency");
    crate::harness::store::freeze_indicator_dependencies(&state.db_pool, credential_run_id)
        .await
        .expect("freeze credential dependency set");
    sqlx::query(
        "UPDATE agent_indicator_runs SET status = 'succeeded', finished_at = now() WHERE id = $1",
    )
    .bind(timed_out_run_id)
    .execute(&state.db_pool)
    .await
    .expect("complete timed out dependency late");

    let other_run_id = crate::harness::store::insert_test_run(
        &state.db_pool,
        analysis_sub_agent_id,
        crate::harness::model::RUN_STATUS_RUNNING,
    )
    .await
    .expect("insert other Analysis run");
    crate::harness::store::prepare_indicator_dependencies(
        &state.db_pool,
        other_run_id,
        &agent_key,
        &["BTC".to_string()],
        boundary - chrono::Duration::hours(1),
        std::time::Duration::from_secs(30),
    )
    .await
    .expect("prepare other dependency set");
    let other_exact_run_id: uuid::Uuid = sqlx::query_scalar(
        "SELECT runs.id FROM harness_run_indicator_dependencies dependencies JOIN agent_indicator_runs runs ON runs.id = dependencies.indicator_run_id WHERE dependencies.analysis_run_id = $1 AND runs.indicator_definition_id = $2",
    )
    .bind(other_run_id)
    .bind(visible_definition)
    .fetch_one(&state.db_pool)
    .await
    .expect("load other exact run");
    sqlx::query("UPDATE agent_indicator_runs SET status = 'succeeded', finished_at = now() WHERE id IN (SELECT indicator_run_id FROM harness_run_indicator_dependencies WHERE analysis_run_id = $1)")
        .bind(other_run_id)
        .execute(&state.db_pool)
        .await
        .expect("complete other dependencies");
    crate::harness::store::freeze_indicator_dependencies(&state.db_pool, other_run_id)
        .await
        .expect("freeze other dependency set");

    sqlx::query("INSERT INTO harness_run_workspace_artifacts (run_id, context_schema_version, context_snapshot, capability_schema_version, capability_snapshot, workspace_status, workspace_created_at) VALUES ($1, $2, '{}'::jsonb, $3, '[]'::jsonb, 'ready', now())")
        .bind(credential_run_id)
        .bind(crate::harness::model::RUN_CONTEXT_SNAPSHOT_SCHEMA_VERSION)
        .bind(crate::harness::model::CAPABILITY_SCHEMA_VERSION)
        .execute(&state.db_pool)
        .await
        .expect("insert ready run artifact");
    let credential = crate::harness::store::issue_run_runtime_credential(
        &state.db_pool,
        &agent_key,
        credential_run_id,
    )
    .await
    .expect("issue Analysis credential");

    let (status, exact) = get_json_response(
        &state,
        &credential.token,
        &format!("/indicators/{visible_definition}/results?timeframe=1h"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(exact.as_array().expect("exact dependencies").len(), 1);
    assert_eq!(exact[0]["id"], visible_run_id.to_string());
    for internal in ["claim_token", "lease_expires_at", "next_attempt_at"] {
        assert!(exact[0].get(internal).is_none());
    }

    let (status, hidden_late) = get_json_response(
        &state,
        &credential.token,
        &format!("/indicators/{timed_out_definition}/results?timeframe=1h"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(hidden_late.as_array().expect("late results").is_empty());

    let (status, cross_run) = get_json_response(
        &state,
        &credential.token,
        &format!(
            "/indicators/{visible_definition}/results?timeframe=1h&run_id={other_exact_run_id}"
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(cross_run.as_array().expect("cross-run results").is_empty());
}
