use std::sync::Arc;

use axum::{
    Json,
    extract::{FromRef, FromRequestParts},
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Response},
};
use serde_json::json;
use tracing::warn;

use crate::{
    agents::store::{resolve_agent_key_by_api_key, touch_api_key_last_used},
    web::AppState,
};

/// An authenticated agent, resolved from the `Authorization: Bearer <api_key>`
/// header. The resolved `agent_key` is the ownership boundary for every
/// subsequent store call — the agent only ever sees its own rows.
#[derive(Debug, Clone)]
pub struct AuthenticatedAgent {
    pub agent_key: String,
}

impl<S> FromRequestParts<S> for AuthenticatedAgent
where
    Arc<AppState>: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = AuthRejection;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let state = Arc::<AppState>::from_ref(state);
        let pool = state.db_pool.clone();

        let header = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok());

        let token = match header.and_then(|h| h.strip_prefix("Bearer ")) {
            Some(token) if !token.is_empty() => token.to_string(),
            _ => return Err(AuthRejection::missing_or_malformed()),
        };

        let agent_key = match resolve_agent_key_by_api_key(&pool, &token).await {
            Ok(Some(key)) => key,
            Ok(None) => return Err(AuthRejection::invalid()),
            Err(error) => {
                warn!(error = ?error, "failed to resolve agent_key by api_key");
                return Err(AuthRejection::server_error());
            }
        };

        if let Err(error) = touch_api_key_last_used(&pool, &token).await {
            warn!(error = ?error, "failed to touch api_key_last_used_at");
        }

        Ok(AuthenticatedAgent { agent_key })
    }
}

/// 401/500 rejection type for the API-key auth extractor. Renders JSON
/// `{ "error": ... }` so the agent API does not depend on the operator
/// HTML error pages.
#[derive(Debug)]
pub struct AuthRejection {
    status: StatusCode,
    message: &'static str,
}

impl AuthRejection {
    fn missing_or_malformed() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: "missing or malformed Authorization header",
        }
    }

    fn invalid() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: "invalid api key",
        }
    }

    fn server_error() -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: "authentication failed",
        }
    }
}

impl IntoResponse for AuthRejection {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}
