//! Execution gateway: orchestrates signing, DB writes, and the live
//! Hyperliquid exchange. The real `ExchangeClient` is constructed in the
//! API handler; the gateway only ever sees a `&dyn ExchangeClient` so
//! orchestration is testable offline with a fake.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use alloy::primitives::{Address, B128};
use anyhow::{Context, Result};
use chrono::Utc;
use hypersdk::hypercore::{
    self, Cloid, PrivateKeySigner,
    types::{
        BatchCancel, BatchOrder, Builder, Cancel, OrderGrouping, OrderRequest, OrderResponseStatus,
        OrderTypePlacement, TimeInForce, TpSl,
    },
};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    db::DbPool,
    hyperliquid::builder_fee::{BuilderFeeCache, MAX_BUILDER_FEE_TENTHS_OF_BP},
    hyperliquid::orders::{
        model::{
            CancelInput, CancelOrdersRequest, OrderResult, PlaceOrderInput, PlaceOrdersRequest,
            PlaceOrdersResponse,
        },
        rounding::{DEFAULT_MARKET_SLIPPAGE_BPS, market_guard_price, round_price, round_size},
        store::{NewOrder, OrderOutcome, find_order_by_oid, insert_order, update_order_outcome},
    },
};

/// Alias for a heap-pinned, `Send` future. Lets the trait be dyn-safe
/// without pulling in the `async-trait` crate (which would force `'static`
/// lifetimes and break borrows in tests).
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub use crate::hyperliquid::builder_fee::BUILDER_RECIPIENT;

// ---- exchange trait -------------------------------------------------------

/// Abstraction over the live Hyperliquid exchange. The gateway takes
/// `&dyn ExchangeClient` so the orchestration can be tested offline with
/// a fake. Error type is a plain `String` so the trait is dyn-safe.
pub trait ExchangeClient: Send + Sync {
    fn place<'a>(
        &'a self,
        batch: BatchOrder,
        nonce: u64,
        trading_account: &'a str,
    ) -> BoxFuture<'a, Result<Vec<OrderResponseStatus>, String>>;
    fn cancel<'a>(
        &'a self,
        batch: BatchCancel,
        nonce: u64,
        trading_account: &'a str,
    ) -> BoxFuture<'a, Result<Vec<OrderResponseStatus>, String>>;
    fn all_mids<'a>(&'a self) -> BoxFuture<'a, Result<HashMap<String, Decimal>, String>>;
    fn open_orders<'a>(
        &'a self,
        trading_account: &'a str,
    ) -> BoxFuture<'a, Result<Vec<CancelInput>, String>>;
    fn positions<'a>(
        &'a self,
        trading_account: &'a str,
    ) -> BoxFuture<'a, Result<Vec<ExchangePosition>, String>>;
}

/// Authoritative exchange position used only for operator-requested exits.
#[derive(Debug, Clone)]
pub struct ExchangePosition {
    pub symbol: String,
    pub szi: Decimal,
}

// ---- real implementation --------------------------------------------------

/// Real Hyperliquid exchange client. Holds a pre-built signer and HTTP
/// client. Constructed by the API handler after loading and decrypting the
/// agent's private key.
pub struct HyperliquidExchange {
    signer: PrivateKeySigner,
    client: hypercore::HttpClient,
}

impl HyperliquidExchange {
    pub fn new(signer: PrivateKeySigner, client: hypercore::HttpClient) -> Self {
        Self { signer, client }
    }
}

impl ExchangeClient for HyperliquidExchange {
    fn place<'a>(
        &'a self,
        batch: BatchOrder,
        nonce: u64,
        trading_account: &'a str,
    ) -> BoxFuture<'a, Result<Vec<OrderResponseStatus>, String>> {
        Box::pin(async move {
            self.client
                .place(
                    &self.signer,
                    batch,
                    nonce,
                    Some(
                        trading_account
                            .parse::<Address>()
                            .map_err(|e| e.to_string())?,
                    ),
                    None,
                )
                .await
                .map_err(|e| e.to_string())
        })
    }

    fn cancel<'a>(
        &'a self,
        batch: BatchCancel,
        nonce: u64,
        trading_account: &'a str,
    ) -> BoxFuture<'a, Result<Vec<OrderResponseStatus>, String>> {
        Box::pin(async move {
            self.client
                .cancel(
                    &self.signer,
                    batch,
                    nonce,
                    Some(
                        trading_account
                            .parse::<Address>()
                            .map_err(|e| e.to_string())?,
                    ),
                    None,
                )
                .await
                .map_err(|e| e.to_string())
        })
    }

    fn all_mids<'a>(&'a self) -> BoxFuture<'a, Result<HashMap<String, Decimal>, String>> {
        Box::pin(async move { self.client.all_mids(None).await.map_err(|e| e.to_string()) })
    }

    fn open_orders<'a>(
        &'a self,
        trading_account: &'a str,
    ) -> BoxFuture<'a, Result<Vec<CancelInput>, String>> {
        Box::pin(async move {
            let account = trading_account
                .parse::<Address>()
                .map_err(|e| e.to_string())?;
            self.client
                .open_orders(account, None)
                .await
                .map_err(|e| e.to_string())
                .map(|orders| {
                    orders
                        .into_iter()
                        .map(|order| CancelInput {
                            symbol: order.coin,
                            oid: order.oid,
                        })
                        .collect()
                })
        })
    }

    fn positions<'a>(
        &'a self,
        trading_account: &'a str,
    ) -> BoxFuture<'a, Result<Vec<ExchangePosition>, String>> {
        Box::pin(async move {
            let account = trading_account
                .parse::<Address>()
                .map_err(|e| e.to_string())?;
            self.client
                .clearinghouse_state(account, None)
                .await
                .map_err(|e| e.to_string())
                .map(|state| {
                    state
                        .asset_positions
                        .into_iter()
                        .map(|asset| ExchangePosition {
                            symbol: asset.position.coin,
                            szi: asset.position.szi,
                        })
                        .collect()
                })
        })
    }
}

// ---- instrument lookup ----------------------------------------------------

/// Subset of `hyperliquid.instruments` the gateway needs at submit time.
#[derive(Debug, Clone)]
pub struct InstrumentMeta {
    pub instrument_id: String,
    pub asset_index: usize,
    pub price_decimals: u32,
    pub size_decimals: u32,
}

/// Load the active instrument row for a symbol. Returns `None` if the
/// symbol is unknown or the row's `asset_index` is NULL (unsupported).
pub async fn load_instrument_meta(pool: &DbPool, symbol: &str) -> Result<Option<InstrumentMeta>> {
    let row: Option<(i32, i32, i32)> = sqlx::query_as(
        "SELECT asset_index, price_decimals, size_decimals \
           FROM hyperliquid.instruments \
           WHERE instrument_id = $1 AND market_type = 'perp' AND active = true",
    )
    .bind(symbol)
    .fetch_optional(pool)
    .await
    .context("failed to load instrument")?;

    let Some((asset_index_i, price_decimals_i, size_decimals_i)) = row else {
        return Ok(None);
    };
    let Some(asset_index) = usize::try_from(asset_index_i).ok() else {
        return Ok(None);
    };
    let Some(price_decimals) = u32::try_from(price_decimals_i).ok() else {
        return Ok(None);
    };
    let Some(size_decimals) = u32::try_from(size_decimals_i).ok() else {
        return Ok(None);
    };
    Ok(Some(InstrumentMeta {
        instrument_id: symbol.to_string(),
        asset_index,
        price_decimals,
        size_decimals,
    }))
}

// ---- error type -----------------------------------------------------------

#[derive(Debug)]
pub enum GatewayError {
    Validation(String),
    Internal(anyhow::Error),
}

impl std::fmt::Display for GatewayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GatewayError::Validation(s) => write!(f, "{s}"),
            GatewayError::Internal(e) => write!(f, "{e:#}"),
        }
    }
}

impl From<anyhow::Error> for GatewayError {
    fn from(e: anyhow::Error) -> Self {
        GatewayError::Internal(e)
    }
}

struct PlaceContext<'a> {
    agent_key: &'a str,
    account_address: &'a str,
    environment: &'a str,
    require_enabled: bool,
}

// ---- place flow -----------------------------------------------------------

/// Place one or more orders through the execution gateway.
///
/// The flow for each entry input:
/// 1. validate, look up `InstrumentMeta`
/// 2. compute rounded price/size, build the entry `OrderRequest`
/// 3. build reduce-only TP/SL `OrderRequest`s if any
/// 4. INSERT one row per leg into `hyperliquid.orders` (status
///    `pending_submission`)
/// 5. POST the batch to the exchange (always with `OrderGrouping::Na`)
/// 6. ZIP the response back to each leg, update the row + append event
pub async fn place_orders(
    pool: &DbPool,
    exchange: &dyn ExchangeClient,
    builder_fee_cache: &BuilderFeeCache,
    agent_key: &str,
    account_address: &str,
    environment: &str,
    req: &PlaceOrdersRequest,
) -> Result<PlaceOrdersResponse, GatewayError> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|error| GatewayError::Internal(error.into()))?;
    let execution = crate::agents::store::lock_agent_execution_tx(&mut tx, agent_key)
        .await
        .map_err(GatewayError::Internal)?;
    let Some(execution) = execution else {
        return Err(GatewayError::Validation(
            "agent owner is unavailable".into(),
        ));
    };
    if !execution.active {
        return Err(GatewayError::Validation("agent is not active".into()));
    }
    if !execution.enabled {
        return Err(GatewayError::Validation(
            "agent is disabled; order placement is blocked".into(),
        ));
    }

    let result = place_orders_unlocked(
        pool,
        exchange,
        builder_fee_cache,
        agent_key,
        account_address,
        environment,
        req,
    )
    .await;
    tx.rollback()
        .await
        .map_err(|error| GatewayError::Internal(error.into()))?;
    result
}

