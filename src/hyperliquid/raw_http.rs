use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::hyperliquid::config::HyperliquidEnvironment;

const MAINNET_INFO_URL: &str = "https://api.hyperliquid.xyz/info";
const TESTNET_INFO_URL: &str = "https://api.hyperliquid-testnet.xyz/info";

#[derive(Debug, Clone)]
pub struct RawHttpConfig {
    pub environment: HyperliquidEnvironment,
    pub account_address: String,
}

#[derive(Debug, Clone)]
pub struct RawHyperliquidHttpClient {
    client: reqwest::Client,
    pub config: RawHttpConfig,
}

impl RawHyperliquidHttpClient {
    pub fn new(config: RawHttpConfig) -> Self {
        Self {
            client: reqwest::Client::new(),
            config,
        }
    }

    pub async fn user_fills_by_time(
        &self,
        start_time: u64,
        end_time: Option<u64>,
    ) -> Result<Vec<RawUserFill>> {
        let mut body = json!({
            "type": "userFillsByTime",
            "user": self.config.account_address,
            "startTime": start_time,
            "aggregateByTime": false,
        });
        if let Some(end_time) = end_time {
            body["endTime"] = json!(end_time);
        }
        self.post(body).await
    }

    pub async fn user_funding(
        &self,
        start_time: u64,
        end_time: Option<u64>,
    ) -> Result<Vec<RawUserFunding>> {
        let mut body = json!({
            "type": "userFunding",
            "user": self.config.account_address,
            "startTime": start_time,
        });
        if let Some(end_time) = end_time {
            body["endTime"] = json!(end_time);
        }
        let response: Value = self.post_value(body).await?;

        parse_user_funding_response(response)
    }

    pub async fn non_user_funding_updates(
        &self,
        start_time: u64,
        end_time: Option<u64>,
    ) -> Result<Vec<RawLedgerUpdate>> {
        let mut body = json!({
            "type": "userNonFundingLedgerUpdates",
            "user": self.config.account_address,
            "startTime": start_time,
        });
        if let Some(end_time) = end_time {
            body["endTime"] = json!(end_time);
        }
        let response: Value = self.post_value(body).await?;
        parse_user_non_funding_ledger_updates_response(response)
    }

    pub async fn historical_orders(&self) -> Result<Vec<RawHistoricalOrder>> {
        self.post(json!({
            "type": "historicalOrders",
            "user": self.config.account_address,
        }))
        .await
    }

    async fn post_value(&self, payload: Value) -> Result<Value> {
        self.post(payload).await
    }

    async fn post<T>(&self, payload: Value) -> Result<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        let response = self
            .client
            .post(self.base_info_url())
            .json(&payload)
            .send()
            .await
            .with_context(|| "failed to call Hyperliquid /info")?
            .error_for_status()
            .with_context(|| {
                format!("Hyperliquid /info returned an error status for payload {payload}")
            })?;

