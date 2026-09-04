use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use tracing::info;
use uuid::Uuid;

use crate::{
    agents::AuthenticatedAgent,
    harness::model::{RunApiScope, SUB_AGENT_KIND_ANALYSIS, SUB_AGENT_KIND_TRADING},
    memory::{
        CreateMemory, MemoryListFilter, MemoryRecord, MemorySourceRun, RESERVED_MEMORY_TYPES,
        memory_expires_at, store as memory_store,
    },
    web::{AppState, ui_events::UiEvent},
};

use super::error::ApiError;

/// Resolve the memory-type restriction for the authenticated credential by
/// inspecting the source run's sub-agent kind. Analysis may write any valid
/// non-reserved type; Trading owns `trading_decision`; Review owns `review`
/// and `agent_learnings`.
async fn required_memory_type_for_credential(
    state: &AppState,
    agent: &AuthenticatedAgent,
) -> Result<Option<String>, ApiError> {
    let Some((run_id, _)) = agent.run_provenance() else {
        return Ok(None);
    };
    let sub_agent_kind: Option<String> = sqlx::query_scalar(
        "SELECT sub_agent_kind FROM harness_sub_agent_runs WHERE id = $1 AND agent_key = $2",
    )
    .bind(run_id)
    .bind(&agent.agent_key)
    .fetch_optional(&state.db_pool)
    .await?;
    match sub_agent_kind.as_deref() {
        Some(kind) if kind == SUB_AGENT_KIND_TRADING => Ok(Some("trading_decision".to_string())),
        Some(kind) if kind == SUB_AGENT_KIND_ANALYSIS => Ok(None),
        Some("review") => Ok(None),
        Some("coding") => Ok(None),
        _ => Err(ApiError::Forbidden("this run role cannot write memories")),
    }
}

fn check_reserved_memory_type(
    input: &CreateMemory,
    required_type: Option<&String>,
) -> Result<(), ApiError> {
    let requested = input.memory_type.trim();
    if let Some(required) = required_type {
        if requested != required {
            return Err(ApiError::Validation(format!(
                "this run role may only write `{required}` memories"
            )));
        }
        return Ok(());
    }
    if RESERVED_MEMORY_TYPES.contains(&requested) {
        return Err(ApiError::Validation(format!(
            "memory type `{requested}` is reserved for the framework"
        )));
    }
    Ok(())
}

/// `POST /api/v1/memories`
pub(super) async fn create_memory(
    State(state): State<Arc<AppState>>,
    agent: AuthenticatedAgent,
    Json(input): Json<CreateMemory>,
) -> Result<Response, ApiError> {
    super::require_run_api_scope(&agent, RunApiScope::MemoryWrite)?;
    if let Err(errors) = input.validate() {
        return Err(ApiError::Validation(errors.join(" ")));
    }
    let required_type = required_memory_type_for_credential(&state, &agent).await?;
    check_reserved_memory_type(&input, required_type.as_ref())?;

    // Provenance is stamped from the authenticated run credential; request
    // metadata can never supply or override it.
    let source_run = agent
        .run_provenance()
        .map(|(run_id, _)| MemorySourceRun { run_id });

    let record = memory_store::insert_memory(&state.db_pool, &agent.agent_key, &input, source_run)
        .await
        .map_err(ApiError::Internal)?;
    let instrument_targets =
        memory_store::list_memory_instrument_targets(&state.db_pool, &agent.agent_key, record.id)
            .await
            .map_err(ApiError::Internal)?;
    state.ui_events.publish(UiEvent::MemoryCreated {
        agent_key: record.agent_key.clone(),
        memory_id: record.id,
    });
    info!(
        agent_key = %record.agent_key,
        memory_id = %record.id,
        "published memory created UI event"
    );

    Ok((
        StatusCode::CREATED,
        Json(MemoryRecordResponse::from_row(record, instrument_targets)),
    )
        .into_response())
}

