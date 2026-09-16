use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
};
use serde_json::{Value, json};
use uuid::Uuid;

use super::show::{AgentIndicatorsQuery, AgentShowQueries, render_agent_show_page};
use crate::{
    agents::{model::AgentDetailRow, store::list_agent_analysis_instrument_options},
    harness::timeframe::parse_timeframe_seconds,
    indicators::{
        runtime::validate_indicator_source,
        store::{
            CreateIndicatorDefinition, IndicatorUpdateResult, NewIndicatorVersion,
            UpdateIndicatorDefinition, create_definition_with_initial_version, delete_definition,
            get_active_version, get_chart_run, get_definition, list_definition_instruments,
            list_definitions, list_latest_results, update_definition_with_new_version,
        },
    },
    web::{
        AppState,
        auth::AuthenticatedUser,
        error::AppError,
        templates::{
            AgentShowTab, AgentsShowPageTemplate, IndicatorDefinitionView, IndicatorFormView,
        },
    },
};

const COMPILER_VERSION: &str = "pine-lang 0.2.6";
const MAX_CHART_BARS: usize = 500;

pub(in crate::web::routes) async fn agents_show_indicators(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path(agent_key): Path<String>,
    Query(query): Query<AgentIndicatorsQuery>,
) -> Result<Response, AppError> {
    render_agent_show_page(
        &state,
        &user,
        &agent_key,
        AgentShowTab::Indicators,
        AgentShowQueries {
            indicators: Some(query),
            ..Default::default()
        },
    )
    .await
}

pub(in crate::web::routes) async fn populate_indicators_tab(
    state: &Arc<AppState>,
    agent: &AgentDetailRow,
    template: &mut AgentsShowPageTemplate,
    query: AgentIndicatorsQuery,
) -> Result<(), AppError> {
    template.analysis_instrument_options =
        list_agent_analysis_instrument_options(&state.db_pool, &agent.agent_key).await?;
    template.analysis_instrument_options_loaded = true;
    template.indicator_errors = query.error.into_iter().collect();
    for definition in list_definitions(&state.db_pool, &agent.agent_key).await? {
        let version = get_active_version(&state.db_pool, &agent.agent_key, definition.id)
            .await?
            .ok_or_else(|| AppError(anyhow::anyhow!("indicator has no active version")))?;
        let latest = list_latest_results(&state.db_pool, &agent.agent_key, definition.id, 1)
            .await?
            .into_iter()
            .next();
        template.indicators.push(IndicatorDefinitionView {
            id: definition.id,
            name: definition.name,
            description: definition.description,
            timeframe: definition.timeframe,
            enabled: definition.enabled,
            version_number: version.version_number,
            created_by_kind: version.created_by_kind,
            instrument_ids: list_definition_instruments(
                &state.db_pool,
                &agent.agent_key,
                definition.id,
            )
            .await?,
            latest_status: latest
                .as_ref()
                .map_or_else(|| "Not run".to_string(), |run| run.status.clone()),
            latest_values: latest
                .as_ref()
                .and_then(|run| run.latest_values.as_ref())
                .map_or_else(String::new, |values| values.to_string()),
            latest_error: latest.and_then(|run| run.error_summary),
        });
    }
    if let Some(id) = query.edit {
        let definition = get_definition(&state.db_pool, &agent.agent_key, id)
            .await?
            .ok_or_else(|| AppError(anyhow::anyhow!("indicator not found")))?;
        let version = get_active_version(&state.db_pool, &agent.agent_key, id)
            .await?
            .ok_or_else(|| AppError(anyhow::anyhow!("indicator active version not found")))?;
        template.indicator_form = IndicatorFormView {
            id: Some(id),
            expected_version_id: definition.active_version_id,
            name: definition.name,
            description: definition.description,
            timeframe: definition.timeframe,
            source: version.source,
            input_values: version.input_values.to_string(),
            enabled: definition.enabled,
            selected_instrument_ids: list_definition_instruments(
                &state.db_pool,
                &agent.agent_key,
                id,
            )
            .await?,
        };
    } else {
        template.indicator_form.enabled = true;
        template.indicator_form.input_values = "{}".to_string();
    }
    Ok(())
}

fn form_values(form: Vec<(String, String)>) -> Result<(IndicatorFormView, Value), String> {
    let mut view = IndicatorFormView {
        enabled: false,
        input_values: "{}".to_string(),
        ..Default::default()
    };
    for (key, value) in form {
        match key.as_str() {
            "name" => view.name = value,
            "description" => view.description = value,
            "timeframe" => view.timeframe = value,
            "source" => view.source = value,
            "input_values" => view.input_values = value,
            "instrument_id" => view.selected_instrument_ids.push(value),
            "enabled" => view.enabled = true,
            _ => {}
        }
    }
    if view.name.trim().is_empty() {
        return Err("indicator name must not be blank".to_string());
    }
    parse_timeframe_seconds(&view.timeframe).map_err(|error| error.to_string())?;
    let inputs = serde_json::from_str(&view.input_values)
        .map_err(|_| "input values must be valid JSON".to_string())?;
    Ok((view, inputs))
}

fn version(source: String, input_values: Value) -> Result<NewIndicatorVersion, String> {
    let validated =
        validate_indicator_source(&source, &input_values).map_err(|error| error.to_string())?;
    Ok(NewIndicatorVersion {
        source,
        compiler_version: COMPILER_VERSION.to_string(),
        metadata: json!({"indicator": validated.metadata, "inputs": validated.inputs}),
        input_values,
        created_by_kind: "operator".to_string(),
        created_by_run_id: None,
        created_by_conversation_id: None,
    })
}