        response
            .json()
            .await
            .with_context(|| "failed to decode Hyperliquid /info response")
    }

    fn base_info_url(&self) -> &'static str {
        match self.config.environment {
            HyperliquidEnvironment::Mainnet => MAINNET_INFO_URL,
            HyperliquidEnvironment::Testnet => TESTNET_INFO_URL,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawUserFill {
    pub coin: String,
    pub px: String,
    pub sz: String,
    pub side: String,
    pub time: u64,
    #[serde(rename = "startPosition")]
    pub start_position: Option<String>,
    pub dir: String,
    #[serde(rename = "closedPnl")]
    pub closed_pnl: Option<String>,
    pub hash: String,
    pub oid: Value,
    pub tid: Option<Value>,
    pub crossed: Option<bool>,
    pub fee: Option<String>,
    #[serde(rename = "feeToken")]
    pub fee_token: Option<String>,
    #[serde(rename = "builderFee")]
    pub builder_fee: Option<String>,
    #[serde(rename = "txHash")]
    pub tx_hash: Option<String>,
    #[serde(flatten)]
    pub extra: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawUserFunding {
    pub time: u64,
    pub coin: Option<String>,
    pub usdc: String,
    #[serde(rename = "fundingRate")]
    pub funding_rate: Option<String>,
    #[serde(rename = "szi")]
    pub position_size: Option<String>,
    pub hash: Option<String>,
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawLedgerUpdate {
    pub time: u64,
    pub hash: String,
    pub delta: Value,
    pub ledger_type: Option<String>,
    #[serde(rename = "usdc")]
    pub usdc: Option<String>,
    #[serde(rename = "token")]
    pub token: Option<String>,
    #[serde(rename = "amount")]
    pub amount: Option<String>,
    #[serde(rename = "fee")]
    pub fee: Option<String>,
    #[serde(rename = "sourceUser")]
    pub source_user: Option<String>,
    #[serde(rename = "destinationUser")]
    pub destination_user: Option<String>,
    #[serde(rename = "txHash")]
    pub tx_hash: Option<String>,
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawHistoricalOrder {
    #[serde(rename = "order")]
    pub order: Value,
    #[serde(rename = "status")]
    pub status: Option<String>,
    #[serde(rename = "statusTimestamp")]
    pub status_timestamp: Option<u64>,
    #[serde(flatten)]
    pub extra: Value,
}

fn parse_user_funding_response(response: Value) -> Result<Vec<RawUserFunding>> {
    let entries = response
        .as_array()
        .cloned()
        .with_context(|| "userFunding response was not an array")?;

    let mut results = Vec::with_capacity(entries.len());
    for entry in entries {
        let object = entry
            .as_object()
            .with_context(|| "userFunding entry was not an object")?;

        let delta = object.get("delta").and_then(Value::as_object);
        let coin = object
            .get("coin")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .or_else(|| {
                delta
                    .and_then(|delta| delta.get("coin"))
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
            });
        let usdc = object
            .get("usdc")
            .and_then(value_as_string)
            .or_else(|| {
                delta
                    .and_then(|delta| delta.get("usdc"))
                    .and_then(value_as_string)
            })
            .context("userFunding entry missing usdc")?;
        let funding_rate = object
            .get("fundingRate")
            .and_then(value_as_string)
            .or_else(|| {
                delta
                    .and_then(|delta| delta.get("fundingRate"))
                    .and_then(value_as_string)
            });
        let position_size = object.get("szi").and_then(value_as_string).or_else(|| {
            delta
                .and_then(|delta| delta.get("szi"))
                .and_then(value_as_string)
        });
        let hash = object.get("hash").and_then(value_as_string).or_else(|| {
            delta
                .and_then(|delta| delta.get("hash"))
                .and_then(value_as_string)
        });
        let time = object
            .get("time")
            .and_then(Value::as_u64)
            .context("userFunding entry missing time")?;

        results.push(RawUserFunding {
            time,
            coin,
            usdc,
            funding_rate,
            position_size,
            hash,
            payload: entry,
        });
    }

    Ok(results)
}

fn parse_user_non_funding_ledger_updates_response(response: Value) -> Result<Vec<RawLedgerUpdate>> {
    let entries = response
        .as_array()
        .cloned()
        .with_context(|| "userNonFundingLedgerUpdates response was not an array")?;

    let mut results = Vec::with_capacity(entries.len());
    for entry in entries {
        let object = entry
            .as_object()
            .with_context(|| "userNonFundingLedgerUpdates entry was not an object")?;

        let delta = object.get("delta").cloned().unwrap_or(Value::Null);
        let delta_object = delta.as_object();
        let ledger_type = object
            .get("type")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .or_else(|| {
                delta_object
                    .and_then(|delta| delta.get("type"))
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
            })
            .or_else(|| {
                delta_object
                    .and_then(first_object_key)
                    .map(ToOwned::to_owned)
            });

        results.push(RawLedgerUpdate {
            time: object
                .get("time")
                .and_then(Value::as_u64)
                .context("ledger entry missing time")?,
            hash: object
                .get("hash")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .context("ledger entry missing hash")?,
            usdc: object.get("usdc").and_then(value_as_string).or_else(|| {
                delta_object
                    .and_then(|delta| delta.get("usdc"))
                    .and_then(value_as_string)
            }),
            token: object.get("token").and_then(value_as_string).or_else(|| {
                delta_object
                    .and_then(|delta| delta.get("token"))
                    .and_then(value_as_string)
            }),
            amount: object.get("amount").and_then(value_as_string).or_else(|| {
                delta_object
                    .and_then(|delta| delta.get("amount"))
                    .and_then(value_as_string)
            }),
            fee: object.get("fee").and_then(value_as_string).or_else(|| {
                delta_object
                    .and_then(|delta| delta.get("fee"))
                    .and_then(value_as_string)
            }),
            source_user: object
                .get("sourceUser")
                .and_then(value_as_string)
                .or_else(|| {
                    delta_object
                        .and_then(|delta| delta.get("sourceUser"))
                        .and_then(value_as_string)
                }),
            destination_user: object
                .get("destinationUser")
                .and_then(value_as_string)
                .or_else(|| {
                    delta_object
                        .and_then(|delta| delta.get("destinationUser"))
                        .and_then(value_as_string)
                }),
            tx_hash: object.get("txHash").and_then(value_as_string).or_else(|| {
                delta_object
                    .and_then(|delta| delta.get("txHash"))
                    .and_then(value_as_string)
            }),
            delta,
            ledger_type,
            payload: entry,
        });
    }

    Ok(results)
}

fn first_object_key(value: &serde_json::Map<String, Value>) -> Option<&str> {
    value.keys().next().map(String::as_str)
}

fn value_as_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}