/// `GET /api/v1/memories`
pub(super) async fn list_memories(
    State(state): State<Arc<AppState>>,
    agent: AuthenticatedAgent,
    Query(filter): Query<MemoryListFilter>,
) -> Result<Response, ApiError> {
    super::require_run_api_scope(&agent, RunApiScope::MemoryRead)?;
    // V1: empty `timeframe=` is treated as invalid (a blank-string match is
    //   never useful — callers should omit the param entirely).
    if let Some(tf) = &filter.timeframe
        && tf.trim().is_empty()
    {
        return Err(ApiError::Validation("timeframe must not be empty".into()));
    }
    if let Some(scope_kind) = &filter.scope_kind
        && !matches!(scope_kind.as_str(), "agent" | "instruments")
    {
        return Err(ApiError::Validation(
            "scope_kind must be `agent` or `instruments`".into(),
        ));
    }

    let rows = memory_store::list_memories(&state.db_pool, &agent.agent_key, &filter)
        .await
        .map_err(ApiError::Internal)?;

    // By default the list endpoint hides rows that the staleness rules
    // consider expired (same `expires_at` rules as `/memories/latest`).
    // Operators / debug tooling can opt in to expired rows via
    // `?include_expired=true` so they can inspect what the just-expired
    // analysis said during a `[SILENT]` incident.
    let now = Utc::now();
    let include_expired = filter.include_expired;
    let matched_count = rows.len();
    let mut expired_count = 0;
    let mut bodies: Vec<MemoryRecordResponse> = Vec::with_capacity(rows.len());
    for row in rows {
        let instrument_targets =
            memory_store::list_memory_instrument_targets(&state.db_pool, &agent.agent_key, row.id)
                .await
                .map_err(ApiError::Internal)?;
        if !include_expired && memory_expires_at(&row).is_some_and(|value| value <= now) {
            expired_count += 1;
            continue;
        }
        bodies.push(MemoryRecordResponse::from_row(row, instrument_targets));
    }
    info!(
        agent_key = %agent.agent_key,
        scope_kind = ?filter.scope_kind,
        instrument_id = ?filter.instrument_id,
        timeframe = ?filter.timeframe,
        memory_type = ?filter.memory_type,
        since = ?filter.since,
        until = ?filter.until,
        limit = ?filter.limit,
        include_expired,
        matched_count,
        expired_count,
        returned_count = bodies.len(),
        "listed agent memories"
    );
    Ok(Json(bodies).into_response())
}

#[derive(Debug, Default, serde::Deserialize)]
pub(super) struct LatestMemoryQuery {
    instrument_id: Option<String>,
    memory_type: Option<String>,
    limit: Option<String>,
}

#[derive(Debug)]
pub(super) struct LatestMemoryRequest {
    pub(super) instrument_id: String,
    pub(super) memory_type: String,
    pub(super) limit: Option<usize>,
}

impl LatestMemoryQuery {
    pub(super) fn validate(self) -> Result<LatestMemoryRequest, ApiError> {
        let Self {
            instrument_id,
            memory_type,
            limit,
        } = self;
        let mut errors = Vec::new();

        let instrument_id = instrument_id
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .or_else(|| {
                errors.push("instrument_id is required.".to_string());
                None
            });

        let memory_type = memory_type
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .or_else(|| {
                errors.push("memory_type is required.".to_string());
                None
            });

        match (instrument_id, memory_type) {
            (Some(instrument_id), Some(memory_type)) => Ok(LatestMemoryRequest {
                instrument_id,
                memory_type,
                limit: parse_latest_memories_limit(limit.as_deref())?,
            }),
            _ => Err(ApiError::Validation(errors.join(" "))),
        }
    }
}

