use std::{collections::HashSet, sync::Arc};

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use uuid::Uuid;

use crate::{
    agents::crypto as agent_crypto,
    agents::{
        AuthenticatedAgent,
        store::{get_agent, list_agent_instrument_ids},
    },
    harness::model::RunApiScope,
    hyperliquid::orders::{
        gateway::{
            CancelAllSummary, CancelOutcome, GatewayError, HyperliquidExchange,
            LiveOrderPlacementContext, cancel_all, cancel_orders, place_orders_with_live_state,
        },
        model::{CancelOrdersRequest, PlaceOrdersRequest, PlaceOrdersResponse},
        store as orders_store,
    },
    web::{AppState, auth::get_user_api_wallet_for_agent},
};

use super::error::ApiError;

/// Build the [`HyperliquidExchange`] for the calling agent by loading
/// and decrypting their Hyperliquid private key.
pub(crate) async fn build_exchange_for_agent(
    state: &AppState,
    agent_key: &str,
) -> anyhow::Result<HyperliquidExchange> {
    let wallet = get_user_api_wallet_for_agent(&state.db_pool, agent_key)
        .await?
        .ok_or_else(|| anyhow::anyhow!("agent not found"))?;
    if !wallet.is_ready() {
        anyhow::bail!("the user's trading signer needs attention");
    }
    let ciphertext = wallet
        .hyperliquid_private_key_ciphertext
        .ok_or_else(|| anyhow::anyhow!("the user's trading signer needs attention"))?;
    let key_id = wallet
        .hyperliquid_private_key_key_id
        .ok_or_else(|| anyhow::anyhow!("the user's trading signer needs attention"))?;

    if state.encryption_key.key_id != key_id {
        return Err(anyhow::anyhow!(
            "agent private key was encrypted with key_id '{key_id}' but server is using '{}'",
            state.encryption_key.key_id
        ));
    }

    let pk_string = agent_crypto::decrypt(&state.encryption_key, &ciphertext)
        .map_err(|e| anyhow::anyhow!("failed to decrypt private key: {e}"))?;

    // Use the `PrivateKeySigner` re-exported by `hypersdk` — the
    // struct's `signer` field is typed as `hypersdk::hypercore::PrivateKeySigner`,
    // which is the v1.x alloy type. Our top-level `alloy` dep is v2.x,
    // so we must explicitly go through the re-export to avoid a
    // version-mismatch error.
    let signer: hypersdk::hypercore::PrivateKeySigner = pk_string
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid private key: {e}"))?;
    let expected_address = wallet
        .api_wallet_address
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("the user's trading signer needs attention"))?;
    if signer.address().to_string().to_ascii_lowercase() != expected_address {
        return Err(anyhow::anyhow!(
            "stored user trading signer address does not match its database address"
        ));
    }
    let main_account = wallet
        .main_wallet_address
        .parse()
        .map_err(|error| anyhow::anyhow!("invalid stored main wallet address: {error}"))?;
    let client = hypersdk::hypercore::mainnet();
    Ok(HyperliquidExchange::new(signer, client, main_account))
}

/// Per-row summary for `GET /api/v1/orders`.
#[derive(Debug, serde::Serialize)]
pub(super) struct OrderListItem {
    id: Uuid,
    created_at: chrono::DateTime<chrono::Utc>,
    symbol: String,
    side: String,
    order_kind: String,
    reduce_only: bool,
    cloid: String,
    exchange_oid: Option<String>,
    status: String,
    filled_size: Option<rust_decimal::Decimal>,
    avg_fill_price: Option<rust_decimal::Decimal>,
    group_id: Option<Uuid>,
    memory_record_ids: serde_json::Value,
    attribution_source: String,
}

impl From<orders_store::OrderRow> for OrderListItem {
    fn from(r: orders_store::OrderRow) -> Self {
        Self {
            id: r.id,
            created_at: r.created_at,
            symbol: r.symbol,
            side: r.side,
            order_kind: r.order_kind,
            reduce_only: r.reduce_only,
            cloid: r.cloid,
            exchange_oid: r.exchange_oid,
            status: r.status,
            filled_size: r.filled_size,
            avg_fill_price: r.avg_fill_price,
            group_id: r.group_id,
            memory_record_ids: r.memory_record_ids,
            attribution_source: r.attribution_source,
        }
    }
}