async fn place_orders_unlocked(
    pool: &DbPool,
    exchange: &dyn ExchangeClient,
    builder_fee_cache: &BuilderFeeCache,
    agent_key: &str,
    account_address: &str,
    environment: &str,
    req: &PlaceOrdersRequest,
) -> Result<PlaceOrdersResponse, GatewayError> {
    req.validate().map_err(GatewayError::Validation)?;

    let mut results: Vec<OrderResult> = Vec::with_capacity(req.orders.len() * 3);

    for input in &req.orders {
        let group_result = place_one(
            pool,
            exchange,
            builder_fee_cache,
            PlaceContext {
                agent_key,
                account_address,
                environment,
                require_enabled: true,
            },
            input,
        )
        .await?;
        results.extend(group_result);
    }

    Ok(PlaceOrdersResponse { results })
}

/// Place one entry (and any attached TP/SL legs) as a single batch.
async fn place_one(
    pool: &DbPool,
    exchange: &dyn ExchangeClient,
    builder_fee_cache: &BuilderFeeCache,
    context: PlaceContext<'_>,
    input: &PlaceOrderInput,
) -> Result<Vec<OrderResult>, GatewayError> {
    input.validate().map_err(GatewayError::Validation)?;

    let owner = load_builder_fee(pool, context.agent_key, context.require_enabled).await?;
    let builder_fee = owner.saved_fee;

    let meta = load_instrument_meta(pool, &input.symbol)
        .await
        .map_err(GatewayError::Internal)?
        .ok_or_else(|| {
            GatewayError::Validation(format!("unknown or unsupported symbol '{}'", input.symbol))
        })?;

    let is_buy = input.side == "buy";

    // ---- Entry order ----
    let (rounded_price, rounded_size, tif) = match input.order_type.as_str() {
        "limit" => {
            let price = input
                .price
                .ok_or_else(|| GatewayError::Validation("limit requires price".into()))?;
            let rp = round_price(price, meta.price_decimals, is_buy);
            let rs = round_size(input.size, meta.size_decimals);
            let tif = map_tif(input.time_in_force.as_deref())?;
            (rp, rs, tif)
        }
        "market" => {
            let mids = exchange
                .all_mids()
                .await
                .map_err(|e| GatewayError::Internal(anyhow::anyhow!("all_mids failed: {e}")))?;
            let mid = mids.get(&meta.instrument_id).copied().ok_or_else(|| {
                GatewayError::Validation(format!(
                    "no mid price available for {}",
                    meta.instrument_id
                ))
            })?;
            let rp = market_guard_price(
                mid,
                is_buy,
                DEFAULT_MARKET_SLIPPAGE_BPS,
                meta.price_decimals,
            );
            let rs = round_size(input.size, meta.size_decimals);
            (rp, rs, TimeInForce::FrontendMarket)
        }
        other => {
            return Err(GatewayError::Validation(format!(
                "unsupported order_type '{other}'"
            )));
        }
    };

    let entry_cloid = Cloid::from(B128::random());
    let entry_cloid_str = format!("{:#x}", entry_cloid);

    // Allocate a group_id when the entry has at least one TP or SL leg.
    let has_tpsl = !input.take_profits.is_empty() || !input.stop_losses.is_empty();
    let group_id = if has_tpsl { Some(Uuid::new_v4()) } else { None };

    // ---- Build TP / SL legs ----
    // Each leg is reduce-only, on the OPPOSITE side of the entry, and
    // shares the same `group_id`.
    let entry_id = Uuid::new_v4();
    let mut batch_orders: Vec<OrderRequest> = Vec::new();
    let mut legs: Vec<LegSpec> = Vec::new();
    let memory_ids: Vec<String> = input.memory_record_ids.clone();

    let entry_leg = LegSpec {
        id: entry_id,
        cloid_str: entry_cloid_str.clone(),
        order_kind: match input.order_type.as_str() {
            "limit" => "limit",
            "market" => "market",
            _ => unreachable!("validated above"),
        }
        .to_string(),
        symbol: input.symbol.clone(),
        instrument_id: Some(meta.instrument_id.clone()),
        side: input.side.clone(),
        reduce_only: input.reduce_only,
        requested_price: input.price,
        rounded_price: Some(rounded_price),
        requested_size: input.size,
        rounded_size: Some(rounded_size),
        trigger_price: None,
        time_in_force: Some(tif_label(tif).to_string()),
        group_id,
        parent_cloid: None,
    };
    batch_orders.push(build_request(
        &meta,
        is_buy,
        rounded_price,
        rounded_size,
        input.reduce_only,
        tif,
        entry_cloid,
    ));
    legs.push(entry_leg);

    // Take-profit legs (limit triggers by default)
    for tp in &input.take_profits {
        let leg_cloid = Cloid::from(B128::random());
        let leg_cloid_str = format!("{:#x}", leg_cloid);
        let leg_id = Uuid::new_v4();
        let trigger_px = round_price(tp.trigger_price, meta.price_decimals, is_buy);
        let limit_px = round_price(
            tp.limit_price.unwrap_or(tp.trigger_price),
            meta.price_decimals,
            !is_buy, // conservative for the closing leg
        );
        let leg_sz = round_size(tp.size.unwrap_or(input.size), meta.size_decimals);
        let is_market = tp.limit_price.is_none();
        batch_orders.push(OrderRequest {
            asset: meta.asset_index,
            is_buy: !is_buy,
            limit_px,
            sz: leg_sz,
            reduce_only: true,
            order_type: OrderTypePlacement::Trigger {
                is_market,
                trigger_px,
                tpsl: TpSl::Tp,
            },
            cloid: leg_cloid,
        });
        legs.push(LegSpec {
            id: leg_id,
            cloid_str: leg_cloid_str,
            order_kind: "take_profit".to_string(),
            symbol: input.symbol.clone(),
            instrument_id: Some(meta.instrument_id.clone()),
            side: close_side(&input.side),
            reduce_only: true,
            requested_price: Some(tp.trigger_price),
            rounded_price: Some(trigger_px),
            requested_size: tp.size.unwrap_or(input.size),
            rounded_size: Some(leg_sz),
            trigger_price: Some(trigger_px),
            time_in_force: None,
            group_id,
            parent_cloid: Some(entry_cloid_str.clone()),
        });
    }

    // Stop-loss legs (always market-on-trigger per plan)
    for sl in &input.stop_losses {
        let leg_cloid = Cloid::from(B128::random());
        let leg_cloid_str = format!("{:#x}", leg_cloid);
        let leg_id = Uuid::new_v4();
        let trigger_px = round_price(sl.trigger_price, meta.price_decimals, is_buy);
        let limit_px = trigger_px;
        let leg_sz = round_size(sl.size.unwrap_or(input.size), meta.size_decimals);
        batch_orders.push(OrderRequest {
            asset: meta.asset_index,
            is_buy: !is_buy,
            limit_px,
            sz: leg_sz,
            reduce_only: true,
            order_type: OrderTypePlacement::Trigger {
                is_market: true,
                trigger_px,
                tpsl: TpSl::Sl,
            },
            cloid: leg_cloid,
        });
        legs.push(LegSpec {
            id: leg_id,
            cloid_str: leg_cloid_str,
            order_kind: "stop_loss".to_string(),
            symbol: input.symbol.clone(),
            instrument_id: Some(meta.instrument_id.clone()),
            side: close_side(&input.side),
            reduce_only: true,
            requested_price: Some(sl.trigger_price),
            rounded_price: Some(trigger_px),
            requested_size: sl.size.unwrap_or(input.size),
            rounded_size: Some(leg_sz),
            trigger_price: Some(trigger_px),
            time_in_force: None,
            group_id,
            parent_cloid: Some(entry_cloid_str.clone()),
        });
    }

    // ---- Insert pending_submission rows for every leg ----
    for leg in &legs {
        let new = NewOrder {
            id: leg.id,
            agent_key: context.agent_key.to_string(),
            account_address: context.account_address.to_string(),
            environment: context.environment.to_string(),
            group_id: leg.group_id,
            parent_cloid: leg.parent_cloid.clone(),
            memory_record_ids: json!(memory_ids),
            attribution_source: input.attribution_source.clone(),
            symbol: leg.symbol.clone(),
            instrument_id: leg.instrument_id.clone(),
            side: leg.side.clone(),
            order_kind: leg.order_kind.clone(),
            reduce_only: leg.reduce_only,
            requested_price: leg.requested_price,
            rounded_price: leg.rounded_price,
            requested_size: leg.requested_size,
            rounded_size: leg.rounded_size,
            trigger_price: leg.trigger_price,
            time_in_force: leg.time_in_force.clone(),
            cloid: leg.cloid_str.clone(),
            status: "pending_submission".to_string(),
            status_detail: None,
            request_payload: place_order_input_to_json(input, builder_fee),
        };
        insert_order(pool, &new)
            .await
            .map_err(GatewayError::Internal)?;
    }

    // ---- Send the batch ----
    let batch = BatchOrder {
        orders: batch_orders,
        grouping: OrderGrouping::Na,
        builder: Some(Builder {
            builder_address: BUILDER_RECIPIENT
                .parse()
                .expect("builder recipient must be a valid address"),
            fee: builder_fee,
        }),
    };
    let nonce = Utc::now().timestamp_millis() as u64;

    let statuses = match exchange.place(batch, nonce, context.account_address).await {
        Ok(statuses) => statuses,
        Err(e) => {
            // Whole-batch failure: mark every leg 'error' and append one
            // event per leg.
            let now = Utc::now();
            let err_payload = json!({"error": &e});
            let e_clone_for_results = e.clone();
            for leg in &legs {
                let outcome = OrderOutcome {
                    order_id: leg.id,
                    account_address: context.account_address.to_string(),
                    environment: context.environment.to_string(),
                    status: "error".to_string(),
                    status_detail: Some(e.clone()),
                    exchange_oid: None,
                    filled_size: None,
                    avg_fill_price: None,
                    status_timestamp: now,
                    source: "http_response".to_string(),
                    response_payload: err_payload.clone(),
                };
                update_order_outcome(pool, &outcome)
                    .await
                    .map_err(GatewayError::Internal)?;
            }
            let _ =
                reconcile_builder_fee_rejection(pool, builder_fee_cache, &owner, builder_fee, &e)
                    .await;
            return Ok(legs
                .into_iter()
                .map(|leg| OrderResult {
                    id: leg.id,
                    cloid: leg.cloid_str,
                    symbol: leg.symbol,
                    side: leg.side,
                    order_kind: leg.order_kind,
                    status: "error".to_string(),
                    exchange_oid: None,
                    group_id: leg.group_id,
                    error: Some(e_clone_for_results.clone()),
                })
                .collect());
        }
    };

    // ---- Map the response back to each leg ----
    let now = Utc::now();
    let builder_fee_rejected = statuses.iter().any(|status| {
        matches!(status, OrderResponseStatus::Error(message) if is_builder_fee_rejection(message))
    });
    let builder_fee_rejection_message = statuses.iter().find_map(|status| match status {
        OrderResponseStatus::Error(message) if is_builder_fee_rejection(message) => {
            Some(message.clone())
        }
        _ => None,
    });
    let has_success = statuses
        .iter()
        .any(|status| !matches!(status, OrderResponseStatus::Error(_)));
    let mut out: Vec<OrderResult> = Vec::with_capacity(legs.len());
    for (leg, status) in legs.into_iter().zip(statuses) {
        let (db_status, oid_opt, filled, avg_px, status_detail) = map_response_status(&status);
        let outcome = OrderOutcome {
            order_id: leg.id,
            account_address: context.account_address.to_string(),
            environment: context.environment.to_string(),
            status: db_status.clone(),
            status_detail: status_detail.clone(),
            exchange_oid: oid_opt.clone(),
            filled_size: filled,
            avg_fill_price: avg_px,
            status_timestamp: now,
            source: "http_response".to_string(),
            response_payload: response_status_to_json(&status),
        };
        update_order_outcome(pool, &outcome)
            .await
            .map_err(GatewayError::Internal)?;
        out.push(OrderResult {
            id: leg.id,
            cloid: leg.cloid_str,
            symbol: leg.symbol,
            side: leg.side,
            order_kind: leg.order_kind,
            status: db_status,
            exchange_oid: oid_opt,
            group_id: leg.group_id,
            error: status_detail,
        });
    }

    if builder_fee_rejected {
        if let Some(message) = builder_fee_rejection_message
            && let Some(remote_max) = reconcile_builder_fee_rejection(
                pool,
                builder_fee_cache,
                &owner,
                builder_fee,
                &message,
            )
            .await
            && has_success
            && !owner.approval_recorded
            && remote_max >= builder_fee
        {
            record_order_evidence(pool, builder_fee_cache, &owner, builder_fee).await;
        }
    } else if has_success {
        record_order_evidence(pool, builder_fee_cache, &owner, builder_fee).await;
    }

    // Silence "unused" warnings for the JSON helper we keep around for
    // event payloads.
    let _ = json!(memory_ids);

    Ok(out)
}

