use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    agents::{AuthenticatedAgent, store::list_agent_analysis_instrument_ids},
    harness::model::RunApiScope,
    indicators::{
        model::{IndicatorDefinition, IndicatorRun, IndicatorVersion},
        runtime::validate_indicator_source,
        store::{
            CreateIndicatorDefinition, FrozenDependencyResultFilter, IndicatorUpdateResult,
            NewIndicatorVersion, UpdateIndicatorDefinition, create_definition_with_initial_version,
            get_active_version, get_definition, get_result_by_id, list_definition_instruments,
            list_definitions, list_frozen_dependency_results, list_latest_results,
            list_latest_results_for_timeframe, normalize_timeframes,
            update_definition_with_new_version,
        },
    },
    web::AppState,
};

use super::error::ApiError;

const COMPILER_VERSION: &str = "pine-lang 0.2.6";

#[derive(Debug, Serialize)]
pub(super) struct IndicatorDetailResponse {
    #[serde(flatten)]
    definition: IndicatorDefinition,
    active_version: IndicatorVersion,
    instrument_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct IndicatorListResponse {
    #[serde(flatten)]
    definition: IndicatorDefinition,
    latest_run: Option<PublicIndicatorRun>,
}

#[derive(Debug, Serialize)]
pub(super) struct PublicIndicatorRun {
    id: Uuid,
    agent_key: String,
    indicator_definition_id: Uuid,
    indicator_version_id: Uuid,
    instrument_id: String,
    timeframe: String,
    scheduled_for: DateTime<Utc>,
    status: String,
    candle_data: Option<Value>,
    plot_data: Option<Value>,
    visual_data: Option<Value>,
    latest_values: Option<Value>,
    diagnostics: Value,
    error_summary: Option<String>,
    attempt_count: i32,
    started_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<IndicatorRun> for PublicIndicatorRun {
    fn from(run: IndicatorRun) -> Self {
        Self {
            id: run.id,
            agent_key: run.agent_key,
            indicator_definition_id: run.indicator_definition_id,
            indicator_version_id: run.indicator_version_id,
            instrument_id: run.instrument_id,
            timeframe: run.timeframe,
            scheduled_for: run.scheduled_for,
            status: run.status,
            candle_data: run.candle_data,
            plot_data: run.plot_data,
            visual_data: run.visual_data,
            latest_values: run.latest_values,
            diagnostics: run.diagnostics,
            error_summary: run.error_summary,
            attempt_count: run.attempt_count,
            started_at: run.started_at,
            finished_at: run.finished_at,
            created_at: run.created_at,
            updated_at: run.updated_at,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CreateIndicatorRequest {
    name: String,
    #[serde(default)]
    description: String,
    timeframes: Vec<String>,
    instrument_ids: Vec<String>,
    source: String,
    #[serde(default = "empty_object")]
    input_values: Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct UpdateIndicatorRequest {
    expected_version_id: Uuid,
    name: String,
    #[serde(default)]
    description: String,
    timeframes: Vec<String>,
    enabled: bool,
    instrument_ids: Vec<String>,
    source: String,
    #[serde(default = "empty_object")]
    input_values: Value,
}

#[derive(Debug, Deserialize)]
pub(super) struct ResultsQuery {
    instrument_id: Option<String>,
    timeframe: String,
    run_id: Option<Uuid>,
    limit: Option<i64>,
}

fn empty_object() -> Value {
    json!({})
}

fn require_scope(agent: &AuthenticatedAgent, scope: RunApiScope) -> Result<(), ApiError> {
    if agent.is_run_credential() {
        super::require_run_api_scope(agent, scope)
    } else {
        Ok(())
    }
}

fn version_for(
    agent: &AuthenticatedAgent,
    source: String,
    input_values: Value,
) -> Result<NewIndicatorVersion, ApiError> {
    let validated = validate_indicator_source(&source, &input_values)
        .map_err(|error| ApiError::Validation(error.to_string()))?;
    let (created_by_kind, created_by_run_id, created_by_conversation_id) =
        if agent.is_run_credential() {
            let (run_id, _) = agent.run_provenance().ok_or(ApiError::Forbidden(
                "indicator changes require a review run credential",
            ))?;
            ("review".to_string(), Some(run_id), None)
        } else if let Some(conversation_id) = agent.conversation_provenance() {
            ("chat".to_string(), None, Some(conversation_id))
        } else {
            ("operator".to_string(), None, None)
        };
    Ok(NewIndicatorVersion {
        source,
        compiler_version: COMPILER_VERSION.to_string(),
        metadata: json!({"indicator": validated.metadata, "inputs": validated.inputs}),
        input_values,
        created_by_kind,
        created_by_run_id,
        created_by_conversation_id,
    })
}

fn validate_definition(name: &str, timeframes: &[String]) -> Result<Vec<String>, ApiError> {
    if name.trim().is_empty() {
        return Err(ApiError::Validation(
            "indicator name must not be blank".to_string(),
        ));
    }
    normalize_timeframes(timeframes).map_err(|error| ApiError::Validation(error.to_string()))
}

async fn detail_response(
    state: &AppState,
    agent_key: &str,
    definition_id: Uuid,
) -> Result<IndicatorDetailResponse, ApiError> {
    let definition = get_definition(&state.db_pool, agent_key, definition_id)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound("indicator not found"))?;
    let active_version = get_active_version(&state.db_pool, agent_key, definition_id)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound("indicator active version not found"))?;
    let instrument_ids = list_definition_instruments(&state.db_pool, agent_key, definition_id)
        .await
        .map_err(ApiError::Internal)?;
    Ok(IndicatorDetailResponse {
        definition,
        active_version,
        instrument_ids,
    })
}

/// `GET /api/v1/analysis-instruments`
pub(super) async fn list_analysis_instruments(
    State(state): State<Arc<AppState>>,
    agent: AuthenticatedAgent,
) -> Result<Json<Vec<String>>, ApiError> {
    require_scope(&agent, RunApiScope::IndicatorRead)?;
    let instrument_ids = list_agent_analysis_instrument_ids(&state.db_pool, &agent.agent_key)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(instrument_ids))
}

/// `GET /api/v1/indicators`
pub(super) async fn list_indicators(
    State(state): State<Arc<AppState>>,
    agent: AuthenticatedAgent,
) -> Result<Json<Vec<IndicatorListResponse>>, ApiError> {
    require_scope(&agent, RunApiScope::IndicatorRead)?;
    let definitions = list_definitions(&state.db_pool, &agent.agent_key)
        .await
        .map_err(ApiError::Internal)?;
    let mut response = Vec::with_capacity(definitions.len());
    let analysis_run_id = match agent.run_provenance() {
        Some((run_id, _)) => crate::harness::store::get_run(&state.db_pool, run_id)
            .await
            .map_err(ApiError::Internal)?
            .filter(|run| run.sub_agent_kind == crate::harness::model::SUB_AGENT_KIND_ANALYSIS)
            .map(|run| run.id),
        None => None,
    };
    for definition in definitions {
        let latest_run = if let Some(run_id) = analysis_run_id {
            list_frozen_dependency_results(
                &state.db_pool,
                run_id,
                &agent.agent_key,
                definition.id,
                FrozenDependencyResultFilter {
                    timeframe: None,
                    instrument_id: None,
                    run_id: None,
                    limit: 1,
                },
            )
            .await
            .map_err(ApiError::Internal)?
            .into_iter()
            .next()
        } else {
            list_latest_results(&state.db_pool, &agent.agent_key, definition.id, 1)
                .await
                .map_err(ApiError::Internal)?
                .into_iter()
                .next()
        };
        response.push(IndicatorListResponse {
            definition,
            latest_run: latest_run.map(Into::into),
        });
    }
    Ok(Json(response))
}

/// `POST /api/v1/indicators`
pub(super) async fn create_indicator(
    State(state): State<Arc<AppState>>,
    agent: AuthenticatedAgent,
    Json(input): Json<CreateIndicatorRequest>,
) -> Result<Json<IndicatorDetailResponse>, ApiError> {
    require_scope(&agent, RunApiScope::IndicatorWrite)?;
    let timeframes = validate_definition(&input.name, &input.timeframes)?;
    let definition = create_definition_with_initial_version(
        &state.db_pool,
        &agent.agent_key,
        &CreateIndicatorDefinition {
            name: input.name,
            description: input.description,
            timeframes,
            enabled: true,
            instrument_ids: input.instrument_ids,
            version: version_for(&agent, input.source, input.input_values)?,
        },
    )
    .await
    .map_err(|error| ApiError::Validation(error.to_string()))?;
    Ok(Json(
        detail_response(&state, &agent.agent_key, definition.id).await?,
    ))
}

/// `GET /api/v1/indicators/{indicator_id}`
pub(super) async fn get_indicator(
    State(state): State<Arc<AppState>>,
    agent: AuthenticatedAgent,
    Path(definition_id): Path<Uuid>,
) -> Result<Json<IndicatorDetailResponse>, ApiError> {
    require_scope(&agent, RunApiScope::IndicatorRead)?;
    Ok(Json(
        detail_response(&state, &agent.agent_key, definition_id).await?,
    ))
}

/// `PUT /api/v1/indicators/{indicator_id}`
pub(super) async fn update_indicator(
    State(state): State<Arc<AppState>>,
    agent: AuthenticatedAgent,
    Path(definition_id): Path<Uuid>,
    Json(input): Json<UpdateIndicatorRequest>,
) -> Result<Json<IndicatorDetailResponse>, ApiError> {
    require_scope(&agent, RunApiScope::IndicatorWrite)?;
    let timeframes = validate_definition(&input.name, &input.timeframes)?;
    let update = UpdateIndicatorDefinition {
        expected_active_version_id: input.expected_version_id,
        name: input.name,
        description: input.description,
        timeframes,
        enabled: input.enabled,
        instrument_ids: input.instrument_ids,
        version: version_for(&agent, input.source, input.input_values)?,
    };
    match update_definition_with_new_version(
        &state.db_pool,
        &agent.agent_key,
        definition_id,
        &update,
    )
    .await
    .map_err(|error| ApiError::Validation(error.to_string()))?
    {
        IndicatorUpdateResult::Updated => Ok(Json(
            detail_response(&state, &agent.agent_key, definition_id).await?,
        )),
        IndicatorUpdateResult::NotFound => Err(ApiError::NotFound("indicator not found")),
        IndicatorUpdateResult::VersionConflict => Err(ApiError::Conflict(
            "indicator active version no longer matches",
        )),
    }
}

/// `GET /api/v1/indicators/{indicator_id}/results`
pub(super) async fn get_indicator_results(
    State(state): State<Arc<AppState>>,
    agent: AuthenticatedAgent,
    Path(definition_id): Path<Uuid>,
    Query(query): Query<ResultsQuery>,
) -> Result<Json<Vec<PublicIndicatorRun>>, ApiError> {
    require_scope(&agent, RunApiScope::IndicatorRead)?;
    let limit = query.limit.unwrap_or(20);
    if !(1..=100).contains(&limit) {
        return Err(ApiError::Validation(
            "limit must be between 1 and 100".to_string(),
        ));
    }
    let instrument_id = query
        .instrument_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let timeframe = query.timeframe.trim();
    crate::harness::timeframe::parse_timeframe_seconds(timeframe)
        .map_err(|error| ApiError::Validation(error.to_string()))?;
    let run_context = match agent.run_provenance() {
        Some((run_id, _)) => crate::harness::store::get_run(&state.db_pool, run_id)
            .await
            .map_err(ApiError::Internal)?,
        None => None,
    };
    let runs = if let Some(run) = run_context
        .filter(|run| run.sub_agent_kind == crate::harness::model::SUB_AGENT_KIND_ANALYSIS)
    {
        list_frozen_dependency_results(
            &state.db_pool,
            run.id,
            &agent.agent_key,
            definition_id,
            FrozenDependencyResultFilter {
                timeframe: Some(timeframe),
                instrument_id,
                run_id: query.run_id,
                limit,
            },
        )
        .await
        .map_err(ApiError::Internal)?
    } else if let Some(run_id) = query.run_id {
        get_result_by_id(
            &state.db_pool,
            &agent.agent_key,
            definition_id,
            timeframe,
            instrument_id,
            run_id,
        )
        .await
        .map_err(ApiError::Internal)?
        .into_iter()
        .collect()
    } else {
        let Some(definition) = get_definition(&state.db_pool, &agent.agent_key, definition_id)
            .await
            .map_err(ApiError::Internal)?
        else {
            return Err(ApiError::NotFound("indicator not found"));
        };
        if !definition.timeframes.iter().any(|value| value == timeframe) {
            return Err(ApiError::Validation(
                "timeframe must be configured on the indicator".to_string(),
            ));
        }
        list_latest_results_for_timeframe(
            &state.db_pool,
            &agent.agent_key,
            definition_id,
            timeframe,
            instrument_id,
            limit,
        )
        .await
        .map_err(ApiError::Internal)?
        .into_iter()
        .collect()
    };
    Ok(Json(runs.into_iter().map(Into::into).collect()))
}