#[derive(Debug, serde::Serialize)]
pub(super) struct OrderEventsResponse {
    events: Vec<orders_store::OrderEventRow>,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct ListFilterQuery {
    status: Option<String>,
    symbol: Option<String>,
    include: Option<String>,
    since: Option<chrono::DateTime<chrono::Utc>>,
    until: Option<chrono::DateTime<chrono::Utc>>,
    limit: Option<i64>,
}

#[derive(Debug, serde::Serialize)]
pub(super) struct CancelOutcomeResponse {
    symbol: String,
    oid: u64,
    status: String,
    error: Option<String>,
    order_id: Option<Uuid>,
}

impl From<CancelOutcome> for CancelOutcomeResponse {
    fn from(o: CancelOutcome) -> Self {
        Self {
            symbol: o.symbol,
            oid: o.oid,
            status: o.status,
            error: o.error,
            order_id: o.order_id,
        }
    }
}

#[derive(Debug, serde::Serialize)]
pub(super) struct CancelAllResponse {
    considered: usize,
    outcomes: Vec<CancelOutcomeResponse>,
}

impl From<CancelAllSummary> for CancelAllResponse {
    fn from(s: CancelAllSummary) -> Self {
        Self {
            considered: s.considered,
            outcomes: s.outcomes.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct CancelAllQuery {
    symbol: Option<String>,
}

pub(super) fn map_gateway_error(err: GatewayError) -> ApiError {
    match err {
        GatewayError::Validation(s) => ApiError::Validation(s),
        GatewayError::Internal(e) => ApiError::Internal(e),
    }
}

/// `POST /api/v1/orders`
pub(super) async fn place_orders_handler(
    State(state): State<Arc<AppState>>,
    agent: AuthenticatedAgent,
    Json(input): Json<PlaceOrdersRequest>,
) -> Result<Response, ApiError> {
    super::require_run_api_scope(&agent, RunApiScope::OrderWrite)?;
    if let Err(msg) = input.validate() {
        return Err(ApiError::Validation(msg));
    }

    let agent_row = get_agent(&state.db_pool, &agent.agent_key)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound("agent not found"))?;
    let selected_instruments = list_agent_instrument_ids(&state.db_pool, &agent.agent_key)
        .await
        .map_err(ApiError::Internal)?;
    if selected_instruments.is_empty() {
        return Err(ApiError::Validation(
            "no currencies are selected for this agent; order placement is disabled".into(),
        ));
    }
    let selected_set: HashSet<String> = selected_instruments.into_iter().collect();
    for (idx, order) in input.orders.iter().enumerate() {
        if !selected_set.contains(&order.symbol) {
            return Err(ApiError::Validation(format!(
                "orders[{idx}]: symbol '{}' is not enabled for this agent",
                order.symbol
            )));
        }
    }
    let Some(account_address) = agent_row.trading_account_address.clone() else {
        return Ok((StatusCode::CONFLICT, "agent has no trading account").into_response());
    };
    let environment = agent_row.environment.clone();

    let exchange = build_exchange_for_agent(&state, &agent.agent_key)
        .await
        .map_err(ApiError::Internal)?;
    let resp = place_orders_with_live_state(
        &state.db_pool,
        &exchange,
        &state.builder_fee_cache,
        &agent.agent_key,
        &input,
        LiveOrderPlacementContext {
            account_address: &account_address,
            environment: &environment,
            live_accounts: &state.live_accounts,
        },
    )
    .await
    .map_err(map_gateway_error)?;

    let body = PlaceOrdersResponse {
        results: resp.results,
    };
    Ok((StatusCode::CREATED, Json(body)).into_response())
}

/// `GET /api/v1/orders?status=&symbol=`
pub(super) async fn list_orders_handler(
    State(state): State<Arc<AppState>>,
    agent: AuthenticatedAgent,
    Query(filter): Query<ListFilterQuery>,
) -> Result<Response, ApiError> {
    super::require_run_api_scope(&agent, RunApiScope::OrderRead)?;
    if let Some(s) = &filter.status
        && s.trim().is_empty()
    {
        return Err(ApiError::Validation("status must not be empty".into()));
    }
    let rows = orders_store::list_orders(
        &state.db_pool,
        &agent.agent_key,
        filter.status.as_deref(),
        filter.symbol.as_deref(),
        filter.since,
        filter.until,
        filter.limit,
    )
    .await
    .map_err(ApiError::Internal)?;
    let body: Vec<OrderListItem> = rows.into_iter().map(Into::into).collect();
    Ok(Json(body).into_response())
}

/// `GET /api/v1/orders/{id}?include=events`
pub(super) async fn get_order_handler(
    State(state): State<Arc<AppState>>,
    agent: AuthenticatedAgent,
    Path(id): Path<String>,
    Query(filter): Query<ListFilterQuery>,
) -> Result<Response, ApiError> {
    super::require_run_api_scope(&agent, RunApiScope::OrderRead)?;
    let id = Uuid::parse_str(&id).map_err(|_| ApiError::BadUuid)?;
    let row = orders_store::get_order(&state.db_pool, &agent.agent_key, id)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound("order not found"))?;

    let mut body = serde_json::to_value(OrderListItem::from(row))
        .map_err(|e| ApiError::Internal(anyhow::anyhow!(e)))?;
    if filter
        .include
        .as_deref()
        .is_some_and(|s| s.split(',').any(|p| p.trim() == "events"))
    {
        let events = orders_store::list_events(&state.db_pool, id)
            .await
            .map_err(ApiError::Internal)?;
        let events_body = serde_json::to_value(OrderEventsResponse { events })
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("failed to serialize events: {e}")))?;
        if let Some(obj) = body.as_object_mut() {
            obj.insert("events".to_string(), events_body["events"].clone());
        }
    }
    Ok(Json(body).into_response())
}

