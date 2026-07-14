use std::collections::HashMap;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use crate::{
    db::DbPool,
    hyperliquid::{
        config::AccountSyncConfig,
        historical::{fetch_funding_window, fetch_ledger_window, fetch_user_fills_window},
        normalize::{
            FundingEventRow, HistoricalOrderRow, InstrumentRow, LedgerEventRow, TradeFillRow,
            ms_to_datetime, parse_decimal,
        },
        raw_http::{
            RawHistoricalOrder, RawHyperliquidHttpClient, RawLedgerUpdate, RawUserFill,
            RawUserFunding,
        },
        sync_state::{SyncStateRow, SyncStatus, SyncStream},
    },
};

pub type InstrumentLookupMap = HashMap<String, (String, String, String)>;

const FUNDING_LOOKBACK_MS: u64 = 7 * 24 * 60 * 60 * 1_000;

#[derive(Debug, Clone)]
pub struct StreamSyncResult {
    pub stream: SyncStream,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SyncSummary {
    pub streams: Vec<StreamSyncResult>,
}

/// Reconcile one account's fills, funding, and ledger streams incrementally.
///
/// Each stream is fetched independently using its durable sync_state row to
/// determine the next window. Failures are recorded per-stream and do not
/// abort the other streams.
pub async fn sync_account_once(
    pool: &DbPool,
    config: &AccountSyncConfig,
    raw_http: &RawHyperliquidHttpClient,
    lookup: &InstrumentLookupMap,
) -> Result<SyncSummary> {
    let mut streams = Vec::with_capacity(3);

    for stream in [SyncStream::Fills, SyncStream::Funding, SyncStream::Ledger] {
        ensure_sync_row(pool, config, stream).await?;
        let result = match reconcile_stream(pool, config, raw_http, lookup, stream).await {
            Ok(result) => result,
            Err(e) => {
                let msg = format!("{:#}", e);
                update_sync_state(
                    pool,
                    config,
                    stream,
                    None,
                    None,
                    SyncStatus::Failed,
                    json!({"error": &msg}),
                )
                .await?;
                StreamSyncResult {
                    stream,
                    error: Some(msg),
                }
            }
        };
        streams.push(result);
    }

    Ok(SyncSummary { streams })
}

/// Reconcile a single stream and update its sync_state.
async fn reconcile_stream(
    pool: &DbPool,
    config: &AccountSyncConfig,
    raw_http: &RawHyperliquidHttpClient,
    lookup: &InstrumentLookupMap,
    stream: SyncStream,
) -> Result<StreamSyncResult> {
    update_sync_state(
        pool,
        config,
        stream,
        None,
        None,
        SyncStatus::Running,
        json!({}),
    )
    .await?;

    let state = load_sync_state(pool, config, stream).await?;
    let now_ms = Utc::now().timestamp_millis() as u64;
    let start_time = compute_start_time(config, &state, stream);
    let end_time = now_ms;

    let (count, last_event_time, last_event_key) = match stream {
        SyncStream::Fills => {
            let fills = fetch_user_fills_window(raw_http, start_time, Some(end_time)).await?;
            let count = fills.len();
            let (last_time, last_key) = fills.iter().fold(
                (None::<DateTime<Utc>>, None::<String>),
                |(last_time, _last_key), fill| {
                    let time = ms_to_datetime(fill.time);
                    let keep_current = last_time.map(|t| time > t).unwrap_or(true);
                    if keep_current {
                        (Some(time), Some(fill.hash.clone()))
                    } else {
                        (last_time, _last_key)
                    }
                },
            );
            for fill in fills {
                upsert_trade_fill(pool, normalize_fill(config, lookup, fill)?).await?;
            }
            (count, last_time, last_key)
        }
        SyncStream::Funding => {
            let funding = fetch_funding_window(raw_http, start_time, Some(end_time)).await?;
            let count = funding.len();
            let (last_time, last_key) = funding.iter().fold(
                (None::<DateTime<Utc>>, None::<String>),
                |(last_time, _last_key), funding| {
                    let time = ms_to_datetime(funding.time);
                    let keep_current = last_time.map(|t| time > t).unwrap_or(true);
                    if keep_current {
                        let key = funding.hash.clone().unwrap_or_else(|| time.to_rfc3339());
                        (Some(time), Some(key))
                    } else {
                        (last_time, _last_key)
                    }
                },
            );
            for funding_row in funding {
                if let Some(row) = normalize_funding(config, lookup, funding_row)? {
                    upsert_funding_event(pool, row).await?;
                }
            }
            (count, last_time, last_key)
        }
        SyncStream::Ledger => {
            let ledger = fetch_ledger_window(raw_http, start_time, Some(end_time)).await?;
            let count = ledger.len();
            let (last_time, last_key) = ledger.iter().fold(
                (None::<DateTime<Utc>>, None::<String>),
                |(last_time, _last_key), ledger| {
                    let time = ms_to_datetime(ledger.time);
                    let keep_current = last_time.map(|t| time > t).unwrap_or(true);
                    if keep_current {
                        (Some(time), Some(ledger.hash.clone()))
                    } else {
                        (last_time, _last_key)
                    }
                },
            );
            for ledger_row in ledger {
                upsert_ledger_event(pool, normalize_ledger(config, lookup, ledger_row)?).await?;
            }
            (count, last_time, last_key)
        }
        _ => (0, None, None),
    };

    update_sync_state(
        pool,
        config,
        stream,
        last_event_time,
        last_event_key.as_deref(),
        SyncStatus::Healthy,
        json!({
            "fetched": count,
            "window_start_ms": start_time,
            "window_end_ms": end_time,
        }),
    )
    .await?;

    Ok(StreamSyncResult {
        stream,
        error: None,
    })
}

fn compute_start_time(config: &AccountSyncConfig, state: &SyncStateRow, stream: SyncStream) -> u64 {
    let overlap_ms = match stream {
        // Hyperliquid's userFunding endpoint can return sparse results for
        // narrow cursor windows. Funding is hourly and idempotently upserted,
        // so use a wider rolling lookback to repair missed rows naturally
        // while staying comfortably below the endpoint's historical row caps.
        SyncStream::Funding => FUNDING_LOOKBACK_MS,
        _ => config.overlap_ms,
    };

    let start = state
        .last_event_time
        .map(|t| {
            let ms = t.timestamp_millis() as u64;
            ms.saturating_sub(overlap_ms)
        })
        .unwrap_or(config.history_start_ms);
    start.max(config.history_start_ms)
}

async fn ensure_sync_row(
    pool: &DbPool,
    config: &AccountSyncConfig,
    stream: SyncStream,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO hyperliquid.sync_state (account_address, environment, stream_name, status, metadata) VALUES ($1, $2, $3, $4, $5) ON CONFLICT (account_address, environment, stream_name) DO NOTHING",
    )
    .bind(&config.account_address)
    .bind(config.environment.as_journal_str())
    .bind(stream.as_str())
    .bind(SyncStatus::Pending.as_str())
    .bind(json!({}))
    .execute(pool)
    .await?;