pub(in crate::web::routes) async fn agents_create_indicator(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
    axum::Form(form): axum::Form<Vec<(String, String)>>,
) -> Result<Response, AppError> {
    let (form, inputs) = match form_values(form) {
        Ok(value) => value,
        Err(error) => {
            return Ok(
                Redirect::to(&format!("/agents/{agent_key}/indicators?error={error}"))
                    .into_response(),
            );
        }
    };
    let result = create_definition_with_initial_version(
        &state.db_pool,
        &agent_key,
        &CreateIndicatorDefinition {
            name: form.name,
            description: form.description,
            timeframe: form.timeframe,
            enabled: form.enabled,
            instrument_ids: form.selected_instrument_ids,
            version: match version(form.source, inputs) {
                Ok(value) => value,
                Err(error) => {
                    return Ok(Redirect::to(&format!(
                        "/agents/{agent_key}/indicators?error={error}"
                    ))
                    .into_response());
                }
            },
        },
    )
    .await;
    match result {
        Ok(_) => Ok(Redirect::to(&format!("/agents/{agent_key}/indicators")).into_response()),
        Err(error) => Ok(
            Redirect::to(&format!("/agents/{agent_key}/indicators?error={error}")).into_response(),
        ),
    }
}

pub(in crate::web::routes) async fn agents_update_indicator(
    State(state): State<Arc<AppState>>,
    Path((agent_key, definition_id)): Path<(String, Uuid)>,
    axum::Form(mut form): axum::Form<Vec<(String, String)>>,
) -> Result<Response, AppError> {
    let expected = form
        .iter()
        .find(|(key, _)| key == "expected_version_id")
        .and_then(|(_, value)| Uuid::parse_str(value).ok());
    form.retain(|(key, _)| key != "expected_version_id");
    let (form, inputs) = match form_values(form) {
        Ok(value) => value,
        Err(error) => {
            return Ok(Redirect::to(&format!(
                "/agents/{agent_key}/indicators?edit={definition_id}&error={error}"
            ))
            .into_response());
        }
    };
    let Some(expected_active_version_id) = expected else {
        return Ok(Redirect::to(&format!(
            "/agents/{agent_key}/indicators?edit={definition_id}&error=Missing+active+version"
        ))
        .into_response());
    };
    let update = UpdateIndicatorDefinition {
        expected_active_version_id,
        name: form.name,
        description: form.description,
        timeframe: form.timeframe,
        enabled: form.enabled,
        instrument_ids: form.selected_instrument_ids,
        version: match version(form.source, inputs) {
            Ok(value) => value,
            Err(error) => {
                return Ok(Redirect::to(&format!(
                    "/agents/{agent_key}/indicators?edit={definition_id}&error={error}"
                ))
                .into_response());
            }
        },
    };
    match update_definition_with_new_version(&state.db_pool, &agent_key, definition_id, &update).await? { IndicatorUpdateResult::Updated => Ok(Redirect::to(&format!("/agents/{agent_key}/indicators")).into_response()), IndicatorUpdateResult::NotFound => Ok((StatusCode::NOT_FOUND, "indicator not found").into_response()), IndicatorUpdateResult::VersionConflict => Ok(Redirect::to(&format!("/agents/{agent_key}/indicators?edit={definition_id}&error=Indicator+changed%3B+reload+before+saving")).into_response()) }
}

pub(in crate::web::routes) async fn agents_delete_indicator(
    State(state): State<Arc<AppState>>,
    Path((agent_key, definition_id)): Path<(String, Uuid)>,
) -> Result<Response, AppError> {
    if !delete_definition(&state.db_pool, &agent_key, definition_id).await? {
        return Ok((StatusCode::NOT_FOUND, "indicator not found").into_response());
    }
    Ok(Redirect::to(&format!("/agents/{agent_key}/indicators")).into_response())
}

#[derive(serde::Deserialize)]
pub(in crate::web::routes) struct ChartQuery {
    indicator_id: Uuid,
    instrument_id: String,
    bars: Option<usize>,
}

pub(in crate::web::routes) async fn agents_indicator_chart_data(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
    Query(query): Query<ChartQuery>,
) -> Result<Response, AppError> {
    if !list_definition_instruments(&state.db_pool, &agent_key, query.indicator_id)
        .await?
        .contains(&query.instrument_id)
    {
        return Ok((StatusCode::NOT_FOUND, "indicator target not found").into_response());
    }
    let Some(run) = get_chart_run(
        &state.db_pool,
        &agent_key,
        query.indicator_id,
        &query.instrument_id,
    )
    .await?
    else {
        return Ok((StatusCode::NOT_FOUND, "no successful indicator run").into_response());
    };
    let bars = query
        .bars
        .unwrap_or(MAX_CHART_BARS)
        .clamp(1, MAX_CHART_BARS);
    let candles = run
        .candle_data
        .unwrap_or_else(|| json!([]))
        .as_array()
        .cloned()
        .unwrap_or_default();
    let candles = candles.into_iter().rev().take(bars).collect::<Vec<_>>().into_iter().rev().map(|candle| json!({"time": candle["opened_at"].as_str().and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok()).map(|value| value.timestamp()).unwrap_or_default(), "open": candle["open"], "high": candle["high"], "low": candle["low"], "close": candle["close"]})).collect::<Vec<_>>();
    Ok(axum::Json(json!({"run": {"id": run.id, "version_id": run.indicator_version_id, "scheduled_for": run.scheduled_for}, "candles": candles, "plots": run.plot_data.unwrap_or_else(|| json!({}))})).into_response())
}
