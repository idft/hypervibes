use std::collections::HashMap;

use anyhow::Result;
use serde_json::{Value, json};

use crate::{
    db::{DbPool, connect, migrate},
    hyperliquid::{
        config::AppConfig,
        historical::fetch_historical_batch,
        instruments::load_instruments,
        normalize::{
            FundingEventRow, HistoricalOrderRow, InstrumentLookup, LedgerEventRow, TradeFillRow,
            ms_to_datetime, parse_decimal,
        },
        polling::PollingSchedule,
        raw_http::{
            RawHistoricalOrder, RawHttpConfig, RawHyperliquidHttpClient, RawLedgerUpdate,
            RawUserFill, RawUserFunding,
        },
        sync_state::{SyncStateRow, SyncStatus, SyncStream},
    },
};

pub struct HyperliquidJournalStore {
    config: AppConfig,
}

impl HyperliquidJournalStore {
    pub fn new(config: AppConfig) -> Self {
        Self { config }
    }

    pub async fn run(&self) -> Result<()> {
        let pool = connect(&self.config.database_url).await?;
        migrate(&pool).await?;

        let instruments = load_instruments(self.config.environment).await?;
        upsert_instruments(&pool, &instruments).await?;

        let raw_http = RawHyperliquidHttpClient::new(RawHttpConfig {
            environment: self.config.environment,
            account_address: self.config.account_address.clone(),
        });

        let lookup = build_lookup(&instruments);
        sync_once(&pool, &self.config, &raw_http, &lookup).await?;

        if self.config.poll_once {
            return Ok(());
        }

        let _schedule = PollingSchedule::default();
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            sync_once(&pool, &self.config, &raw_http, &lookup).await?;
        }
    }
}

async fn sync_once(
    pool: &DbPool,
    config: &AppConfig,
    raw_http: &RawHyperliquidHttpClient,
    lookup: &HashMap<String, (String, String, String)>,
) -> Result<()> {
    ensure_sync_rows(pool, config).await?;
    let (fills, funding, ledger, orders) =
        fetch_historical_batch(raw_http, config.history_start_ms, None).await?;

    for fill in fills {
        upsert_trade_fill(pool, normalize_fill(config, lookup, fill)?).await?;
    }
    for funding_row in funding {
        if let Some(row) = normalize_funding(config, lookup, funding_row)? {
            upsert_funding_event(pool, row).await?;
        }
    }
    for ledger_row in ledger {
        upsert_ledger_event(pool, normalize_ledger(config, lookup, ledger_row)?).await?;
    }
    for order in orders {
        upsert_historical_order(pool, normalize_order(config, lookup, order)?).await?;
    }

    for stream in [
        SyncStream::Instruments,
        SyncStream::Fills,
        SyncStream::Funding,
        SyncStream::Ledger,
        SyncStream::HistoricalOrders,
    ] {
        update_sync_state(
            pool,
            config,
            stream,
            None,
            None,
            SyncStatus::Healthy,
            json!({}),
        )
        .await?;
    }

    Ok(())
}

fn build_lookup(
    instruments: &[crate::hyperliquid::normalize::InstrumentRow],
) -> HashMap<String, (String, String, String)> {
    let mut lookup = HashMap::with_capacity(instruments.len());
    for instrument in instruments {
        lookup.insert(
            instrument.raw_symbol.clone(),
            (
                instrument.instrument_id.clone(),
                instrument.symbol.clone(),
                instrument.base_asset.clone(),
            ),
        );
    }
    lookup
}

fn resolve_instrument<'a>(
    lookup: &'a HashMap<String, (String, String, String)>,
    raw_coin: &str,
) -> InstrumentLookup<'a> {
    let Some((instrument_id, symbol, base_asset)) = lookup.get(raw_coin) else {
        return InstrumentLookup {
            instrument_id: None,
            symbol: None,
            base_asset: None,
        };
    };

    InstrumentLookup {
        instrument_id: Some(instrument_id.as_str()),
        symbol: Some(symbol.as_str()),
        base_asset: Some(base_asset.as_str()),
    }
}

fn normalize_fill(
    config: &AppConfig,
    lookup: &HashMap<String, (String, String, String)>,
    fill: RawUserFill,
) -> Result<TradeFillRow> {
    let payload = serde_json::to_value(&fill)?;
    let resolved = resolve_instrument(lookup, &fill.coin);
    let price = parse_decimal(&fill.px)?;
    let size = parse_decimal(&fill.sz)?;
    let now = chrono::Utc::now();

    Ok(TradeFillRow {
        hash: fill.hash.clone(),
        account_address: config.account_address.clone(),
        environment: config.environment.as_journal_str().to_string(),
        event_time: ms_to_datetime(fill.time),
        event_type: "fill".to_string(),
        source_stream: "fills".to_string(),
        instrument_id: resolved.instrument_id.map(ToOwned::to_owned),
        asset: resolved.base_asset.map(ToOwned::to_owned),
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
    config: &AppConfig,
    lookup: &HashMap<String, (String, String, String)>,
    funding: RawUserFunding,
) -> Result<Option<FundingEventRow>> {
    let resolved = funding
        .coin
        .as_deref()
        .map(|coin| resolve_instrument(lookup, coin))
        .unwrap_or(InstrumentLookup {
            instrument_id: None,
            symbol: None,
            base_asset: None,
        });
    let Some(instrument_id) = resolved.instrument_id else {
        return Ok(None);
    };

    Ok(Some(FundingEventRow {
        account_address: config.account_address.clone(),
        environment: config.environment.as_journal_str().to_string(),
        instrument_id: instrument_id.to_string(),
        event_time: ms_to_datetime(funding.time),
        event_type: "funding".to_string(),
        source_stream: "funding".to_string(),
        asset: resolved.base_asset.map(ToOwned::to_owned),
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
        inserted_at: chrono::Utc::now(),
    }))
}

fn normalize_ledger(
    config: &AppConfig,
    lookup: &HashMap<String, (String, String, String)>,
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
        inserted_at: chrono::Utc::now(),
    })
}