fn map_response_status(
    status: &OrderResponseStatus,
) -> (
    String,
    Option<String>,
    Option<Decimal>,
    Option<Decimal>,
    Option<String>,
) {
    match status {
        OrderResponseStatus::Success => ("submitted".to_string(), None, None, None, None),
        OrderResponseStatus::WaitingForTrigger => ("submitted".to_string(), None, None, None, None),
        OrderResponseStatus::WaitingForFill => ("submitted".to_string(), None, None, None, None),
        OrderResponseStatus::Resting { oid, .. } => (
            "resting".to_string(),
            Some(oid.to_string()),
            None,
            None,
            None,
        ),
        OrderResponseStatus::Filled {
            total_sz,
            avg_px,
            oid,
        } => (
            "filled".to_string(),
            Some(oid.to_string()),
            Some(*total_sz),
            Some(*avg_px),
            None,
        ),
        OrderResponseStatus::Error(msg) => {
            ("rejected".to_string(), None, None, None, Some(msg.clone()))
        }
    }
}

fn map_cancel_response_status(status: &OrderResponseStatus) -> (String, Option<String>) {
    match status {
        OrderResponseStatus::Success => ("canceled".to_string(), None),
        OrderResponseStatus::Error(msg) => ("error".to_string(), Some(msg.clone())),
        other => {
            let (status, _, _, _, detail) = map_response_status(other);
            (status, detail)
        }
    }
}

/// Convert an `OrderResponseStatus` into a JSON value for storage in
/// `response_payload`. The SDK type is `Deserialize`-only, so we map
/// each variant explicitly.
fn response_status_to_json(status: &OrderResponseStatus) -> Value {
    match status {
        OrderResponseStatus::Success => json!({"status": "success"}),
        OrderResponseStatus::WaitingForTrigger => json!({"status": "waiting_for_trigger"}),
        OrderResponseStatus::WaitingForFill => json!({"status": "waiting_for_fill"}),
        OrderResponseStatus::Resting { oid, cloid } => json!({
            "status": "resting",
            "oid": oid.to_string(),
            "cloid": cloid.as_ref().map(|c| format!("{c:#x}")),
        }),
        OrderResponseStatus::Filled {
            total_sz,
            avg_px,
            oid,
        } => json!({
            "status": "filled",
            "oid": oid.to_string(),
            "total_sz": total_sz.to_string(),
            "avg_px": avg_px.to_string(),
        }),
        OrderResponseStatus::Error(msg) => json!({"status": "error", "message": msg}),
    }
}

/// Convert a `PlaceOrderInput` to a JSON value for storage in
/// `request_payload`. The input type is `Deserialize`-only by design
/// (we don't want to echo arbitrary fields back to clients), so we map
/// explicitly.
fn place_order_input_to_json(input: &PlaceOrderInput, builder_fee: u32) -> Value {
    json!({
        "symbol": input.symbol,
        "side": input.side,
        "order_type": input.order_type,
        "size": input.size.to_string(),
        "price": input.price.map(|p| p.to_string()),
        "time_in_force": input.time_in_force,
        "reduce_only": input.reduce_only,
        "take_profits": input.take_profits.iter().map(trigger_input_to_json).collect::<Vec<_>>(),
        "stop_losses": input.stop_losses.iter().map(trigger_input_to_json).collect::<Vec<_>>(),
        "memory_record_ids": input.memory_record_ids,
        "builder": {"b": BUILDER_RECIPIENT, "f": builder_fee},
    })
}

/// Load the owner-scoped saved fee and the owner context needed for post-order
/// approval reconciliation. Hyperliquid approval is intentionally not checked
/// here; orders submit optimistically with the saved fee.
#[derive(Debug, sqlx::FromRow)]
struct BuilderFeeOwnerRow {
    user_id: Uuid,
    wallet_address: String,
    builder_fee_tenths_of_bp: i16,
    builder_fee_approved_at: Option<chrono::DateTime<Utc>>,
    lifecycle: String,
    enabled: bool,
    api_wallet_expires_at: Option<chrono::DateTime<Utc>>,
    api_wallet_expiry_checked_at: Option<chrono::DateTime<Utc>>,
    private_key_ciphertext: Option<Vec<u8>>,
    private_key_key_id: Option<String>,
}

#[derive(Debug)]
struct BuilderFeeOwner {
    user_id: Uuid,
    wallet_address: String,
    saved_fee: u32,
    approval_recorded: bool,
}

async fn load_builder_fee(
    pool: &DbPool,
    agent_key: &str,
    require_enabled: bool,
) -> Result<BuilderFeeOwner, GatewayError> {
    let row = sqlx::query_as::<_, BuilderFeeOwnerRow>(
        "SELECT users.id AS user_id, users.wallet_address, users.builder_fee_tenths_of_bp,
                users.builder_fee_approved_at, agents.lifecycle, agents.enabled,
                users.api_wallet_expires_at, users.api_wallet_expiry_checked_at,
                users.hyperliquid_private_key_ciphertext AS private_key_ciphertext,
                users.hyperliquid_private_key_key_id AS private_key_key_id
           FROM agents
           JOIN users ON users.id = agents.user_id
          WHERE agents.agent_key = $1",
    )
    .bind(agent_key)
    .fetch_optional(pool)
    .await
    .map_err(|error| GatewayError::Internal(error.into()))?;
    let Some(owner_row) = row else {
        return Err(GatewayError::Validation(
            "agent owner is unavailable".into(),
        ));
    };
    if owner_row.lifecycle != crate::agents::model::AGENT_LIFECYCLE_ACTIVE {
        return Err(GatewayError::Validation("agent is not active".into()));
    }
    if require_enabled && !owner_row.enabled {
        return Err(GatewayError::Validation(
            "agent is disabled; order placement is blocked".into(),
        ));
    }
    if owner_row.private_key_ciphertext.is_none()
        || owner_row.private_key_key_id.is_none()
        || owner_row
            .api_wallet_expires_at
            .is_some_and(|expires_at| expires_at <= Utc::now())
    {
        return Err(GatewayError::Validation(
            "the user's trading signer needs attention".into(),
        ));
    }
    if owner_row.api_wallet_expires_at.is_some() && owner_row.api_wallet_expiry_checked_at.is_none()
    {
        return Err(GatewayError::Validation(
            "the user's trading signer needs attention".into(),
        ));
    }
    if owner_row.builder_fee_tenths_of_bp <= 0 {
        return Err(GatewayError::Validation(
            "builder fee must be positive".into(),
        ));
    }
    let saved_fee = u32::try_from(owner_row.builder_fee_tenths_of_bp)
        .map_err(|_| GatewayError::Validation("builder fee must be positive".into()))?;
    Ok(BuilderFeeOwner {
        user_id: owner_row.user_id,
        wallet_address: owner_row.wallet_address,
        saved_fee,
        approval_recorded: owner_row.builder_fee_approved_at.is_some(),
    })
}

/// Hyperliquid has no stable numeric code for this rejection, so keep the
/// compatibility heuristic narrow and isolated from order handling.
fn is_builder_fee_rejection(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("builder")
        && message.contains("fee")
        && (message.contains("approval")
            || message.contains("approved")
            || message.contains("permission"))
}

