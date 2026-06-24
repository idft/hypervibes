use std::{collections::HashSet, sync::Arc};

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{DateTime, Duration, Utc};
use rust_decimal::Decimal;
use serde_json::json;
use tracing::{error, info};
use uuid::Uuid;

use crate::{
    agents::crypto as agent_crypto,
    agents::{
        AuthenticatedAgent,
        store::{
            JobContextKind, get_agent, get_agent_private_key_ciphertext,
            touch_job_context_last_used,
        },
    },
    hyperliquid::{
        live_state::{AccountKey, AccountLiveState, LiveOpenOrder, LivePosition},
        orders::{
            gateway::{
                CancelAllSummary, CancelOutcome, GatewayError, HyperliquidExchange, cancel_all,
                cancel_orders, place_orders,
            },
            model::{
                CancelOrdersRequest, OrderResult as GatewayOrderResult, PlaceOrdersRequest,
                PlaceOrdersResponse,
            },
            store as orders_store,
        },
    },
    memory::{CreateMemory, MemoryListFilter, MemoryRecord, store as memory_store},
    web::{AppState, ui_events::UiEvent},
};

/// Build the `/api/v1` sub-router. Merged into the main router in
/// `src/web/routes.rs`.
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/memories", post(create_memory).get(list_memories))
        .route("/memories/latest", get(list_latest_memories))
        .route("/memories/{id}", get(get_memory_by_id))
        .route("/account", get(get_account))
        .route("/job-context", get(get_job_context))
        .route(
            "/orders",
            post(place_orders_handler).get(list_orders_handler),
        )
        .route("/orders/cancel", post(cancel_orders_handler))
        .route("/orders/cancel-all", post(cancel_all_handler))
        .route("/orders/{id}", get(get_order_handler))
        .with_state(state)
}

/// Wire the `/api/v1` sub-router into a parent router.
pub fn merge(parent: Router, state: Arc<AppState>) -> Router {
    parent.nest("/api/v1", router(state))
}

/// JSON error type for the agent API. Distinct from the operator HTML
/// `AppError` in `src/web/routes.rs` — the agent API must always return
/// `{ "error": "..." }` so trading agents can parse it.
#[derive(Debug)]
pub enum ApiError {
    BadRequest(String),
    NotFound(&'static str),
    Validation(String),
    BadUuid,
    Internal(anyhow::Error),
}

impl ApiError {
    fn message(&self) -> String {
        match self {
            ApiError::BadRequest(msg) => msg.clone(),
            ApiError::NotFound(msg) => (*msg).to_string(),
            ApiError::Validation(msg) => msg.clone(),
            ApiError::BadUuid => "invalid memory id".to_string(),
            ApiError::Internal(_) => "internal server error".to_string(),
        }
    }