fn normalize_order(
    config: &AppConfig,
    lookup: &HashMap<String, (String, String, String)>,
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
        inserted_at: chrono::Utc::now(),
    })
}

async fn ensure_sync_rows(pool: &DbPool, config: &AppConfig) -> Result<()> {
    for stream in [
        SyncStream::Instruments,
        SyncStream::Fills,
        SyncStream::Funding,
        SyncStream::Ledger,
        SyncStream::HistoricalOrders,
    ] {
        sqlx::query(
            "INSERT INTO hyperliquid.sync_state (account_address, environment, stream_name, status, metadata) VALUES ($1, $2, $3, $4, $5) ON CONFLICT (account_address, environment, stream_name) DO NOTHING",
        )
        .bind(&config.account_address)
        .bind(config.environment.as_journal_str())
        .bind(stream.as_str())
        .bind("pending")
        .bind(json!({}))
        .execute(pool)
        .await?;
    }

    Ok(())
}

async fn update_sync_state(
    pool: &DbPool,
    config: &AppConfig,
    stream: SyncStream,
    last_event_time: Option<chrono::DateTime<chrono::Utc>>,
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
    .bind(match status {
        SyncStatus::Pending => "pending",
        SyncStatus::Running => "running",
        SyncStatus::Healthy => "healthy",
        SyncStatus::Failed => "failed",
    })
    .bind(metadata)
    .execute(pool)
    .await?;

    Ok(())
}

async fn upsert_instruments(
    pool: &DbPool,
    instruments: &[crate::hyperliquid::normalize::InstrumentRow],
) -> Result<()> {
    for instrument in instruments {
        sqlx::query(
            "INSERT INTO hyperliquid.instruments (instrument_id, symbol, raw_symbol, market_type, base_asset, quote_asset, settlement_asset, asset_index, price_decimals, size_decimals, tick_size, lot_size, is_hip3, active, created_at, updated_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16) ON CONFLICT (instrument_id) DO UPDATE SET symbol = EXCLUDED.symbol, raw_symbol = EXCLUDED.raw_symbol, market_type = EXCLUDED.market_type, base_asset = EXCLUDED.base_asset, quote_asset = EXCLUDED.quote_asset, settlement_asset = EXCLUDED.settlement_asset, asset_index = EXCLUDED.asset_index, price_decimals = EXCLUDED.price_decimals, size_decimals = EXCLUDED.size_decimals, tick_size = EXCLUDED.tick_size, lot_size = EXCLUDED.lot_size, is_hip3 = EXCLUDED.is_hip3, active = EXCLUDED.active, updated_at = EXCLUDED.updated_at",
        )
        .bind(&instrument.instrument_id)
        .bind(&instrument.symbol)
        .bind(&instrument.raw_symbol)
        .bind(match instrument.market_type {
            crate::hyperliquid::normalize::MarketType::Perp => "perp",
            crate::hyperliquid::normalize::MarketType::Spot => "spot",
            crate::hyperliquid::normalize::MarketType::Outcome => "outcome",
        })
        .bind(&instrument.base_asset)
        .bind(&instrument.quote_asset)
        .bind(&instrument.settlement_asset)
        .bind(instrument.asset_index)
        .bind(instrument.price_decimals)
        .bind(instrument.size_decimals)
        .bind(instrument.tick_size)
        .bind(instrument.lot_size)
        .bind(instrument.is_hip3)
        .bind(instrument.active)
        .bind(instrument.created_at)
        .bind(instrument.updated_at)
        .execute(pool)
        .await?;
    }

    Ok(())
}

async fn upsert_trade_fill(pool: &DbPool, row: TradeFillRow) -> Result<()> {
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

async fn upsert_funding_event(pool: &DbPool, row: FundingEventRow) -> Result<()> {
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

async fn upsert_ledger_event(pool: &DbPool, row: LedgerEventRow) -> Result<()> {
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

async fn upsert_historical_order(pool: &DbPool, row: HistoricalOrderRow) -> Result<()> {
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

#[allow(dead_code)]
fn _sync_state_row_example() -> SyncStateRow {
    SyncStateRow::new("0x0".to_string(), "mainnet".to_string(), SyncStream::Fills)
}