async fn reconcile_builder_fee_rejection(
    pool: &DbPool,
    cache: &BuilderFeeCache,
    owner: &BuilderFeeOwner,
    saved_fee: u32,
    message: &str,
) -> Option<u32> {
    if !is_builder_fee_rejection(message) {
        return None;
    }
    let remote_max = match cache
        .refresh(&owner.wallet_address, BUILDER_RECIPIENT)
        .await
    {
        Ok(maximum) => maximum,
        Err(error) => {
            tracing::warn!(
                user_id = %owner.user_id,
                error = %error,
                "builder fee rejection follow-up lookup failed"
            );
            return None;
        }
    };
    if let Some(remote_fee) = remote_max
        .min(MAX_BUILDER_FEE_TENTHS_OF_BP)
        .try_into()
        .ok()
        .filter(|fee: &i16| *fee > 0)
        && let Err(error) = sqlx::query(
            "UPDATE users SET builder_fee_tenths_of_bp = $2, updated_at = now()
               WHERE id = $1",
        )
        .bind(owner.user_id)
        .bind(remote_fee)
        .execute(pool)
        .await
    {
        tracing::warn!(
            user_id = %owner.user_id,
            error = %error,
            "failed to persist remote builder fee maximum"
        );
    }
    if remote_max < saved_fee
        && let Err(error) = sqlx::query(
            "UPDATE users SET builder_fee_approved_at = NULL, updated_at = now()
               WHERE id = $1 AND builder_fee_approved_at IS NOT NULL",
        )
        .bind(owner.user_id)
        .execute(pool)
        .await
    {
        tracing::warn!(
            user_id = %owner.user_id,
            error = %error,
            "failed to clear insufficient builder fee approval"
        );
    }
    Some(remote_max)
}

async fn record_order_evidence(
    pool: &DbPool,
    cache: &BuilderFeeCache,
    owner: &BuilderFeeOwner,
    saved_fee: u32,
) {
    cache
        .record_order_evidence(&owner.wallet_address, BUILDER_RECIPIENT, saved_fee)
        .await;
    if owner.approval_recorded {
        return;
    }
    if let Err(error) = sqlx::query(
        "UPDATE users SET builder_fee_approved_at = now(), updated_at = now()
           WHERE id = $1 AND builder_fee_approved_at IS NULL",
    )
    .bind(owner.user_id)
    .execute(pool)
    .await
    {
        tracing::warn!(
            user_id = %owner.user_id,
            error = %error,
            "failed to record successful builder fee order evidence"
        );
    }
}

fn trigger_input_to_json(t: &super::model::TriggerInput) -> Value {
    json!({
        "trigger_price": t.trigger_price.to_string(),
        "limit_price": t.limit_price.map(|p| p.to_string()),
        "size": t.size.map(|s| s.to_string()),
    })
}

fn map_tif(tif: Option<&str>) -> Result<TimeInForce, GatewayError> {
    match tif.unwrap_or("gtc").to_ascii_lowercase().as_str() {
        "gtc" => Ok(TimeInForce::Gtc),
        "ioc" => Ok(TimeInForce::Ioc),
        "alo" => Ok(TimeInForce::Alo),
        other => Err(GatewayError::Validation(format!(
            "unsupported time_in_force '{other}'"
        ))),
    }
}

fn tif_label(tif: TimeInForce) -> &'static str {
    match tif {
        TimeInForce::Gtc => "gtc",
        TimeInForce::Ioc => "ioc",
        TimeInForce::Alo => "alo",
        TimeInForce::FrontendMarket => "frontend_market",
    }
}

fn close_side(side: &str) -> String {
    if side == "buy" {
        "sell".to_string()
    } else {
        "buy".to_string()
    }
}

fn build_request(
    meta: &InstrumentMeta,
    is_buy: bool,
    limit_px: Decimal,
    sz: Decimal,
    reduce_only: bool,
    tif: TimeInForce,
    cloid: Cloid,
) -> OrderRequest {
    OrderRequest {
        asset: meta.asset_index,
        is_buy,
        limit_px,
        sz,
        reduce_only,
        order_type: OrderTypePlacement::Limit { tif },
        cloid,
    }
}

/// Internal per-leg spec used while building one batch.
#[derive(Debug, Clone)]
struct LegSpec {
    id: Uuid,
    cloid_str: String,
    order_kind: String,
    symbol: String,
    instrument_id: Option<String>,
    side: String,
    reduce_only: bool,
    requested_price: Option<Decimal>,
    rounded_price: Option<Decimal>,
    requested_size: Decimal,
    rounded_size: Option<Decimal>,
    trigger_price: Option<Decimal>,
    time_in_force: Option<String>,
    group_id: Option<Uuid>,
    parent_cloid: Option<String>,
}

// ---- cancel flow ----------------------------------------------------------

/// Per-input cancel outcome surfaced to the API.
#[derive(Debug)]
pub struct CancelOutcome {
    pub symbol: String,
    pub oid: u64,
    pub status: String,
    pub error: Option<String>,
    pub order_id: Option<Uuid>,
}

/// Cancel a batch of orders by exchange `oid`.
///
/// For each input: resolve the instrument, look up the local order by
/// `oid`, ownership-check against `agent_key`, then call
/// `exchange.cancel`. Cancelling an unknown / already-gone oid is
/// reported per-item and never 500s the whole call.
pub async fn cancel_orders(
    pool: &DbPool,
    exchange: &dyn ExchangeClient,
    agent_key: &str,
    account_address: &str,
    environment: &str,
    req: &CancelOrdersRequest,
) -> Result<Vec<CancelOutcome>, GatewayError> {
    cancel_orders_inner(
        pool,
        exchange,
        agent_key,
        account_address,
        environment,
        req,
        false,
    )
    .await
}

async fn cancel_orders_inner(
    pool: &DbPool,
    exchange: &dyn ExchangeClient,
    agent_key: &str,
    account_address: &str,
    environment: &str,
    req: &CancelOrdersRequest,
    allow_untracked: bool,
) -> Result<Vec<CancelOutcome>, GatewayError> {
    let mut outcomes: Vec<CancelOutcome> = Vec::with_capacity(req.orders.len());
    let nonce = Utc::now().timestamp_millis() as u64;

    // Build a per-input decision first. We only ever add to the cancel
    // batch when (a) the symbol is known, (b) the local row exists for
    // this account+oid, and (c) the row is owned by the calling agent.
    // Otherwise the input becomes a per-item error / not_found
    // outcome and the cancel is NEVER sent to the exchange. This is
    // the security boundary: an agent cannot cancel another agent's
    // (or any unknown) order via this gateway.
    enum Decision {
        Send(Cancel),
        NotFound,
        NotOwned(Uuid),
        UnknownSymbol,
    }

    let mut decisions: Vec<Decision> = Vec::with_capacity(req.orders.len());

    for c in &req.orders {
        let meta_row = match load_instrument_meta(pool, &c.symbol)
            .await
            .map_err(GatewayError::Internal)?
        {
            Some(m) => m,
            None => {
                decisions.push(Decision::UnknownSymbol);
                continue;
            }
        };

        let local_row = find_order_by_oid(pool, account_address, environment, &c.oid.to_string())
            .await
            .map_err(GatewayError::Internal)?;

        match local_row {
            None if allow_untracked => decisions.push(Decision::Send(Cancel {
                asset: meta_row.asset_index,
                oid: c.oid,
            })),
            None => decisions.push(Decision::NotFound),
            Some(row) if row.agent_key != agent_key => decisions.push(Decision::NotOwned(row.id)),
            Some(_) => decisions.push(Decision::Send(Cancel {
                asset: meta_row.asset_index,
                oid: c.oid,
            })),
        }
    }

    // Build the batch from the decisions.
    let batch_cancels: Vec<Cancel> = decisions
        .iter()
        .filter_map(|d| match d {
            Decision::Send(c) => Some(c.clone()),
            _ => None,
        })
        .collect();

    let statuses = if batch_cancels.is_empty() {
        Vec::new()
    } else {
        exchange
            .cancel(
                BatchCancel {
                    cancels: batch_cancels.clone(),
                },
                nonce,
                account_address,
            )
            .await
            .map_err(|e| GatewayError::Internal(anyhow::anyhow!("cancel failed: {e}")))?
    };

    // Walk the inputs again; pop one status from the front for each
    // `Send` decision.
    let now = Utc::now();
    let mut status_iter = statuses.into_iter();

    for (input, decision) in req.orders.iter().zip(decisions) {
        match decision {
            Decision::UnknownSymbol => outcomes.push(CancelOutcome {
                symbol: input.symbol.clone(),
                oid: input.oid,
                status: "error".to_string(),
                error: Some(format!("unknown symbol '{}'", input.symbol)),
                order_id: None,
            }),
            Decision::NotFound => outcomes.push(CancelOutcome {
                symbol: input.symbol.clone(),
                oid: input.oid,
                status: "not_found".to_string(),
                error: None,
                order_id: None,
            }),
            Decision::NotOwned(order_id) => outcomes.push(CancelOutcome {
                symbol: input.symbol.clone(),
                oid: input.oid,
                status: "error".to_string(),
                error: Some("order not owned by this agent".to_string()),
                order_id: Some(order_id),
            }),
            Decision::Send(_) => {
                let status = status_iter.next().ok_or_else(|| {
                    GatewayError::Internal(anyhow::anyhow!(
                        "exchange returned fewer cancel statuses than inputs"
                    ))
                })?;
                let (db_status, status_detail) = map_cancel_response_status(&status);

                // Find the local row to update. We just verified it
                // exists; refetch by oid for simplicity.
                if let Some(row) =
                    find_order_by_oid(pool, account_address, environment, &input.oid.to_string())
                        .await
                        .map_err(GatewayError::Internal)?
                {
                    let outcome = OrderOutcome {
                        order_id: row.id,
                        account_address: account_address.to_string(),
                        environment: environment.to_string(),
                        status: db_status.clone(),
                        status_detail: status_detail.clone(),
                        exchange_oid: Some(input.oid.to_string()),
                        filled_size: None,
                        avg_fill_price: None,
                        status_timestamp: now,
                        source: "http_response".to_string(),
                        response_payload: response_status_to_json(&status),
                    };
                    update_order_outcome(pool, &outcome)
                        .await
                        .map_err(GatewayError::Internal)?;
                    outcomes.push(CancelOutcome {
                        symbol: input.symbol.clone(),
                        oid: input.oid,
                        status: db_status,
                        error: status_detail,
                        order_id: Some(row.id),
                    });
                } else {
                    outcomes.push(CancelOutcome {
                        symbol: input.symbol.clone(),
                        oid: input.oid,
                        status: db_status,
                        error: status_detail,
                        order_id: None,
                    });
                }
            }
        }
    }

    Ok(outcomes)
}