pub(super) fn parse_latest_memories_limit(limit: Option<&str>) -> Result<Option<usize>, ApiError> {
    let Some(raw_limit) = limit.map(str::trim) else {
        return Ok(None);
    };

    let value = raw_limit
        .parse::<usize>()
        .map_err(|_| ApiError::BadRequest("limit must be an integer >= 1".into()))?;

    if value < 1 {
        return Err(ApiError::BadRequest("limit must be an integer >= 1".into()));
    }

    Ok(Some(value))
}

/// `GET /api/v1/memories/latest`
pub(super) async fn list_latest_memories(
    State(state): State<Arc<AppState>>,
    agent: AuthenticatedAgent,
    Query(query): Query<LatestMemoryQuery>,
) -> Result<Response, ApiError> {
    super::require_run_api_scope(&agent, RunApiScope::MemoryRead)?;
    let LatestMemoryRequest {
        instrument_id,
        memory_type,
        limit,
    } = query.validate()?;
    let rows = memory_store::list_latest_memory_candidates(
        &state.db_pool,
        &agent.agent_key,
        &instrument_id,
        &memory_type,
    )
    .await
    .map_err(ApiError::Internal)?;

    let now = Utc::now();
    let mut seen_timeframes = std::collections::HashSet::new();
    let mut bodies = Vec::new();

    for row in rows {
        let Some(timeframe) = row.timeframe.clone() else {
            continue;
        };
        let expires_at = memory_expires_at(&row);
        if expires_at.is_some_and(|value| value <= now) {
            continue;
        }
        if !seen_timeframes.insert(timeframe) {
            continue;
        }
        bodies.push(LatestMemoryResponse::from_record(row, expires_at));
        if limit.is_some_and(|value| bodies.len() >= value) {
            break;
        }
    }

    Ok(Json(bodies).into_response())
}

#[derive(Debug, Default, serde::Deserialize)]
pub(super) struct TradingContextQuery {
    pub(super) instrument_id: Option<String>,
}

/// `GET /api/v1/memories/trading-context?instrument_id=`
pub(super) async fn get_trading_context(
    State(state): State<Arc<AppState>>,
    agent: AuthenticatedAgent,
    Query(query): Query<TradingContextQuery>,
) -> Result<Response, ApiError> {
    super::require_run_api_scope(&agent, RunApiScope::MemoryRead)?;
    let instrument_id = query
        .instrument_id
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::Validation("instrument_id is required.".into()))?;
    let now = Utc::now();
    let evidence = memory_store::get_trading_context_evidence(
        &state.db_pool,
        &agent.agent_key,
        &instrument_id,
        now,
    )
    .await
    .map_err(ApiError::Internal)?;
    Ok(Json(serde_json::json!({
        "instrument_id": instrument_id,
        "evidence": evidence,
    }))
    .into_response())
}

/// `GET /api/v1/memories/{id}`
pub(super) async fn get_memory_by_id(
    State(state): State<Arc<AppState>>,
    agent: AuthenticatedAgent,
    Path(id): Path<String>,
    Query(filter): Query<MemoryDetailQuery>,
) -> Result<Response, ApiError> {
    super::require_run_api_scope(&agent, RunApiScope::MemoryRead)?;
    let id = Uuid::parse_str(&id).map_err(|_| ApiError::BadUuid)?;
    let record = memory_store::get_memory(&state.db_pool, &agent.agent_key, id)
        .await
        .map_err(ApiError::Internal)?;

    match record {
        Some(r) => {
            let instrument_targets =
                memory_store::list_memory_instrument_targets(&state.db_pool, &agent.agent_key, id)
                    .await
                    .map_err(ApiError::Internal)?;
            let mut body =
                serde_json::to_value(MemoryRecordResponse::from_row(r, instrument_targets))
                    .map_err(|e| ApiError::Internal(anyhow::anyhow!(e)))?;
            if filter.includes("links") {
                let outgoing =
                    memory_store::list_memory_links_from(&state.db_pool, &agent.agent_key, id)
                        .await
                        .map_err(ApiError::Internal)?;
                let incoming =
                    memory_store::list_memory_links_to(&state.db_pool, &agent.agent_key, id)
                        .await
                        .map_err(ApiError::Internal)?;
                if let Some(obj) = body.as_object_mut() {
                    obj.insert(
                        "links_from".to_string(),
                        serde_json::to_value(outgoing)
                            .map_err(|e| ApiError::Internal(anyhow::anyhow!(e)))?,
                    );
                    obj.insert(
                        "links_to".to_string(),
                        serde_json::to_value(incoming)
                            .map_err(|e| ApiError::Internal(anyhow::anyhow!(e)))?,
                    );
                }
            }
            Ok(Json(body).into_response())
        }
        None => Err(ApiError::NotFound("memory not found")),
    }
}

