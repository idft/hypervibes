use axum::{Json, http::StatusCode, response::{IntoResponse, Response}};
use serde_json::json;
use tracing::error;

/// JSON error type for the agent API. Distinct from the operator HTML
/// `AppError` in `src/web/routes.rs` — the agent API must always return
/// `{ "error": "..." }` so trading agents can parse it.
#[derive(Debug)]
pub(super) enum ApiError {
    BadRequest(String),
    NotFound(&'static str),
    Validation(String),
    BadUuid,
    Internal(anyhow::Error),
}

impl ApiError {
    pub(super) fn message(&self) -> String {
        match self {
            ApiError::BadRequest(msg) => msg.clone(),
            ApiError::NotFound(msg) => (*msg).to_string(),
            ApiError::Validation(msg) => msg.clone(),
            ApiError::BadUuid => "invalid memory id".to_string(),
            ApiError::Internal(_) => "internal server error".to_string(),
        }
    }

    pub(super) fn status(&self) -> StatusCode {
        match self {
            ApiError::BadRequest(_) => StatusCode::BAD_REQUEST,
            ApiError::NotFound(_) => StatusCode::NOT_FOUND,
            ApiError::Validation(_) => StatusCode::UNPROCESSABLE_ENTITY,
            ApiError::BadUuid => StatusCode::NOT_FOUND,
            ApiError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl<E> From<E> for ApiError
where
    E: Into<anyhow::Error>,
{
    fn from(error: E) -> Self {
        ApiError::Internal(error.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        if let ApiError::Internal(ref e) = self {
            error!(error = ?e, "agent API request failed");
        }
        let body = Json(json!({ "error": self.message() }));
        (self.status(), body).into_response()
    }
}