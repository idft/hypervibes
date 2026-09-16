use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    agents::{AuthenticatedAgent, store::list_agent_analysis_instrument_ids},
    harness::{model::RunApiScope, timeframe::parse_timeframe_seconds},
    indicators::{
        model::{IndicatorDefinition, IndicatorRun, IndicatorVersion},
        runtime::validate_indicator_source,
        store::{
            CreateIndicatorDefinition, IndicatorUpdateResult, NewIndicatorVersion,
            UpdateIndicatorDefinition, create_definition_with_initial_version, get_active_version,
            get_definition, list_definition_instruments, list_definitions, list_latest_results,
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
    latest_run: Option<IndicatorRun>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CreateIndicatorRequest {
    name: String,
    #[serde(default)]
    description: String,
    timeframe: String,
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
    timeframe: String,
    enabled: bool,
    instrument_ids: Vec<String>,
    source: String,
    #[serde(default = "empty_object")]
    input_values: Value,
}

#[derive(Debug, Deserialize)]
pub(super) struct ResultsQuery {
    instrument_id: Option<String>,
    timeframe: Option<String>,
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

fn validate_definition(name: &str, timeframe: &str) -> Result<(), ApiError> {
    if name.trim().is_empty() {
        return Err(ApiError::Validation(
            "indicator name must not be blank".to_string(),
        ));
    }
    parse_timeframe_seconds(timeframe).map_err(|error| ApiError::Validation(error.to_string()))?;
    Ok(())
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
    for definition in definitions {
        let latest_run = list_latest_results(&state.db_pool, &agent.agent_key, definition.id, 1)
            .await
            .map_err(ApiError::Internal)?
            .into_iter()
            .next();
        response.push(IndicatorListResponse {
            definition,
            latest_run,
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
    validate_definition(&input.name, &input.timeframe)?;
    let definition = create_definition_with_initial_version(
        &state.db_pool,
        &agent.agent_key,
        &CreateIndicatorDefinition {
            name: input.name,
            description: input.description,
            timeframe: input.timeframe,
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
    validate_definition(&input.name, &input.timeframe)?;
    let update = UpdateIndicatorDefinition {
        expected_active_version_id: input.expected_version_id,
        name: input.name,
        description: input.description,
        timeframe: input.timeframe,
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
) -> Result<Json<Vec<IndicatorRun>>, ApiError> {
    require_scope(&agent, RunApiScope::IndicatorRead)?;
    if get_definition(&state.db_pool, &agent.agent_key, definition_id)
        .await
        .map_err(ApiError::Internal)?
        .is_none()
    {
        return Err(ApiError::NotFound("indicator not found"));
    }
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
    let timeframe = query
        .timeframe
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let runs = list_latest_results(&state.db_pool, &agent.agent_key, definition_id, 100)
        .await
        .map_err(ApiError::Internal)?
        .into_iter()
        .filter(|run| instrument_id.is_none_or(|value| run.instrument_id == value))
        .filter(|run| timeframe.is_none_or(|value| run.timeframe == value))
        .take(limit as usize)
        .collect();
    Ok(Json(runs))
}
