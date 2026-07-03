use std::{collections::HashSet, sync::Arc};

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
    memory::{
        CreateMemory, MemoryListFilter, MemoryRecord, memory_expires_at, store as memory_store,
    },
    web::{AppState, ui_events::UiEvent},
};

use super::error::ApiError;

/// `POST /api/v1/memories`
pub(super) async fn create_memory(
    State(state): State<Arc<AppState>>,
    agent: AuthenticatedAgent,
    Json(input): Json<CreateMemory>,
) -> Result<Response, ApiError> {
    if let Err(errors) = input.validate() {
        return Err(ApiError::Validation(errors.join(" ")));
    }

    let record = memory_store::insert_memory(&state.db_pool, &agent.agent_key, &input)
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
        Json(MemoryRecordResponse::from(record)),
    )
        .into_response())
}

/// `GET /api/v1/memories`
pub(super) async fn list_memories(
    State(state): State<Arc<AppState>>,
    agent: AuthenticatedAgent,
    Query(filter): Query<MemoryListFilter>,
) -> Result<Response, ApiError> {
    // V1: empty `timeframe=` is treated as invalid (a blank-string match is
    //   never useful — callers should omit the param entirely).
    if let Some(tf) = &filter.timeframe
        && tf.trim().is_empty()
    {
        return Err(ApiError::Validation("timeframe must not be empty".into()));
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
    let bodies: Vec<MemoryRecordResponse> = rows
        .into_iter()
        .filter(|row| include_expired || !memory_expires_at(row).is_some_and(|value| value <= now))
        .map(MemoryRecordResponse::from)
        .collect();
    Ok(Json(bodies).into_response())
}

#[derive(Debug, Default, serde::Deserialize)]
pub(super) struct LatestMemoryQuery {
    symbol: Option<String>,
    memory_type: Option<String>,
    limit: Option<String>,
}

#[derive(Debug)]
pub(super) struct LatestMemoryRequest {
    pub(super) symbol: String,
    pub(super) memory_type: String,
    pub(super) limit: Option<usize>,
}

impl LatestMemoryQuery {
    pub(super) fn validate(self) -> Result<LatestMemoryRequest, ApiError> {
        let Self {
            symbol,
            memory_type,
            limit,
        } = self;
        let mut errors = Vec::new();

        let symbol = symbol
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .or_else(|| {
                errors.push("symbol is required.".to_string());
                None
            });

        let memory_type = memory_type
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .or_else(|| {
                errors.push("memory_type is required.".to_string());
                None
            });

        match (symbol, memory_type) {
            (Some(symbol), Some(memory_type)) => Ok(LatestMemoryRequest {
                symbol,
                memory_type,
                limit: parse_latest_memories_limit(limit.as_deref())?,
            }),
            _ => Err(ApiError::Validation(errors.join(" "))),
        }
    }
}

pub(super) fn parse_latest_memories_limit(
    limit: Option<&str>,
) -> Result<Option<usize>, ApiError> {
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
    let LatestMemoryRequest {
        symbol,
        memory_type,
        limit,
    } = query.validate()?;
    let rows = memory_store::list_latest_memory_candidates(
        &state.db_pool,
        &agent.agent_key,
        &symbol,
        &memory_type,
    )
    .await
    .map_err(ApiError::Internal)?;

    let now = Utc::now();
    let mut seen_timeframes = HashSet::new();
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

/// `GET /api/v1/memories/{id}`
pub(super) async fn get_memory_by_id(
    State(state): State<Arc<AppState>>,
    agent: AuthenticatedAgent,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let id = Uuid::parse_str(&id).map_err(|_| ApiError::BadUuid)?;
    let record = memory_store::get_memory(&state.db_pool, &agent.agent_key, id)
        .await
        .map_err(ApiError::Internal)?;

    match record {
        Some(r) => Ok(Json(MemoryRecordResponse::from(r)).into_response()),
        None => Err(ApiError::NotFound("memory not found")),
    }
}

/// Response shape returned to agents. Today it mirrors [`MemoryRecord`]
/// 1:1, but keeping a dedicated type lets us evolve the wire format
/// without breaking the DB row struct.
#[derive(Debug, serde::Serialize)]
pub struct MemoryRecordResponse {
    pub id: Uuid,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub agent_key: String,
    pub symbol: String,
    pub timeframe: Option<String>,
    pub memory_type: String,
    pub summary: String,
    pub content: String,
    pub metadata: serde_json::Value,
    /// RFC 3339 timestamp at which this memory becomes stale, or `null`
    /// if the row has no explicit or implicit expiration (e.g. an
    /// observation memory with no `valid_for_seconds` / `stale_after`).
    ///
    /// For `memory_type="analysis"` the same defaults documented for
    /// `GET /api/v1/memories/latest` apply (`15m` => 30m, `1h` => 120m,
    /// `1d` => 48h, unknown => 30m — all 2x the schedule interval so the
    /// trading loop has a one-cycle fallback if the next analysis is
    /// delayed).
    pub expires_at: Option<DateTime<Utc>>,
}

impl From<MemoryRecord> for MemoryRecordResponse {
    fn from(r: MemoryRecord) -> Self {
        let expires_at = memory_expires_at(&r);
        Self {
            id: r.id,
            created_at: r.created_at,
            agent_key: r.agent_key,
            symbol: r.symbol,
            timeframe: r.timeframe,
            memory_type: r.memory_type,
            summary: r.summary,
            content: r.content,
            metadata: r.metadata,
            expires_at,
        }
    }
}

#[derive(Debug, serde::Serialize)]
pub(super) struct LatestMemoryResponse {
    id: Uuid,
    created_at: DateTime<Utc>,
    agent_key: String,
    symbol: String,
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
            symbol: record.symbol,
            timeframe: record.timeframe,
            memory_type: record.memory_type,
            summary: record.summary,
            content: record.content,
            metadata: record.metadata,
            expires_at,
        }
    }
}