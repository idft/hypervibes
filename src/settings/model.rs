#[derive(Debug, Clone, sqlx::FromRow)]
pub struct UserSettingsRow {
    pub opencode_system_prompt: String,
}
