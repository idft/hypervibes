use std::sync::Arc;

use askama::Template;
use axum::{
    Form, Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use chrono::Utc;
use serde::Serialize;

use crate::{
    agents::{
        crypto::{encrypt, generate_api_key},
        keys::derive_wallet_address,
        model::{AgentRegistryRow, CreateAgentForm, slugify_agent_key},
        store::{delete_agent as delete_agent_in_store, get_agent, insert_agent, list_agents},
    },
    web::{
        AppState,
        templates::{
            AgentsNewPageTemplate, AgentsPageTemplate, AgentsShowPageTemplate, SummaryCard,
        },
    },
};

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(root))
        .route("/healthz", get(healthz))
        .route("/agents", get(agents_index).post(create_agent))
        .route("/agents/new", get(agents_new))
        .route("/agents/{agent_key}", get(agents_show))
        .route("/agents/{agent_key}/delete", post(delete_agent))
        .with_state(state)
}

async fn root() -> Redirect {
    Redirect::to("/agents")
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
}

async fn healthz(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let _ = &state.db_pool;
    (StatusCode::OK, Json(HealthResponse { status: "ok" }))
}

async fn agents_index(State(state): State<Arc<AppState>>) -> Result<Html<String>, AppError> {
    let agents = list_agents(&state.db_pool).await?;

    let active = agents.iter().filter(|a| a.enabled).count();
    let disabled = agents.len() - active;

    let template = AgentsPageTemplate {
        summary_cards: vec![
            SummaryCard {
                label: "Active agents",
                value: active.to_string(),
                detail: "Enabled for runtime supervision",
            },
            SummaryCard {
                label: "Disabled agents",
                value: disabled.to_string(),
                detail: "Retained but not scheduled",
            },
            SummaryCard {
                label: "Registered agents",
                value: agents.len().to_string(),
                detail: "Total agents in the registry",
            },
            SummaryCard {
                label: "API keys",
                value: agents.len().to_string(),
                detail: "One app credential per agent",
            },
        ],
        agents,
    };

    Ok(Html(template.render()?))
}

async fn agents_new() -> Result<Html<String>, AppError> {
    let template = AgentsNewPageTemplate {
        form: CreateAgentForm::default(),
        errors: Vec::new(),
    };
    Ok(Html(template.render()?))
}

async fn agents_show(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    match get_agent(&state.db_pool, &agent_key).await? {
        Some(agent) => {
            let template = AgentsShowPageTemplate { agent };
            Ok(Html(template.render()?).into_response())
        }
        None => Ok((StatusCode::NOT_FOUND, "agent not found").into_response()),
    }
}

async fn delete_agent(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    let deleted = delete_agent_in_store(&state.db_pool, &agent_key).await?;
    if deleted {
        Ok(Redirect::to("/agents").into_response())
    } else {
        Ok((StatusCode::NOT_FOUND, "agent not found").into_response())
    }
}

async fn create_agent(
    State(state): State<Arc<AppState>>,
    Form(form): Form<CreateAgentForm>,
) -> Result<Response, AppError> {
    if let Err(errors) = form.validate() {
        return Ok(render_new_form(form, errors));
    }

    let wallet_address = match derive_wallet_address(&form.hyperliquid_private_key) {
        Ok(addr) => addr,
        Err(e) => {
            return Ok(render_new_form(
                form,
                vec![format!("Hyperliquid private key is invalid: {e}")],
            ));
        }
    };

    let ciphertext = match encrypt(&state.encryption_key, &form.hyperliquid_private_key) {
        Ok(ct) => ct,
        Err(e) => {
            return Ok(render_new_form(
                form,
                vec![format!("Failed to encrypt private key: {e}")],
            ));
        }
    };

    let now = Utc::now();
    let agent_key = slugify_agent_key(&form.display_name);

    let row = AgentRegistryRow {
        agent_key,
        created_at: now,
        updated_at: now,
        enabled: form.enabled(),
        display_name: form.display_name.trim().to_string(),
        prompt: String::new(),
        wallet_address,
        api_key: generate_api_key(),
        api_key_last_used_at: None,
        hyperliquid_private_key_ciphertext: ciphertext,
        hyperliquid_private_key_key_id: state.encryption_key.key_id.clone(),
    };

    if let Err(e) = insert_agent(&state.db_pool, &row).await {
        let errors = match unique_violation_message(&e) {
            Some(msg) => vec![msg],
            None => {
                return Err(AppError(e));
            }
        };
        return Ok(render_new_form(form, errors));
    }

    Ok(Redirect::to("/agents").into_response())
}

