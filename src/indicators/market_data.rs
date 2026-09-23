use std::{
    collections::{BTreeSet, HashMap, VecDeque},
    hash::Hash,
    str::FromStr,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context, Result as AnyhowResult, anyhow, ensure};
use async_trait::async_trait;
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
pub const DEFAULT_CANDLE_CACHE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandleFetchErrorKind {
    Retryable,
    Terminal,
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub struct CandleFetchError {
    pub kind: CandleFetchErrorKind,
    message: String,
}

impl CandleFetchError {
    pub(crate) fn retryable(error: impl std::fmt::Display) -> Self {
        Self {
            kind: CandleFetchErrorKind::Retryable,
            message: error.to_string(),
        }
    }

    fn terminal(error: impl std::fmt::Display) -> Self {
        Self {
            kind: CandleFetchErrorKind::Terminal,
            message: error.to_string(),
        }
    }
}

#[async_trait]
pub trait IndicatorCandleClient: Send + Sync {
    async fn fetch_closed_candles(
        &self,
        instrument_id: &str,
        timeframe: &str,
        boundary: DateTime<Utc>,
        requested_bars: usize,
    ) -> Result<Vec<Candle>, CandleFetchError>;
}

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
    ) -> Result<Vec<Candle>, CandleFetchError> {
        <Self as IndicatorCandleClient>::fetch_closed_candles(
            self,
            instrument_id,
            timeframe,
            boundary,
            requested_bars,
        )
        .await
    }

    async fn fetch_closed_candles_inner(
        &self,
        instrument_id: &str,
        timeframe: &str,
        boundary: DateTime<Utc>,
        requested_bars: usize,
    ) -> AnyhowResult<Vec<Candle>> {
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
            .await
            .map_err(|error| anyhow!(error))?;
        let candles = normalize_closed_candles(raw, boundary, interval_millis)
            .map_err(|error| anyhow!(CandleFetchError::retryable(error)))?;
        if candles.len() < requested_bars {
            return Err(anyhow!(CandleFetchError::retryable(format!(
                "Hyperliquid returned only {} closed candles; {requested_bars} are required",
                candles.len()
            ))));
        }
        let skip = candles.len() - requested_bars;
        Ok(candles.into_iter().skip(skip).collect())
    }

    async fn post<T>(&self, payload: Value) -> Result<T, CandleFetchError>
    where
        T: for<'de> Deserialize<'de>,
    {
        let response = self
            .client
            .post(&self.info_url)
            .json(&payload)
            .send()
            .await
            .map_err(classify_request_error)?;
        let status = response.status();
        if !status.is_success() {
            let error = format!("Hyperliquid candle request failed with HTTP {status}");
            return Err(if is_retryable_status(status) {
                CandleFetchError::retryable(error)
            } else {
                CandleFetchError::terminal(error)
            });
        }
        response.json().await.map_err(|error| {
            CandleFetchError::retryable(format!(
                "failed to decode Hyperliquid candle response: {error}"
            ))
        })
    }
}

fn is_retryable_status(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::REQUEST_TIMEOUT
        || status == reqwest::StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
}

fn classify_request_error(error: reqwest::Error) -> CandleFetchError {
    if error.is_connect() || error.is_timeout() {
        CandleFetchError::retryable(error)
    } else {
        CandleFetchError::terminal(error)
    }
}

