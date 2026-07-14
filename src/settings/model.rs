#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SystemSettingRow {
    pub value: String,
}
