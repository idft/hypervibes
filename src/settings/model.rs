#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SystemSettingRow {
    pub value: String,
    pub description: Option<String>,
    pub updated_at: DateTime<Utc>,
}
use chrono::{DateTime, Utc};
