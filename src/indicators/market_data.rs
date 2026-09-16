use std::{collections::BTreeSet, str::FromStr, time::Duration};

use anyhow::{Context, Result, anyhow, ensure};
use chrono::{DateTime, TimeZone, Utc};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    harness::timeframe::parse_timeframe_seconds,
    indicators::{model::Candle, runtime::MAX_INDICATOR_HISTORY_BARS},
};

const MAINNET_INFO_URL: &str = "https://api.hyperliquid.xyz/info";
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);

/// Full OHLCV candle client for deterministic indicator execution. It is
/// deliberately separate from the lossy operator UI market-data cache.
#[derive(Clone)]
pub struct IndicatorMarketDataClient {
    client: reqwest::Client,
    info_url: String,
}

impl IndicatorMarketDataClient {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(HTTP_TIMEOUT)
                .build()
                .expect("indicator market-data HTTP client configuration is valid"),
            info_url: MAINNET_INFO_URL.to_string(),
        }
    }

    #[cfg(test)]
    fn with_info_url(client: reqwest::Client, info_url: String) -> Self {
        Self { client, info_url }
    }

    /// Fetch exactly `requested_bars` complete candles whose close is no later
    /// than `boundary`. A shortened history is an error because Pine warm-up
    /// calculations would otherwise be silently wrong.
    pub async fn fetch_closed_candles(
        &self,
        instrument_id: &str,
        timeframe: &str,
        boundary: DateTime<Utc>,
        requested_bars: usize,
    ) -> Result<Vec<Candle>> {
        ensure!(
            !instrument_id.trim().is_empty(),
            "indicator instrument must not be blank"
        );
        ensure!(
            requested_bars > 0 && requested_bars <= MAX_INDICATOR_HISTORY_BARS,
            "indicator history must be 1-{MAX_INDICATOR_HISTORY_BARS} candles"
        );
        let interval_seconds = parse_timeframe_seconds(timeframe)?;
        let interval_millis = interval_seconds
            .checked_mul(1_000)
            .ok_or_else(|| anyhow!("indicator timeframe is too large"))?;
        let expanded_bars =
            i64::try_from(requested_bars).context("indicator history is too large")? + 2;
        let start_time = boundary
            .timestamp_millis()
            .checked_sub(
                interval_millis
                    .checked_mul(expanded_bars)
                    .ok_or_else(|| anyhow!("indicator candle window is too large"))?,
            )
            .ok_or_else(|| anyhow!("indicator candle window is before the Unix epoch"))?;
        let raw: Vec<RawCandle> = self
            .post(json!({
                "type": "candleSnapshot",
                "req": {
                    "coin": instrument_id.trim(),
                    "interval": timeframe.trim(),
                    "startTime": start_time,
                    "endTime": boundary.timestamp_millis(),
                }
            }))
            .await?;
        let candles = normalize_closed_candles(raw, boundary, interval_millis)?;
        ensure!(
            candles.len() >= requested_bars,
            "Hyperliquid returned only {} closed candles; {requested_bars} are required",
            candles.len()
        );
        let skip = candles.len() - requested_bars;
        Ok(candles.into_iter().skip(skip).collect())
    }

    async fn post<T>(&self, payload: Value) -> Result<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        self.client
            .post(&self.info_url)
            .json(&payload)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
            .context("failed to decode Hyperliquid candle response")
    }
}

impl Default for IndicatorMarketDataClient {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
struct RawCandle {
    #[serde(rename = "t", deserialize_with = "deserialize_timestamp")]
    opened_at_ms: i64,
    #[serde(rename = "o", deserialize_with = "deserialize_decimal")]
    open: Decimal,
    #[serde(rename = "h", deserialize_with = "deserialize_decimal")]
    high: Decimal,
    #[serde(rename = "l", deserialize_with = "deserialize_decimal")]
    low: Decimal,
    #[serde(rename = "c", deserialize_with = "deserialize_decimal")]
    close: Decimal,
    #[serde(rename = "v", deserialize_with = "deserialize_decimal")]
    volume: Decimal,
}

fn deserialize_timestamp<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    match value {
        Value::Number(value) => value
            .as_i64()
            .ok_or_else(|| serde::de::Error::custom("candle timestamp was not an integer")),
        Value::String(value) => value
            .parse()
            .map_err(|_| serde::de::Error::custom("candle timestamp was not numeric")),
        _ => Err(serde::de::Error::custom(
            "candle timestamp had an unexpected type",
        )),
    }
}