    Ok(())
}

async fn load_sync_state(
    pool: &DbPool,
    config: &AccountSyncConfig,
    stream: SyncStream,
) -> Result<SyncStateRow> {
    let row = sqlx::query_as::<_, SyncStateRow>(
        "SELECT account_address, environment, stream_name, last_event_time, last_event_key, last_synced_at, status, metadata FROM hyperliquid.sync_state WHERE account_address = $1 AND environment = $2 AND stream_name = $3",
    )
    .bind(&config.account_address)
    .bind(config.environment.as_journal_str())
    .bind(stream.as_str())
    .fetch_one(pool)
    .await
    .context("failed to load sync_state row")?;

    Ok(row)
}

async fn update_sync_state(
    pool: &DbPool,
    config: &AccountSyncConfig,
    stream: SyncStream,
    last_event_time: Option<DateTime<Utc>>,
    last_event_key: Option<&str>,
    status: SyncStatus,
    metadata: Value,
) -> Result<()> {
    sqlx::query(
        "UPDATE hyperliquid.sync_state SET last_event_time = COALESCE($4, last_event_time), last_event_key = COALESCE($5, last_event_key), last_synced_at = NOW(), status = $6, metadata = $7 WHERE account_address = $1 AND environment = $2 AND stream_name = $3",
    )
    .bind(&config.account_address)
    .bind(config.environment.as_journal_str())
    .bind(stream.as_str())
    .bind(last_event_time)
    .bind(last_event_key)
    .bind(status.as_str())
    .bind(metadata)
    .execute(pool)
    .await?;

    Ok(())
}

