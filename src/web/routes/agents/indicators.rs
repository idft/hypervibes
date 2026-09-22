use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
};
use rust_decimal::prelude::ToPrimitive;
use serde_json::{Value, json};
use uuid::Uuid;

use super::show::{AgentIndicatorsQuery, AgentShowQueries, render_agent_show_page};
use crate::{
    agents::{model::AgentDetailRow, store::list_agent_analysis_instrument_options},
    harness::timeframe::parse_timeframe_seconds,
    indicators::{
        model::{Candle, INDICATOR_VISUAL_DATA_VERSION, IndicatorMarker, IndicatorVisualData},
        runtime::{IndicatorInputMetadata, validate_indicator_source},
        store::{
            CreateIndicatorDefinition, IndicatorUpdateResult, NewIndicatorVersion,
            UpdateIndicatorDefinition, create_definition_with_initial_version, delete_definition,
            get_active_version, get_chart_run, get_definition, get_version,
            list_definition_instruments, list_definitions, list_latest_results,
            update_definition_with_new_version,
        },
    },
    web::{
        AppState,
        auth::AuthenticatedUser,
        error::AppError,
        templates::{
            AgentShowTab, AgentsShowPageTemplate, IndicatorDefinitionView, IndicatorFormView,
            IndicatorInputView, IndicatorInstrumentRunView, IndicatorPage,
        },
    },
};

const COMPILER_VERSION: &str = "pine-lang 0.2.6";
const MAX_CHART_BARS: usize = 500;

fn decode_visual_data(value: Option<&Value>) -> Result<IndicatorVisualData, AppError> {
    let visual_data = match value {
        None | Some(Value::Null) => IndicatorVisualData {
            version: INDICATOR_VISUAL_DATA_VERSION,
            markers: Vec::new(),
        },
        Some(value) => serde_json::from_value(value.clone()).map_err(|error| {
            AppError(anyhow::anyhow!(
                "indicator run has invalid visual data: {error}"
            ))
        })?,
    };
    if visual_data.version != INDICATOR_VISUAL_DATA_VERSION {
        return Err(AppError(anyhow::anyhow!(
            "indicator run has unsupported visual data version {}",
            visual_data.version
        )));
    }
    Ok(visual_data)
}

fn marker_target_index(bar_index: usize, offset: i64) -> Option<usize> {
    if offset >= 0 {
        usize::try_from(offset)
            .ok()
            .and_then(|offset| bar_index.checked_add(offset))
    } else {
        usize::try_from(offset.unsigned_abs())
            .ok()
            .and_then(|offset| bar_index.checked_sub(offset))
    }
}

fn resolve_markers(
    visual_data: IndicatorVisualData,
    candles: &[Candle],
    start_index: usize,
) -> Result<Vec<Value>, AppError> {
    let mut markers = Vec::new();
    for marker in visual_data.markers {
        let (bar_index, offset) = match &marker {
            IndicatorMarker::Plotshape {
                bar_index, offset, ..
            }
            | IndicatorMarker::Plotchar {
                bar_index, offset, ..
            }
            | IndicatorMarker::Plotarrow {
                bar_index, offset, ..
            } => (*bar_index, *offset),
        };
        if bar_index >= candles.len() {
            return Err(AppError(anyhow::anyhow!(
                "indicator marker references an invalid source bar"
            )));
        }
        let Some(target_index) = marker_target_index(bar_index, offset) else {
            continue;
        };
        if target_index >= candles.len() || target_index < start_index {
            continue;
        }
        let time = candles[target_index].opened_at.timestamp();
        let resolved = match marker {
            IndicatorMarker::Plotshape {
                value,
                title,
                text,
                style,
                location,
                color,
                text_color,
                size,
                ..
            } => {
                let price = (location == "absolute").then_some(value);
                json!({
                    "kind": "plotshape", "time": time, "title": title, "text": text,
                    "style": style, "location": location, "color": color,
                    "text_color": text_color, "size": size, "price": price,
                })
            }
            IndicatorMarker::Plotchar {
                value,
                title,
                character,
                text,
                location,
                color,
                text_color,
                size,
                ..
            } => {
                let price = (location == "absolute").then_some(value);
                json!({
                    "kind": "plotchar", "time": time, "title": title,
                    "character": character, "text": text, "location": location,
                    "color": color, "text_color": text_color, "size": size, "price": price,
                })
            }
            IndicatorMarker::Plotarrow {
                value,
                title,
                color_up,
                color_down,
                ..
            } => {
                let (direction, color) = if value > 0.0 {
                    ("up", color_up)
                } else {
                    ("down", color_down)
                };
                json!({
                    "kind": "plotarrow", "time": time, "title": title,
                    "direction": direction, "value": value, "color": color,
                })
            }
        };
        markers.push((time, resolved));
    }
    markers.sort_by_key(|(time, _)| *time);
    Ok(markers.into_iter().map(|(_, marker)| marker).collect())
}

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