    fn status(&self) -> StatusCode {
        match self {
            ApiError::BadRequest(_) => StatusCode::BAD_REQUEST,
            ApiError::NotFound(_) => StatusCode::NOT_FOUND,
            ApiError::Validation(_) => StatusCode::UNPROCESSABLE_ENTITY,
            ApiError::BadUuid => StatusCode::NOT_FOUND,
            ApiError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl<E> From<E> for ApiError
where
    E: Into<anyhow::Error>,
{
    fn from(error: E) -> Self {
        ApiError::Internal(error.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        if let ApiError::Internal(ref e) = self {
            error!(error = ?e, "agent API request failed");
        }
        let body = Json(json!({ "error": self.message() }));
        (self.status(), body).into_response()
    }
}

/// `POST /api/v1/memories`
async fn create_memory(
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
async fn list_memories(
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

    let bodies: Vec<MemoryRecordResponse> =
        rows.into_iter().map(MemoryRecordResponse::from).collect();
    Ok(Json(bodies).into_response())
}

#[derive(Debug, Default, serde::Deserialize)]
struct LatestMemoryQuery {
    symbol: Option<String>,
    memory_type: Option<String>,
    limit: Option<String>,
}

#[derive(Debug)]
struct LatestMemoryRequest {
    symbol: String,
    memory_type: String,
    limit: Option<usize>,
}

impl LatestMemoryQuery {
    fn validate(self) -> Result<LatestMemoryRequest, ApiError> {
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

fn parse_latest_memories_limit(limit: Option<&str>) -> Result<Option<usize>, ApiError> {
    let Some(raw_limit) = limit.map(str::trim) else {
        return Ok(None);
    };

    let value = raw_limit
        .parse::<usize>()
        .map_err(|_| ApiError::BadRequest("limit must be an integer >= 1".into()))?;

    if value < 1 {
        return Err(ApiError::BadRequest(
            "limit must be an integer >= 1".into(),
        ));
    }

    Ok(Some(value))
}

/// `GET /api/v1/memories/latest`
async fn list_latest_memories(
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
        let expires_at = latest_memory_expires_at(&row);
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
async fn get_memory_by_id(
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

/// `GET /api/v1/account`
///
/// Snapshot of the calling agent's Hyperliquid live account state. The
/// operator UI uses the SSE stream at `/agents/{agent_key}/live/stream`
/// instead; this is the agent-facing JSON poll.
async fn get_account(
    State(state): State<Arc<AppState>>,
    agent: AuthenticatedAgent,
) -> Result<Response, ApiError> {
    let row = get_agent(&state.db_pool, &agent.agent_key)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound("agent not found"))?;
    let body = live_agent_snapshot(&state, &row);
    Ok(Json(body).into_response())
}

/// Snapshot of the calling agent's live account state. Returned to the
/// agent as JSON; the operator UI's SSE stream emits the rendered view
/// types from `src/web/templates.rs` directly.
#[derive(Debug, Clone, serde::Serialize)]
struct LiveAgentSnapshot {
    agent_key: String,
    account_address: String,
    environment: String,
    account_data: AccountDataStatus,
    balance: Option<AccountBalance>,
    open_positions: Vec<LivePosition>,
    open_orders: Vec<LiveOpenOrder>,
}

#[derive(Debug, Clone, serde::Serialize)]
struct AccountDataStatus {
    available: bool,
    as_of: Option<DateTime<Utc>>,
    stale: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
struct AccountBalance {
    exchange: &'static str,
    model: &'static str,
    total_equity_usd: Decimal,
    available_to_trade_usd: Decimal,
    available_to_withdraw_usd: Decimal,
    margin_used_usd: Decimal,
    unrealized_pnl_usd: Decimal,
    collateral_balances: Vec<CollateralBalance>,
}

#[derive(Debug, Clone, serde::Serialize)]
struct CollateralBalance {
    asset: String,
    total: Decimal,
    available: Decimal,
}

const ACCOUNT_DATA_MAX_AGE: Duration = Duration::minutes(2);
const COLLATERAL_ASSETS: &[&str] = &["USDC", "USDE", "USDT0", "USDH"];

fn live_agent_snapshot(
    state: &AppState,
    row: &crate::agents::model::AgentDetailRow,
) -> LiveAgentSnapshot {
    let key = AccountKey::new(&row.wallet_address, &row.environment);
    let snapshot = state.live_accounts.get(&key);
    let Some(snapshot) = snapshot else {
        return unavailable_live_agent_snapshot(row);
    };
    let Some(updated_at) = snapshot.updated_at else {
        return unavailable_live_agent_snapshot(row);
    };
    if Utc::now() - updated_at > ACCOUNT_DATA_MAX_AGE {
        return unavailable_live_agent_snapshot(row);
    }

    LiveAgentSnapshot {
        agent_key: row.agent_key.clone(),
        account_address: row.wallet_address.clone(),
        environment: row.environment.clone(),
        account_data: AccountDataStatus {
            available: true,
            as_of: Some(updated_at),
            stale: false,
        },
        balance: Some(account_balance_from_live_state(&snapshot)),
        open_positions: snapshot.open_positions.clone(),
        open_orders: snapshot.open_orders.clone(),
    }
}

fn unavailable_live_agent_snapshot(
    row: &crate::agents::model::AgentDetailRow,
) -> LiveAgentSnapshot {
    LiveAgentSnapshot {
        agent_key: row.agent_key.clone(),
        account_address: row.wallet_address.clone(),
        environment: row.environment.clone(),
        account_data: AccountDataStatus {
            available: false,
            as_of: None,
            stale: true,
        },
        balance: None,
        open_positions: Vec::new(),
        open_orders: Vec::new(),
    }
}

fn account_balance_from_live_state(state: &AccountLiveState) -> AccountBalance {
    let mut collateral_balances = Vec::new();

    for asset in COLLATERAL_ASSETS {
        let total = state
            .spot_balances
            .iter()
            .filter(|balance| balance.coin == *asset)
            .filter_map(|balance| balance.total)
            .sum();
        let available = state
            .spot_balances
            .iter()
            .filter(|balance| balance.coin == *asset)
            .filter_map(|balance| balance.available)
            .sum();
        if total > Decimal::ZERO || available > Decimal::ZERO {
            collateral_balances.push(CollateralBalance {
                asset: (*asset).to_string(),
                total,
                available,
            });
        }
    }

    let collateral_total: Decimal = collateral_balances
        .iter()
        .map(|balance| balance.total)
        .sum();
    let collateral_available: Decimal = collateral_balances
        .iter()
        .map(|balance| balance.available)
        .sum();
    let margin_withdrawable = state
        .margin
        .as_ref()
        .and_then(|margin| margin.withdrawable)
        .unwrap_or(Decimal::ZERO);
    let margin_account_value = state
        .margin
        .as_ref()
        .and_then(|margin| margin.account_value)
        .unwrap_or(Decimal::ZERO);
    let margin_used_usd = state
        .margin
        .as_ref()
        .and_then(|margin| margin.total_margin_used)
        .unwrap_or(Decimal::ZERO);
    let unrealized_pnl_usd: Decimal = state
        .open_positions
        .iter()
        .filter_map(|position| position.unrealized_pnl)
        .sum();
    let available_to_trade_usd = collateral_available.max(margin_withdrawable);

    AccountBalance {
        exchange: "hyperliquid",
        model: "unified_cross_margin",
        total_equity_usd: collateral_total.max(margin_account_value),
        available_to_trade_usd,
        available_to_withdraw_usd: available_to_trade_usd,
        margin_used_usd,
        unrealized_pnl_usd,
        collateral_balances,
    }
}

#[derive(Debug, serde::Deserialize)]
struct JobContextQuery {
    job_kind: String,
}

#[derive(Debug, Clone, Copy)]
enum JobKind {
    Analysis,
    Trading,
}

impl JobKind {
    fn parse(value: &str) -> Result<Self, ApiError> {
        match value {
            "analysis" => Ok(Self::Analysis),
            "trading" => Ok(Self::Trading),
            other => Err(ApiError::Validation(format!(
                "invalid job_kind '{other}', expected analysis or trading"
            ))),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Analysis => "analysis",
            Self::Trading => "trading",
        }
    }

    fn context_kind(self) -> JobContextKind {
        match self {
            Self::Analysis => JobContextKind::Analysis,
            Self::Trading => JobContextKind::Trading,
        }
    }
}

#[derive(Debug, serde::Serialize)]
struct JobContextResponse {
    agent_key: String,
    job_kind: String,
    display_name: String,
    environment: String,
    prompt: String,
    account: Option<LiveAgentSnapshot>,
}

async fn get_job_context(
    State(state): State<Arc<AppState>>,
    agent: AuthenticatedAgent,
    Query(query): Query<JobContextQuery>,
) -> Result<Response, ApiError> {
    let job_kind = JobKind::parse(query.job_kind.trim())?;
    let row = get_agent(&state.db_pool, &agent.agent_key)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound("agent not found"))?;
    touch_job_context_last_used(&state.db_pool, &agent.agent_key, job_kind.context_kind())
        .await
        .map_err(ApiError::Internal)?;
    let (prompt, account) = match job_kind {
        JobKind::Analysis => (row.analysis_prompt.clone(), None),
        JobKind::Trading => (
            row.trading_prompt.clone(),
            Some(live_agent_snapshot(&state, &row)),
        ),
    };

    Ok(Json(JobContextResponse {
        agent_key: row.agent_key.clone(),
        job_kind: job_kind.as_str().to_string(),
        display_name: row.display_name.clone(),
        environment: row.environment.clone(),
        prompt,
        account,
    })
    .into_response())
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
}

impl From<MemoryRecord> for MemoryRecordResponse {
    fn from(r: MemoryRecord) -> Self {
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
        }
    }
}

#[derive(Debug, serde::Serialize)]
struct LatestMemoryResponse {
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
    fn from_record(record: MemoryRecord, expires_at: Option<DateTime<Utc>>) -> Self {
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

fn latest_memory_expires_at(row: &MemoryRecord) -> Option<DateTime<Utc>> {
    stale_after(&row.metadata)
        .or_else(|| {
            valid_for_seconds(&row.metadata)
                .map(|seconds| row.created_at + Duration::seconds(seconds))
        })
        .or_else(|| {
            if row.memory_type == "analysis" {
                Some(
                    row.created_at
                        + analysis_default_valid_for(row.timeframe.as_deref().unwrap_or("")),
                )
            } else {
                None
            }
        })
}

fn valid_for_seconds(metadata: &serde_json::Value) -> Option<i64> {
    metadata
        .get("valid_for_seconds")
        .and_then(|value| match value {
            serde_json::Value::Number(number) => number
                .as_i64()
                .filter(|seconds| *seconds > 0)
                .or_else(|| {
                    number
                        .as_u64()
                        .and_then(|seconds| i64::try_from(seconds).ok())
                        .filter(|seconds| *seconds > 0)
                })
                .or_else(|| {
                    let seconds = number.as_f64()?;
                    if !seconds.is_finite() || seconds < 1.0 || seconds > i64::MAX as f64 {
                        return None;
                    }
                    Some(seconds.floor() as i64)
                }),
            _ => None,
        })
}

fn stale_after(metadata: &serde_json::Value) -> Option<DateTime<Utc>> {
    metadata
        .get("stale_after")
        .and_then(|value| value.as_str())
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc))
}

fn analysis_default_valid_for(timeframe: &str) -> Duration {
    match timeframe {
        "15m" => Duration::minutes(20),
        "1h" => Duration::minutes(90),
        "1d" => Duration::hours(36),
        _ => Duration::minutes(20),
    }
}

// ---- order endpoints ------------------------------------------------------

/// Build the [`HyperliquidExchange`] for the calling agent by loading
/// and decrypting their Hyperliquid private key.
async fn build_exchange_for_agent(
    state: &AppState,
    agent_key: &str,
) -> Result<HyperliquidExchange, ApiError> {
    let (ciphertext, key_id) = get_agent_private_key_ciphertext(&state.db_pool, agent_key)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound("agent not found"))?;

    if state.encryption_key.key_id != key_id {
        return Err(ApiError::Internal(anyhow::anyhow!(
            "agent private key was encrypted with key_id '{key_id}' but server is using '{}'",
            state.encryption_key.key_id
        )));
    }

    let pk_string = agent_crypto::decrypt(&state.encryption_key, &ciphertext)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("failed to decrypt private key: {e}")))?;

    // Use the `PrivateKeySigner` re-exported by `hypersdk` — the
    // struct's `signer` field is typed as `hypersdk::hypercore::PrivateKeySigner`,
    // which is the v1.x alloy type. Our top-level `alloy` dep is v2.x,
    // so we must explicitly go through the re-export to avoid a
    // version-mismatch error.
    let signer: hypersdk::hypercore::PrivateKeySigner = pk_string
        .parse()
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("invalid private key: {e}")))?;
    let client = hypersdk::hypercore::mainnet();
    Ok(HyperliquidExchange::new(signer, client))
}

/// Per-leg outcome as serialized to the agent. The struct itself is
/// only used as a serde target for `From<GatewayOrderResult>`, so the
/// direct field access is gated behind `#[allow(dead_code)]`.
#[allow(dead_code)]
#[derive(Debug, serde::Serialize)]
struct OrderResultResponse {
    #[allow(dead_code)]
    id: Uuid,
    #[allow(dead_code)]
    cloid: String,
    #[allow(dead_code)]
    symbol: String,
    #[allow(dead_code)]
    side: String,
    #[allow(dead_code)]
    order_kind: String,
    #[allow(dead_code)]
    status: String,
    #[allow(dead_code)]
    exchange_oid: Option<String>,
    #[allow(dead_code)]
    group_id: Option<Uuid>,
    #[allow(dead_code)]
    error: Option<String>,
}

impl From<GatewayOrderResult> for OrderResultResponse {
    fn from(r: GatewayOrderResult) -> Self {
        Self {
            id: r.id,
            cloid: r.cloid,
            symbol: r.symbol,
            side: r.side,
            order_kind: r.order_kind,
            status: r.status,
            exchange_oid: r.exchange_oid,
            group_id: r.group_id,
            error: r.error,
        }
    }
}

/// Per-row summary for `GET /api/v1/orders`.
#[derive(Debug, serde::Serialize)]
struct OrderListItem {
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
        }
    }
}

#[derive(Debug, serde::Serialize)]
struct OrderEventsResponse {
    events: Vec<orders_store::OrderEventRow>,
}

#[derive(Debug, serde::Deserialize)]
struct ListFilterQuery {
    status: Option<String>,
    symbol: Option<String>,
    include: Option<String>,
}

#[derive(Debug, serde::Serialize)]
struct CancelOutcomeResponse {
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
struct CancelAllResponse {
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
struct CancelAllQuery {
    symbol: Option<String>,
}

fn map_gateway_error(err: GatewayError) -> ApiError {
    match err {
        GatewayError::Validation(s) => ApiError::Validation(s),
        GatewayError::Internal(e) => ApiError::Internal(e),
    }
}

/// `POST /api/v1/orders`
async fn place_orders_handler(
    State(state): State<Arc<AppState>>,
    agent: AuthenticatedAgent,
    Json(input): Json<PlaceOrdersRequest>,
) -> Result<Response, ApiError> {
    if let Err(msg) = input.validate() {
        return Err(ApiError::Validation(msg));
    }

    let agent_row = get_agent(&state.db_pool, &agent.agent_key)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound("agent not found"))?;
    let account_address = agent_row.wallet_address.clone();
    let environment = agent_row.environment.clone();