fn build_lookup(instruments: &[InstrumentRow]) -> InstrumentLookupMap {
    let mut lookup = HashMap::with_capacity(instruments.len());
    for instrument in instruments {
        lookup.insert(
            instrument.instrument_id.clone(),
            (
                instrument.instrument_id.clone(),
                instrument.name.clone(),
                instrument.base_asset.clone(),
            ),
        );
    }
    lookup
}

fn resolve_instrument<'a>(
    lookup: &'a InstrumentLookupMap,
    raw_coin: &str,
) -> crate::hyperliquid::normalize::InstrumentLookup<'a> {
    let Some((instrument_id, symbol, base_asset)) = lookup.get(raw_coin) else {
        return crate::hyperliquid::normalize::InstrumentLookup {
            instrument_id: None,
            symbol: None,
            base_asset: None,
        };
    };

    crate::hyperliquid::normalize::InstrumentLookup {
        instrument_id: Some(instrument_id.as_str()),
        symbol: Some(symbol.as_str()),
        base_asset: Some(base_asset.as_str()),
    }
}

fn normalize_fill(
    config: &AccountSyncConfig,
    lookup: &InstrumentLookupMap,
    fill: RawUserFill,
) -> Result<TradeFillRow> {
    let payload = serde_json::to_value(&fill)?;
    let resolved = resolve_instrument(lookup, &fill.coin);
    let price = parse_decimal(&fill.px)?;
    let size = parse_decimal(&fill.sz)?;
    let now = Utc::now();

    Ok(TradeFillRow {
        hash: fill.hash.clone(),
        account_address: config.account_address.clone(),
        environment: config.environment.as_journal_str().to_string(),
        event_time: ms_to_datetime(fill.time),
        event_type: "fill".to_string(),
        source_stream: "fills".to_string(),
        instrument_id: resolved.instrument_id.map(ToOwned::to_owned),
        asset: resolved
            .base_asset
            .map(ToOwned::to_owned)
            .or_else(|| Some(fill.coin.clone())),
        symbol: resolved.symbol.map(ToOwned::to_owned),
        fee_usdc: fill.fee.as_deref().map(parse_decimal).transpose()?,
        realized_pnl_usdc: fill.closed_pnl.as_deref().map(parse_decimal).transpose()?,
        fill_time: ms_to_datetime(fill.time),
        direction: fill.dir,
        side: fill.side,
        price,
        size,
        trade_value: Some(price * size),
        order_id: Some(fill.oid.to_string()),
        trade_id: fill
            .tid
            .unwrap_or(Value::String(fill.hash.clone()))
            .to_string(),
        start_position: fill
            .start_position
            .as_deref()
            .map(parse_decimal)
            .transpose()?,
        fee: fill.fee.as_deref().map(parse_decimal).transpose()?,
        fee_token: fill.fee_token,
        builder_fee: fill.builder_fee.as_deref().map(parse_decimal).transpose()?,
        crossed: fill.crossed,
        tx_hash: fill.tx_hash,
        payload,
        ingest_source: "http".to_string(),
        inserted_at: now,
    })
}

