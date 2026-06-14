use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