#[derive(Debug, Default, serde::Deserialize)]
pub(super) struct MemoryDetailQuery {
    include: Option<String>,
}

impl MemoryDetailQuery {
    fn includes(&self, key: &str) -> bool {
        self.include
            .as_deref()
            .is_some_and(|value| value.split(',').any(|part| part.trim() == key))
    }
}

/// Response shape returned to agents. Keeping a dedicated type lets the wire
/// format evolve without breaking the DB row struct.
#[derive(Debug, serde::Serialize)]
pub struct MemoryRecordResponse {
    pub id: Uuid,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub agent_key: String,
    pub scope_kind: String,
    pub instrument_targets: Vec<String>,
    pub timeframe: Option<String>,
    pub memory_type: String,
    pub summary: String,
    pub content: String,
    pub metadata: serde_json::Value,
    pub source_run_id: Option<i64>,
    /// RFC 3339 timestamp at which this memory becomes stale, or `null`
    /// if the row has no explicit or implicit expiration (e.g. an
    /// `observation` memory with no `valid_for_seconds` / `stale_after`).
    ///
    /// Analysis-produced research rows use the schedule-derived defaults
    /// (`15m` => 30m, `1h` => 120m, `1d` => 48h, unknown => 30m — all 2x the
    /// schedule interval so the trading loop has a one-cycle fallback if the
    /// next analysis is delayed).
    pub expires_at: Option<DateTime<Utc>>,
}

impl MemoryRecordResponse {
    pub fn from_row(r: MemoryRecord, instrument_targets: Vec<String>) -> Self {
        let expires_at = memory_expires_at(&r);
        Self {
            id: r.id,
            created_at: r.created_at,
            agent_key: r.agent_key,
            scope_kind: r.scope_kind,
            instrument_targets,
            timeframe: r.timeframe,
            memory_type: r.memory_type,
            summary: r.summary,
            content: r.content,
            metadata: r.metadata,
            source_run_id: r.source_run_id,
            expires_at,
        }
    }
}

impl From<MemoryRecord> for MemoryRecordResponse {
    fn from(r: MemoryRecord) -> Self {
        Self::from_row(r, Vec::new())
    }
}

#[derive(Debug, serde::Serialize)]
pub(super) struct LatestMemoryResponse {
    id: Uuid,
    created_at: DateTime<Utc>,
    agent_key: String,
    scope_kind: String,
    instrument_targets: Vec<String>,
    timeframe: Option<String>,
    memory_type: String,
    summary: String,
    content: String,
    metadata: serde_json::Value,
    expires_at: Option<DateTime<Utc>>,
}

impl LatestMemoryResponse {
    pub(super) fn from_record(record: MemoryRecord, expires_at: Option<DateTime<Utc>>) -> Self {
        Self {
            id: record.id,
            created_at: record.created_at,
            agent_key: record.agent_key,
            scope_kind: record.scope_kind,
            instrument_targets: Vec::new(),
            timeframe: record.timeframe,
            memory_type: record.memory_type,
            summary: record.summary,
            content: record.content,
            metadata: record.metadata,
            expires_at,
        }
    }
}
