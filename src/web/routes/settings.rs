use std::sync::Arc;

use askama::Template;
use axum::{
    Form,
    extract::State,
    response::{Html, IntoResponse, Redirect, Response},
};
use serde::Deserialize;

use crate::{
    web::{
        error::AppError,
        AppState,
        templates::SettingsPageTemplate,
    },
};
#[derive(Debug, Clone, Default, Deserialize)]
pub(in crate::web::routes) struct SettingsUpdateForm {
    #[serde(default)]
    pub system_prompt: String,
}
pub(in crate::web::routes) async fn settings_index(
    State(state): State<Arc<AppState>>,
) -> Result<Response, AppError> {
    let row = crate::settings::store::get_setting(&state.db_pool, "opencode_system_prompt").await?;
    let system_prompt = row
        .map(|r| r.value)
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| crate::agents::prompts::DEFAULT_SYSTEM_PROMPT.to_string());
    let html = SettingsPageTemplate {
        system_prompt,
        current_path: "/settings".to_string(),
    }
    .render()?;
    Ok(Html(html).into_response())
}
pub(in crate::web::routes) async fn settings_update(
    State(state): State<Arc<AppState>>,
    Form(form): Form<SettingsUpdateForm>,
) -> Result<Response, AppError> {
    crate::settings::store::upsert_setting(
        &state.db_pool,
        "opencode_system_prompt",
        &form.system_prompt,
        Some("Base system prompt prepended to every OpenCode agent job prompt."),
    )
    .await?;
    Ok(Redirect::to("/settings").into_response())
}
