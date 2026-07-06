use std::sync::Arc;

use askama::Template;
use axum::{
    Form,
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
};

use super::shared::unique_violation_message;
use crate::{
    agents::{
        model::CreateAgentRuntimeForm,
        store::{insert_agent_runtime, list_agent_runtimes},
    },
    web::{
        AppState,
        error::AppError,
        templates::{AgentRuntimeView, BackendsNewPageTemplate, BackendsPageTemplate},
    },
};
pub(in crate::web::routes) async fn backends_index(
    State(state): State<Arc<AppState>>,
) -> Result<Html<String>, AppError> {
    let template = BackendsPageTemplate {
        runtimes: list_agent_runtimes(&state.db_pool)
            .await?
            .into_iter()
            .map(AgentRuntimeView::from_row)
            .collect(),
        current_path: "/backends".to_string(),
    };
    Ok(Html(template.render()?))
}
pub(in crate::web::routes) async fn backends_new() -> Result<Html<String>, AppError> {
    let template = BackendsNewPageTemplate {
        form: CreateAgentRuntimeForm {
            backend_kind: crate::agents::model::BACKEND_KIND_OPENCODE.to_string(),
            enabled: Some("on".to_string()),
            ..Default::default()
        },
        errors: Vec::new(),
        current_path: "/backends/new".to_string(),
    };
    Ok(Html(template.render()?))
}
pub(in crate::web::routes) async fn create_backend(
    State(state): State<Arc<AppState>>,
    Form(form): Form<CreateAgentRuntimeForm>,
) -> Result<Response, AppError> {
    if let Err(errors) = form.validate() {
        return Ok(render_backend_form(form, errors));
    }

    if let Err(error) = insert_agent_runtime(&state.db_pool, &form).await {
        let errors = match unique_violation_message(&error) {
            Some(message) => vec![message],
            None => return Err(AppError(error)),
        };
        return Ok(render_backend_form(form, errors));
    }

    Ok(Redirect::to("/backends").into_response())
}
pub(in crate::web::routes) fn render_backend_form(
    form: CreateAgentRuntimeForm,
    errors: Vec<String>,
) -> Response {
    let template = BackendsNewPageTemplate {
        form,
        errors,
        current_path: "/backends/new".to_string(),
    };
    match template.render() {
        Ok(body) => (StatusCode::UNPROCESSABLE_ENTITY, Html(body)).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("template error: {error}"),
        )
            .into_response(),
    }
}
