use chrono::{DateTime, Utc};

#[derive(Debug, Clone, sqlx::FromRow)]
#[cfg_attr(not(test), allow(dead_code))]
pub struct SystemSettingRow {
    pub value: String,
    pub description: Option<String>,
    pub updated_at: DateTime<Utc>,
}
