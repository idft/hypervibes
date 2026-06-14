use chrono::{DateTime, TimeZone, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarketType {
    Perp,
    Spot,
    Outcome,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstrumentRow {
    pub instrument_id: String,
    pub symbol: String,
    pub raw_symbol: String,
    pub market_type: MarketType,
    pub base_asset: String,
    pub quote_asset: String,
    pub settlement_asset: Option<String>,
    pub asset_index: Option<i32>,
    pub price_decimals: i32,
    pub size_decimals: i32,
    pub tick_size: Decimal,
    pub lot_size: Decimal,
    pub is_hip3: bool,
    pub active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeFillRow {
    pub hash: String,
    pub account_address: String,
    pub environment: String,
    pub event_time: DateTime<Utc>,
    pub event_type: String,
    pub source_stream: String,
    pub instrument_id: Option<String>,
    pub asset: Option<String>,
    pub symbol: Option<String>,
    pub fee_usdc: Option<Decimal>,
    pub realized_pnl_usdc: Option<Decimal>,
    pub fill_time: DateTime<Utc>,
    pub direction: String,
    pub side: String,
    pub price: Decimal,
    pub size: Decimal,
    pub trade_value: Option<Decimal>,
    pub order_id: Option<String>,
    pub trade_id: String,
    pub start_position: Option<Decimal>,
    pub fee: Option<Decimal>,
    pub fee_token: Option<String>,
    pub builder_fee: Option<Decimal>,
    pub crossed: Option<bool>,
    pub tx_hash: Option<String>,
    pub payload: Value,
    pub ingest_source: String,
    pub inserted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FundingEventRow {
    pub account_address: String,
    pub environment: String,
    pub instrument_id: String,
    pub event_time: DateTime<Utc>,
    pub event_type: String,
    pub source_stream: String,
    pub asset: Option<String>,
    pub symbol: Option<String>,
    pub fee_usdc: Option<Decimal>,
    pub realized_pnl_usdc: Option<Decimal>,
    pub usdc: Decimal,
    pub position_size: Option<Decimal>,
    pub funding_rate: Option<Decimal>,
    pub hash: Option<String>,
    pub payload: Value,
    pub ingest_source: String,
    pub inserted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerEventRow {
    pub hash: String,
    pub account_address: String,
    pub environment: String,
    pub event_time: DateTime<Utc>,
    pub event_type: String,
    pub source_stream: String,
    pub instrument_id: Option<String>,
    pub asset: Option<String>,
    pub symbol: Option<String>,
    pub fee_usdc: Option<Decimal>,
    pub realized_pnl_usdc: Option<Decimal>,
    pub ledger_type: String,
    pub usdc: Option<Decimal>,
    pub token: Option<String>,
    pub amount: Option<Decimal>,
    pub fee: Option<Decimal>,
    pub source_user: Option<String>,
    pub destination_user: Option<String>,
    pub tx_hash: Option<String>,
    pub details: Value,
    pub payload: Value,
    pub ingest_source: String,
    pub inserted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoricalOrderRow {
    pub account_address: String,
    pub environment: String,
    pub order_id: String,
    pub event_time: DateTime<Utc>,
    pub source_stream: String,
    pub instrument_id: Option<String>,
    pub asset: Option<String>,
    pub symbol: Option<String>,
    pub order_status: Option<String>,
    pub side: Option<String>,
    pub order_type: Option<String>,
    pub price: Option<Decimal>,
    pub size: Option<Decimal>,
    pub filled_size: Option<Decimal>,
    pub reduce_only: Option<bool>,
    pub time_in_force: Option<String>,
    pub client_order_id: Option<String>,
    pub status_timestamp: Option<DateTime<Utc>>,
    pub payload: Value,
    pub ingest_source: String,
    pub inserted_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct InstrumentLookup<'a> {
    pub instrument_id: Option<&'a str>,
    pub symbol: Option<&'a str>,
    pub base_asset: Option<&'a str>,
}

pub fn ms_to_datetime(ms: u64) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(ms as i64)
        .single()
        .expect("hyperliquid timestamp should fit in chrono")
}

pub fn parse_decimal(value: &str) -> anyhow::Result<Decimal> {
    value.parse().map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::{ms_to_datetime, parse_decimal};

    #[test]
    fn parses_decimal_values() {
        let value = parse_decimal("123.45").expect("decimal should parse");
        assert_eq!(value.to_string(), "123.45");
    }

    #[test]
    fn converts_millis_to_datetime() {
        assert_eq!(ms_to_datetime(0).to_rfc3339(), "1970-01-01T00:00:00+00:00");
    }
}