fn deserialize_decimal<'de, D>(deserializer: D) -> Result<Decimal, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    let text = match value {
        Value::String(value) => value,
        Value::Number(value) => value.to_string(),
        _ => return Err(serde::de::Error::custom("candle value was not numeric")),
    };
    Decimal::from_str(&text).map_err(|_| serde::de::Error::custom("candle value was not a decimal"))
}

fn normalize_closed_candles(
    raw: Vec<RawCandle>,
    boundary: DateTime<Utc>,
    interval_millis: i64,
) -> Result<Vec<Candle>> {
    let boundary_ms = boundary.timestamp_millis();
    let mut seen = BTreeSet::new();
    let mut candles = Vec::with_capacity(raw.len());
    for candle in raw {
        ensure!(
            seen.insert(candle.opened_at_ms),
            "Hyperliquid returned duplicate candle timestamps"
        );
        ensure!(
            candle.open > Decimal::ZERO
                && candle.high > Decimal::ZERO
                && candle.low > Decimal::ZERO
                && candle.close > Decimal::ZERO
                && candle.volume >= Decimal::ZERO,
            "Hyperliquid returned invalid nonpositive candle values"
        );
        ensure!(
            candle.low <= candle.open
                && candle.open <= candle.high
                && candle.low <= candle.close
                && candle.close <= candle.high,
            "Hyperliquid returned an inconsistent OHLC candle"
        );
        let closes_at = candle
            .opened_at_ms
            .checked_add(interval_millis)
            .ok_or_else(|| anyhow!("candle close time overflowed"))?;
        if closes_at <= boundary_ms {
            candles.push(Candle {
                opened_at: Utc
                    .timestamp_millis_opt(candle.opened_at_ms)
                    .single()
                    .ok_or_else(|| anyhow!("candle timestamp is out of range"))?,
                open: candle.open,
                high: candle.high,
                low: candle.low,
                close: candle.close,
                volume: candle.volume,
            });
        }
    }
    candles.sort_by_key(|candle| candle.opened_at);
    Ok(candles)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(value: Value) -> RawCandle {
        serde_json::from_value(value).expect("parse raw candle")
    }

    #[test]
    fn normalizes_full_ohlcv_and_excludes_open_candles() {
        let boundary = Utc
            .timestamp_millis_opt(180_000)
            .single()
            .expect("boundary");
        let candles = normalize_closed_candles(
            vec![
                raw(json!({"t": "120000", "o": "2", "h": "4", "l": "1", "c": "3", "v": "10"})),
                raw(json!({"t": 60_000, "o": 1, "h": 3, "l": 1, "c": 2, "v": 0})),
                raw(json!({"t": 180_000, "o": 3, "h": 5, "l": 2, "c": 4, "v": 11})),
            ],
            boundary,
            60_000,
        )
        .expect("normalize candles");
        assert_eq!(candles.len(), 2);
        assert_eq!(candles[0].opened_at.timestamp_millis(), 60_000);
        assert_eq!(candles[1].close, Decimal::from(3));
    }

    #[test]
    fn rejects_duplicate_or_inconsistent_candles() {
        let boundary = Utc
            .timestamp_millis_opt(180_000)
            .single()
            .expect("boundary");
        let duplicate = json!({"t": 60_000, "o": 1, "h": 3, "l": 1, "c": 2, "v": 1});
        assert!(
            normalize_closed_candles(
                vec![raw(duplicate.clone()), raw(duplicate)],
                boundary,
                60_000
            )
            .is_err()
        );
        assert!(
            normalize_closed_candles(
                vec![raw(
                    json!({"t": 60_000, "o": 4, "h": 3, "l": 1, "c": 2, "v": 1})
                )],
                boundary,
                60_000
            )
            .is_err()
        );
    }

    #[test]
    fn client_constructor_can_be_retargeted_for_http_tests() {
        let client = IndicatorMarketDataClient::with_info_url(
            reqwest::Client::new(),
            "http://127.0.0.1:1/info".to_string(),
        );
        assert_eq!(client.info_url, "http://127.0.0.1:1/info");
    }
}
