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
    harness::{model::RunApiScope, store::authenticate_run_runtime_credential},
    web::AppState,
};

/// An authenticated agent, resolved from the `Authorization: Bearer <api_key>`
/// header. The resolved `agent_key` is the ownership boundary for every
/// subsequent store call — the agent only ever sees its own rows.
#[derive(Debug, Clone)]
pub struct AuthenticatedAgent {
    pub agent_key: String,
    credential: AuthenticatedAgentCredential,
}

#[derive(Debug, Clone)]
enum AuthenticatedAgentCredential {
    Permanent,
    Run {
        run_id: i64,
        capability_schema_version: i32,
        api_scopes: Vec<RunApiScope>,
    },
}

impl AuthenticatedAgent {
    /// Permanent agent credentials preserve the existing operator API surface.
    /// Run credentials are restricted to the scope frozen at materialization.
    pub fn permits(&self, scope: RunApiScope) -> bool {
        match &self.credential {
            AuthenticatedAgentCredential::Permanent => true,
            AuthenticatedAgentCredential::Run { api_scopes, .. } => api_scopes.contains(&scope),
        }
    }

    pub fn run_provenance(&self) -> Option<(i64, i32)> {
        match &self.credential {
            AuthenticatedAgentCredential::Permanent => None,
            AuthenticatedAgentCredential::Run {
                run_id,
                capability_schema_version,
                ..
            } => Some((*run_id, *capability_schema_version)),
        }
    }

    pub fn is_run_credential(&self) -> bool {
        matches!(self.credential, AuthenticatedAgentCredential::Run { .. })
    }
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

        let runtime_credential = match authenticate_run_runtime_credential(&pool, &token).await {
            Ok(credential) => credential,
            Err(error) => {
                warn!(error = ?error, "failed to authenticate run runtime credential");
                return Err(AuthRejection::server_error());
            }
        };
        if let Some(credential) = runtime_credential {
            return Ok(AuthenticatedAgent {
                agent_key: credential.agent_key,
                credential: AuthenticatedAgentCredential::Run {
                    run_id: credential.run_id,
                    capability_schema_version: credential.capability_schema_version,
                    api_scopes: credential.api_scopes,
                },
            });
        }

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

        Ok(AuthenticatedAgent {
            agent_key,
            credential: AuthenticatedAgentCredential::Permanent,
        })
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
