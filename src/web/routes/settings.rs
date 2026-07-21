use std::sync::Arc;

use askama::Template;
use axum::{
    Form,
    extract::State,
    response::{Html, IntoResponse, Redirect, Response},
};
use serde::Deserialize;

use crate::{
    settings::store::{get_user_settings, upsert_user_system_prompt},
    web::{AppState, auth::AuthenticatedUser, error::AppError, templates::SettingsPageTemplate},
};
pub(in crate::web::routes) async fn settings_index(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
) -> Result<Response, AppError> {
    let system_prompt = get_user_settings(&state.db_pool, user.id)
        .await?
        .map(|settings| settings.opencode_system_prompt)
        .unwrap_or_default();
    Ok(Html(
        SettingsPageTemplate {
            system_prompt,
            current_path: "/settings".to_string(),
        }
        .render()?,
    )
    .into_response())
}

#[derive(Deserialize)]
pub(in crate::web::routes) struct SettingsForm {
    system_prompt: String,
}
pub(in crate::web::routes) async fn settings_update(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Form(form): Form<SettingsForm>,
) -> Result<Response, AppError> {
    upsert_user_system_prompt(&state.db_pool, user.id, form.system_prompt.trim()).await?;
    Ok(Redirect::to("/settings").into_response())
}