/// `POST /api/v1/orders/cancel`
pub(super) async fn cancel_orders_handler(
    State(state): State<Arc<AppState>>,
    agent: AuthenticatedAgent,
    Json(input): Json<CancelOrdersRequest>,
) -> Result<Response, ApiError> {
    super::require_run_api_scope(&agent, RunApiScope::OrderWrite)?;
    if input.orders.is_empty() {
        return Err(ApiError::Validation("orders must not be empty".into()));
    }
    let agent_row = get_agent(&state.db_pool, &agent.agent_key)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound("agent not found"))?;
    let Some(account_address) = agent_row.trading_account_address.clone() else {
        return Ok((StatusCode::CONFLICT, "agent has no trading account").into_response());
    };
    let exchange = build_exchange_for_agent(&state, &agent.agent_key)
        .await
        .map_err(ApiError::Internal)?;
    let outcomes = cancel_orders(
        &state.db_pool,
        &exchange,
        &agent.agent_key,
        &account_address,
        &agent_row.environment,
        &input,
    )
    .await
    .map_err(map_gateway_error)?;
    let body: Vec<CancelOutcomeResponse> = outcomes.into_iter().map(Into::into).collect();
    Ok(Json(body).into_response())
}

/// `POST /api/v1/orders/cancel-all?symbol=`
pub(super) async fn cancel_all_handler(
    State(state): State<Arc<AppState>>,
    agent: AuthenticatedAgent,
    Query(filter): Query<CancelAllQuery>,
) -> Result<Response, ApiError> {
    super::require_run_api_scope(&agent, RunApiScope::OrderWrite)?;
    let agent_row = get_agent(&state.db_pool, &agent.agent_key)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound("agent not found"))?;
    let Some(account_address) = agent_row.trading_account_address.clone() else {
        return Ok((StatusCode::CONFLICT, "agent has no trading account").into_response());
    };
    let exchange = build_exchange_for_agent(&state, &agent.agent_key)
        .await
        .map_err(ApiError::Internal)?;
    let summary = cancel_all(
        &state.db_pool,
        &exchange,
        &agent.agent_key,
        &account_address,
        &agent_row.environment,
        filter.symbol.as_deref(),
    )
    .await
    .map_err(map_gateway_error)?;
    Ok(Json(CancelAllResponse::from(summary)).into_response())
}