fn normalize_funding(
    config: &AccountSyncConfig,
    lookup: &InstrumentLookupMap,
    funding: RawUserFunding,
) -> Result<Option<FundingEventRow>> {
    let resolved = funding
        .coin
        .as_deref()
        .map(|coin| resolve_instrument(lookup, coin))
        .unwrap_or(crate::hyperliquid::normalize::InstrumentLookup {
            instrument_id: None,
            symbol: None,
            base_asset: None,
        });
    let Some(instrument_id) = resolved.instrument_id.map(ToOwned::to_owned) else {
        return Ok(None);
    };

    Ok(Some(FundingEventRow {
        account_address: config.account_address.clone(),
        environment: config.environment.as_journal_str().to_string(),
        instrument_id,
        event_time: ms_to_datetime(funding.time),
        event_type: "funding".to_string(),
        source_stream: "funding".to_string(),
        asset: resolved
            .base_asset
            .map(ToOwned::to_owned)
            .or_else(|| funding.coin.clone()),
        symbol: resolved.symbol.map(ToOwned::to_owned),
        fee_usdc: None,
        realized_pnl_usdc: Some(parse_decimal(&funding.usdc)?),
        usdc: parse_decimal(&funding.usdc)?,
        position_size: funding
            .position_size
            .as_deref()
            .map(parse_decimal)
            .transpose()?,
        funding_rate: funding
            .funding_rate
            .as_deref()
            .map(parse_decimal)
            .transpose()?,
        hash: funding.hash,
        payload: funding.payload,
        ingest_source: "http".to_string(),
        inserted_at: Utc::now(),
    }))
}

fn normalize_ledger(
    config: &AccountSyncConfig,
    lookup: &InstrumentLookupMap,
    ledger: RawLedgerUpdate,
) -> Result<LedgerEventRow> {
    let payload = serde_json::to_value(&ledger)?;
    let ledger_type = ledger
        .ledger_type
        .clone()
        .unwrap_or_else(|| "ledger_update".to_string());
    let raw_coin = ledger
        .delta
        .get("coin")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let resolved = resolve_instrument(lookup, raw_coin);

    Ok(LedgerEventRow {
        hash: ledger.hash.clone(),
        account_address: config.account_address.clone(),
        environment: config.environment.as_journal_str().to_string(),
        event_time: ms_to_datetime(ledger.time),
        event_type: ledger_type.clone(),
        source_stream: "ledger".to_string(),
        instrument_id: resolved.instrument_id.map(ToOwned::to_owned),
        asset: resolved.base_asset.map(ToOwned::to_owned),
        symbol: resolved.symbol.map(ToOwned::to_owned),
        fee_usdc: None,
        realized_pnl_usdc: None,
        ledger_type,
        usdc: ledger.usdc.as_deref().map(parse_decimal).transpose()?,
        token: ledger.token,
        amount: ledger.amount.as_deref().map(parse_decimal).transpose()?,
        fee: ledger.fee.as_deref().map(parse_decimal).transpose()?,
        source_user: ledger.source_user,
        destination_user: ledger.destination_user,
        tx_hash: ledger.tx_hash,
        details: ledger.delta,
        payload,
        ingest_source: "http".to_string(),
        inserted_at: Utc::now(),
    })
}

fn normalize_order(
    config: &AccountSyncConfig,
    lookup: &InstrumentLookupMap,
    order: RawHistoricalOrder,
) -> Result<HistoricalOrderRow> {
    let payload = serde_json::to_value(&order)?;
    let coin = order
        .order
        .get("coin")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let resolved = resolve_instrument(lookup, coin);
    let oid = order
        .order
        .get("oid")
        .cloned()
        .unwrap_or(Value::Null)
        .to_string();
    let event_ms = order
        .status_timestamp
        .or_else(|| order.order.get("timestamp").and_then(Value::as_u64))
        .unwrap_or(0);

    Ok(HistoricalOrderRow {
        account_address: config.account_address.clone(),
        environment: config.environment.as_journal_str().to_string(),
        order_id: oid,
        event_time: ms_to_datetime(event_ms),
        source_stream: "historical_orders".to_string(),
        instrument_id: resolved.instrument_id.map(ToOwned::to_owned),
        asset: resolved.base_asset.map(ToOwned::to_owned),
        symbol: resolved.symbol.map(ToOwned::to_owned),
        order_status: order.status,
        side: order
            .order
            .get("side")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        order_type: order
            .order
            .get("orderType")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        price: order
            .order
            .get("limitPx")
            .and_then(Value::as_str)
            .map(parse_decimal)
            .transpose()?,
        size: order
            .order
            .get("sz")
            .and_then(Value::as_str)
            .map(parse_decimal)
            .transpose()?,
        filled_size: order
            .order
            .get("filledSz")
            .and_then(Value::as_str)
            .map(parse_decimal)
            .transpose()?,
        reduce_only: order.order.get("reduceOnly").and_then(Value::as_bool),
        time_in_force: order
            .order
            .get("tif")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        client_order_id: order
            .order
            .get("cloid")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        status_timestamp: order.status_timestamp.map(ms_to_datetime),
        payload,
        ingest_source: "http".to_string(),
        inserted_at: Utc::now(),
    })
}

