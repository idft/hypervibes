use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, State},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::{
    agents::{AuthenticatedAgent, store::get_agent},
    hyperliquid::queries::{AccountTransactionWindow, list_account_transactions_in_window},
    web::AppState,
};

use super::error::ApiError;

#[derive(Debug, Deserialize)]
pub(super) struct TransactionQuery {
    since: DateTime<Utc>,
    until: DateTime<Utc>,
    symbol: Option<String>,
    event_category: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
}

/// `GET /api/v1/account/transactions?since=&until=`
///
/// Agent-scoped durable Hyperliquid account-journal events. This intentionally
/// reads the application-owned journal rather than issuing an exchange request.
pub(super) async fn list_account_transactions(
    State(state): State<Arc<AppState>>,
    agent: AuthenticatedAgent,
    Query(query): Query<TransactionQuery>,
) -> Result<Response, ApiError> {
    if query.until <= query.since {
        return Err(ApiError::Validation("until must be after since".into()));
    }
    if query.offset.is_some_and(|offset| offset < 0) {
        return Err(ApiError::Validation("offset must be non-negative".into()));
    }
    if let Some(category) = &query.event_category
        && !matches!(category.as_str(), "fill" | "funding" | "ledger")
    {
        return Err(ApiError::Validation(
            "event_category must be fill, funding, or ledger".into(),
        ));
    }
    let row = get_agent(&state.db_pool, &agent.agent_key)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound("agent not found"))?;
    let body = list_account_transactions_in_window(
        &state.db_pool,
        &row.wallet_address,
        &row.environment,
        AccountTransactionWindow {
            since: query.since,
            until: query.until,
            symbol: query.symbol.as_deref(),
            event_category: query.event_category.as_deref(),
            limit: query.limit.unwrap_or(500),
            offset: query.offset.unwrap_or(0),
        },
    )
    .await
    .map_err(ApiError::Internal)?;
    Ok(Json(body).into_response())
}