/// Cancel every open order for the agent, optionally restricted to one
/// symbol. Returns the per-input cancel outcomes plus the count of orders
/// considered.
pub async fn cancel_all(
    pool: &DbPool,
    exchange: &dyn ExchangeClient,
    agent_key: &str,
    account_address: &str,
    environment: &str,
    symbol: Option<&str>,
) -> Result<CancelAllSummary, GatewayError> {
    let inputs: Vec<CancelInput> = exchange
        .open_orders(account_address)
        .await
        .map_err(|error| GatewayError::Internal(anyhow::anyhow!("open_orders failed: {error}")))?
        .into_iter()
        .filter(|order| symbol.is_none_or(|requested| requested == order.symbol))
        .collect();
    let considered = inputs.len();

    if inputs.is_empty() {
        return Ok(CancelAllSummary {
            considered,
            outcomes: Vec::new(),
        });
    }

    let req = CancelOrdersRequest { orders: inputs };
    let outcomes = cancel_orders_inner(
        pool,
        exchange,
        agent_key,
        account_address,
        environment,
        &req,
        false,
    )
    .await?;
    Ok(CancelAllSummary {
        considered,
        outcomes,
    })
}

/// Cancel every order that Hyperliquid currently reports for the agent's
/// account. This server-authorized operation intentionally permits exchange
/// orders without a local journal row, which is required for emergency stop
/// and position exits to remain effective when local state is stale.
pub async fn cancel_all_exchange_orders(
    pool: &DbPool,
    exchange: &dyn ExchangeClient,
    agent_key: &str,
    account_address: &str,
    environment: &str,
    symbol: Option<&str>,
) -> Result<CancelAllSummary, GatewayError> {
    let inputs: Vec<CancelInput> = exchange
        .open_orders(account_address)
        .await
        .map_err(|error| GatewayError::Internal(anyhow::anyhow!("open_orders failed: {error}")))?
        .into_iter()
        .filter(|order| symbol.is_none_or(|requested| requested == order.symbol))
        .collect();
    let considered = inputs.len();
    if inputs.is_empty() {
        return Ok(CancelAllSummary {
            considered,
            outcomes: Vec::new(),
        });
    }

    let outcomes = cancel_orders_inner(
        pool,
        exchange,
        agent_key,
        account_address,
        environment,
        &CancelOrdersRequest { orders: inputs },
        true,
    )
    .await?;
    Ok(CancelAllSummary {
        considered,
        outcomes,
    })
}

/// Submit a server-authorized reduce-only market order for the current
/// exchange position. This is deliberately separate from the agent-facing
/// placement API, so a disabled agent can still be exited but cannot open
/// exposure.
pub async fn close_exchange_position(
    pool: &DbPool,
    exchange: &dyn ExchangeClient,
    builder_fee_cache: &BuilderFeeCache,
    agent_key: &str,
    account_address: &str,
    environment: &str,
    symbol: &str,
) -> Result<Option<Vec<OrderResult>>, GatewayError> {
    let position = exchange
        .positions(account_address)
        .await
        .map_err(|error| GatewayError::Internal(anyhow::anyhow!("positions failed: {error}")))?
        .into_iter()
        .find(|position| position.symbol == symbol && !position.szi.is_zero());
    let Some(position) = position else {
        return Ok(None);
    };
    let input = PlaceOrderInput {
        symbol: position.symbol,
        side: if position.szi.is_sign_negative() {
            "buy".to_string()
        } else {
            "sell".to_string()
        },
        order_type: "market".to_string(),
        size: position.szi.abs(),
        price: None,
        time_in_force: None,
        reduce_only: true,
        take_profits: Vec::new(),
        stop_losses: Vec::new(),
        memory_record_ids: Vec::new(),
        attribution_source: "manual".to_string(),
    };
    place_one(
        pool,
        exchange,
        builder_fee_cache,
        PlaceContext {
            agent_key,
            account_address,
            environment,
            require_enabled: false,
        },
        &input,
    )
    .await
    .map(Some)
}

/// Summary of a `cancel_all` call. `considered` is the number of open
/// orders found; `outcomes` is the per-input cancel result list.
#[derive(Debug)]
pub struct CancelAllSummary {
    pub considered: usize,
    pub outcomes: Vec<CancelOutcome>,
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::super::model;
    use rust_decimal_macros::dec;
    use tokio::sync::Mutex;

    use super::*;
    use crate::hyperliquid::builder_fee::{BuilderFeeLookup, LookupFuture};
    use crate::hyperliquid::orders::store;
    use crate::{
        agents::{
            crypto::{EncryptionKey, encrypt},
            keys::derive_wallet_address,
            model::AgentRegistryRow,
            store::insert_agent,
        },
        test_db,
    };

    // ---- fake exchange ----------------------------------------------------

    /// `OrderResponseStatus` is not `Clone`, so we store the canned
    /// responses behind a `Mutex<Option<_>>` and `take()` them on the
    /// single call each test makes. Strings and `HashMap` are `Clone`
    /// and stay as cloned values.
    #[derive(Default)]
    struct FakeExchange {
        place_statuses: Mutex<Option<Vec<OrderResponseStatus>>>,
        place_err: Mutex<Option<String>>,
        cancel_statuses: Mutex<Option<Vec<OrderResponseStatus>>>,
        cancel_err: Mutex<Option<String>>,
        mids: Mutex<HashMap<String, Decimal>>,
        last_place_batch: Mutex<Option<BatchOrder>>,
        last_cancel_batch: Mutex<Option<BatchCancel>>,
        place_call_count: Mutex<usize>,
    }

    struct TestBuilderFeeLookup {
        result: Mutex<Result<u32, String>>,
        calls: AtomicUsize,
    }