pub(in crate::web::routes) async fn agents_new_indicator(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path(agent_key): Path<String>,
    Query(query): Query<AgentIndicatorsQuery>,
) -> Result<Response, AppError> {
    render_indicator_page(&state, &user, &agent_key, IndicatorPage::Editor, query).await
}

pub(in crate::web::routes) async fn agents_edit_indicator(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path((agent_key, indicator_id)): Path<(String, Uuid)>,
    Query(mut query): Query<AgentIndicatorsQuery>,
) -> Result<Response, AppError> {
    query.edit = Some(indicator_id);
    render_indicator_page(&state, &user, &agent_key, IndicatorPage::Editor, query).await
}

pub(in crate::web::routes) async fn agents_show_indicator(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path((agent_key, indicator_id)): Path<(String, Uuid)>,
) -> Result<Response, AppError> {
    let query = AgentIndicatorsQuery {
        detail: Some(indicator_id),
        ..Default::default()
    };
    render_indicator_page(&state, &user, &agent_key, IndicatorPage::Detail, query).await
}

async fn render_indicator_page(
    state: &Arc<AppState>,
    user: &AuthenticatedUser,
    agent_key: &str,
    page: IndicatorPage,
    mut query: AgentIndicatorsQuery,
) -> Result<Response, AppError> {
    query.page = page;
    render_agent_show_page(
        state,
        user,
        agent_key,
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
    template.indicator_page = query.page;
    template.indicator_errors = query.error.into_iter().collect();
    for definition in list_definitions(&state.db_pool, &agent.agent_key).await? {
        let version = get_active_version(&state.db_pool, &agent.agent_key, definition.id)
            .await?
            .ok_or_else(|| AppError(anyhow::anyhow!("indicator has no active version")))?;
        let instrument_ids =
            list_definition_instruments(&state.db_pool, &agent.agent_key, definition.id).await?;
        let latest_runs =
            list_latest_results(&state.db_pool, &agent.agent_key, definition.id, 100).await?;
        let instrument_runs = instrument_ids
            .iter()
            .map(|instrument_id| {
                let latest = latest_runs
                    .iter()
                    .find(|run| run.instrument_id == *instrument_id);
                IndicatorInstrumentRunView {
                    instrument_id: instrument_id.clone(),
                    status: latest.map_or_else(|| "Not run".to_string(), |run| run.status.clone()),
                    latest_values: latest
                        .and_then(|run| run.latest_values.as_ref())
                        .map_or_else(String::new, |values| values.to_string()),
                    latest_error: latest.and_then(|run| run.error_summary.clone()),
                }
            })
            .collect();
        template.indicators.push(IndicatorDefinitionView {
            id: definition.id,
            name: definition.name,
            description: definition.description,
            timeframe: definition.timeframe,
            enabled: definition.enabled,
            version_number: version.version_number,
            created_by_kind: version.created_by_kind,
            instrument_ids,
            instrument_runs,
        });
    }
    if let Some(id) = query.detail {
        template.indicator_detail = template
            .indicators
            .iter()
            .find(|indicator| indicator.id == id)
            .cloned();
        if template.indicator_detail.is_none() {
            return Err(AppError(anyhow::anyhow!("indicator not found")));
        }
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
            inputs: input_views(&version.metadata, &version.input_values)?,
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
        template.indicator_form.selected_instrument_ids = template
            .analysis_instrument_options
            .iter()
            .filter(|option| option.selected)
            .map(|option| option.instrument_id.clone())
            .collect();
    }
    Ok(())
}

fn input_views(metadata: &Value, values: &Value) -> Result<Vec<IndicatorInputView>, AppError> {
    let inputs = serde_json::from_value::<Vec<IndicatorInputMetadata>>(
        metadata.get("inputs").cloned().unwrap_or_else(|| json!([])),
    )?;
    let values = values
        .as_object()
        .ok_or_else(|| AppError(anyhow::anyhow!("indicator input values are not an object")))?;
    Ok(inputs
        .into_iter()
        .map(|input| {
            let value = values.get(&input.title).unwrap_or(&input.default);
            IndicatorInputView {
                title: input.title,
                group: input.group,
                kind: input.kind,
                value: value_to_form_string(value),
                checked: value.as_bool().unwrap_or(false),
                min_value: input
                    .min_value
                    .map_or_else(String::new, |value| value.to_string()),
                max_value: input
                    .max_value
                    .map_or_else(String::new, |value| value.to_string()),
                step: input
                    .step
                    .map_or_else(String::new, |value| value.to_string()),
            }
        })
        .collect())
}

fn value_to_form_string(value: &Value) -> String {
    value
        .as_str()
        .map_or_else(|| value.to_string(), ToString::to_string)
}

fn form_values(form: Vec<(String, String)>) -> Result<(IndicatorFormView, Value), String> {
    let mut view = IndicatorFormView {
        enabled: false,
        ..Default::default()
    };
    let mut inputs = serde_json::Map::new();
    for (key, value) in form {
        if let Some((kind, title)) = key
            .strip_prefix("indicator-input-")
            .and_then(|key| key.split_once('-'))
        {
            let value = match kind {
                "bool" => Value::Bool(value == "true"),
                "int" => value
                    .parse::<i64>()
                    .map(Value::from)
                    .map_err(|_| format!("input '{title}' must be an integer"))?,
                "float" => {
                    let value = value
                        .parse::<f64>()
                        .map_err(|_| format!("input '{title}' must be a number"))?;
                    if !value.is_finite() {
                        return Err(format!("input '{title}' must be a finite number"));
                    }
                    json!(value)
                }
                "string" | "source" => Value::String(value),
                _ => return Err("indicator input has an unsupported type".to_string()),
            };
            inputs.insert(title.to_string(), value);
            continue;
        }
        match key.as_str() {
            "name" => view.name = value,
            "description" => view.description = value,
            "timeframe" => view.timeframe = value,
            "source" => view.source = value,
            "instrument_id" => view.selected_instrument_ids.push(value),
            "enabled" => view.enabled = true,
            _ => {}
        }
    }
    if view.name.trim().is_empty() {
        return Err("indicator name must not be blank".to_string());
    }
    parse_timeframe_seconds(&view.timeframe).map_err(|error| error.to_string())?;
    Ok((view, Value::Object(inputs)))
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
                Redirect::to(&format!("/agents/{agent_key}/indicators/new?error={error}"))
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
                        "/agents/{agent_key}/indicators/new?error={error}"
                    ))
                    .into_response());
                }
            },
        },
    )
    .await;
    match result {
        Ok(definition) => Ok(Redirect::to(&format!(
            "/agents/{agent_key}/indicators/{}/edit",
            definition.id
        ))
        .into_response()),
        Err(error) => Ok(Redirect::to(&format!(
            "/agents/{agent_key}/indicators/new?error={error}"
        ))
        .into_response()),
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
                "/agents/{agent_key}/indicators/{definition_id}/edit?error={error}"
            ))
            .into_response());
        }
    };
    let Some(expected_active_version_id) = expected else {
        return Ok(Redirect::to(&format!(
            "/agents/{agent_key}/indicators/{definition_id}/edit?error=Missing+active+version"
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
                    "/agents/{agent_key}/indicators/{definition_id}/edit?error={error}"
                ))
                .into_response());
            }
        },
    };
    match update_definition_with_new_version(&state.db_pool, &agent_key, definition_id, &update).await? { IndicatorUpdateResult::Updated => Ok(Redirect::to(&format!("/agents/{agent_key}/indicators/{definition_id}/edit")).into_response()), IndicatorUpdateResult::NotFound => Ok((StatusCode::NOT_FOUND, "indicator not found").into_response()), IndicatorUpdateResult::VersionConflict => Ok(Redirect::to(&format!("/agents/{agent_key}/indicators/{definition_id}/edit?error=Indicator+changed%3B+reload+before+saving")).into_response()) }
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
    let Some(definition) = get_definition(&state.db_pool, &agent_key, query.indicator_id).await?
    else {
        return Ok((StatusCode::NOT_FOUND, "indicator not found").into_response());
    };
    let version = get_version(
        &state.db_pool,
        &agent_key,
        query.indicator_id,
        run.indicator_version_id,
    )
    .await?
    .ok_or_else(|| AppError(anyhow::anyhow!("indicator run version not found")))?;
    let overlay = version
        .metadata
        .get("indicator")
        .and_then(|metadata| metadata.get("overlay"))
        .and_then(Value::as_bool)
        .ok_or_else(|| AppError(anyhow::anyhow!("indicator version has invalid metadata")))?;
    let bars = query
        .bars
        .unwrap_or(MAX_CHART_BARS)
        .clamp(1, MAX_CHART_BARS);
    let all_candles =
        serde_json::from_value::<Vec<Candle>>(run.candle_data.unwrap_or_else(|| json!([])))?;
    let start_index = all_candles.len().saturating_sub(bars);
    let visual_data = decode_visual_data(run.visual_data.as_ref())?;
    let markers = resolve_markers(visual_data, &all_candles, start_index)?;
    let candles = &all_candles[start_index..];
    let plots = run
        .plot_data
        .unwrap_or_else(|| json!({}))
        .as_object()
        .ok_or_else(|| AppError(anyhow::anyhow!("indicator run has invalid plot data")))?
        .iter()
        .map(|(title, values)| {
            let values = values
                .as_array()
                .ok_or_else(|| AppError(anyhow::anyhow!("indicator plot has invalid values")))?
                .iter()
                .skip(start_index)
                .zip(candles)
                .filter_map(|(value, candle)| {
                    value
                        .as_f64()
                        .map(|value| json!({"time": candle.opened_at.timestamp(), "value": value}))
                })
                .collect::<Vec<_>>();
            Ok(json!({"title": title, "values": values}))
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    let candles = candles
        .iter()
        .map(|candle| -> Result<Value, AppError> {
            Ok(json!({
                "time": candle.opened_at.timestamp(),
                "open": candle.open.to_f64().ok_or_else(|| AppError(anyhow::anyhow!("candle open is outside chart range")))?,
                "high": candle.high.to_f64().ok_or_else(|| AppError(anyhow::anyhow!("candle high is outside chart range")))?,
                "low": candle.low.to_f64().ok_or_else(|| AppError(anyhow::anyhow!("candle low is outside chart range")))?,
                "close": candle.close.to_f64().ok_or_else(|| AppError(anyhow::anyhow!("candle close is outside chart range")))?,
            }))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(axum::Json(json!({
        "indicator": {"id": definition.id, "name": definition.name, "overlay": overlay},
        "run": {"id": run.id, "version": version.version_number, "scheduled_for": run.scheduled_for},
        "candles": candles,
        "plots": plots,
        "markers": markers,
    }))
    .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_views_preserve_validated_metadata_and_saved_values() {
        let views = input_views(
            &json!({"inputs": [{
                "kind": "int",
                "title": "Length",
                "group": "EMA",
                "default": 20,
                "min_value": 1.0,
                "max_value": 100.0,
                "step": 1.0
            }]}),
            &json!({"Length": 10}),
        )
        .expect("build views");

        assert_eq!(views.len(), 1);
        assert_eq!(views[0].title, "Length");
        assert_eq!(views[0].value, "10");
        assert_eq!(views[0].min_value, "1");
        assert_eq!(views[0].max_value, "100");
    }

    #[test]
    fn form_values_parses_typed_indicator_inputs() {
        let (_, inputs) = form_values(vec![
            ("name".to_string(), "EMA".to_string()),
            ("timeframe".to_string(), "1h".to_string()),
            ("source".to_string(), "indicator(\"EMA\")".to_string()),
            ("indicator-input-int-Length".to_string(), "20".to_string()),
            (
                "indicator-input-float-Multiplier".to_string(),
                "2.5".to_string(),
            ),
            ("indicator-input-bool-Show".to_string(), "false".to_string()),
            ("indicator-input-bool-Show".to_string(), "true".to_string()),
            (
                "indicator-input-source-Price".to_string(),
                "close".to_string(),
            ),
        ])
        .expect("parse form");

        assert_eq!(
            inputs,
            json!({"Length": 20, "Multiplier": 2.5, "Show": true, "Price": "close"})
        );
    }
}
