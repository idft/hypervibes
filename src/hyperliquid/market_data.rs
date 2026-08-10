//! Public market data used by the operator UI.
//!
//! Market data is deliberately separate from account monitoring: it is not an
//! authoritative account snapshot and is never used to approve trading.

use std::{
    collections::{BTreeSet, HashMap},
    str::FromStr,
    sync::RwLock,
    time::{Duration, Instant},
};

use chrono::Utc;
use futures::StreamExt;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::Mutex;
use tracing::warn;

const MAINNET_INFO_URL: &str = "https://api.hyperliquid.xyz/info";
const MID_REFRESH_INTERVAL: Duration = Duration::from_secs(15);
const CANDLE_REFRESH_INTERVAL: Duration = Duration::from_secs(5 * 60);
const PRICE_HISTORY_HOURS: i64 = 24;
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_CONCURRENT_CANDLE_REQUESTS: usize = 4;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MarketPrice {
    pub current: Option<Decimal>,
    pub prices_24h: Vec<Decimal>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MarketDataSnapshot {
    prices: HashMap<String, MarketPrice>,
}

impl MarketDataSnapshot {
    pub fn price_for(&self, coin: &str) -> Option<&MarketPrice> {
        self.prices.get(coin)
    }
}

#[derive(Debug, Clone)]
struct CachedCandles {
    prices: Vec<Decimal>,
    updated_at: Instant,
}

#[derive(Default)]
struct MarketDataCache {
    mids: HashMap<String, Decimal>,
    mids_updated_at: Option<Instant>,
    candles: HashMap<String, CachedCandles>,
}

/// Shared, in-memory cache of public Hyperliquid prices for the operator UI.
pub struct MarketDataStore {
    client: reqwest::Client,
    allow_network: bool,
    cache: RwLock<MarketDataCache>,
    refresh_lock: Mutex<()>,
}

impl Default for MarketDataStore {
    fn default() -> Self {
        Self::new()
    }
}

impl MarketDataStore {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(HTTP_TIMEOUT)
                .build()
                .expect("market data HTTP client configuration is valid"),
            allow_network: cfg!(not(test)),
            cache: RwLock::new(MarketDataCache::default()),
            refresh_lock: Mutex::new(()),
        }
    }

    /// Refresh the requested symbols if their shared cache entries have aged
    /// out. Concurrent callers coalesce behind one refresh lock.
    pub async fn refresh(&self, coins: &[String]) {
        let coins: BTreeSet<&str> = coins
            .iter()
            .map(String::as_str)
            .filter(|coin| !coin.is_empty())
            .collect();
        if coins.is_empty() {
            return;
        }

        // Route tests construct the normal app state; never let that turn an
        // offline test into a public exchange request.
        if !self.allow_network {
            return;
        }

        let _refresh_guard = self.refresh_lock.lock().await;
        let now = Instant::now();
        let (refresh_mids, candle_coins) = {
            let cache = self
                .cache
                .read()
                .expect("market data cache lock is not poisoned");
            let refresh_mids = cache
                .mids_updated_at
                .is_none_or(|updated_at| now.duration_since(updated_at) >= MID_REFRESH_INTERVAL);
            let candle_coins = coins
                .iter()
                .filter(|coin| {
                    cache.candles.get(**coin).is_none_or(|cached| {
                        now.duration_since(cached.updated_at) >= CANDLE_REFRESH_INTERVAL
                    })
                })
                .map(|coin| (*coin).to_string())
                .collect::<Vec<_>>();
            (refresh_mids, candle_coins)
        };

        if refresh_mids {
            match self.fetch_all_mids().await {
                Ok(mids) => {
                    let mut cache = self
                        .cache
                        .write()
                        .expect("market data cache lock is not poisoned");
                    cache.mids = mids;
                    cache.mids_updated_at = Some(Instant::now());
                }
                Err(error) => warn!(error = ?error, "failed to refresh Hyperliquid mid prices"),
            }
        }

        let candle_results = futures::stream::iter(candle_coins)
            .map(|coin| async move {
                let result = self.fetch_hourly_closes(&coin).await;
                (coin, result)
            })
            .buffer_unordered(MAX_CONCURRENT_CANDLE_REQUESTS)
            .collect::<Vec<_>>()
            .await;
        for (coin, result) in candle_results {
            match result {
                Ok(prices) => {
                    let mut cache = self
                        .cache
                        .write()
                        .expect("market data cache lock is not poisoned");
                    cache.candles.insert(
                        coin,
                        CachedCandles {
                            prices,
                            updated_at: Instant::now(),
                        },
                    );
                }
                Err(error) => warn!(coin, error = ?error, "failed to refresh Hyperliquid candles"),
            }
        }
    }

    pub fn snapshot(&self) -> MarketDataSnapshot {
        let cache = self
            .cache
            .read()
            .expect("market data cache lock is not poisoned");
        let coins: BTreeSet<&str> = cache
            .mids
            .keys()
            .map(String::as_str)
            .chain(cache.candles.keys().map(String::as_str))
            .collect();
        let prices = coins
            .into_iter()
            .map(|coin| {
                let current = cache.mids.get(coin).copied();
                let mut prices_24h = cache
                    .candles
                    .get(coin)
                    .map(|cached| cached.prices.clone())
                    .unwrap_or_default();
                if let Some(current) = current
                    && prices_24h.last().copied() != Some(current)
                {
                    prices_24h.push(current);
                }
                (
                    coin.to_string(),
                    MarketPrice {
                        current,
                        prices_24h,
                    },
                )
            })
            .collect();
        MarketDataSnapshot { prices }
    }

    #[cfg(test)]
    pub fn replace_for_test(&self, prices: HashMap<String, MarketPrice>) {
        let now = Instant::now();
        let mut cache = self
            .cache
            .write()
            .expect("market data cache lock is not poisoned");
        cache.mids = prices
            .iter()
            .filter_map(|(coin, price)| price.current.map(|value| (coin.clone(), value)))
            .collect();
        cache.mids_updated_at = Some(now);
        cache.candles = prices
            .into_iter()
            .map(|(coin, price)| {
                (
                    coin,
                    CachedCandles {
                        prices: price.prices_24h,
                        updated_at: now,
                    },
                )
            })
            .collect();
    }

    async fn fetch_all_mids(&self) -> anyhow::Result<HashMap<String, Decimal>> {
        let response: HashMap<String, String> = self.post(json!({ "type": "allMids" })).await?;
        Ok(response
            .into_iter()
            .filter_map(|(coin, value)| Decimal::from_str(&value).ok().map(|value| (coin, value)))
            .collect())
    }

    async fn fetch_hourly_closes(&self, coin: &str) -> anyhow::Result<Vec<Decimal>> {
        let end_time = Utc::now().timestamp_millis();
        let start_time = end_time - chrono::Duration::hours(PRICE_HISTORY_HOURS).num_milliseconds();
        let candles: Vec<RawCandle> = self
            .post(json!({
                "type": "candleSnapshot",
                "req": {
                    "coin": coin,
                    "interval": "1h",
                    "startTime": start_time,
                    "endTime": end_time,
                }
            }))
            .await?;
        let mut candles = candles;
        candles.sort_by_key(|candle| candle.opened_at);
        Ok(candles
            .into_iter()
            .filter_map(|candle| Decimal::from_str(&candle.close).ok())
            .collect())
    }

    async fn post<T>(&self, payload: Value) -> anyhow::Result<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        let response = self
            .client
            .post(MAINNET_INFO_URL)
            .json(&payload)
            .send()
            .await?
            .error_for_status()?;
        Ok(response.json().await?)
    }
}