    impl BuilderFeeLookup for TestBuilderFeeLookup {
        fn max_builder_fee<'a>(&'a self, _user: &'a str, _builder: &'a str) -> LookupFuture<'a> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                self.result.lock().await.clone()
            })
        }
    }

    fn test_cache() -> BuilderFeeCache {
        configured_cache(Ok(10)).0
    }

    fn configured_cache(result: Result<u32, &str>) -> (BuilderFeeCache, Arc<TestBuilderFeeLookup>) {
        let lookup = Arc::new(TestBuilderFeeLookup {
            result: Mutex::new(result.map_err(str::to_string)),
            calls: AtomicUsize::new(0),
        });
        (BuilderFeeCache::new(lookup.clone()), lookup)
    }

    impl FakeExchange {
        async fn new() -> Self {
            Self::default()
        }

        async fn with_place_statuses(statuses: Vec<OrderResponseStatus>) -> Self {
            let me = Self::new().await;
            *me.place_statuses.lock().await = Some(statuses);
            me
        }

        async fn with_cancel_statuses(statuses: Vec<OrderResponseStatus>) -> Self {
            let me = Self::new().await;
            *me.cancel_statuses.lock().await = Some(statuses);
            me
        }

        async fn with_place_error(msg: &str) -> Self {
            let me = Self::new().await;
            *me.place_err.lock().await = Some(msg.to_string());
            me
        }
    }

    impl ExchangeClient for FakeExchange {
        fn place<'a>(
            &'a self,
            batch: BatchOrder,
            _nonce: u64,
            _trading_account: &'a str,
        ) -> BoxFuture<'a, Result<Vec<OrderResponseStatus>, String>> {
            Box::pin(async move {
                *self.place_call_count.lock().await += 1;
                *self.last_place_batch.lock().await = Some(batch);
                if let Some(err) = self.place_err.lock().await.clone() {
                    return Err(err);
                }
                Ok(self.place_statuses.lock().await.take().unwrap_or_default())
            })
        }

        fn cancel<'a>(
            &'a self,
            batch: BatchCancel,
            _nonce: u64,
            _trading_account: &'a str,
        ) -> BoxFuture<'a, Result<Vec<OrderResponseStatus>, String>> {
            Box::pin(async move {
                *self.last_cancel_batch.lock().await = Some(batch);
                if let Some(err) = self.cancel_err.lock().await.clone() {
                    return Err(err);
                }
                Ok(self.cancel_statuses.lock().await.take().unwrap_or_default())
            })
        }

        fn all_mids<'a>(&'a self) -> BoxFuture<'a, Result<HashMap<String, Decimal>, String>> {
            Box::pin(async move { Ok(self.mids.lock().await.clone()) })
        }

        fn open_orders<'a>(
            &'a self,
            _trading_account: &'a str,
        ) -> BoxFuture<'a, Result<Vec<CancelInput>, String>> {
            Box::pin(async move { Ok(Vec::new()) })
        }

        fn positions<'a>(
            &'a self,
            _trading_account: &'a str,
        ) -> BoxFuture<'a, Result<Vec<ExchangePosition>, String>> {
            Box::pin(async move { Ok(Vec::new()) })
        }
    }

    // ---- test helpers -----------------------------------------------------

    fn sample_agent(suffix: &str) -> AgentRegistryRow {
        let private_key_raw = format!("deterministic-{suffix}");
        let mut bytes = [0u8; 32];
        let raw = private_key_raw.as_bytes();
        for (i, b) in raw.iter().enumerate() {
            if i >= 32 {
                break;
            }
            bytes[i] = *b;
        }
        let private_key = format!("0x{}", hex::encode(bytes));
        let wallet = derive_wallet_address(&private_key).unwrap();
        let now = Utc::now();
        let ts = now.timestamp_millis();
        AgentRegistryRow {
            agent_key: format!("gw-test-{suffix}-{ts}"),
            user_id: crate::test_db::test_user_id(),
            created_at: now,
            updated_at: now,
            enabled: true,
            lifecycle: crate::agents::model::AGENT_LIFECYCLE_ACTIVE.to_string(),
            display_name: format!("GW Test {suffix}"),
            trading_account_address: Some(wallet),
            environment: "live".to_string(),
            api_key: format!("vta_gw-{suffix}-{ts}"),
            api_key_last_used_at: None,
            runtime_config: serde_json::json!({}),
        }
    }

    async fn seed(pool: &DbPool, suffix: &str) -> (String, String) {
        let row = sample_agent(suffix);
        let key = row.agent_key.clone();
        let acct = row
            .trading_account_address
            .clone()
            .expect("trading account");
        insert_agent(pool, &row).await.expect("insert agent");
        sqlx::query(
            "UPDATE users SET builder_fee_tenths_of_bp = 10, builder_fee_approved_at = now() WHERE id = $1",
        )
        .bind(crate::test_db::test_user_id())
        .execute(pool)
        .await
        .expect("approve builder fee");
        (key, acct)
    }

    async fn seed_instrument(pool: &DbPool, symbol: &str, asset_index: i32, sz: i32) {
        sqlx::query(
            "INSERT INTO hyperliquid.instruments
                (instrument_id, name, market_type, base_asset, quote_asset,
                 settlement_asset, asset_index, price_decimals, size_decimals,
                 lot_size, max_leverage, is_hip3, active, created_at, updated_at)
             VALUES ($1, $1, 'perp', $1, 'USDC', 'USDC', $2, $3, $4,
                     0.001, 50, false, true, now(), now())
             ON CONFLICT (instrument_id) DO NOTHING",
        )
        .bind(symbol)
        .bind(asset_index)
        .bind(6 - sz) // perp price_decimals = 6 - size_decimals
        .bind(sz)
        .execute(pool)
        .await
        .expect("seed instrument");
    }

    fn limit_buy(symbol: &str, size: Decimal, price: Decimal) -> PlaceOrderInput {
        PlaceOrderInput {
            symbol: symbol.to_string(),
            side: "buy".to_string(),
            order_type: "limit".to_string(),
            size,
            price: Some(price),
            time_in_force: Some("gtc".to_string()),
            reduce_only: false,
            take_profits: vec![],
            stop_losses: vec![],
            memory_record_ids: vec![],
            attribution_source: "agent".to_string(),
        }
    }

    // ---- tests ------------------------------------------------------------

    #[tokio::test]
    async fn place_single_limit_creates_one_row_and_returns_resting() {
        let pool = test_db::pool().await;
        let (agent_key, account) = seed(&pool, "single").await;
        seed_instrument(&pool, "BTC", 0, 5).await;

        let exchange = FakeExchange::with_place_statuses(vec![OrderResponseStatus::Resting {
            oid: 42,
            cloid: None,
        }])
        .await;

        let req = PlaceOrdersRequest {
            orders: vec![limit_buy("BTC", dec!(0.1), dec!(50000))],
        };
        let resp = place_orders(
            &pool,
            &exchange,
            &test_cache(),
            &agent_key,
            &account,
            "live",
            &req,
        )
        .await
        .expect("place_orders ok");

        assert_eq!(resp.results.len(), 1);
        let r = &resp.results[0];
        assert_eq!(r.status, "resting");
        assert_eq!(r.exchange_oid.as_deref(), Some("42"));
        assert_eq!(r.order_kind, "limit");
        assert!(r.error.is_none());

        // The DB has exactly one row for this agent, status resting.
        let stored = store::list_orders(&pool, &agent_key, None, None, None, None, None)
            .await
            .unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].status, "resting");
        assert_eq!(stored[0].exchange_oid.as_deref(), Some("42"));
        assert_eq!(stored[0].cloid, r.cloid);
        let (rounded_size,): (Option<Decimal>,) =
            sqlx::query_as("SELECT rounded_size FROM hyperliquid.orders WHERE id = $1")
                .bind(stored[0].id)
                .fetch_one(&pool)
                .await
                .expect("fetch rounded size");
        assert_eq!(rounded_size, Some(dec!(0.1)));
        let batch = exchange
            .last_place_batch
            .lock()
            .await
            .take()
            .expect("batch");
        let builder = batch.builder.expect("builder fee");
        assert_eq!(
            builder.builder_address.to_string().to_ascii_lowercase(),
            BUILDER_RECIPIENT
        );
        assert_eq!(builder.fee, 10);
        let (payload,): (Value,) =
            sqlx::query_as("SELECT request_payload FROM hyperliquid.orders WHERE id = $1")
                .bind(stored[0].id)
                .fetch_one(&pool)
                .await
                .expect("fetch request payload");
        assert_eq!(payload["builder"], json!({"b": BUILDER_RECIPIENT, "f": 10}));
    }

    #[tokio::test]
    async fn disabled_agent_cannot_submit_orders() {
        let pool = test_db::pool().await;
        let (agent_key, account) = seed(&pool, "disabled").await;
        sqlx::query("UPDATE agents SET enabled = false WHERE agent_key = $1")
            .bind(&agent_key)
            .execute(&pool)
            .await
            .expect("disable agent");
        let exchange = FakeExchange::new().await;
        let req = PlaceOrdersRequest {
            orders: vec![limit_buy("BTC", dec!(0.1), dec!(50000))],
        };

        let error = place_orders(
            &pool,
            &exchange,
            &test_cache(),
            &agent_key,
            &account,
            "live",
            &req,
        )
        .await
        .expect_err("disabled agent must be rejected");

        assert_eq!(
            error.to_string(),
            "agent is disabled; order placement is blocked"
        );
        assert_eq!(*exchange.place_call_count.lock().await, 0);
    }

    #[tokio::test]
    async fn place_limit_with_tp_and_sl_creates_three_rows_in_one_group() {
        let pool = test_db::pool().await;
        let (agent_key, account) = seed(&pool, "tpsl").await;
        seed_instrument(&pool, "BTC", 0, 5).await;

        let exchange = FakeExchange::with_place_statuses(vec![
            OrderResponseStatus::Resting {
                oid: 1,
                cloid: None,
            },
            OrderResponseStatus::Resting {
                oid: 2,
                cloid: None,
            },
            OrderResponseStatus::Resting {
                oid: 3,
                cloid: None,
            },
        ])
        .await;

        let mut input = limit_buy("BTC", dec!(0.1), dec!(50000));
        input.take_profits = vec![model::TriggerInput {
            trigger_price: dec!(55000),
            limit_price: Some(dec!(55100)),
            size: None,
        }];
        input.stop_losses = vec![model::TriggerInput {
            trigger_price: dec!(48000),
            limit_price: None,
            size: Some(dec!(0.1)),
        }];

        let req = PlaceOrdersRequest {
            orders: vec![input],
        };
        let resp = place_orders(
            &pool,
            &exchange,
            &test_cache(),
            &agent_key,
            &account,
            "live",
            &req,
        )
        .await
        .expect("place ok");
        assert_eq!(resp.results.len(), 3);

        let stored = store::list_orders(&pool, &agent_key, None, None, None, None, None)
            .await
            .unwrap();
        assert_eq!(stored.len(), 3);
        let group_ids: std::collections::HashSet<_> =
            stored.iter().filter_map(|r| r.group_id).collect();
        assert_eq!(group_ids.len(), 1, "all three legs share one group_id");

        // Find each leg by kind (DESC order by created_at means the
        // stop-loss row is the most recent, so we can't rely on
        // positional access).
        let entry = stored.iter().find(|r| r.order_kind == "limit").unwrap();
        let tp = stored
            .iter()
            .find(|r| r.order_kind == "take_profit")
            .unwrap();
        let sl = stored.iter().find(|r| r.order_kind == "stop_loss").unwrap();
        assert_eq!(entry.symbol, "BTC");
        // TP leg is on the opposite side (sell) and reduce-only.
        assert_eq!(tp.side, "sell");
        assert!(tp.reduce_only);
        // SL leg: stop-market, so limit_px == trigger_px
        assert_eq!(sl.side, "sell");
        assert!(sl.reduce_only);
        let (rounded_price, trigger_price): (Option<Decimal>, Option<Decimal>) = sqlx::query_as(
            "SELECT rounded_price, trigger_price FROM hyperliquid.orders WHERE id = $1",
        )
        .bind(sl.id)
        .fetch_one(&pool)
        .await
        .expect("fetch stop prices");
        assert_eq!(rounded_price, trigger_price);
        let batch = exchange
            .last_place_batch
            .lock()
            .await
            .take()
            .expect("batch");
        assert_eq!(batch.orders.len(), 3);
        assert_eq!(batch.builder.expect("builder fee").fee, 10);
    }

    #[tokio::test]
    async fn missing_approval_marker_does_not_block_order_submission() {
        let pool = test_db::pool().await;
        let (agent_key, account) = seed(&pool, "fee-invalid").await;
        seed_instrument(&pool, "BTC", 0, 5).await;
        let req = PlaceOrdersRequest {
            orders: vec![limit_buy("BTC", dec!(0.1), dec!(50000))],
        };

        sqlx::query("UPDATE users SET builder_fee_approved_at = NULL WHERE id = $1")
            .bind(crate::test_db::test_user_id())
            .execute(&pool)
            .await
            .expect("clear saved approval");
        let exchange = FakeExchange::with_place_statuses(vec![OrderResponseStatus::Resting {
            oid: 1,
            cloid: None,
        }])
        .await;
        let response = place_orders(
            &pool,
            &exchange,
            &test_cache(),
            &agent_key,
            &account,
            "live",
            &req,
        )
        .await
        .expect("saved fee is submitted without local approval marker");
        assert_eq!(response.results[0].status, "resting");
        assert_eq!(*exchange.place_call_count.lock().await, 1);
        let (approved_at,): (Option<chrono::DateTime<Utc>>,) =
            sqlx::query_as("SELECT builder_fee_approved_at FROM users WHERE id = $1")
                .bind(crate::test_db::test_user_id())
                .fetch_one(&pool)
                .await
                .expect("read approval evidence");
        assert!(approved_at.is_some());

        sqlx::query("UPDATE agents SET lifecycle = 'pending_funding' WHERE agent_key = $1")
            .bind(&agent_key)
            .execute(&pool)
            .await
            .expect("deactivate agent");
        let inactive = FakeExchange::new().await;
        assert!(
            place_orders(
                &pool,
                &inactive,
                &test_cache(),
                &agent_key,
                &account,
                "live",
                &req
            )
            .await
            .is_err()
        );
        assert_eq!(*inactive.place_call_count.lock().await, 0);
    }

    #[tokio::test]
    async fn builder_rejection_refreshes_once_and_clears_insufficient_approval() {
        let pool = test_db::pool().await;
        let (agent_key, account) = seed(&pool, "fee-rejected").await;
        seed_instrument(&pool, "BTC", 0, 5).await;
        let exchange = FakeExchange::with_place_statuses(vec![OrderResponseStatus::Error(
            "builder fee approval rejected".to_string(),
        )])
        .await;
        let (cache, lookup) = configured_cache(Ok(9));
        let req = PlaceOrdersRequest {
            orders: vec![limit_buy("BTC", dec!(0.1), dec!(50000))],
        };

        let response = place_orders(&pool, &exchange, &cache, &agent_key, &account, "live", &req)
            .await
            .expect("rejection is returned as an order result");
        assert_eq!(response.results[0].status, "rejected");
        assert_eq!(*exchange.place_call_count.lock().await, 1);
        assert_eq!(lookup.calls.load(Ordering::SeqCst), 1);
        let (approved_at,): (Option<chrono::DateTime<Utc>>,) =
            sqlx::query_as("SELECT builder_fee_approved_at FROM users WHERE id = $1")
                .bind(crate::test_db::test_user_id())
                .fetch_one(&pool)
                .await
                .expect("read approval");
        assert!(approved_at.is_none());
        let (saved_fee,): (i16,) =
            sqlx::query_as("SELECT builder_fee_tenths_of_bp FROM users WHERE id = $1")
                .bind(crate::test_db::test_user_id())
                .fetch_one(&pool)
                .await
                .expect("read synchronized fee");
        assert_eq!(saved_fee, 9);
    }

    #[tokio::test]
    async fn multiple_builder_rejections_and_whole_call_rejection_refresh_once() {
        let pool = test_db::pool().await;
        let (agent_key, account) = seed(&pool, "fee-many-rejected").await;
        seed_instrument(&pool, "BTC", 0, 5).await;
        let mut input = limit_buy("BTC", dec!(0.1), dec!(50000));
        input.take_profits = vec![model::TriggerInput {
            trigger_price: dec!(55000),
            limit_price: Some(dec!(55100)),
            size: None,
        }];
        input.stop_losses = vec![model::TriggerInput {
            trigger_price: dec!(48000),
            limit_price: None,
            size: None,
        }];
        let req = PlaceOrdersRequest {
            orders: vec![input],
        };
        let exchange = FakeExchange::with_place_statuses(vec![
            OrderResponseStatus::Error("builder fee approval denied".to_string()),
            OrderResponseStatus::Error("BUILDER FEE permission denied".to_string()),
            OrderResponseStatus::Error("builder fee not approved".to_string()),
        ])
        .await;
        let (cache, lookup) = configured_cache(Ok(9));
        let response = place_orders(&pool, &exchange, &cache, &agent_key, &account, "live", &req)
            .await
            .expect("per-leg rejection response");
        assert_eq!(response.results.len(), 3);
        assert!(
            response
                .results
                .iter()
                .all(|result| result.status == "rejected")
        );
        assert_eq!(lookup.calls.load(Ordering::SeqCst), 1);

        let whole_error = FakeExchange::with_place_error("builder fee approval rejected").await;
        let (whole_cache, whole_lookup) = configured_cache(Ok(9));
        let whole_response = place_orders(
            &pool,
            &whole_error,
            &whole_cache,
            &agent_key,
            &account,
            "live",
            &PlaceOrdersRequest {
                orders: vec![limit_buy("BTC", dec!(0.1), dec!(50000))],
            },
        )
        .await
        .expect("whole-call rejection response");
        assert_eq!(
            whole_response.results[0].error.as_deref(),
            Some("builder fee approval rejected")
        );
        assert_eq!(whole_lookup.calls.load(Ordering::SeqCst), 1);
        assert_eq!(*whole_error.place_call_count.lock().await, 1);
    }

    #[tokio::test]
    async fn unrelated_rejection_and_failed_follow_up_preserve_local_state() {
        let pool = test_db::pool().await;
        let (agent_key, account) = seed(&pool, "fee-unrelated").await;
        seed_instrument(&pool, "BTC", 0, 5).await;
        let unrelated = FakeExchange::with_place_statuses(vec![OrderResponseStatus::Error(
            "insufficient margin".to_string(),
        )])
        .await;
        let (unrelated_cache, unrelated_lookup) = configured_cache(Ok(9));
        let req = PlaceOrdersRequest {
            orders: vec![limit_buy("BTC", dec!(0.1), dec!(50000))],
        };
        place_orders(
            &pool,
            &unrelated,
            &unrelated_cache,
            &agent_key,
            &account,
            "live",
            &req,
        )
        .await
        .expect("unrelated rejection response");
        assert_eq!(unrelated_lookup.calls.load(Ordering::SeqCst), 0);

        let failed_follow_up =
            FakeExchange::with_place_error("builder fee approval rejected").await;
        let (failed_cache, failed_lookup) = configured_cache(Err("info unavailable"));
        place_orders(
            &pool,
            &failed_follow_up,
            &failed_cache,
            &agent_key,
            &account,
            "live",
            &req,
        )
        .await
        .expect("original whole-call error response");
        assert_eq!(failed_lookup.calls.load(Ordering::SeqCst), 1);
        let (approved_at,): (Option<chrono::DateTime<Utc>>,) =
            sqlx::query_as("SELECT builder_fee_approved_at FROM users WHERE id = $1")
                .bind(crate::test_db::test_user_id())
                .fetch_one(&pool)
                .await
                .expect("read approval");
        assert!(approved_at.is_some());
        assert_eq!(*failed_follow_up.place_call_count.lock().await, 1);
    }

    #[test]
    fn builder_fee_rejection_matcher_requires_approval_language() {
        assert!(is_builder_fee_rejection("Builder fee approval rejected"));
        assert!(is_builder_fee_rejection("builder fee permission denied"));
        assert!(!is_builder_fee_rejection("builder fee too high"));
        assert!(!is_builder_fee_rejection("insufficient margin"));
    }

    #[tokio::test]
    async fn builder_fee_is_scoped_to_the_agent_owner() {
        let pool = test_db::pool().await;
        let (a_key, a_account) = seed(&pool, "fee-owner-a").await;
        let (b_key, b_account) = seed(&pool, "fee-owner-b").await;
        seed_instrument(&pool, "BTC", 0, 5).await;
        let other_user = Uuid::new_v4();
        let other_private_key = "59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d";
        let other_signer_address =
            derive_wallet_address(other_private_key).expect("signer address");
        let other_encryption_key = EncryptionKey::new(
            "test",
            [
                0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22,
                23, 24, 25, 26, 27, 28, 29, 30, 31,
            ],
        );
        let other_ciphertext =
            encrypt(&other_encryption_key, other_private_key).expect("encrypt signer");
        sqlx::query(
            "INSERT INTO users (id, wallet_address, api_wallet_address,
                                hyperliquid_private_key_ciphertext, hyperliquid_private_key_key_id,
                                api_wallet_approved_at, builder_fee_tenths_of_bp, builder_fee_approved_at)
             VALUES ($1, $2, $3, $4, 'test', now(), 17, now())",
        )
        .bind(other_user)
        .bind("0x0000000000000000000000000000000000000017")
        .bind(other_signer_address)
        .bind(other_ciphertext)
        .execute(&pool)
        .await
        .expect("insert second owner");
        sqlx::query("UPDATE agents SET user_id = $2 WHERE agent_key = $1")
            .bind(&b_key)
            .bind(other_user)
            .execute(&pool)
            .await
            .expect("assign second owner");
        let req = PlaceOrdersRequest {
            orders: vec![limit_buy("BTC", dec!(0.1), dec!(50000))],
        };
        let a_exchange = FakeExchange::with_place_statuses(vec![OrderResponseStatus::Resting {
            oid: 1,
            cloid: None,
        }])
        .await;
        let b_exchange = FakeExchange::with_place_statuses(vec![OrderResponseStatus::Resting {
            oid: 2,
            cloid: None,
        }])
        .await;
        place_orders(
            &pool,
            &a_exchange,
            &test_cache(),
            &a_key,
            &a_account,
            "live",
            &req,
        )
        .await
        .expect("first owner order");
        place_orders(
            &pool,
            &b_exchange,
            &test_cache(),
            &b_key,
            &b_account,
            "live",
            &req,
        )
        .await
        .expect("second owner order");
        assert_eq!(
            a_exchange
                .last_place_batch
                .lock()
                .await
                .as_ref()
                .unwrap()
                .builder
                .as_ref()
                .unwrap()
                .fee,
            10
        );
        assert_eq!(
            b_exchange
                .last_place_batch
                .lock()
                .await
                .as_ref()
                .unwrap()
                .builder
                .as_ref()
                .unwrap()
                .fee,
            17
        );
    }

    #[tokio::test]
    async fn spot_orders_are_rejected_without_an_exchange_call() {
        let pool = test_db::pool().await;
        let (agent_key, account) = seed(&pool, "spot-fee").await;
        sqlx::query(
            "INSERT INTO hyperliquid.instruments
                (instrument_id, name, market_type, base_asset, quote_asset, settlement_asset,
                 asset_index, price_decimals, size_decimals, lot_size, max_leverage, is_hip3,
                 active, created_at, updated_at)
             VALUES ('PURR/USDC', 'PURR/USDC', 'spot', 'PURR', 'USDC', 'USDC',
                     10, 2, 3, 0.001, 1, false, true, now(), now())",
        )
        .execute(&pool)
        .await
        .expect("seed spot instrument");
        let exchange = FakeExchange::new().await;
        let req = PlaceOrdersRequest {
            orders: vec![limit_buy("PURR/USDC", dec!(1), dec!(1))],
        };
        let error = place_orders(
            &pool,
            &exchange,
            &test_cache(),
            &agent_key,
            &account,
            "live",
            &req,
        )
        .await
        .expect_err("spot order must be rejected");
        assert!(error.to_string().contains("unsupported"));
        assert_eq!(*exchange.place_call_count.lock().await, 0);
    }

    #[tokio::test]
    async fn place_market_uses_mid_and_frontend_market() {
        let pool = test_db::pool().await;
        let (agent_key, account) = seed(&pool, "mkt").await;
        seed_instrument(&pool, "BTC", 0, 5).await;

        let ex = FakeExchange::with_place_statuses(vec![OrderResponseStatus::Filled {
            total_sz: dec!(0.1),
            avg_px: dec!(50100),
            oid: 99,
        }])
        .await;
        ex.mids.lock().await.insert("BTC".to_string(), dec!(50000));

        let mut input = limit_buy("BTC", dec!(0.1), dec!(50000));
        input.order_type = "market".to_string();
        input.price = None;

        let req = PlaceOrdersRequest {
            orders: vec![input],
        };
        let resp = place_orders(
            &pool,
            &ex,
            &test_cache(),
            &agent_key,
            &account,
            "live",
            &req,
        )
        .await
        .expect("place ok");

        assert_eq!(resp.results.len(), 1);
        assert_eq!(resp.results[0].status, "filled");
        assert_eq!(resp.results[0].exchange_oid.as_deref(), Some("99"));

        // Check that the batch used FrontendMarket.
        let batch = ex.last_place_batch.lock().await.clone().expect("batch");
        assert_eq!(batch.orders.len(), 1);
        match &batch.orders[0].order_type {
            OrderTypePlacement::Limit { tif } => {
                assert!(matches!(tif, TimeInForce::FrontendMarket));
            }
            _ => panic!("expected Limit with FrontendMarket"),
        }
    }

    #[tokio::test]
    async fn place_unknown_symbol_returns_validation_error() {
        let pool = test_db::pool().await;
        let (agent_key, account) = seed(&pool, "unk").await;
        // No instrument seeded for "NOPE".

        let exchange = FakeExchange::new().await;
        let req = PlaceOrdersRequest {
            orders: vec![limit_buy("NOPE", dec!(0.1), dec!(100))],
        };
        let err = place_orders(
            &pool,
            &exchange,
            &test_cache(),
            &agent_key,
            &account,
            "live",
            &req,
        )
        .await
        .expect_err("validation error");
        match err {
            GatewayError::Validation(s) => assert!(s.contains("NOPE")),
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn place_error_from_exchange_marks_all_legs_error() {
        let pool = test_db::pool().await;
        let (agent_key, account) = seed(&pool, "perr").await;
        seed_instrument(&pool, "BTC", 0, 5).await;

        let exchange = FakeExchange::with_place_error("network down").await;

        let mut input = limit_buy("BTC", dec!(0.1), dec!(50000));
        input.take_profits = vec![model::TriggerInput {
            trigger_price: dec!(55000),
            limit_price: Some(dec!(55100)),
            size: None,
        }];
        let req = PlaceOrdersRequest {
            orders: vec![input],
        };

        let resp = place_orders(
            &pool,
            &exchange,
            &test_cache(),
            &agent_key,
            &account,
            "live",
            &req,
        )
        .await
        .expect("error path returns Ok with per-leg errors");
        assert_eq!(resp.results.len(), 2);
        for r in &resp.results {
            assert_eq!(r.status, "error");
            assert_eq!(r.error.as_deref(), Some("network down"));
        }
        let stored = store::list_orders(&pool, &agent_key, None, None, None, None, None)
            .await
            .unwrap();
        for s in &stored {
            assert_eq!(s.status, "error");
        }
    }

    #[tokio::test]
    async fn cancel_by_oid_updates_local_row_to_canceled() {
        let pool = test_db::pool().await;
        let (agent_key, account) = seed(&pool, "cancel").await;
        seed_instrument(&pool, "BTC", 0, 5).await;

        // First place a resting order.
        let place_ex = FakeExchange::with_place_statuses(vec![OrderResponseStatus::Resting {
            oid: 7,
            cloid: None,
        }])
        .await;
        let req = PlaceOrdersRequest {
            orders: vec![limit_buy("BTC", dec!(0.1), dec!(50000))],
        };
        place_orders(
            &pool,
            &place_ex,
            &test_cache(),
            &agent_key,
            &account,
            "live",
            &req,
        )
        .await
        .expect("place ok");

        // Now cancel it.
        let cancel_ex =
            FakeExchange::with_cancel_statuses(vec![OrderResponseStatus::Success]).await;
        let cancel_req = CancelOrdersRequest {
            orders: vec![CancelInput {
                symbol: "BTC".to_string(),
                oid: 7,
            }],
        };
        let outcomes = cancel_orders(&pool, &cancel_ex, &agent_key, &account, "live", &cancel_req)
            .await
            .expect("cancel ok");
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].status, "canceled");

        // The local row's exchange_oid matches, status updated.
        let stored = store::list_orders(&pool, &agent_key, None, None, None, None, None)
            .await
            .unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].exchange_oid.as_deref(), Some("7"));
        assert_eq!(stored[0].status, "canceled");
    }

    #[tokio::test]
    async fn cancel_by_oid_cross_agent_is_rejected() {
        let pool = test_db::pool().await;
        let (a_key, a_acct) = seed(&pool, "xag-a").await;
        let (b_key, b_acct) = seed(&pool, "xag-b").await;
        seed_instrument(&pool, "BTC", 0, 5).await;

        // Agent A places a resting order.
        let place_ex = FakeExchange::with_place_statuses(vec![OrderResponseStatus::Resting {
            oid: 11,
            cloid: None,
        }])
        .await;
        let req = PlaceOrdersRequest {
            orders: vec![limit_buy("BTC", dec!(0.1), dec!(50000))],
        };
        place_orders(
            &pool,
            &place_ex,
            &test_cache(),
            &a_key,
            &a_acct,
            "live",
            &req,
        )
        .await
        .expect("place ok");

        // Agent B tries to cancel it.
        let cancel_ex =
            FakeExchange::with_cancel_statuses(vec![OrderResponseStatus::Success]).await;
        let cancel_req = CancelOrdersRequest {
            orders: vec![CancelInput {
                symbol: "BTC".to_string(),
                oid: 11,
            }],
        };
        let outcomes = cancel_orders(&pool, &cancel_ex, &b_key, &b_acct, "live", &cancel_req)
            .await
            .expect("cross-agent cancel returns Ok with per-item error");

        // The local lookup is account-scoped. B's account is different
        // from A's, so the lookup returns no row → the cancel is NOT
        // sent to the exchange, and the per-item outcome is
        // "not_found". This is the security boundary.
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].status, "not_found");
        assert!(outcomes[0].error.is_none());
    }

    #[tokio::test]
    async fn cancel_unknown_symbol_reports_per_item_error() {
        let pool = test_db::pool().await;
        let (agent_key, account) = seed(&pool, "cunk").await;
        // No instrument.

        let exchange = FakeExchange::new().await;
        let req = CancelOrdersRequest {
            orders: vec![CancelInput {
                symbol: "NOPE".to_string(),
                oid: 1,
            }],
        };
        let outcomes = cancel_orders(&pool, &exchange, &agent_key, &account, "live", &req)
            .await
            .expect("ok");
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].status, "error");
        assert!(
            outcomes[0]
                .error
                .as_deref()
                .unwrap_or("")
                .contains("unknown")
        );
    }

    /// Regression test for the high/medium-priority bug: if the
    /// exchange returns fewer cancel statuses than sent, the iterator
    /// used to `.expect()` and panic the async task. The fix returns a
    /// `GatewayError::Internal` instead.
    #[tokio::test]
    async fn cancel_with_short_response_returns_internal_error() {
        let pool = test_db::pool().await;
        let (agent_key, account) = seed(&pool, "short").await;
        seed_instrument(&pool, "BTC", 0, 5).await;

        // Place a resting order so the cancel flow has a real local
        // row to look up.
        let place_ex = FakeExchange::with_place_statuses(vec![OrderResponseStatus::Resting {
            oid: 7,
            cloid: None,
        }])
        .await;
        let req = PlaceOrdersRequest {
            orders: vec![limit_buy("BTC", dec!(0.1), dec!(50000))],
        };
        place_orders(
            &pool,
            &place_ex,
            &test_cache(),
            &agent_key,
            &account,
            "live",
            &req,
        )
        .await
        .expect("place ok");

        // Cancel: the FakeExchange returns an empty status vec
        // (no `with_cancel_statuses` was called). Previously this
        // panicked inside the status iterator; now it must surface
        // as `GatewayError::Internal`.
        let cancel_ex = FakeExchange::new().await;
        let cancel_req = CancelOrdersRequest {
            orders: vec![CancelInput {
                symbol: "BTC".to_string(),
                oid: 7,
            }],
        };
        let err = cancel_orders(&pool, &cancel_ex, &agent_key, &account, "live", &cancel_req)
            .await
            .expect_err("must return Internal when exchange returns fewer statuses");
        match err {
            GatewayError::Internal(e) => {
                assert!(
                    e.to_string().contains("fewer cancel statuses"),
                    "unexpected error message: {e:#}"
                );
            }
            other => panic!("expected Internal, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn place_request_validation_rejects_empty() {
        let pool = test_db::pool().await;
        let (agent_key, account) = seed(&pool, "vempty").await;
        let ex = FakeExchange::new().await;
        let req = PlaceOrdersRequest { orders: vec![] };
        let err = place_orders(
            &pool,
            &ex,
            &test_cache(),
            &agent_key,
            &account,
            "live",
            &req,
        )
        .await
        .expect_err("validation");
        match err {
            GatewayError::Validation(s) => assert!(s.contains("empty")),
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn place_request_validation_rejects_bad_payload() {
        let pool = test_db::pool().await;
        let (agent_key, account) = seed(&pool, "vbad").await;
        let ex = FakeExchange::new().await;
        let mut bad = limit_buy("BTC", dec!(-1), dec!(50000));
        bad.symbol = "BTC".to_string();
        let req = PlaceOrdersRequest { orders: vec![bad] };
        let err = place_orders(
            &pool,
            &ex,
            &test_cache(),
            &agent_key,
            &account,
            "live",
            &req,
        )
        .await
        .expect_err("validation");
        match err {
            GatewayError::Validation(s) => assert!(s.contains("size")),
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    // ensure unused import warnings stay quiet
    fn _unused(_j: serde_json::Value) {}
}
