use std::str::FromStr;

use anyhow::{anyhow, Error};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(rename_all = "snake_case")]
pub enum SyncStream {
    Fills,
    Funding,
    Ledger,
    HistoricalOrders,
    Instruments,
}

impl SyncStream {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fills => "fills",
            Self::Funding => "funding",
            Self::Ledger => "ledger",
            Self::HistoricalOrders => "historical_orders",
            Self::Instruments => "instruments",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncStatus {
    Pending,
    Running,
    Healthy,
    Failed,
}

impl SyncStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Healthy => "healthy",
            Self::Failed => "failed",
        }
    }
}

impl FromStr for SyncStatus {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "pending" => Ok(Self::Pending),
            "running" => Ok(Self::Running),
            "healthy" => Ok(Self::Healthy),
            "failed" => Ok(Self::Failed),
            _ => Err(anyhow!("invalid sync status: {value}")),
        }
    }
}

impl<'r> sqlx::Decode<'r, sqlx::Postgres> for SyncStatus {
    fn decode(
        value: sqlx::postgres::PgValueRef<'r>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync + 'static>> {
        let value = <&str as sqlx::Decode<'r, sqlx::Postgres>>::decode(value)?;
        Self::from_str(value).map_err(Into::into)
    }
}

impl sqlx::Type<sqlx::Postgres> for SyncStatus {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <String as sqlx::Type<sqlx::Postgres>>::type_info()
    }

    fn compatible(ty: &sqlx::postgres::PgTypeInfo) -> bool {
        <String as sqlx::Type<sqlx::Postgres>>::compatible(ty)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct SyncStateRow {
    pub account_address: String,
    pub environment: String,
    pub stream_name: String,
    pub last_event_time: Option<DateTime<Utc>>,
    pub last_event_key: Option<String>,
    pub last_synced_at: Option<DateTime<Utc>>,
    pub status: SyncStatus,
    pub metadata: Value,
}

impl SyncStateRow {
    pub fn new(account_address: String, environment: String, stream: SyncStream) -> Self {
        Self {
            account_address,
            environment,
            stream_name: stream.as_str().to_string(),
            last_event_time: None,
            last_event_key: None,
            last_synced_at: None,
            status: SyncStatus::Pending,
            metadata: Value::Object(Default::default()),
        }
    }
}
