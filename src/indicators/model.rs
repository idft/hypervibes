use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candle {
    pub opened_at: DateTime<Utc>,
    pub open: Decimal,
    pub high: Decimal,
    pub low: Decimal,
    pub close: Decimal,
    pub volume: Decimal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndicatorDiagnostic {
    pub severity: String,
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndicatorMetadata {
    pub name: String,
    pub overlay: bool,
    pub plot_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndicatorDefinition {
    pub id: Uuid,
    pub agent_key: String,
    pub name: String,
    pub description: String,
    pub timeframe: String,
    pub enabled: bool,
    pub active_version_id: Uuid,
}
