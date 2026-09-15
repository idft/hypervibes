use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
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

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct IndicatorDefinition {
    pub id: Uuid,
    pub agent_key: String,
    pub name: String,
    pub description: String,
    pub timeframe: String,
    pub enabled: bool,
    pub active_version_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct IndicatorVersion {
    pub id: Uuid,
    pub indicator_definition_id: Uuid,
    pub version_number: i32,
    pub source: String,
    pub source_sha256: String,
    pub compiler_version: String,
    pub metadata: Value,
    pub input_values: Value,
    pub created_by_kind: String,
    pub created_by_run_id: Option<i64>,
    pub created_by_conversation_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct IndicatorRun {
    pub id: Uuid,
    pub agent_key: String,
    pub indicator_definition_id: Uuid,
    pub indicator_version_id: Uuid,
    pub instrument_id: String,
    pub timeframe: String,
    pub scheduled_for: DateTime<Utc>,
    pub status: String,
    pub candle_data: Option<Value>,
    pub plot_data: Option<Value>,
    pub latest_values: Option<Value>,
    pub diagnostics: Value,
    pub error_summary: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