    let exchange = build_exchange_for_agent(&state, &agent.agent_key).await?;
    let resp = place_orders(
        &state.db_pool,
        &exchange,
        &agent.agent_key,
        &account_address,
        &environment,
        &input,
    )
    .await
    .map_err(map_gateway_error)?;

    let body = PlaceOrdersResponse {
        results: resp.results.into_iter().map(Into::into).collect(),
    };
    Ok((StatusCode::CREATED, Json(body)).into_response())
}

/// `GET /api/v1/orders?status=&symbol=`
async fn list_orders_handler(
    State(state): State<Arc<AppState>>,
    agent: AuthenticatedAgent,
    Query(filter): Query<ListFilterQuery>,
) -> Result<Response, ApiError> {
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
    )
    .await
    .map_err(ApiError::Internal)?;
    let body: Vec<OrderListItem> = rows.into_iter().map(Into::into).collect();
    Ok(Json(body).into_response())
}

/// `GET /api/v1/orders/{id}?include=events`
async fn get_order_handler(
    State(state): State<Arc<AppState>>,
    agent: AuthenticatedAgent,
    Path(id): Path<String>,
    Query(filter): Query<ListFilterQuery>,
) -> Result<Response, ApiError> {
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
async fn cancel_orders_handler(
    State(state): State<Arc<AppState>>,
    agent: AuthenticatedAgent,
    Json(input): Json<CancelOrdersRequest>,
) -> Result<Response, ApiError> {
    if input.orders.is_empty() {
        return Err(ApiError::Validation("orders must not be empty".into()));
    }
    let agent_row = get_agent(&state.db_pool, &agent.agent_key)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound("agent not found"))?;
    let exchange = build_exchange_for_agent(&state, &agent.agent_key).await?;
    let outcomes = cancel_orders(
        &state.db_pool,
        &exchange,
        &agent.agent_key,
        &agent_row.wallet_address,
        &agent_row.environment,
        &input,
    )
    .await
    .map_err(map_gateway_error)?;
    let body: Vec<CancelOutcomeResponse> = outcomes.into_iter().map(Into::into).collect();
    Ok(Json(body).into_response())
}

/// `POST /api/v1/orders/cancel-all?symbol=`
async fn cancel_all_handler(
    State(state): State<Arc<AppState>>,
    agent: AuthenticatedAgent,
    Query(filter): Query<CancelAllQuery>,
) -> Result<Response, ApiError> {
    let agent_row = get_agent(&state.db_pool, &agent.agent_key)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound("agent not found"))?;
    let exchange = build_exchange_for_agent(&state, &agent.agent_key).await?;
    let summary = cancel_all(
        &state.db_pool,
        &exchange,
        &agent.agent_key,
        &agent_row.wallet_address,
        &agent_row.environment,
        filter.symbol.as_deref(),
    )
    .await
    .map_err(map_gateway_error)?;
    Ok(Json(CancelAllResponse::from(summary)).into_response())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use chrono::{Duration, Utc};
    use rust_decimal_macros::dec;
    use serde_json::json;
    use tower::util::ServiceExt;
    use uuid::Uuid;

    use crate::{
        agents::{
            crypto::EncryptionKey,
            keys::derive_wallet_address,
            model::AgentRegistryRow,
            store::{get_agent, insert_agent},
        },
        hyperliquid::live_state::{
            AccountKey, AccountLiveState, LiveMarginState, LiveOpenOrder, LivePosition,
            LiveSpotBalance,
        },
        test_db,
        web::{
            AppState, api,
            ui_events::{UiEvent, UiEventHub},
        },
    };

    async fn test_state() -> Arc<AppState> {
        let pool = test_db::pool().await;
        Arc::new(AppState {
            db_pool: pool,
            encryption_key: EncryptionKey::new(
                "test",
                [
                    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21,
                    22, 23, 24, 25, 26, 27, 28, 29, 30, 31,
                ],
            ),
            live_accounts: Arc::new(crate::hyperliquid::live_state::LiveAccountStore::new()),
            ui_events: Arc::new(UiEventHub::new()),
            hermes: None,
            hermes_dashboard_link_url: "http://127.0.0.1:19119".to_string(),
        })
    }

