use askama::Template;
use axum::extract::OriginalUri;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use tracing::error;

use crate::web::templates::{Navbar, NotFoundPageTemplate, ServerErrorPageTemplate};

#[derive(Debug)]
pub(crate) struct AppError(pub anyhow::Error);

impl<E> From<E> for AppError
where
    E: Into<anyhow::Error>,
{
    fn from(error: E) -> Self {
        Self(error.into())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        error!(error = ?self.0, "request failed");

        let template = ServerErrorPageTemplate {
            message: format!("Internal server error: {}", self.0),
            current_path: String::new(),
            navbar: Navbar::default(),
        };

        match template.render() {
            Ok(body) => (StatusCode::INTERNAL_SERVER_ERROR, Html(body)).into_response(),
            Err(render_error) => {
                error!(error = ?render_error, "failed to render server error page");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("internal server error: {}", self.0),
                )
                    .into_response()
            }
        }
    }
}

pub(crate) async fn not_found(OriginalUri(uri): OriginalUri) -> Response {
    not_found_response(uri.path())
}

pub(crate) fn not_found_response(path: &str) -> Response {
    let template = NotFoundPageTemplate {
        current_path: path.to_string(),
        navbar: Navbar::default(),
    };

    match template.render() {
        Ok(body) => (StatusCode::NOT_FOUND, Html(body)).into_response(),
        Err(render_error) => {
            error!(error = ?render_error, "failed to render not found page");
            StatusCode::NOT_FOUND.into_response()
        }
    }
}