fn render_new_form(form: CreateAgentForm, errors: Vec<String>) -> Response {
    let template = AgentsNewPageTemplate { form, errors };
    match template.render() {
        Ok(body) => (StatusCode::UNPROCESSABLE_ENTITY, Html(body)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("template error: {e}"),
        )
            .into_response(),
    }
}

fn unique_violation_message(error: &anyhow::Error) -> Option<String> {
    let db_err = error.downcast_ref::<sqlx::Error>()?.as_database_error()?;
    if db_err.is_unique_violation() {
        let constraint = db_err.constraint().unwrap_or("unknown");
        if constraint.contains("agent_key") {
            Some("An agent with this agent key already exists.".to_string())
        } else if constraint.contains("wallet_address") {
            Some("An agent with this wallet address already exists.".to_string())
        } else if constraint.contains("api_key") {
            Some("An agent with this API key already exists.".to_string())
        } else {
            Some("This agent conflicts with an existing registry entry.".to_string())
        }
    } else {
        None
    }
}

#[derive(Debug)]
struct AppError(anyhow::Error);

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
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("internal server error: {}", self.0),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::util::ServiceExt;

    use crate::{
        agents::crypto::EncryptionKey,
        db::{connect, migrate},
    };

    fn db_url() -> Option<String> {
        std::env::var("DATABASE_URL").ok()
    }

    async fn test_state() -> Option<Arc<AppState>> {
        let database_url = db_url()?;
        let pool = connect(&database_url).await.ok()?;
        migrate(&pool).await.ok()?;
        Some(Arc::new(AppState {
            db_pool: pool,
            encryption_key: EncryptionKey::new(
                "test",
                [
                    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21,
                    22, 23, 24, 25, 26, 27, 28, 29, 30, 31,
                ],
            ),
        }))
    }

    #[tokio::test]
    async fn get_agents_renders_db_data() {
        let Some(state) = test_state().await else {
            eprintln!("DATABASE_URL not set; skipping route test");
            return;
        };

        let app = router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/agents")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn post_agents_with_invalid_private_key_returns_validation_error() {
        let Some(state) = test_state().await else {
            eprintln!("DATABASE_URL not set; skipping route test");
            return;
        };

        let app = router(state);
        let body = "display_name=Test Agent&hyperliquid_private_key=not-a-key";
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/agents")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    fn random_private_key() -> String {
        use rand::Rng;
        format!("0x{}", hex::encode(rand::thread_rng().r#gen::<[u8; 32]>()))
    }

    #[tokio::test]
    async fn post_delete_agent_removes_agent_and_redirects() {
        let Some(state) = test_state().await else {
            eprintln!("DATABASE_URL not set; skipping route test");
            return;
        };

        let app = router(state);
        let timestamp = chrono::Utc::now().timestamp_millis();
        let display_name = format!("DeleteRouteTest{}", timestamp);
        let agent_key = slugify_agent_key(&display_name);
        let private_key = random_private_key();
        let body = format!(
            "display_name={}&hyperliquid_private_key={}&enabled=on",
            display_name, private_key
        );

        // Create the agent.
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/agents")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER);

        // Verify the agent exists.
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{}", agent_key))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // Delete the agent.
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/agents/{}/delete", agent_key))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        let location = response
            .headers()
            .get("location")
            .expect("redirect location header")
            .to_str()
            .unwrap();
        assert_eq!(location, "/agents");

        // Verify the agent is gone.
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{}", agent_key))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