    fn random_private_key() -> String {
        use rand::Rng;
        format!("0x{}", hex::encode(rand::thread_rng().r#gen::<[u8; 32]>()))
    }

    async fn seed_agent(state: &Arc<AppState>, suffix: &str) -> (String, String) {
        seed_agent_with_prompts(state, suffix, "", "").await
    }

    async fn seed_agent_with_prompts(
        state: &Arc<AppState>,
        suffix: &str,
        analysis_prompt: &str,
        trading_prompt: &str,
    ) -> (String, String) {
        let timestamp = Utc::now().timestamp_millis();
        let display_name = format!("MemApiTest{}{}", suffix, timestamp);
        let agent_key = crate::agents::model::slugify_agent_key(&display_name);
        let private_key = random_private_key();
        let wallet_address = derive_wallet_address(&private_key).expect("derives");
        let api_key = format!("vta_memapi-{suffix}-{timestamp}");
        let now = Utc::now();
        let row = AgentRegistryRow {
            agent_key: agent_key.clone(),
            created_at: now,
            updated_at: now,
            enabled: true,
            display_name,
            analysis_prompt: analysis_prompt.to_string(),
            trading_prompt: trading_prompt.to_string(),
            wallet_address,
            environment: "live".to_string(),
            api_key: api_key.clone(),
            api_key_last_used_at: None,
            analysis_context_last_used_at: None,
            trading_context_last_used_at: None,
            hyperliquid_private_key_ciphertext: Vec::new(),
            hyperliquid_private_key_key_id: "test".to_string(),
        };
        insert_agent(&state.db_pool, &row)
            .await
            .expect("insert agent");
        (agent_key, api_key)
    }

    fn app(state: Arc<AppState>) -> Router {
        api::router(state)
    }

    fn json_body<T: serde::Serialize>(value: &T) -> (Option<(&'static str, String)>, Body) {
        let body = serde_json::to_vec(value).expect("serialize");
        (
            Some(("content-type", "application/json".to_string())),
            Body::from(body),
        )
    }

    async fn insert_memory_at(
        state: &Arc<AppState>,
        agent_key: &str,
        created_at: chrono::DateTime<Utc>,
        symbol: &str,
        timeframe: Option<&str>,
        memory_type: &str,
        summary: &str,
        metadata: serde_json::Value,
    ) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO memory.records (
                id, created_at, agent_key, symbol, timeframe, memory_type, summary, content, metadata
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(id)
        .bind(created_at)
        .bind(agent_key)
        .bind(symbol)
        .bind(timeframe)
        .bind(memory_type)
        .bind(summary)
        .bind(format!("body for {summary}"))
        .bind(metadata)
        .execute(&state.db_pool)
        .await
        .expect("insert memory record");
        id
    }

    async fn latest_memories_response(
        state: &Arc<AppState>,
        api_key: &str,
        uri: &str,
    ) -> (StatusCode, serde_json::Value) {
        let request = Request::builder()
            .method("GET")
            .uri(uri)
            .header("authorization", format!("Bearer {api_key}"))
            .body(Body::empty())
            .unwrap();
        let response = app(Arc::clone(state)).oneshot(request).await.unwrap();
        let status = response.status();
        let body_bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let body = serde_json::from_slice(&body_bytes).unwrap();
        (status, body)
    }

    async fn get_json_response(
        state: &Arc<AppState>,
        api_key: &str,
        uri: &str,
    ) -> (StatusCode, serde_json::Value) {
        let request = Request::builder()
            .uri(uri)
            .header("authorization", format!("Bearer {api_key}"))
            .body(Body::empty())
            .unwrap();
        let response = app(Arc::clone(state)).oneshot(request).await.unwrap();
        let status = response.status();
        let body_bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let body = serde_json::from_slice(&body_bytes).unwrap();
        (status, body)
    }

    #[tokio::test]
    async fn post_memories_creates_record() {
        let state = test_state().await;
        let mut ui_events = state.ui_events.subscribe();

        let (agent_key, api_key) = seed_agent(&state, "create").await;

        let body = serde_json::json!({
            "symbol": "BTC",
            "timeframe": "1h",
            "memory_type": "plan",
            "summary": "buy pullback",
            "content": "BTC reclaimed the prior breakout level",
            "metadata": { "confidence": 0.72 }
        });
        let (headers, body) = json_body(&body);
        let mut builder = Request::builder()
            .method("POST")
            .uri("/memories")
            .header("authorization", format!("Bearer {api_key}"));
        if let Some((k, v)) = headers {
            builder = builder.header(k, v);
        }
        let response = app(Arc::clone(&state))
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let ct = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(
            ct.starts_with("application/json"),
            "expected application/json content-type, got {ct}"
        );

        let body_bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(body["symbol"], "BTC");
        assert_eq!(body["timeframe"], "1h");
        assert_eq!(body["summary"], "buy pullback");
        assert_eq!(body["metadata"]["confidence"], 0.72);
        assert!(body["id"].is_string());

        let event = ui_events.recv().await.expect("memory UI event");
        assert_eq!(
            event,
            UiEvent::MemoryCreated {
                agent_key,
                memory_id: Uuid::parse_str(body["id"].as_str().unwrap()).unwrap(),
            }
        );
    }

    #[tokio::test]
    async fn post_memories_without_auth_returns_401_json() {
        let state = test_state().await;

        let body = serde_json::json!({
            "symbol": "BTC",
            "memory_type": "plan",
            "summary": "x",
            "content": "y"
        });
        let (headers, body) = json_body(&body);
        let mut builder = Request::builder().method("POST").uri("/memories");
        if let Some((k, v)) = headers {
            builder = builder.header(k, v);
        }
        let response = app(state)
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let body_bytes = axum::body::to_bytes(response.into_body(), 16 * 1024)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert!(body["error"].is_string());
    }

    #[tokio::test]
    async fn post_memories_with_invalid_bearer_returns_401_json() {
        let state = test_state().await;

        let body = serde_json::json!({
            "symbol": "BTC",
            "memory_type": "plan",
            "summary": "x",
            "content": "y"
        });
        let (headers, body) = json_body(&body);
        let mut builder = Request::builder()
            .method("POST")
            .uri("/memories")
            .header("authorization", "Bearer vta_does-not-exist");
        if let Some((k, v)) = headers {
            builder = builder.header(k, v);
        }
        let response = app(state)
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn post_memories_empty_field_returns_422_json() {
        let state = test_state().await;

        let (_agent_key, api_key) = seed_agent(&state, "val").await;

        let body = serde_json::json!({
            "symbol": "BTC",
            "memory_type": "plan",
            "summary": "  ",
            "content": "y"
        });
        let (headers, body) = json_body(&body);
        let mut builder = Request::builder()
            .method("POST")
            .uri("/memories")
            .header("authorization", format!("Bearer {api_key}"));
        if let Some((k, v)) = headers {
            builder = builder.header(k, v);
        }
        let response = app(state)
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body_bytes = axum::body::to_bytes(response.into_body(), 16 * 1024)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert!(body["error"].as_str().unwrap().contains("summary"));
    }

    #[tokio::test]
    async fn post_memories_non_object_metadata_returns_422() {
        let state = test_state().await;

        let (_agent_key, api_key) = seed_agent(&state, "meta").await;

        let body = serde_json::json!({
            "symbol": "BTC",
            "memory_type": "plan",
            "summary": "x",
            "content": "y",
            "metadata": [1, 2, 3]
        });
        let (headers, body) = json_body(&body);
        let mut builder = Request::builder()
            .method("POST")
            .uri("/memories")
            .header("authorization", format!("Bearer {api_key}"));
        if let Some((k, v)) = headers {
            builder = builder.header(k, v);
        }
        let response = app(state)
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn auth_touches_api_key_last_used_at() {
        let state = test_state().await;

        let (agent_key, api_key) = seed_agent(&state, "touch").await;

        let body = serde_json::json!({
            "symbol": "BTC",
            "memory_type": "plan",
            "summary": "x",
            "content": "y"
        });
        let (headers, body) = json_body(&body);
        let mut builder = Request::builder()
            .method("POST")
            .uri("/memories")
            .header("authorization", format!("Bearer {api_key}"));
        if let Some((k, v)) = headers {
            builder = builder.header(k, v);
        }
        let response = app(Arc::clone(&state))
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);

        let agent = get_agent(&state.db_pool, &agent_key)
            .await
            .unwrap()
            .expect("present");
        assert!(agent.api_key_last_used_at.is_some());
    }

    #[tokio::test]
    async fn list_memories_filters_and_orders_desc() {
        let state = test_state().await;

        let (_agent_key, api_key) = seed_agent(&state, "list").await;

        for summary in ["a", "b", "c"] {
            let body = serde_json::json!({
                "symbol": "BTC",
                "timeframe": "1h",
                "memory_type": "plan",
                "summary": summary,
                "content": format!("body {summary}"),
            });
            let (headers, body) = json_body(&body);
            let mut builder = Request::builder()
                .method("POST")
                .uri("/memories")
                .header("authorization", format!("Bearer {api_key}"));
            if let Some((k, v)) = headers {
                builder = builder.header(k, v);
            }
            let response = app(Arc::clone(&state))
                .oneshot(builder.body(body).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::CREATED);
        }

        // Also insert a general (NULL timeframe) memory.
        let body = serde_json::json!({
            "symbol": "BTC",
            "memory_type": "plan",
            "summary": "general",
            "content": "general"
        });
        let (headers, body) = json_body(&body);
        let mut builder = Request::builder()
            .method("POST")
            .uri("/memories")
            .header("authorization", format!("Bearer {api_key}"));
        if let Some((k, v)) = headers {
            builder = builder.header(k, v);
        }
        let response = app(Arc::clone(&state))
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);

        // List the 1h timeframe: should return the 3 plan memories in DESC order.
        let request = Request::builder()
            .uri("/memories?symbol=BTC&timeframe=1h&limit=10")
            .header("authorization", format!("Bearer {api_key}"))
            .body(Body::empty())
            .unwrap();
        let response = app(Arc::clone(&state)).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body_bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let rows: Vec<serde_json::Value> = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(rows.len(), 3);
        let summaries: Vec<&str> = rows
            .iter()
            .map(|r| r["summary"].as_str().unwrap())
            .collect();
        assert_eq!(summaries, vec!["c", "b", "a"]);

        // No timeframe => NULL-timeframe memories only.
        let request = Request::builder()
            .uri("/memories?symbol=BTC")
            .header("authorization", format!("Bearer {api_key}"))
            .body(Body::empty())
            .unwrap();
        let response = app(Arc::clone(&state)).oneshot(request).await.unwrap();
        let body_bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let rows: Vec<serde_json::Value> = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["summary"], "general");
        assert!(rows[0]["timeframe"].is_null());

        // Scope check: another agent must not see any of these rows.
        let (_other_key, other_api_key) = seed_agent(&state, "other").await;
        let request = Request::builder()
            .uri("/memories?symbol=BTC&timeframe=1h")
            .header("authorization", format!("Bearer {other_api_key}"))
            .body(Body::empty())
            .unwrap();
        let response = app(Arc::clone(&state)).oneshot(request).await.unwrap();
        let body_bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let rows: Vec<serde_json::Value> = serde_json::from_slice(&body_bytes).unwrap();
        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn latest_memories_returns_latest_valid_per_timeframe() {
        let state = test_state().await;
        let (agent_key, api_key) = seed_agent(&state, "latest-per-tf").await;
        let now = Utc::now();

        insert_memory_at(
            &state,
            &agent_key,
            now - Duration::minutes(12),
            "BTC",
            Some("15m"),
            "analysis",
            "15m-old",
            json!({}),
        )
        .await;
        insert_memory_at(
            &state,
            &agent_key,
            now - Duration::minutes(5),
            "BTC",
            Some("15m"),
            "analysis",
            "15m-new",
            json!({}),
        )
        .await;
        insert_memory_at(
            &state,
            &agent_key,
            now - Duration::minutes(30),
            "BTC",
            Some("1h"),
            "analysis",
            "1h-new",
            json!({}),
        )
        .await;
        insert_memory_at(
            &state,
            &agent_key,
            now - Duration::hours(4),
            "BTC",
            Some("1d"),
            "analysis",
            "1d-new",
            json!({}),
        )
        .await;

        let (status, body) = latest_memories_response(
            &state,
            &api_key,
            "/memories/latest?symbol=BTC&memory_type=analysis",
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let rows = body.as_array().expect("array response");
        assert_eq!(rows.len(), 3);
        let summaries: Vec<&str> = rows
            .iter()
            .map(|row| row["summary"].as_str().unwrap())
            .collect();
        assert_eq!(summaries, vec!["15m-new", "1h-new", "1d-new"]);
        assert!(rows.iter().all(|row| row["expires_at"].is_string()));
    }

    #[tokio::test]
    async fn latest_memories_applies_limit_after_grouping() {
        let state = test_state().await;
        let (agent_key, api_key) = seed_agent(&state, "latest-limit").await;
        let now = Utc::now();

        insert_memory_at(
            &state,
            &agent_key,
            now - Duration::minutes(12),
            "BTC",
            Some("15m"),
            "analysis",
            "15m-old",
            json!({}),
        )
        .await;
        insert_memory_at(
            &state,
            &agent_key,
            now - Duration::minutes(5),
            "BTC",
            Some("15m"),
            "analysis",
            "15m-new",
            json!({}),
        )
        .await;
        insert_memory_at(
            &state,
            &agent_key,
            now - Duration::minutes(30),
            "BTC",
            Some("1h"),
            "analysis",
            "1h-new",
            json!({}),
        )
        .await;
        insert_memory_at(
            &state,
            &agent_key,
            now - Duration::hours(4),
            "BTC",
            Some("1d"),
            "analysis",
            "1d-new",
            json!({}),
        )
        .await;

        let (status, body) = latest_memories_response(
            &state,
            &api_key,
            "/memories/latest?symbol=BTC&memory_type=analysis&limit=2",
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let rows = body.as_array().expect("array response");
        assert_eq!(rows.len(), 2);
        let summaries: Vec<&str> = rows
            .iter()
            .map(|row| row["summary"].as_str().unwrap())
            .collect();
        assert_eq!(summaries, vec!["15m-new", "1h-new"]);
    }

    #[tokio::test]
    async fn latest_memories_excludes_stale_analysis() {
        let state = test_state().await;
        let (agent_key, api_key) = seed_agent(&state, "latest-stale").await;
        let now = Utc::now();

        insert_memory_at(
            &state,
            &agent_key,
            now - Duration::minutes(1),
            "BTC",
            Some("15m"),
            "analysis",
            "stale",
            json!({ "stale_after": (now - Duration::seconds(1)).to_rfc3339() }),
        )
        .await;

        let (status, body) = latest_memories_response(
            &state,
            &api_key,
            "/memories/latest?symbol=BTC&memory_type=analysis",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!([]));
    }

    #[tokio::test]
    async fn latest_memories_uses_analysis_timeframe_defaults() {
        let state = test_state().await;
        let (agent_key, api_key) = seed_agent(&state, "latest-defaults").await;
        let now = Utc::now();

        insert_memory_at(
            &state,
            &agent_key,
            now - Duration::hours(37),
            "BTC",
            Some("1d"),
            "analysis",
            "expired-by-default",
            json!({}),
        )
        .await;

        let (status, body) = latest_memories_response(
            &state,
            &api_key,
            "/memories/latest?symbol=BTC&memory_type=analysis",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!([]));
    }

    #[tokio::test]
    async fn latest_memories_filters_by_memory_type() {
        let state = test_state().await;
        let (agent_key, api_key) = seed_agent(&state, "latest-type").await;
        let now = Utc::now();

        insert_memory_at(
            &state,
            &agent_key,
            now - Duration::minutes(4),
            "BTC",
            Some("15m"),
            "analysis",
            "analysis-row",
            json!({}),
        )
        .await;
        insert_memory_at(
            &state,
            &agent_key,
            now - Duration::minutes(1),
            "BTC",
            Some("15m"),
            "reflection",
            "reflection-row",
            json!({}),
        )
        .await;

        let (status, body) = latest_memories_response(
            &state,
            &api_key,
            "/memories/latest?symbol=BTC&memory_type=analysis",
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let rows = body.as_array().expect("array response");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["summary"], "analysis-row");
        assert_eq!(rows[0]["memory_type"], "analysis");
    }

    #[tokio::test]
    async fn latest_memories_excludes_null_timeframe() {
        let state = test_state().await;
        let (agent_key, api_key) = seed_agent(&state, "latest-null-tf").await;

        insert_memory_at(
            &state,
            &agent_key,
            Utc::now(),
            "BTC",
            None,
            "analysis",
            "general",
            json!({}),
        )
        .await;

        let (status, body) = latest_memories_response(
            &state,
            &api_key,
            "/memories/latest?symbol=BTC&memory_type=analysis",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!([]));
    }

    #[tokio::test]
    async fn latest_memories_requires_symbol_and_memory_type() {
        let state = test_state().await;
        let (_agent_key, api_key) = seed_agent(&state, "latest-validate").await;

        let (status, body) = latest_memories_response(&state, &api_key, "/memories/latest").await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            body["error"]
                .as_str()
                .unwrap()
                .contains("symbol is required")
        );
        assert!(
            body["error"]
                .as_str()
                .unwrap()
                .contains("memory_type is required")
        );

        let (status, body) = latest_memories_response(
            &state,
            &api_key,
            "/memories/latest?symbol=%20%20&memory_type=%20%20",
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            body["error"]
                .as_str()
                .unwrap()
                .contains("symbol is required")
        );
        assert!(
            body["error"]
                .as_str()
                .unwrap()
                .contains("memory_type is required")
        );
    }

    #[tokio::test]
    async fn latest_memories_rejects_invalid_limit() {
        let state = test_state().await;
        let (_agent_key, api_key) = seed_agent(&state, "latest-bad-limit").await;

        for uri in [
            "/memories/latest?symbol=BTC&memory_type=analysis&limit=0",
            "/memories/latest?symbol=BTC&memory_type=analysis&limit=-1",
            "/memories/latest?symbol=BTC&memory_type=analysis&limit=abc",
        ] {
            let (status, body) = latest_memories_response(&state, &api_key, uri).await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert_eq!(body["error"], "limit must be an integer >= 1");
        }
    }

    #[tokio::test]
    async fn latest_memories_is_scoped_to_authenticated_agent() {
        let state = test_state().await;
        let (agent_a_key, agent_a_api_key) = seed_agent(&state, "latest-scope-a").await;
        let (agent_b_key, _agent_b_api_key) = seed_agent(&state, "latest-scope-b").await;
        let now = Utc::now();

        insert_memory_at(
            &state,
            &agent_a_key,
            now - Duration::minutes(2),
            "BTC",
            Some("15m"),
            "analysis",
            "agent-a",
            json!({}),
        )
        .await;
        insert_memory_at(
            &state,
            &agent_b_key,
            now - Duration::minutes(1),
            "BTC",
            Some("15m"),
            "analysis",
            "agent-b",
            json!({}),
        )
        .await;

        let (status, body) = latest_memories_response(
            &state,
            &agent_a_api_key,
            "/memories/latest?symbol=BTC&memory_type=analysis",
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let rows = body.as_array().expect("array response");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["summary"], "agent-a");
        assert_eq!(rows[0]["agent_key"], agent_a_key);
    }

    #[tokio::test]
    async fn get_memory_by_id_returns_200_or_404() {
        let state = test_state().await;

        let (_agent_key, api_key) = seed_agent(&state, "get").await;
        let (_other_key, other_api_key) = seed_agent(&state, "get-other").await;

        let body = serde_json::json!({
            "symbol": "BTC",
            "timeframe": "1h",
            "memory_type": "plan",
            "summary": "x",
            "content": "y"
        });
        let (headers, body) = json_body(&body);
        let mut builder = Request::builder()
            .method("POST")
            .uri("/memories")
            .header("authorization", format!("Bearer {api_key}"));
        if let Some((k, v)) = headers {
            builder = builder.header(k, v);
        }
        let response = app(Arc::clone(&state))
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let body_bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let created: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        let id = created["id"].as_str().unwrap().to_string();

        // Owner can fetch.
        let request = Request::builder()
            .uri(format!("/memories/{id}"))
            .header("authorization", format!("Bearer {api_key}"))
            .body(Body::empty())
            .unwrap();
        let response = app(Arc::clone(&state)).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body_bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let fetched: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(fetched["id"], created["id"]);

        // Another agent gets 404 (do not leak existence).
        let request = Request::builder()
            .uri(format!("/memories/{id}"))
            .header("authorization", format!("Bearer {other_api_key}"))
            .body(Body::empty())
            .unwrap();
        let response = app(Arc::clone(&state)).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        // Invalid UUID -> 404.
        let request = Request::builder()
            .uri("/memories/not-a-uuid")
            .header("authorization", format!("Bearer {api_key}"))
            .body(Body::empty())
            .unwrap();
        let response = app(Arc::clone(&state)).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn get_account_returns_200_with_unavailable_data_when_orchestrator_has_no_snapshot() {
        let state = test_state().await;

        let (agent_key, api_key) = seed_agent(&state, "acct-empty").await;

        let request = Request::builder()
            .uri("/account")
            .header("authorization", format!("Bearer {api_key}"))
            .body(Body::empty())
            .unwrap();
        let response = app(Arc::clone(&state)).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body_bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(body["agent_key"], serde_json::Value::from(agent_key));
        assert_eq!(body["account_data"]["available"], false);
        assert_eq!(body["account_data"]["stale"], true);
        assert!(body["account_data"]["as_of"].is_null());
        assert!(body["balance"].is_null());
        assert_eq!(body["open_positions"], json!([]));
        assert_eq!(body["open_orders"], json!([]));
        assert!(body.get("connected").is_none());
        assert!(body.get("state").is_none());
    }

    #[tokio::test]
    async fn get_account_returns_fresh_account_contract_after_live_state_seeded() {
        let state = test_state().await;

        let (agent_key, api_key) = seed_agent(&state, "acct-live").await;
        let row = get_agent(&state.db_pool, &agent_key)
            .await
            .unwrap()
            .expect("present");
        let key = AccountKey::new(&row.wallet_address, &row.environment);
        state.live_accounts.replace(
            key,
            AccountLiveState {
                account_address: row.wallet_address.clone(),
                environment: row.environment.clone(),
                updated_at: Some(Utc::now()),
                margin: Some(LiveMarginState {
                    account_value: Some(dec!(100)),
                    withdrawable: Some(dec!(75)),
                    total_margin_used: Some(dec!(25)),
                    ..Default::default()
                }),
                ..Default::default()
            },
        );

        let request = Request::builder()
            .uri("/account")
            .header("authorization", format!("Bearer {api_key}"))
            .body(Body::empty())
            .unwrap();
        let response = app(Arc::clone(&state)).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body_bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(body["agent_key"], serde_json::Value::from(agent_key));
        assert_eq!(
            body["account_address"],
            serde_json::Value::from(row.wallet_address)
        );
        assert_eq!(
            body["environment"],
            serde_json::Value::from(row.environment)
        );
        assert_eq!(body["account_data"]["available"], true);
        assert_eq!(body["account_data"]["stale"], false);
        assert!(body["account_data"]["as_of"].is_string());
        assert_eq!(body["balance"]["exchange"], "hyperliquid");
        assert_eq!(body["balance"]["model"], "unified_cross_margin");
        assert_eq!(body["balance"]["available_to_trade_usd"], "75");
        assert_eq!(body["open_positions"], json!([]));
        assert_eq!(body["open_orders"], json!([]));
        assert!(body.get("connected").is_none());
        assert!(body.get("state").is_none());
    }

    #[tokio::test]
    async fn get_account_without_auth_returns_401_json() {
        let state = test_state().await;

        let request = Request::builder()
            .uri("/account")
            .body(Body::empty())
            .unwrap();
        let response = app(Arc::clone(&state)).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let body_bytes = axum::body::to_bytes(response.into_body(), 16 * 1024)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert!(body["error"].is_string());
    }

    #[tokio::test]
    async fn get_account_with_invalid_bearer_returns_401_json() {
        let state = test_state().await;

        let request = Request::builder()
            .uri("/account")
            .header("authorization", "Bearer vta_does-not-exist")
            .body(Body::empty())
            .unwrap();
        let response = app(Arc::clone(&state)).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn get_job_context_analysis_returns_prompt_without_account() {
        let state = test_state().await;

        let (agent_key, api_key) = seed_agent_with_prompts(
            &state,
            "job-analysis",
            "Focus on 15m structure and volatility.",
            "Trade only confirmed setups.",
        )
        .await;

        let request = Request::builder()
            .uri("/job-context?job_kind=analysis")
            .header("authorization", format!("Bearer {api_key}"))
            .body(Body::empty())
            .unwrap();
        let response = app(Arc::clone(&state)).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body_bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(body["agent_key"], agent_key);
        assert_eq!(body["job_kind"], "analysis");
        assert_eq!(body["prompt"], "Focus on 15m structure and volatility.");
        assert!(body["account"].is_null());

        let stored = get_agent(&state.db_pool, &agent_key)
            .await
            .unwrap()
            .expect("present");
        assert!(stored.analysis_context_last_used_at.is_some());
        assert!(stored.trading_context_last_used_at.is_none());
    }

    #[tokio::test]
    async fn get_job_context_trading_returns_prompt_and_account() {
        let state = test_state().await;

        let (agent_key, api_key) = seed_agent_with_prompts(
            &state,
            "job-trading",
            "Analyze first.",
            "Manage risk tightly and protect open positions.",
        )
        .await;
        let row = get_agent(&state.db_pool, &agent_key)
            .await
            .unwrap()
            .expect("present");
        let key = AccountKey::new(&row.wallet_address, &row.environment);
        state.live_accounts.replace(
            key,
            AccountLiveState {
                account_address: row.wallet_address.clone(),
                environment: row.environment.clone(),
                updated_at: Some(Utc::now()),
                ..Default::default()
            },
        );

        let request = Request::builder()
            .uri("/job-context?job_kind=trading")
            .header("authorization", format!("Bearer {api_key}"))
            .body(Body::empty())
            .unwrap();
        let response = app(Arc::clone(&state)).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body_bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(body["agent_key"], agent_key);
        assert_eq!(body["job_kind"], "trading");
        assert_eq!(
            body["prompt"],
            "Manage risk tightly and protect open positions."
        );
        assert_eq!(body["account"]["agent_key"], body["agent_key"]);
        assert_eq!(body["account"]["environment"], "live");
        assert!(body["account"].get("state").is_none());
        assert!(body["account"].get("connected").is_none());
        assert_eq!(body["account"]["account_data"]["available"], true);
        assert_eq!(body["account"]["balance"]["model"], "unified_cross_margin");
        assert!(body["account"]["open_positions"].is_array());
        assert!(body["account"]["open_orders"].is_array());

        let stored = get_agent(&state.db_pool, &agent_key)
            .await
            .unwrap()
            .expect("present");
        assert!(stored.analysis_context_last_used_at.is_none());
        assert!(stored.trading_context_last_used_at.is_some());
    }

    #[tokio::test]
    async fn get_job_context_trading_uses_spot_collateral_when_margin_account_value_is_zero() {
        let state = test_state().await;
        let (agent_key, api_key) = seed_agent(&state, "job-spot-collateral").await;
        let row = get_agent(&state.db_pool, &agent_key)
            .await
            .unwrap()
            .expect("present");
        let key = AccountKey::new(&row.wallet_address, &row.environment);
        state.live_accounts.replace(
            key,
            AccountLiveState {
                account_address: row.wallet_address.clone(),
                environment: row.environment.clone(),
                updated_at: Some(Utc::now()),
                margin: Some(LiveMarginState {
                    account_value: Some(dec!(0)),
                    withdrawable: Some(dec!(0)),
                    total_margin_used: Some(dec!(0)),
                    ..Default::default()
                }),
                spot_balances: vec![
                    LiveSpotBalance {
                        coin: "USDC".to_string(),
                        total: Some(dec!(222.922072)),
                        available: Some(dec!(222.922072)),
                        ..Default::default()
                    },
                    LiveSpotBalance {
                        coin: "USDE".to_string(),
                        total: Some(dec!(0)),
                        available: Some(dec!(0)),
                        ..Default::default()
                    },
                    LiveSpotBalance {
                        coin: "USDT0".to_string(),
                        total: Some(dec!(0)),
                        available: Some(dec!(0)),
                        ..Default::default()
                    },
                    LiveSpotBalance {
                        coin: "USDH".to_string(),
                        total: Some(dec!(0)),
                        available: Some(dec!(0)),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
        );

        let (status, body) =
            get_json_response(&state, &api_key, "/job-context?job_kind=trading").await;
        assert_eq!(status, StatusCode::OK);
        let account = &body["account"];
        assert_eq!(account["account_data"]["available"], true);
        assert_eq!(account["account_data"]["stale"], false);
        assert_eq!(account["balance"]["model"], "unified_cross_margin");
        assert_eq!(account["balance"]["available_to_trade_usd"], "222.922072");
        assert_eq!(account["balance"]["total_equity_usd"], "222.922072");
        assert_eq!(
            account["balance"]["collateral_balances"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            account["balance"]["collateral_balances"][0],
            json!({ "asset": "USDC", "total": "222.922072", "available": "222.922072" })
        );
        assert!(account.get("state").is_none());
        assert!(account.get("margin").is_none());
        assert!(account.get("spot_balances").is_none());
    }

    #[tokio::test]
    async fn get_job_context_trading_returns_unavailable_contract_for_stale_snapshot() {
        let state = test_state().await;
        let (agent_key, api_key) = seed_agent(&state, "job-stale-account").await;
        let row = get_agent(&state.db_pool, &agent_key)
            .await
            .unwrap()
            .expect("present");
        let key = AccountKey::new(&row.wallet_address, &row.environment);
        state.live_accounts.replace(
            key,
            AccountLiveState {
                account_address: row.wallet_address.clone(),
                environment: row.environment.clone(),
                updated_at: Some(Utc::now() - super::ACCOUNT_DATA_MAX_AGE - Duration::seconds(1)),
                margin: Some(LiveMarginState {
                    account_value: Some(dec!(100)),
                    withdrawable: Some(dec!(100)),
                    ..Default::default()
                }),
                ..Default::default()
            },
        );

        let (status, body) =
            get_json_response(&state, &api_key, "/job-context?job_kind=trading").await;
        assert_eq!(status, StatusCode::OK);
        let account = &body["account"];
        assert_eq!(account["account_data"]["available"], false);
        assert_eq!(account["account_data"]["stale"], true);
        assert!(account["account_data"]["as_of"].is_null());
        assert!(account["balance"].is_null());
        assert_eq!(account["open_positions"], json!([]));
        assert_eq!(account["open_orders"], json!([]));
    }

    #[tokio::test]
    async fn get_job_context_trading_preserves_full_open_positions_and_orders() {
        let state = test_state().await;
        let (agent_key, api_key) = seed_agent(&state, "job-pos-orders").await;
        let row = get_agent(&state.db_pool, &agent_key)
            .await
            .unwrap()
            .expect("present");
        let key = AccountKey::new(&row.wallet_address, &row.environment);
        state.live_accounts.replace(
            key,
            AccountLiveState {
                account_address: row.wallet_address.clone(),
                environment: row.environment.clone(),
                updated_at: Some(Utc::now()),
                open_positions: vec![LivePosition {
                    coin: "BTC".to_string(),
                    szi: Some(dec!(0.25)),
                    entry_px: Some(dec!(65000)),
                    unrealized_pnl: Some(dec!(12.5)),
                    liquidation_px: Some(dec!(50000)),
                    margin_used: Some(dec!(250)),
                    position_value: Some(dec!(16250)),
                    leverage_type: Some("cross".to_string()),
                    leverage_value: Some(3),
                    ..Default::default()
                }],
                open_orders: vec![LiveOpenOrder {
                    coin: "BTC".to_string(),
                    side: Some("A".to_string()),
                    limit_px: Some(dec!(70000)),
                    sz: Some(dec!(0.1)),
                    orig_sz: Some(dec!(0.1)),
                    oid: Some("12345".to_string()),
                    cloid: Some("0xabc".to_string()),
                    order_type: Some("Limit".to_string()),
                    tif: Some("Gtc".to_string()),
                    reduce_only: Some(true),
                    is_trigger: Some(true),
                    trigger_px: Some(dec!(69000)),
                    trigger_condition: Some("Price above".to_string()),
                    is_position_tpsl: Some(true),
                    ..Default::default()
                }],
                ..Default::default()
            },
        );

        let (status, body) =
            get_json_response(&state, &api_key, "/job-context?job_kind=trading").await;
        assert_eq!(status, StatusCode::OK);
        let position = &body["account"]["open_positions"][0];
        assert_eq!(position["coin"], "BTC");
        assert_eq!(position["szi"], "0.25");
        assert_eq!(position["entry_px"], "65000");
        assert_eq!(position["unrealized_pnl"], "12.5");
        assert_eq!(position["liquidation_px"], "50000");
        assert_eq!(position["margin_used"], "250");
        assert_eq!(position["position_value"], "16250");
        assert_eq!(position["leverage_type"], "cross");
        assert_eq!(position["leverage_value"], 3);

        let order = &body["account"]["open_orders"][0];
        assert_eq!(order["coin"], "BTC");
        assert_eq!(order["side"], "A");
        assert_eq!(order["limit_px"], "70000");
        assert_eq!(order["sz"], "0.1");
        assert_eq!(order["orig_sz"], "0.1");
        assert_eq!(order["oid"], "12345");
        assert_eq!(order["cloid"], "0xabc");
        assert_eq!(order["order_type"], "Limit");
        assert_eq!(order["tif"], "Gtc");
        assert_eq!(order["reduce_only"], true);
        assert_eq!(order["is_trigger"], true);
        assert_eq!(order["trigger_px"], "69000");
        assert_eq!(order["trigger_condition"], "Price above");
        assert_eq!(order["is_position_tpsl"], true);
    }

    #[tokio::test]
    async fn get_job_context_unknown_kind_returns_422_json() {
        let state = test_state().await;

        let (_agent_key, api_key) = seed_agent(&state, "job-bad-kind").await;

        let request = Request::builder()
            .uri("/job-context?job_kind=bogus")
            .header("authorization", format!("Bearer {api_key}"))
            .body(Body::empty())
            .unwrap();
        let response = app(Arc::clone(&state)).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

        let body_bytes = axum::body::to_bytes(response.into_body(), 16 * 1024)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert!(body["error"].as_str().unwrap().contains("job_kind"));

        let stored = get_agent(&state.db_pool, &_agent_key)
            .await
            .unwrap()
            .expect("present");
        assert!(stored.analysis_context_last_used_at.is_none());
        assert!(stored.trading_context_last_used_at.is_none());
    }

    #[tokio::test]
    async fn get_job_context_without_auth_returns_401_json() {
        let state = test_state().await;

        let request = Request::builder()
            .uri("/job-context?job_kind=analysis")
            .body(Body::empty())
            .unwrap();
        let response = app(state).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn get_job_context_without_auth_does_not_touch_checkins() {
        let state = test_state().await;

        let (agent_key, _api_key) = seed_agent(&state, "job-no-auth-touch").await;

        let request = Request::builder()
            .uri("/job-context?job_kind=analysis")
            .body(Body::empty())
            .unwrap();
        let response = app(Arc::clone(&state)).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let stored = get_agent(&state.db_pool, &agent_key)
            .await
            .unwrap()
            .expect("present");
        assert!(stored.analysis_context_last_used_at.is_none());
        assert!(stored.trading_context_last_used_at.is_none());
    }

    #[tokio::test]
    async fn get_job_context_is_scoped_to_calling_agent() {
        let state = test_state().await;

        let (agent_a_key, agent_a_api_key) =
            seed_agent_with_prompts(&state, "job-scope-a", "A analysis", "A trading").await;
        let (_agent_b_key, agent_b_api_key) =
            seed_agent_with_prompts(&state, "job-scope-b", "B analysis", "B trading").await;

        let request = Request::builder()
            .uri("/job-context?job_kind=analysis")
            .header("authorization", format!("Bearer {agent_b_api_key}"))
            .body(Body::empty())
            .unwrap();
        let response = app(Arc::clone(&state)).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body_bytes = axum::body::to_bytes(response.into_body(), 16 * 1024)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_ne!(agent_a_api_key, agent_b_api_key);
        assert_ne!(body["agent_key"], agent_a_key);
        assert_eq!(body["prompt"], "B analysis");
    }

    // ---- orders endpoint tests -----------------------------------------

    #[tokio::test]
    async fn post_orders_without_auth_returns_401_json() {
        let state = test_state().await;

        let body = serde_json::json!({
            "orders": [{
                "symbol": "BTC",
                "side": "buy",
                "order_type": "limit",
                "size": "0.1",
                "price": "50000"
            }]
        });
        let (headers, body) = json_body(&body);
        let mut builder = Request::builder().method("POST").uri("/orders");
        if let Some((k, v)) = headers {
            builder = builder.header(k, v);
        }
        let response = app(Arc::clone(&state))
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let body_bytes = axum::body::to_bytes(response.into_body(), 16 * 1024)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert!(body["error"].is_string());
    }

    #[tokio::test]
    async fn post_orders_validation_error_returns_422() {
        let state = test_state().await;

        let (_agent_key, api_key) = seed_agent(&state, "ord-val").await;

        // Missing price on a limit order.
        let body = serde_json::json!({
            "orders": [{
                "symbol": "BTC",
                "side": "buy",
                "order_type": "limit",
                "size": "0.1"
            }]
        });
        let (headers, body) = json_body(&body);
        let mut builder = Request::builder()
            .method("POST")
            .uri("/orders")
            .header("authorization", format!("Bearer {api_key}"));
        if let Some((k, v)) = headers {
            builder = builder.header(k, v);
        }
        let response = app(Arc::clone(&state))
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body_bytes = axum::body::to_bytes(response.into_body(), 16 * 1024)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert!(
            body["error"].as_str().unwrap().contains("price"),
            "got: {body}"
        );
    }

    #[tokio::test]
    async fn post_orders_empty_list_returns_422() {
        let state = test_state().await;

        let (_agent_key, api_key) = seed_agent(&state, "ord-empty").await;

        let body = serde_json::json!({ "orders": [] });
        let (headers, body) = json_body(&body);
        let mut builder = Request::builder()
            .method("POST")
            .uri("/orders")
            .header("authorization", format!("Bearer {api_key}"));
        if let Some((k, v)) = headers {
            builder = builder.header(k, v);
        }
        let response = app(Arc::clone(&state))
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn get_order_by_id_returns_404_for_other_agent() {
        let state = test_state().await;

        let (owner_key, owner_api) = seed_agent(&state, "ord-own").await;
        let (_other_key, other_api) = seed_agent(&state, "ord-other").await;

        // Insert a fake order directly via the store.
        use crate::hyperliquid::orders::store as orders_store;
        use rust_decimal_macros::dec;
        use serde_json::json;
        let id = uuid::Uuid::new_v4();
        orders_store::insert_order(
            &state.db_pool,
            &orders_store::NewOrder {
                id,
                agent_key: owner_key.clone(),
                account_address: "0xtest".to_string(),
                environment: "live".to_string(),
                group_id: None,
                parent_cloid: None,
                memory_record_ids: json!([]),
                symbol: "BTC".to_string(),
                instrument_id: None,
                side: "buy".to_string(),
                order_kind: "limit".to_string(),
                reduce_only: false,
                requested_price: Some(dec!(50000)),
                rounded_price: Some(dec!(50000)),
                requested_size: dec!(0.1),
                rounded_size: Some(dec!(0.1)),
                trigger_price: None,
                time_in_force: Some("gtc".to_string()),
                cloid: "0xcloid_owner".to_string(),
                status: "resting".to_string(),
                status_detail: None,
                request_payload: json!({}),
            },
        )
        .await
        .expect("insert order");

        // Owner can fetch.
        let request = Request::builder()
            .uri(format!("/orders/{id}"))
            .header("authorization", format!("Bearer {owner_api}"))
            .body(Body::empty())
            .unwrap();
        let response = app(Arc::clone(&state)).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // Other agent gets 404.
        let request = Request::builder()
            .uri(format!("/orders/{id}"))
            .header("authorization", format!("Bearer {other_api}"))
            .body(Body::empty())
            .unwrap();
        let response = app(Arc::clone(&state)).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn list_orders_returns_only_callers_orders() {
        let state = test_state().await;

        let (a_key, a_api) = seed_agent(&state, "lst-a").await;
        let (_b_key, _b_api) = seed_agent(&state, "lst-b").await;

        use crate::hyperliquid::orders::store as orders_store;
        use rust_decimal_macros::dec;
        use serde_json::json;
        for (i, cloid) in ["cloid-a-1", "cloid-a-2"].iter().enumerate() {
            orders_store::insert_order(
                &state.db_pool,
                &orders_store::NewOrder {
                    id: uuid::Uuid::new_v4(),
                    agent_key: a_key.clone(),
                    account_address: "0xa".to_string(),
                    environment: "live".to_string(),
                    group_id: None,
                    parent_cloid: None,
                    memory_record_ids: json!([]),
                    symbol: "BTC".to_string(),
                    instrument_id: None,
                    side: "buy".to_string(),
                    order_kind: "limit".to_string(),
                    reduce_only: false,
                    requested_price: Some(dec!(50000)),
                    rounded_price: Some(dec!(50000)),
                    requested_size: dec!(0.1),
                    rounded_size: Some(dec!(0.1)),
                    trigger_price: None,
                    time_in_force: Some("gtc".to_string()),
                    cloid: cloid.to_string(),
                    status: if i == 0 {
                        "resting".to_string()
                    } else {
                        "filled".to_string()
                    },
                    status_detail: None,
                    request_payload: json!({}),
                },
            )
            .await
            .expect("insert order");
        }

        let request = Request::builder()
            .uri("/orders")
            .header("authorization", format!("Bearer {a_api}"))
            .body(Body::empty())
            .unwrap();
        let response = app(Arc::clone(&state)).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body_bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let rows: Vec<serde_json::Value> = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r["symbol"] == "BTC"));
    }
}