#[derive(Debug, Deserialize)]
struct RawCandle {
    #[serde(rename = "t", deserialize_with = "deserialize_timestamp")]
    opened_at: u64,
    #[serde(rename = "c")]
    close: String,
}

fn deserialize_timestamp<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    match value {
        Value::Number(value) => value.as_u64().ok_or_else(|| {
            serde::de::Error::custom("candle timestamp was not an unsigned integer")
        }),
        Value::String(value) => value
            .parse()
            .map_err(|_| serde::de::Error::custom("candle timestamp was not numeric")),
        _ => Err(serde::de::Error::custom(
            "candle timestamp had an unexpected type",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_numeric_or_string_candle_timestamps() {
        let numeric: RawCandle =
            serde_json::from_value(json!({ "t": 1_700_000_000_000u64, "c": "100" }))
                .expect("numeric timestamp parses");
        let string: RawCandle = serde_json::from_value(json!({ "t": "1700000000000", "c": "100" }))
            .expect("string timestamp parses");

        assert_eq!(numeric.opened_at, string.opened_at);
    }

    #[test]
    fn snapshot_appends_the_latest_mid_to_candle_history() {
        let store = MarketDataStore::new();
        store.replace_for_test(HashMap::from([(
            "BTC".to_string(),
            MarketPrice {
                current: Some(Decimal::new(101, 0)),
                prices_24h: vec![Decimal::new(99, 0), Decimal::new(100, 0)],
            },
        )]));

        let price = store
            .snapshot()
            .price_for("BTC")
            .cloned()
            .expect("BTC snapshot");
        assert_eq!(price.current, Some(Decimal::new(101, 0)));
        assert_eq!(price.prices_24h.last(), Some(&Decimal::new(101, 0)));
    }
}