#[async_trait]
impl IndicatorCandleClient for IndicatorMarketDataClient {
    async fn fetch_closed_candles(
        &self,
        instrument_id: &str,
        timeframe: &str,
        boundary: DateTime<Utc>,
        requested_bars: usize,
    ) -> Result<Vec<Candle>, CandleFetchError> {
        match self
            .fetch_closed_candles_inner(instrument_id, timeframe, boundary, requested_bars)
            .await
        {
            Ok(candles) => Ok(candles),
            Err(error) => match error.downcast::<CandleFetchError>() {
                Ok(error) => Err(error),
                Err(error) => Err(CandleFetchError::terminal(error)),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CandleRequestKey {
    instrument_id: String,
    timeframe: String,
    boundary: DateTime<Utc>,
    requested_bars: usize,
}

struct InFlightFetch {
    result: Mutex<Option<Result<Arc<Vec<Candle>>, CandleFetchError>>>,
    completed: tokio::sync::Notify,
}

struct CachedCandles {
    candles: Arc<Vec<Candle>>,
    estimated_bytes: usize,
}

#[derive(Default)]
struct CandleCacheState {
    entries: HashMap<CandleRequestKey, CachedCandles>,
    lru: VecDeque<CandleRequestKey>,
    in_flight: HashMap<CandleRequestKey, Arc<InFlightFetch>>,
    bytes: usize,
}

#[derive(Clone)]
pub struct CandleFetchCache {
    client: Arc<dyn IndicatorCandleClient>,
    state: Arc<Mutex<CandleCacheState>>,
    max_bytes: usize,
}

impl CandleFetchCache {
    pub fn new(client: Arc<dyn IndicatorCandleClient>) -> Self {
        Self::with_capacity(client, DEFAULT_CANDLE_CACHE_BYTES)
    }

    pub fn with_capacity(client: Arc<dyn IndicatorCandleClient>, max_bytes: usize) -> Self {
        Self {
            client,
            state: Arc::new(Mutex::new(CandleCacheState::default())),
            max_bytes,
        }
    }

    pub async fn fetch_closed_candles(
        &self,
        instrument_id: &str,
        timeframe: &str,
        boundary: DateTime<Utc>,
        requested_bars: usize,
    ) -> Result<Arc<Vec<Candle>>, CandleFetchError> {
        let key = CandleRequestKey {
            instrument_id: instrument_id.trim().to_string(),
            timeframe: timeframe.trim().to_string(),
            boundary,
            requested_bars,
        };
        let (in_flight, leader) = {
            let mut state = self.state.lock().expect("candle cache lock poisoned");
            if let Some(candles) = state
                .entries
                .get(&key)
                .map(|entry| Arc::clone(&entry.candles))
            {
                touch_lru(&mut state.lru, &key);
                return Ok(candles);
            }
            if let Some(in_flight) = state.in_flight.get(&key) {
                (Arc::clone(in_flight), false)
            } else {
                let in_flight = Arc::new(InFlightFetch {
                    result: Mutex::new(None),
                    completed: tokio::sync::Notify::new(),
                });
                state.in_flight.insert(key.clone(), Arc::clone(&in_flight));
                (in_flight, true)
            }
        };

        if leader {
            let result = self
                .client
                .fetch_closed_candles(
                    &key.instrument_id,
                    &key.timeframe,
                    key.boundary,
                    key.requested_bars,
                )
                .await
                .and_then(|candles| {
                    validate_candle_window(
                        &candles,
                        &key.timeframe,
                        key.boundary,
                        key.requested_bars,
                    )?;
                    Ok(candles)
                })
                .map(Arc::new);
            {
                let mut state = self.state.lock().expect("candle cache lock poisoned");
                state.in_flight.remove(&key);
                if let Ok(candles) = &result {
                    insert_cached(&mut state, key.clone(), Arc::clone(candles), self.max_bytes);
                }
            }
            *in_flight
                .result
                .lock()
                .expect("in-flight candle lock poisoned") = Some(result.clone());
            in_flight.completed.notify_waiters();
            return result;
        }

        loop {
            let notified = in_flight.completed.notified();
            if let Some(result) = in_flight
                .result
                .lock()
                .expect("in-flight candle lock poisoned")
                .clone()
            {
                return result;
            }
            notified.await;
        }
    }
}

fn validate_candle_window(
    candles: &[Candle],
    timeframe: &str,
    boundary: DateTime<Utc>,
    requested_bars: usize,
) -> Result<(), CandleFetchError> {
    let interval_seconds =
        parse_timeframe_seconds(timeframe).map_err(CandleFetchError::terminal)?;
    let interval_millis = interval_seconds
        .checked_mul(1_000)
        .ok_or_else(|| CandleFetchError::terminal("indicator timeframe is too large"))?;
    if candles.len() != requested_bars {
        return Err(CandleFetchError::retryable(format!(
            "candle data is incomplete: received {} of {requested_bars} required candles",
            candles.len()
        )));
    }
    let Some(last) = candles.last() else {
        return Err(CandleFetchError::retryable("candle data is incomplete"));
    };
    let expected_last_open = boundary
        .timestamp_millis()
        .checked_sub(interval_millis)
        .ok_or_else(|| CandleFetchError::terminal("indicator candle boundary is out of range"))?;
    if last.opened_at.timestamp_millis() != expected_last_open {
        return Err(CandleFetchError::retryable(
            "candle data is incomplete at the requested boundary",
        ));
    }
    for (index, candle) in candles.iter().enumerate() {
        let opened_at = candle.opened_at.timestamp_millis();
        if opened_at.rem_euclid(interval_millis) != 0 {
            return Err(CandleFetchError::retryable(
                "candle data contains an unaligned timestamp",
            ));
        }
        if index > 0
            && opened_at - candles[index - 1].opened_at.timestamp_millis() != interval_millis
        {
            return Err(CandleFetchError::retryable(
                "candle data contains a gap or overlap",
            ));
        }
    }
    Ok(())
}

fn touch_lru(lru: &mut VecDeque<CandleRequestKey>, key: &CandleRequestKey) {
    if let Some(index) = lru.iter().position(|candidate| candidate == key) {
        lru.remove(index);
    }
    lru.push_back(key.clone());
}

fn insert_cached(
    state: &mut CandleCacheState,
    key: CandleRequestKey,
    candles: Arc<Vec<Candle>>,
    max_bytes: usize,
) {
    let estimated_bytes = std::mem::size_of::<Candle>()
        .saturating_mul(candles.len())
        .saturating_add(key.instrument_id.len())
        .saturating_add(key.timeframe.len());
    if estimated_bytes > max_bytes {
        return;
    }
    while state.bytes.saturating_add(estimated_bytes) > max_bytes {
        let Some(oldest) = state.lru.pop_front() else {
            break;
        };
        if let Some(removed) = state.entries.remove(&oldest) {
            state.bytes = state.bytes.saturating_sub(removed.estimated_bytes);
        }
    }
    state.bytes = state.bytes.saturating_add(estimated_bytes);
    state.entries.insert(
        key.clone(),
        CachedCandles {
            candles,
            estimated_bytes,
        },
    );
    touch_lru(&mut state.lru, &key);
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
) -> AnyhowResult<Vec<Candle>> {
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
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    struct FakeCandleClient {
        calls: AtomicUsize,
        fail_first: bool,
    }

    fn aligned_hour_boundary() -> DateTime<Utc> {
        Utc.timestamp_opt(Utc::now().timestamp() / 3_600 * 3_600, 0)
            .single()
            .expect("aligned hour boundary")
    }

    #[async_trait]
    impl IndicatorCandleClient for FakeCandleClient {
        async fn fetch_closed_candles(
            &self,
            _instrument_id: &str,
            _timeframe: &str,
            boundary: DateTime<Utc>,
            _requested_bars: usize,
        ) -> Result<Vec<Candle>, CandleFetchError> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(25)).await;
            if self.fail_first && call == 0 {
                return Err(CandleFetchError::retryable("temporary failure"));
            }
            let seconds = parse_timeframe_seconds(_timeframe).expect("test timeframe");
            Ok(vec![Candle {
                opened_at: boundary - chrono::Duration::seconds(seconds),
                open: Decimal::ONE,
                high: Decimal::ONE,
                low: Decimal::ONE,
                close: Decimal::ONE,
                volume: Decimal::ZERO,
            }])
        }
    }

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

    #[test]
    fn http_408_is_retryable() {
        assert!(is_retryable_status(reqwest::StatusCode::REQUEST_TIMEOUT));
        assert!(!is_retryable_status(reqwest::StatusCode::BAD_REQUEST));
    }

    #[test]
    fn candle_windows_must_be_aligned_contiguous_and_close_at_the_boundary() {
        let boundary = Utc
            .timestamp_millis_opt(180_000)
            .single()
            .expect("boundary");
        let candle = |opened_at_ms| Candle {
            opened_at: Utc
                .timestamp_millis_opt(opened_at_ms)
                .single()
                .expect("candle time"),
            open: Decimal::ONE,
            high: Decimal::ONE,
            low: Decimal::ONE,
            close: Decimal::ONE,
            volume: Decimal::ZERO,
        };
        assert!(
            validate_candle_window(&[candle(60_000), candle(120_000)], "1m", boundary, 2).is_ok()
        );

        for invalid in [
            vec![candle(60_001), candle(120_000)],
            vec![candle(0), candle(120_000)],
            vec![candle(0), candle(60_000)],
            vec![candle(120_000)],
        ] {
            let error = validate_candle_window(&invalid, "1m", boundary, 2)
                .expect_err("invalid window must fail");
            assert_eq!(error.kind, CandleFetchErrorKind::Retryable);
        }
    }

    struct LaggingCandleClient {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl IndicatorCandleClient for LaggingCandleClient {
        async fn fetch_closed_candles(
            &self,
            _instrument_id: &str,
            _timeframe: &str,
            boundary: DateTime<Utc>,
            _requested_bars: usize,
        ) -> Result<Vec<Candle>, CandleFetchError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(vec![Candle {
                opened_at: boundary - chrono::Duration::minutes(2),
                open: Decimal::ONE,
                high: Decimal::ONE,
                low: Decimal::ONE,
                close: Decimal::ONE,
                volume: Decimal::ZERO,
            }])
        }
    }

    #[tokio::test]
    async fn cache_rejects_and_does_not_store_lagging_candles() {
        let client = Arc::new(LaggingCandleClient {
            calls: AtomicUsize::new(0),
        });
        let cache = CandleFetchCache::new(client.clone());
        let boundary = Utc
            .timestamp_millis_opt(180_000)
            .single()
            .expect("boundary");
        for _ in 0..2 {
            let error = cache
                .fetch_closed_candles("BTC", "1m", boundary, 1)
                .await
                .expect_err("lagging data must remain retryable");
            assert_eq!(error.kind, CandleFetchErrorKind::Retryable);
        }
        assert_eq!(client.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn cache_coalesces_matching_in_flight_requests() {
        let client = Arc::new(FakeCandleClient {
            calls: AtomicUsize::new(0),
            fail_first: false,
        });
        let cache = CandleFetchCache::new(client.clone());
        let boundary = aligned_hour_boundary();
        let (first, second) = tokio::join!(
            cache.fetch_closed_candles("BTC", "1h", boundary, 1),
            cache.fetch_closed_candles("BTC", "1h", boundary, 1),
        );
        assert!(Arc::ptr_eq(
            &first.expect("first fetch"),
            &second.expect("second fetch")
        ));
        assert_eq!(client.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn cache_does_not_store_failures_or_cross_request_keys() {
        let client = Arc::new(FakeCandleClient {
            calls: AtomicUsize::new(0),
            fail_first: true,
        });
        let cache = CandleFetchCache::new(client.clone());
        let boundary = aligned_hour_boundary();
        assert!(
            cache
                .fetch_closed_candles("BTC", "1h", boundary, 1)
                .await
                .is_err()
        );
        cache
            .fetch_closed_candles("BTC", "1h", boundary, 1)
            .await
            .expect("retry fetch");
        cache
            .fetch_closed_candles("BTC", "1h", boundary + chrono::Duration::hours(1), 1)
            .await
            .expect("different boundary");
        assert_eq!(client.calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn cache_evicts_by_estimated_bytes() {
        let client = Arc::new(FakeCandleClient {
            calls: AtomicUsize::new(0),
            fail_first: false,
        });
        let one_entry = std::mem::size_of::<Candle>() + "BTC".len() + "1h".len();
        let cache = CandleFetchCache::with_capacity(client.clone(), one_entry);
        let boundary = aligned_hour_boundary();
        cache
            .fetch_closed_candles("BTC", "1h", boundary, 1)
            .await
            .expect("first key");
        cache
            .fetch_closed_candles("BTC", "1h", boundary + chrono::Duration::hours(1), 1)
            .await
            .expect("second key");
        cache
            .fetch_closed_candles("BTC", "1h", boundary, 1)
            .await
            .expect("evicted first key");
        assert_eq!(client.calls.load(Ordering::SeqCst), 3);
    }
}
