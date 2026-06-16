use std::{convert::Infallible, sync::Arc};

use askama::Template;
use axum::{
    Form, Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{
        Html, IntoResponse, Redirect, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use chrono::Utc;
use futures::StreamExt;
use serde::Serialize;
use tokio_stream::wrappers::BroadcastStream;
use tracing::{error, warn};

use crate::{
    agents::{
        crypto::{encrypt, generate_api_key},
        keys::derive_wallet_address,
        model::{AgentRegistryRow, CreateAgentForm, slugify_agent_key},
        store::{delete_agent as delete_agent_in_store, get_agent, insert_agent, list_agents},
    },
    hyperliquid::{
        live_state::{AccountKey, AccountLiveState, LiveConnectionStatus},
        queries::{
            BalanceSeriesBucket, fetch_balance_series, list_account_sync_state,
            list_account_transactions,
        },
    },
    web::{
        AppState,
        templates::{
            AccountBalancePartialTemplate, AccountBalanceView, AgentListEntry,
            AgentsNewPageTemplate, AgentsPageTemplate, AgentsShowPageTemplate,
            BalanceSparklinesPartialTemplate, OpenOrdersPartialTemplate, OpenOrdersView,
            OpenPositionsPartialTemplate, OpenPositionsView, ServerErrorPageTemplate,
            SparklineView, SummaryCard, TransactionView,
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
        .route("/agents/{agent_key}/live/stream", get(agent_live_stream))
        .route("/api/agents/{agent_key}/live", get(agent_live))
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

    let entries: Vec<AgentListEntry> = agents
        .into_iter()
        .map(|row| {
            let account_key = AccountKey::new(&row.wallet_address, &row.environment);
            let snapshot =
                state
                    .live_accounts
                    .get(&account_key)
                    .unwrap_or_else(|| AccountLiveState {
                        account_address: account_key.account_address.clone(),
                        environment: account_key.environment.clone(),
                        status: LiveConnectionStatus::Starting,
                        ..Default::default()
                    });
            let account_balance = AccountBalanceView::from_live_state(snapshot);
            AgentListEntry {
                row,
                account_balance,
            }
        })
        .collect();

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
                value: entries.len().to_string(),
                detail: "Total agents in the registry",
            },
            SummaryCard {
                label: "API keys",
                value: entries.len().to_string(),
                detail: "One app credential per agent",
            },
        ],
        agents: entries,
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

#[derive(Debug, Serialize)]
struct LiveAgentSnapshot {
    agent_key: String,
    account_address: String,
    environment: String,
    connected: bool,
    state: Option<AccountLiveState>,
}

async fn agent_live(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    let agent = match get_agent(&state.db_pool, &agent_key).await? {
        Some(agent) => agent,
        None => return Ok((StatusCode::NOT_FOUND, "agent not found").into_response()),
    };
    let key = AccountKey::new(&agent.wallet_address, &agent.environment);
    let snapshot = state.live_accounts.get(&key);
    let body = LiveAgentSnapshot {
        agent_key: agent.agent_key.clone(),
        account_address: agent.wallet_address.clone(),
        environment: agent.environment.clone(),
        connected: snapshot.as_ref().is_some_and(|s| {
            s.status == crate::hyperliquid::live_state::LiveConnectionStatus::Connected
        }),
        state: snapshot,
    };
    Ok(Json(body).into_response())
}

async fn agents_show(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    match get_agent(&state.db_pool, &agent_key).await? {
        Some(agent) => {
            let transactions = match list_account_transactions(
                &state.db_pool,
                &agent.wallet_address,
                &agent.environment,
                100,
            )
            .await
            {
                Ok(rows) => rows.into_iter().map(TransactionView::from_row).collect(),
                Err(error) => {
                    warn!(
                        agent_key = %agent.agent_key,
                        wallet_address = %agent.wallet_address,
                        environment = %agent.environment,
                        error = ?error,
                        "failed to list account transactions for agent page"
                    );
                    Vec::new()
                }
            };
            let sync_state = match list_account_sync_state(
                &state.db_pool,
                &agent.wallet_address,
                &agent.environment,
            )
            .await
            {
                Ok(rows) => rows,
                Err(error) => {
                    warn!(
                        agent_key = %agent.agent_key,
                        wallet_address = %agent.wallet_address,
                        environment = %agent.environment,
                        error = ?error,
                        "failed to list account sync state for agent page"
                    );
                    Vec::new()
                }
            };
            let account_key = AccountKey::new(&agent.wallet_address, &agent.environment);
            let live_snapshot =
                state
                    .live_accounts
                    .get(&account_key)
                    .unwrap_or_else(|| AccountLiveState {
                        account_address: account_key.account_address.clone(),
                        environment: account_key.environment.clone(),
                        status: LiveConnectionStatus::Starting,
                        ..Default::default()
                    });
            let account_balance_view = AccountBalanceView::from_live_state(live_snapshot.clone());
            let account_balance_html =
                AccountBalancePartialTemplate::render_view(account_balance_view)
                    .map_err(anyhow::Error::from)?;
            let open_positions_view = OpenPositionsView::from_live_state(live_snapshot.clone());
            let open_positions_html =
                OpenPositionsPartialTemplate::render_view(open_positions_view)
                    .map_err(anyhow::Error::from)?;
            let open_orders_view = OpenOrdersView::from_live_state(live_snapshot.clone());
            let open_orders_html = OpenOrdersPartialTemplate::render_view(open_orders_view)
                .map_err(anyhow::Error::from)?;

            let now = Utc::now();
            let since_24h = now - chrono::Duration::hours(24);
            let since_30d = now - chrono::Duration::days(30);
            let series_24h = match fetch_balance_series(
                &state.db_pool,
                &agent.wallet_address,
                &agent.environment,
                since_24h,
                BalanceSeriesBucket::Hour,
            )
            .await
            {
                Ok(points) => points,
                Err(error) => {
                    warn!(
                        agent_key = %agent.agent_key,
                        wallet_address = %agent.wallet_address,
                        environment = %agent.environment,
                        error = ?error,
                        "failed to fetch 24h balance series for agent page"
                    );
                    Vec::new()
                }
            };
            let series_30d = match fetch_balance_series(
                &state.db_pool,
                &agent.wallet_address,
                &agent.environment,
                since_30d,
                BalanceSeriesBucket::Day,
            )
            .await
            {
                Ok(points) => points,
                Err(error) => {
                    warn!(
                        agent_key = %agent.agent_key,
                        wallet_address = %agent.wallet_address,
                        environment = %agent.environment,
                        error = ?error,
                        "failed to fetch 30d balance series for agent page"
                    );
                    Vec::new()
                }
            };
            let sparklines = vec![
                SparklineView::from_series("24 hours", &series_24h, 240, 48),
                SparklineView::from_series("30 days", &series_30d, 240, 48),
            ];
            let sparklines_html = BalanceSparklinesPartialTemplate::render_view(sparklines)
                .map_err(anyhow::Error::from)?;

            let template = AgentsShowPageTemplate {
                agent,
                transactions,
                sync_state,
                account_balance_html,
                open_positions_html,
                open_orders_html,
                sparklines_html,
            };
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

/// Stream all live account views for `agent_key` over a single Server-Sent
/// Events connection.
///
/// Using one connection (rather than one per card) keeps the agent detail
/// page well under the browser's per-host HTTP/1.1 connection limit, which
/// previously starved ordinary navigation requests when three separate SSE
/// streams were held open simultaneously.
///
/// Each account mutation emits three named events on this single stream:
/// `balance`, `positions`, and `orders`. The `data` field of each is the
/// freshly rendered partial for that section, which the HTMX SSE extension
/// routes to the matching `sse-swap="..."` element. The stream begins with
/// an initial snapshot of all three (or `Starting`/`Loading` placeholders if
/// no live state exists yet) and then re-emits all three on every
/// [`LiveAccountStore`] mutation for the matching account.
async fn agent_live_stream(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    let agent = match get_agent(&state.db_pool, &agent_key).await? {
        Some(agent) => agent,
        None => return Ok((StatusCode::NOT_FOUND, "agent not found").into_response()),
    };

    let account_key = AccountKey::new(&agent.wallet_address, &agent.environment);
    let live_accounts = Arc::clone(&state.live_accounts);

    // Emit a snapshot up-front so the UI never sits on the initial-render
    // placeholder if the orchestrator already produced a value before the
    // SSE connection opened. If no snapshot exists yet, fall back to a
    // `Starting`-status placeholder so the UI can still render the
    // connection status / "Loading…" caption.
    let initial_snapshot = live_accounts
        .get(&account_key)
        .unwrap_or_else(|| AccountLiveState {
            account_address: account_key.account_address.clone(),
            environment: account_key.environment.clone(),
            status: LiveConnectionStatus::Starting,
            ..Default::default()
        });
    let initial_events = render_live_events(&initial_snapshot)?;

    let account_key_filter = account_key.clone();
    let live_accounts_filter = Arc::clone(&live_accounts);
    let notifications = BroadcastStream::new(live_accounts.subscribe())
        .filter_map(move |item| {
            let account_key = account_key_filter.clone();
            async move {
                match item {
                    Ok(key) if key == account_key => Some(key),
                    Ok(_) => None,
                    Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(_)) => {
                        // Slow consumers can drop intermediate
                        // notifications; treat a lagged notification as a
                        // request to re-emit the current state so the UI
                        // catches up.
                        Some(account_key.clone())
                    }
                }
            }
        })
        .flat_map(move |_key| {
            let live_accounts = Arc::clone(&live_accounts_filter);
            let key = account_key.clone();
            let events = match live_accounts.get(&key) {
                Some(snapshot) => match render_live_events(&snapshot) {
                    Ok(events) => events,
                    Err(e) => {
                        warn!(error = ?e, "failed to render live SSE events");
                        Vec::new()
                    }
                },
                None => Vec::new(),
            };
            tokio_stream::iter(events.into_iter().map(Ok::<Event, Infallible>))
        });

    let stream = tokio_stream::iter(
        initial_events
            .into_iter()
            .map(Ok::<Event, Infallible>)
            .collect::<Vec<_>>(),
    )
    .chain(notifications);
    let sse =
        Sse::new(stream).keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(15)));
    Ok(sse.into_response())
}

/// Render the `balance`, `positions`, and `orders` SSE events for a single
/// live-state snapshot.
fn render_live_events(state: &AccountLiveState) -> Result<Vec<Event>, AppError> {
    Ok(vec![
        render_account_balance_event(state)?,
        render_open_positions_event(state)?,
        render_open_orders_event(state)?,
    ])
}

fn render_account_balance_event(state: &AccountLiveState) -> Result<Event, AppError> {
    let view = AccountBalanceView::from_live_state(state.clone());
    let html = AccountBalancePartialTemplate::render_view(view)?;
    Ok(Event::default().event("balance").data(html))
}

fn render_open_positions_event(state: &AccountLiveState) -> Result<Event, AppError> {
    let view = OpenPositionsView::from_live_state(state.clone());
    let html = OpenPositionsPartialTemplate::render_view(view)?;
    Ok(Event::default().event("positions").data(html))
}

fn render_open_orders_event(state: &AccountLiveState) -> Result<Event, AppError> {
    let view = OpenOrdersView::from_live_state(state.clone());
    let html = OpenOrdersPartialTemplate::render_view(view)?;
    Ok(Event::default().event("orders").data(html))
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
        environment: "live".to_string(),
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
        } else if constraint.contains("wallet") {
            Some("An agent with this wallet address and environment already exists.".to_string())
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
        error!(error = ?self.0, "request failed");

        let template = ServerErrorPageTemplate {
            message: format!("Internal server error: {}", self.0),
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt as _;
    use tower::util::ServiceExt;

    use crate::{
        agents::crypto::EncryptionKey,
        test_db,
    };

    async fn test_state() -> Arc<AppState> {
        let pool = test_db::pool().await;
        Arc::new(AppState {
            db_pool: pool,
            encryption_key: EncryptionKey::new(
                "test",
                [
                    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21,
                    22, 23, 24, 25, 26, 27, 28, 29, 30, 31,
                ],
            ),
            live_accounts: Arc::new(crate::hyperliquid::live_state::LiveAccountStore::new()),
        })
    }

    /// Read bytes from an SSE body until the timeout fires. SSE response
    /// bodies are long-lived streams that never EOF, so the naive
    /// `axum::body::to_bytes` hangs forever; we only care about the
    /// initial snapshot of events here. The body is polled frame by frame
    /// and the result is whatever data arrived before the deadline.
    async fn read_sse_chunk(body: Body, timeout_ms: u64) -> String {
        let mut body = body;
        let mut buf = Vec::<u8>::new();
        let deadline = tokio::time::Instant::now()
            + tokio::time::Duration::from_millis(timeout_ms);

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(remaining, body.frame()).await {
                Ok(Some(Ok(frame))) => {
                    if let Some(data) = frame.data_ref() {
                        buf.extend_from_slice(data);
                    }
                }
                Ok(Some(Err(e))) => panic!("failed to read SSE body: {e}"),
                Ok(None) => break,
                Err(_) => break,
            }
        }

        if buf.is_empty() {
            panic!("timed out reading SSE body");
        }

        String::from_utf8_lossy(&buf).into_owned()
    }

    #[tokio::test]
    async fn get_agents_renders_db_data() {
        let state = test_state().await;

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
        let state = test_state().await;

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

    #[tokio::test]
    async fn get_live_route_returns_empty_state_for_known_agent() {
        let state = test_state().await;

        // Insert a fresh agent directly so we can look it up by agent_key.
        let timestamp = chrono::Utc::now().timestamp_millis();
        let display_name = format!("LiveRouteTest{}", timestamp);
        let agent_key = slugify_agent_key(&display_name);
        let private_key = random_private_key();
        let wallet_address = derive_wallet_address(&private_key).expect("derives");
        let now = Utc::now();
        let row = crate::agents::model::AgentRegistryRow {
            agent_key: agent_key.clone(),
            created_at: now,
            updated_at: now,
            enabled: true,
            display_name: display_name.clone(),
            prompt: String::new(),
            wallet_address: wallet_address.clone(),
            environment: "live".to_string(),
            api_key: format!("test-key-{timestamp}"),
            api_key_last_used_at: None,
            hyperliquid_private_key_ciphertext: Vec::new(),
            hyperliquid_private_key_key_id: "test".to_string(),
        };
        insert_agent(&state.db_pool, &row)
            .await
            .expect("inserts agent");

        let app = router(state.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/agents/{}/live", agent_key))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body_bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(
            body["agent_key"],
            serde_json::Value::from(agent_key.clone())
        );
        assert_eq!(
            body["account_address"],
            serde_json::Value::from(wallet_address.clone())
        );
        assert_eq!(body["environment"], serde_json::Value::from("live"));
        assert_eq!(body["connected"], serde_json::Value::from(false));
        assert!(body["state"].is_null());

        // Now seed a live state for that account and confirm the route
        // returns the populated snapshot.
        let key = AccountKey::new(&wallet_address, "live");
        state.live_accounts.set_status(
            &key,
            crate::hyperliquid::live_state::LiveConnectionStatus::Connected,
        );

        let app = router(state.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/agents/{}/live", agent_key))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body_bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(body["connected"], serde_json::Value::from(true));
        assert_eq!(
            body["state"]["status"],
            serde_json::Value::from("connected")
        );
    }

    #[tokio::test]
    async fn get_live_route_returns_404_for_unknown_agent() {
        let state = test_state().await;

        let app = router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/agents/does-not-exist-12345/live")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn account_balance_stream_returns_404_for_unknown_agent() {
        let state = test_state().await;

        let app = router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/agents/does-not-exist-12345/live/stream")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    #[ignore = "SSE body read hangs in pglite-oxide test environment; see AGENTS.md"]
    async fn account_balance_stream_emits_initial_loading_placeholder() {
        let state = test_state().await;

        let (agent_key, _wallet_address) = match insert_test_agent(&state).await {
            Some(pair) => pair,
            None => return,
        };

        let app = router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{}/live/stream", agent_key))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok()),
            Some("text/event-stream")
        );

        // Read just the first event boundary of the body so we capture the
        // initial event without waiting for the keep-alive timer.
        let text = read_sse_chunk(response.into_body(), 250).await;
        assert!(
            text.contains("event: balance"),
            "missing event line in {text}"
        );
        assert!(text.contains("Loading"), "expected placeholder in {text}");
    }

    #[tokio::test]
    #[ignore = "SSE body read hangs in pglite-oxide test environment; see AGENTS.md"]
    async fn account_balance_stream_emits_initial_value_when_state_present() {
        let state = test_state().await;

        let (agent_key, wallet_address) = match insert_test_agent(&state).await {
            Some(pair) => pair,
            None => return,
        };

        let key = AccountKey::new(&wallet_address, "live");
        state.live_accounts.replace(
            key.clone(),
            AccountLiveState {
                account_address: key.account_address.clone(),
                environment: key.environment.clone(),
                status: LiveConnectionStatus::Connected,
                margin: Some(crate::hyperliquid::live_state::LiveMarginState {
                    account_value: Some(rust_decimal::Decimal::new(123_4567, 4)),
                    ..Default::default()
                }),
                updated_at: Some(Utc::now()),
                ..Default::default()
            },
        );

        let app = router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{}/live/stream", agent_key))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let text = read_sse_chunk(response.into_body(), 250).await;
        assert!(text.contains("event: balance"));
        assert!(text.contains("123.4567"));
        assert!(text.contains("USDC"));
    }

    #[tokio::test]
    #[ignore = "SSE body read hangs in pglite-oxide test environment; see AGENTS.md"]
    async fn account_balance_stream_emits_updates_when_state_changes() {
        let state = test_state().await;

        let (agent_key, wallet_address) = match insert_test_agent(&state).await {
            Some(pair) => pair,
            None => return,
        };

        let key = AccountKey::new(&wallet_address, "live");

        let app = router(state.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{}/live/stream", agent_key))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // Update the live store; the SSE consumer should observe the
        // notification and emit a new event.
        state.live_accounts.replace(
            key.clone(),
            AccountLiveState {
                account_address: key.account_address.clone(),
                environment: key.environment.clone(),
                status: LiveConnectionStatus::Connected,
                margin: Some(crate::hyperliquid::live_state::LiveMarginState {
                    account_value: Some(rust_decimal::Decimal::new(99_0000, 4)),
                    ..Default::default()
                }),
                updated_at: Some(Utc::now()),
                ..Default::default()
            },
        );

        // Longer wait: this test expects a second event after we mutate
        // live_accounts, which only happens once a real broadcast fires.
        let text = read_sse_chunk(response.into_body(), 2000).await;
        assert!(text.contains("event: balance"));
        assert!(text.contains("99.0000"));
    }

    async fn insert_test_agent(state: &Arc<AppState>) -> Option<(String, String)> {
        let timestamp = chrono::Utc::now().timestamp_millis();
        let display_name = format!("BalanceStreamTest{}", timestamp);
        let agent_key = slugify_agent_key(&display_name);
        let private_key = random_private_key();
        let wallet_address = match derive_wallet_address(&private_key) {
            Ok(addr) => addr,
            Err(_) => return None,
        };
        let now = Utc::now();
        let row = crate::agents::model::AgentRegistryRow {
            agent_key: agent_key.clone(),
            created_at: now,
            updated_at: now,
            enabled: true,
            display_name,
            prompt: String::new(),
            wallet_address: wallet_address.clone(),
            environment: "live".to_string(),
            api_key: format!("balance-stream-test-{timestamp}"),
            api_key_last_used_at: None,
            hyperliquid_private_key_ciphertext: Vec::new(),
            hyperliquid_private_key_key_id: "test".to_string(),
        };
        if insert_agent(&state.db_pool, &row).await.is_err() {
            return None;
        }
        Some((agent_key, wallet_address))
    }

    fn random_private_key() -> String {
        use rand::Rng;
        format!("0x{}", hex::encode(rand::thread_rng().r#gen::<[u8; 32]>()))
    }

    #[tokio::test]
    async fn post_delete_agent_removes_agent_and_redirects() {
        let state = test_state().await;

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

    #[tokio::test]
    async fn open_positions_stream_returns_404_for_unknown_agent() {
        let state = test_state().await;

        let app = router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/agents/does-not-exist-12345/live/stream")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    #[ignore = "SSE body read hangs in pglite-oxide test environment; see AGENTS.md"]
    async fn open_positions_stream_emits_initial_loading_placeholder() {
        let state = test_state().await;

        let (agent_key, _wallet_address) = match insert_test_agent(&state).await {
            Some(pair) => pair,
            None => return,
        };

        let app = router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{}/live/stream", agent_key))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok()),
            Some("text/event-stream")
        );

        let text = read_sse_chunk(response.into_body(), 250).await;
        assert!(
            text.contains("event: positions"),
            "missing event line in {text}"
        );
        assert!(text.contains("Loading"), "expected placeholder in {text}");
    }

    #[tokio::test]
    #[ignore = "SSE body read hangs in pglite-oxide test environment; see AGENTS.md"]
    async fn open_positions_stream_emits_initial_rows_when_state_present() {
        let state = test_state().await;

        let (agent_key, wallet_address) = match insert_test_agent(&state).await {
            Some(pair) => pair,
            None => return,
        };

        let key = AccountKey::new(&wallet_address, "live");
        state.live_accounts.replace(
            key.clone(),
            AccountLiveState {
                account_address: key.account_address.clone(),
                environment: key.environment.clone(),
                status: LiveConnectionStatus::Connected,
                updated_at: Some(Utc::now()),
                open_positions: vec![crate::hyperliquid::live_state::LivePosition {
                    coin: "BTC".to_string(),
                    szi: Some(rust_decimal::Decimal::new(1, 0)),
                    entry_px: Some(rust_decimal::Decimal::new(30000, 0)),
                    liquidation_px: Some(rust_decimal::Decimal::new(25000, 0)),
                    margin_used: Some(rust_decimal::Decimal::new(6000, 0)),
                    position_value: Some(rust_decimal::Decimal::new(30000, 0)),
                    unrealized_pnl: Some(rust_decimal::Decimal::new(1500, 0)),
                    return_on_equity: Some(rust_decimal::Decimal::new(25, 2)),
                    leverage_type: Some("cross".to_string()),
                    leverage_value: Some(5),
                    max_leverage: Some(50),
                }],
                ..Default::default()
            },
        );

        let app = router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{}/live/stream", agent_key))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let text = read_sse_chunk(response.into_body(), 250).await;
        assert!(text.contains("event: positions"));
        assert!(text.contains("BTC"));
        assert!(text.contains("long"));
        assert!(text.contains("+25.00%"));
    }

    #[tokio::test]
    async fn open_orders_stream_returns_404_for_unknown_agent() {
        let state = test_state().await;

        let app = router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/agents/does-not-exist-12345/live/stream")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    #[ignore = "SSE body read hangs in pglite-oxide test environment; see AGENTS.md"]
    async fn open_orders_stream_emits_initial_loading_placeholder() {
        let state = test_state().await;

        let (agent_key, _wallet_address) = match insert_test_agent(&state).await {
            Some(pair) => pair,
            None => return,
        };

        let app = router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{}/live/stream", agent_key))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok()),
            Some("text/event-stream")
        );

        let text = read_sse_chunk(response.into_body(), 250).await;
        assert!(
            text.contains("event: orders"),
            "missing event line in {text}"
        );
        assert!(text.contains("Loading"), "expected placeholder in {text}");
    }

    #[tokio::test]
    #[ignore = "SSE body read hangs in pglite-oxide test environment; see AGENTS.md"]
    async fn open_orders_stream_emits_initial_rows_when_state_present() {
        let state = test_state().await;

        let (agent_key, wallet_address) = match insert_test_agent(&state).await {
            Some(pair) => pair,
            None => return,
        };

        let key = AccountKey::new(&wallet_address, "live");
        state.live_accounts.replace(
            key.clone(),
            AccountLiveState {
                account_address: key.account_address.clone(),
                environment: key.environment.clone(),
                status: LiveConnectionStatus::Connected,
                updated_at: Some(Utc::now()),
                open_orders: vec![crate::hyperliquid::live_state::LiveOpenOrder {
                    coin: "ETH".to_string(),
                    side: Some("buy".to_string()),
                    limit_px: Some(rust_decimal::Decimal::new(1900, 0)),
                    sz: Some(rust_decimal::Decimal::new(1, 0)),
                    orig_sz: Some(rust_decimal::Decimal::new(1, 0)),
                    oid: Some("123".to_string()),
                    timestamp: Some(chrono::Utc::now().timestamp_millis() as u64),
                    cloid: None,
                    order_type: Some("limit".to_string()),
                    tif: Some("Gtc".to_string()),
                    reduce_only: Some(false),
                    is_trigger: Some(false),
                    trigger_px: None,
                    trigger_condition: None,
                    is_position_tpsl: Some(false),
                }],
                ..Default::default()
            },
        );

        let app = router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{}/live/stream", agent_key))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let text = read_sse_chunk(response.into_body(), 250).await;
        assert!(text.contains("event: positions"));
        assert!(text.contains("BTC"));
        assert!(text.contains("long"));
        assert!(text.contains("+25.00%"));
    }
}
