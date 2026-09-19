use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
};
use serde::Deserialize;

use crate::{
    agents::{
        AuthenticatedAgent,
        store::{list_agent_trading_instrument_ids, set_agent_trading_instrument_enabled},
    },
    harness::model::RunApiScope,
    web::AppState,
};

use super::{error::ApiError, require_run_api_scope};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SetTradingInstrumentRequest {
    enabled: bool,
}

/// `GET /api/v1/trading-instruments`
pub(super) async fn list_trading_instruments(
    State(state): State<Arc<AppState>>,
    agent: AuthenticatedAgent,
) -> Result<Json<Vec<String>>, ApiError> {
    require_run_api_scope(&agent, RunApiScope::TradingInstrumentRead)?;
    let instruments = list_agent_trading_instrument_ids(&state.db_pool, &agent.agent_key)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(instruments))
}

/// `PUT /api/v1/trading-instruments/{instrument_id}`
pub(super) async fn set_trading_instrument(
    State(state): State<Arc<AppState>>,
    agent: AuthenticatedAgent,
    Path(instrument_id): Path<String>,
    Json(input): Json<SetTradingInstrumentRequest>,
) -> Result<Json<Vec<String>>, ApiError> {
    require_run_api_scope(&agent, RunApiScope::TradingInstrumentWrite)?;
    set_agent_trading_instrument_enabled(
        &state.db_pool,
        &agent.agent_key,
        &instrument_id,
        input.enabled,
    )
    .await
    .map_err(|error| ApiError::Validation(error.to_string()))?;
    let instruments = list_agent_trading_instrument_ids(&state.db_pool, &agent.agent_key)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(instruments))
}