pub async fn upsert_trade_fill(pool: &DbPool, row: TradeFillRow) -> Result<()> {
    sqlx::query(
        "INSERT INTO hyperliquid.trade_fills (hash, account_address, environment, event_time, event_type, source_stream, instrument_id, asset, symbol, fee_usdc, realized_pnl_usdc, fill_time, direction, side, price, size, trade_value, order_id, trade_id, start_position, fee, fee_token, builder_fee, crossed, tx_hash, payload, ingest_source, inserted_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22,$23,$24,$25,$26,$27,$28) ON CONFLICT (hash, trade_id) DO UPDATE SET event_time = EXCLUDED.event_time, event_type = EXCLUDED.event_type, source_stream = EXCLUDED.source_stream, instrument_id = EXCLUDED.instrument_id, asset = EXCLUDED.asset, symbol = EXCLUDED.symbol, fee_usdc = EXCLUDED.fee_usdc, realized_pnl_usdc = EXCLUDED.realized_pnl_usdc, fill_time = EXCLUDED.fill_time, direction = EXCLUDED.direction, side = EXCLUDED.side, price = EXCLUDED.price, size = EXCLUDED.size, trade_value = EXCLUDED.trade_value, order_id = EXCLUDED.order_id, start_position = EXCLUDED.start_position, fee = EXCLUDED.fee, fee_token = EXCLUDED.fee_token, builder_fee = EXCLUDED.builder_fee, crossed = EXCLUDED.crossed, tx_hash = EXCLUDED.tx_hash, payload = EXCLUDED.payload, ingest_source = EXCLUDED.ingest_source",
    )
    .bind(row.hash)
    .bind(row.account_address)
    .bind(row.environment)
    .bind(row.event_time)
    .bind(row.event_type)
    .bind(row.source_stream)
    .bind(row.instrument_id)
    .bind(row.asset)
    .bind(row.symbol)
    .bind(row.fee_usdc)
    .bind(row.realized_pnl_usdc)
    .bind(row.fill_time)
    .bind(row.direction)
    .bind(row.side)
    .bind(row.price)
    .bind(row.size)
    .bind(row.trade_value)
    .bind(row.order_id)
    .bind(row.trade_id)
    .bind(row.start_position)
    .bind(row.fee)
    .bind(row.fee_token)
    .bind(row.builder_fee)
    .bind(row.crossed)
    .bind(row.tx_hash)
    .bind(row.payload)
    .bind(row.ingest_source)
    .bind(row.inserted_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn upsert_funding_event(pool: &DbPool, row: FundingEventRow) -> Result<()> {
    sqlx::query(
        "INSERT INTO hyperliquid.funding_events (account_address, environment, instrument_id, event_time, event_type, source_stream, asset, symbol, fee_usdc, realized_pnl_usdc, usdc, position_size, funding_rate, hash, payload, ingest_source, inserted_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17) ON CONFLICT (account_address, environment, instrument_id, event_time) DO UPDATE SET event_type = EXCLUDED.event_type, source_stream = EXCLUDED.source_stream, asset = EXCLUDED.asset, symbol = EXCLUDED.symbol, fee_usdc = EXCLUDED.fee_usdc, realized_pnl_usdc = EXCLUDED.realized_pnl_usdc, usdc = EXCLUDED.usdc, position_size = EXCLUDED.position_size, funding_rate = EXCLUDED.funding_rate, hash = EXCLUDED.hash, payload = EXCLUDED.payload, ingest_source = EXCLUDED.ingest_source",
    )
    .bind(row.account_address)
    .bind(row.environment)
    .bind(row.instrument_id)
    .bind(row.event_time)
    .bind(row.event_type)
    .bind(row.source_stream)
    .bind(row.asset)
    .bind(row.symbol)
    .bind(row.fee_usdc)
    .bind(row.realized_pnl_usdc)
    .bind(row.usdc)
    .bind(row.position_size)
    .bind(row.funding_rate)
    .bind(row.hash)
    .bind(row.payload)
    .bind(row.ingest_source)
    .bind(row.inserted_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn upsert_ledger_event(pool: &DbPool, row: LedgerEventRow) -> Result<()> {
    sqlx::query(
        "INSERT INTO hyperliquid.ledger_events (hash, account_address, environment, event_time, event_type, source_stream, instrument_id, asset, symbol, fee_usdc, realized_pnl_usdc, ledger_type, usdc, token, amount, fee, source_user, destination_user, tx_hash, details, payload, ingest_source, inserted_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22,$23) ON CONFLICT (hash) DO UPDATE SET event_time = EXCLUDED.event_time, event_type = EXCLUDED.event_type, source_stream = EXCLUDED.source_stream, instrument_id = EXCLUDED.instrument_id, asset = EXCLUDED.asset, symbol = EXCLUDED.symbol, fee_usdc = EXCLUDED.fee_usdc, realized_pnl_usdc = EXCLUDED.realized_pnl_usdc, ledger_type = EXCLUDED.ledger_type, usdc = EXCLUDED.usdc, token = EXCLUDED.token, amount = EXCLUDED.amount, fee = EXCLUDED.fee, source_user = EXCLUDED.source_user, destination_user = EXCLUDED.destination_user, tx_hash = EXCLUDED.tx_hash, details = EXCLUDED.details, payload = EXCLUDED.payload, ingest_source = EXCLUDED.ingest_source",
    )
    .bind(row.hash)
    .bind(row.account_address)
    .bind(row.environment)
    .bind(row.event_time)
    .bind(row.event_type)
    .bind(row.source_stream)
    .bind(row.instrument_id)
    .bind(row.asset)
    .bind(row.symbol)
    .bind(row.fee_usdc)
    .bind(row.realized_pnl_usdc)
    .bind(row.ledger_type)
    .bind(row.usdc)
    .bind(row.token)
    .bind(row.amount)
    .bind(row.fee)
    .bind(row.source_user)
    .bind(row.destination_user)
    .bind(row.tx_hash)
    .bind(row.details)
    .bind(row.payload)
    .bind(row.ingest_source)
    .bind(row.inserted_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Sync historical orders for an account. This is not part of the USDC
/// transaction journal and runs on a slower cadence.
pub async fn sync_historical_orders_once(
    pool: &DbPool,
    config: &AccountSyncConfig,
    raw_http: &RawHyperliquidHttpClient,
    lookup: &InstrumentLookupMap,
) -> Result<usize> {
    ensure_sync_row(pool, config, SyncStream::HistoricalOrders).await?;
    update_sync_state(
        pool,
        config,
        SyncStream::HistoricalOrders,
        None,
        None,
        SyncStatus::Running,
        json!({}),
    )
    .await?;

    let orders = raw_http.historical_orders().await?;
    let count = orders.len();
    for order in orders {
        upsert_historical_order(pool, normalize_order(config, lookup, order)?).await?;
    }

    update_sync_state(
        pool,
        config,
        SyncStream::HistoricalOrders,
        None,
        None,
        SyncStatus::Healthy,
        json!({"fetched": count}),
    )
    .await?;

    Ok(count)
}

pub async fn upsert_historical_order(pool: &DbPool, row: HistoricalOrderRow) -> Result<()> {
    sqlx::query(
        "INSERT INTO hyperliquid.historical_orders (account_address, environment, order_id, event_time, source_stream, instrument_id, asset, symbol, order_status, side, order_type, price, size, filled_size, reduce_only, time_in_force, client_order_id, status_timestamp, payload, ingest_source, inserted_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21) ON CONFLICT (account_address, environment, order_id) DO UPDATE SET event_time = EXCLUDED.event_time, source_stream = EXCLUDED.source_stream, instrument_id = EXCLUDED.instrument_id, asset = EXCLUDED.asset, symbol = EXCLUDED.symbol, order_status = EXCLUDED.order_status, side = EXCLUDED.side, order_type = EXCLUDED.order_type, price = EXCLUDED.price, size = EXCLUDED.size, filled_size = EXCLUDED.filled_size, reduce_only = EXCLUDED.reduce_only, time_in_force = EXCLUDED.time_in_force, client_order_id = EXCLUDED.client_order_id, status_timestamp = EXCLUDED.status_timestamp, payload = EXCLUDED.payload, ingest_source = EXCLUDED.ingest_source",
    )
    .bind(row.account_address)
    .bind(row.environment)
    .bind(row.order_id)
    .bind(row.event_time)
    .bind(row.source_stream)
    .bind(row.instrument_id)
    .bind(row.asset)
    .bind(row.symbol)
    .bind(row.order_status)
    .bind(row.side)
    .bind(row.order_type)
    .bind(row.price)
    .bind(row.size)
    .bind(row.filled_size)
    .bind(row.reduce_only)
    .bind(row.time_in_force)
    .bind(row.client_order_id)
    .bind(row.status_timestamp)
    .bind(row.payload)
    .bind(row.ingest_source)
    .bind(row.inserted_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub fn build_instrument_lookup(instruments: &[InstrumentRow]) -> InstrumentLookupMap {
    build_lookup(instruments)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    use crate::hyperliquid::config::HyperliquidEnvironment;

    #[test]
    fn compute_start_time_uses_history_start_when_no_state() {
        let config = AccountSyncConfig {
            account_address: "0x0".to_string(),
            environment: HyperliquidEnvironment::Mainnet,
            history_start_ms: 1_000_000,
            overlap_ms: 60_000,
        };
        let state = SyncStateRow::new("0x0".to_string(), "live".to_string(), SyncStream::Fills);
        assert_eq!(
            compute_start_time(&config, &state, SyncStream::Fills),
            1_000_000
        );
    }

    #[test]
    fn compute_start_time_applies_overlap_and_floor() {
        let config = AccountSyncConfig {
            account_address: "0x0".to_string(),
            environment: HyperliquidEnvironment::Mainnet,
            history_start_ms: 1_000_000,
            overlap_ms: 60_000,
        };
        let mut state = SyncStateRow::new("0x0".to_string(), "live".to_string(), SyncStream::Fills);
        state.last_event_time = Some(Utc.timestamp_millis_opt(2_000_000).single().unwrap());
        assert_eq!(
            compute_start_time(&config, &state, SyncStream::Fills),
            1_940_000
        );
    }

    #[test]
    fn compute_start_time_floors_to_history_start() {
        let config = AccountSyncConfig {
            account_address: "0x0".to_string(),
            environment: HyperliquidEnvironment::Mainnet,
            history_start_ms: 1_000_000,
            overlap_ms: 60_000,
        };
        let mut state = SyncStateRow::new("0x0".to_string(), "live".to_string(), SyncStream::Fills);
        state.last_event_time = Some(Utc.timestamp_millis_opt(1_050_000).single().unwrap());
        assert_eq!(
            compute_start_time(&config, &state, SyncStream::Fills),
            1_000_000
        );
    }

    #[test]
    fn compute_start_time_uses_wide_funding_lookback() {
        let config = AccountSyncConfig {
            account_address: "0x0".to_string(),
            environment: HyperliquidEnvironment::Mainnet,
            history_start_ms: 1_000_000,
            overlap_ms: 60_000,
        };
        let mut state =
            SyncStateRow::new("0x0".to_string(), "live".to_string(), SyncStream::Funding);
        state.last_event_time = Some(
            Utc.timestamp_millis_opt((1_000_000 + FUNDING_LOOKBACK_MS + 60_000) as i64)
                .single()
                .unwrap(),
        );

        assert_eq!(
            compute_start_time(&config, &state, SyncStream::Funding),
            1_060_000
        );
    }
}
