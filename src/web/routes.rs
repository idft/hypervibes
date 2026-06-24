use std::{convert::Infallible, sync::Arc};

use askama::Template;
use axum::{
    Form, Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{
        Html, IntoResponse, Redirect, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use chrono::Utc;
use futures::StreamExt;
use serde::Deserialize;
use serde::Serialize;
use tokio_stream::wrappers::BroadcastStream;
use tracing::{error, warn};

use crate::{
    agents::{
        crypto::{encrypt, generate_api_key},
        keys::derive_wallet_address,
        model::{AgentRegistryRow, CreateAgentForm, slugify_agent_key},
        prompts::{DEFAULT_ANALYSIS_STRATEGY_PROMPT, DEFAULT_TRADING_STRATEGY_PROMPT},
        store::{
            delete_agent as delete_agent_in_store, get_agent, insert_agent, list_agents,
            update_agent_prompts,
        },
    },
    hermes::HermesHealth,
    hyperliquid::{
        live_state::{AccountKey, AccountLiveState, LiveConnectionStatus},
        queries::{
            BalanceSeriesBucket, fetch_balance_series, list_account_sync_state,
            list_all_account_transactions,
        },
    },
    memory::{
        get_latest_agent_memory_by_type, get_memory as get_memory_record, list_agent_memories,
    },
    web::{
        AppState,
        templates::{
            AccountBalancePartialTemplate, AccountBalanceView, AgentListEntry,
            AgentMemoryDetailPartialTemplate, AgentMemoryTimelinePartialTemplate, AgentShowTab,
            AgentsNewPageTemplate, AgentsPageTemplate, AgentsShowPageTemplate,
            BalanceSparklinesPartialTemplate, HermesPageTemplate,
            LatestTradeExecutionSummaryPartialTemplate, MemoryView,
            OpenOrdersPartialTemplate, OpenOrdersView, OpenPositionsPartialTemplate,
            OpenPositionsView, ServerErrorPageTemplate, SparklineView, SyncStateView,
            TransactionView,
        },
        ui_events::UiEvent,
    },
};

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(root))
        .route("/healthz", get(healthz))
        .route("/agents", get(agents_index).post(create_agent))
        .route("/agents/new", get(agents_new))
        .route("/agents/{agent_key}", get(agents_show))
        .route(
            "/agents/{agent_key}/transactions",
            get(agents_show_transactions),
        )
        .route("/agents/{agent_key}/memories", get(agents_show_memories))
        .route(
            "/agents/{agent_key}/memories/stream",
            get(agent_memories_stream),
        )
        .route(
            "/agents/{agent_key}/memories/{memory_id}",
            get(agents_show_memory_detail),
        )
        .route(
            "/agents/{agent_key}/prompts",
            get(agents_show_prompts).post(agents_update_prompts),
        )
        .route("/agents/{agent_key}/settings", get(agents_show_settings))
        .route("/agents/{agent_key}/delete", post(delete_agent))
        .route("/agents/{agent_key}/live/stream", get(agent_live_stream))
        .route("/hermes", get(hermes_page))
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
            let api_key_last_used_iso = row
                .api_key_last_used_at
                .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true));
            AgentListEntry {
                row,
                account_balance,
                api_key_last_used_iso,
            }
        })
        .collect();

    let template = AgentsPageTemplate {
        agents: entries,
        current_path: "/agents".to_string(),
    };

    Ok(Html(template.render()?))
}

async fn agents_new() -> Result<Html<String>, AppError> {
    let template = AgentsNewPageTemplate {
        form: CreateAgentForm::default(),
        errors: Vec::new(),
        current_path: "/agents/new".to_string(),
    };
    Ok(Html(template.render()?))
}

async fn agents_show(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    render_agent_show_page(&state, &agent_key, AgentShowTab::Positions, None).await
}

async fn agents_show_transactions(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    render_agent_show_page(&state, &agent_key, AgentShowTab::Transactions, None).await
}

#[derive(Debug, Default, Deserialize)]
struct AgentMemoriesQuery {
    #[serde(default)]
    date: String,
}

async fn agents_show_memories(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
    Query(query): Query<AgentMemoriesQuery>,
) -> Result<Response, AppError> {
    render_agent_show_page(&state, &agent_key, AgentShowTab::Memories, Some(query)).await
}

async fn agents_show_memory_detail(
    State(state): State<Arc<AppState>>,
    Path((agent_key, memory_id)): Path<(String, uuid::Uuid)>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    tracing::info!(agent_key = %agent.agent_key, "opened memories SSE stream");

    let Some(memory) = get_memory_record(&state.db_pool, &agent.agent_key, memory_id).await? else {
        return Ok((StatusCode::NOT_FOUND, "memory not found").into_response());
    };

    let html = AgentMemoryDetailPartialTemplate::render_view(MemoryView::from_record(memory))?;
    Ok(Html(html).into_response())
}

async fn agent_memories_stream(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
    Query(query): Query<AgentMemoriesQuery>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };

    let (_, selected_date_text, _, since, until) = parse_memory_date_filter(&query.date);
    let db_pool = state.db_pool.clone();
    let agent_key = agent.agent_key.clone();
    let agent_key_for_render = agent_key.clone();
    let selected_date_text_filter = selected_date_text.clone();

    let notifications = BroadcastStream::new(state.ui_events.subscribe())
        .filter_map(move |item| {
            let agent_key = agent_key.clone();
            async move {
                match item {
                    Ok(UiEvent::MemoryCreated { agent_key: event_agent_key, memory_id })
                        if event_agent_key == agent_key =>
                    {
                        tracing::info!(agent_key = %agent_key, memory_id = %memory_id, "memories SSE received memory event");
                        Some(())
                    }
                    Ok(_) => None,
                    Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(_)) => {
                        Some(())
                    }
                }
            }
        })
        .filter_map(move |_| {
            let db_pool = db_pool.clone();
            let agent_key = agent_key_for_render.clone();
            let selected_date_text = selected_date_text_filter.clone();
            async move {
                match render_memory_timeline_event(&db_pool, &agent_key, since, until, selected_date_text)
                    .await
                {
                    Ok(event) => Some(Ok::<Event, Infallible>(event)),
                    Err(e) => {
                        warn!(agent_key = %agent_key, error = ?e, "failed to render memories SSE event");
                        None
                    }
                }
            }
        });

    let sse = Sse::new(notifications)
        .keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(15)));
    Ok(sse.into_response())
}

async fn render_memory_timeline_event(
    pool: &crate::db::DbPool,
    agent_key: &str,
    since: Option<chrono::DateTime<Utc>>,
    until: Option<chrono::DateTime<Utc>>,
    selected_date_text: Option<String>,
) -> Result<Event, AppError> {
    let rows = list_agent_memories(pool, agent_key, since, until).await?;
    let timeline = crate::web::templates::build_memory_timeline_for_sse(agent_key, &rows);
    let html =
        AgentMemoryTimelinePartialTemplate::render_view(timeline, rows.len(), selected_date_text)?;
    Ok(Event::default().event("memories-timeline").data(html))
}

async fn agents_show_prompts(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    render_agent_show_page(&state, &agent_key, AgentShowTab::Prompts, None).await
}

#[derive(Debug, Default, Deserialize)]
struct UpdateAgentPromptsForm {
    #[serde(default)]
    analysis_prompt: String,
    #[serde(default)]
    trading_prompt: String,
}

async fn agents_update_prompts(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
    Form(form): Form<UpdateAgentPromptsForm>,
) -> Result<Response, AppError> {
    let updated = update_agent_prompts(
        &state.db_pool,
        &agent_key,
        form.analysis_prompt.trim(),
        form.trading_prompt.trim(),
    )
    .await?;

    if !updated {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    }

    Ok(Redirect::to(&format!("/agents/{agent_key}/prompts")).into_response())
}

async fn agents_show_settings(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    render_agent_show_page(&state, &agent_key, AgentShowTab::Settings, None).await
}

async fn render_agent_show_page(
    state: &Arc<AppState>,
    agent_key: &str,
    active_tab: AgentShowTab,
    memories_query: Option<AgentMemoriesQuery>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };

    let mut template = AgentsShowPageTemplate::new(agent.clone(), active_tab);

    match active_tab {
        AgentShowTab::Positions => {
            populate_positions_tab(state, &agent, &mut template).await?;
        }
        AgentShowTab::Transactions => {
            template.transactions = match list_all_account_transactions(
                &state.db_pool,
                &agent.wallet_address,
                &agent.environment,
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
                        "failed to list full account transactions for agent page"
                    );
                    Vec::new()
                }
            };
        }
        AgentShowTab::Memories => {
            let memory_query = memories_query.unwrap_or_default();
            let (filter_date_value, selected_date_text, filter_error_text, since, until) =
                parse_memory_date_filter(&memory_query.date);

            match list_agent_memories(&state.db_pool, &agent.agent_key, since, until).await {
                Ok(rows) => template.set_memories(
                    rows,
                    filter_date_value,
                    selected_date_text,
                    filter_error_text,
                ),
                Err(error) => {
                    warn!(
                        agent_key = %agent.agent_key,
                        error = ?error,
                        "failed to list agent memories for operator page"
                    );
                }
            }
        }
        AgentShowTab::Prompts => {}
        AgentShowTab::Settings => {
            template.sync_state = match list_account_sync_state(
                &state.db_pool,
                &agent.wallet_address,
                &agent.environment,
            )
            .await
            {
                Ok(rows) => rows.into_iter().map(SyncStateView::from_row).collect(),
                Err(error) => {
                    warn!(
                        agent_key = %agent.agent_key,
                        wallet_address = %agent.wallet_address,
                        environment = %agent.environment,
                        error = ?error,
                        "failed to list account sync state for agent settings page"
                    );
                    Vec::new()
                }
            };
        }
    }

    Ok(Html(template.render()?).into_response())
}

fn parse_memory_date_filter(
    raw: &str,
) -> (
    String,
    Option<String>,
    Option<String>,
    Option<chrono::DateTime<Utc>>,
    Option<chrono::DateTime<Utc>>,
) {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return (String::new(), None, None, None, None);
    }

    let Ok(date) = chrono::NaiveDate::parse_from_str(trimmed, "%Y-%m-%d") else {
        return (
            trimmed.to_string(),
            None,
            Some("Use YYYY-MM-DD to filter memories by UTC date.".to_string()),
            None,
            None,
        );
    };

    let Some(start_of_day) = date.and_hms_opt(0, 0, 0) else {
        return (
            trimmed.to_string(),
            None,
            Some("That date could not be parsed.".to_string()),
            None,
            None,
        );
    };
    let Some(next_day) = date.succ_opt() else {
        return (
            trimmed.to_string(),
            None,
            Some("That date is out of range.".to_string()),
            None,
            None,
        );
    };
    let Some(end_of_day) = next_day.and_hms_opt(0, 0, 0) else {
        return (
            trimmed.to_string(),
            None,
            Some("That date is out of range.".to_string()),
            None,
            None,
        );
    };

    (
        trimmed.to_string(),
        Some(date.format("%A, %B %-d, %Y").to_string()),
        None,
        Some(chrono::DateTime::<Utc>::from_naive_utc_and_offset(
            start_of_day,
            Utc,
        )),
        Some(chrono::DateTime::<Utc>::from_naive_utc_and_offset(
            end_of_day, Utc,
        )),
    )
}

async fn populate_positions_tab(
    state: &Arc<AppState>,
    agent: &crate::agents::model::AgentDetailRow,
    template: &mut AgentsShowPageTemplate,
) -> Result<(), AppError> {
    let account_key = AccountKey::new(&agent.wallet_address, &agent.environment);
    let live_snapshot = state
        .live_accounts
        .get(&account_key)
        .unwrap_or_else(|| AccountLiveState {
            account_address: account_key.account_address.clone(),
            environment: account_key.environment.clone(),
            status: LiveConnectionStatus::Starting,
            ..Default::default()
        });

    let account_balance_view = AccountBalanceView::from_live_state(live_snapshot.clone());
    template.account_balance_html =
        AccountBalancePartialTemplate::render_view(account_balance_view.clone())
            .map_err(anyhow::Error::from)?;

    let open_positions_view = OpenPositionsView::from_live_state(live_snapshot.clone());
    template.open_positions_html = OpenPositionsPartialTemplate::render_view(open_positions_view)
        .map_err(anyhow::Error::from)?;

    let open_orders_view = OpenOrdersView::from_live_state(live_snapshot.clone());
    template.open_orders_html =
        OpenOrdersPartialTemplate::render_view(open_orders_view).map_err(anyhow::Error::from)?;
    template.latest_trade_execution_summary_html =
        LatestTradeExecutionSummaryPartialTemplate::render_view(
            get_latest_agent_memory_by_type(
        &state.db_pool,
        &agent.agent_key,
        "trade_execution",
    )
    .await?
    .map(|memory| memory.summary),
        )
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
    template.sparklines_html =
        BalanceSparklinesPartialTemplate::render_view(sparklines).map_err(anyhow::Error::from)?;

    Ok(())
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
/// `balance`, `positions`, and `orders`. A companion memory-driven event,
/// `latest-trade-execution-summary`, refreshes the Open Orders subheader when a
/// new `trade_execution` memory arrives. The `data` field of each is the
/// freshly rendered partial for that section, which the HTMX SSE extension
/// routes to the matching `sse-swap="..."` element. The stream begins with
/// an initial snapshot and then re-emits updates for the matching account.
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
    let mut initial_events = render_live_events(&initial_snapshot)?;
    initial_events.push(
        render_latest_trade_execution_summary_event(&state.db_pool, &agent.agent_key).await?,
    );

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

    let summary_db_pool = state.db_pool.clone();
    let summary_agent_key_filter = agent.agent_key.clone();
    let summary_agent_key_render = agent.agent_key.clone();
    let summary_notifications = BroadcastStream::new(state.ui_events.subscribe())
        .filter_map(move |item| {
            let agent_key = summary_agent_key_filter.clone();
            async move {
                match item {
                    Ok(UiEvent::MemoryCreated {
                        agent_key: event_agent_key,
                        memory_id,
                    }) if event_agent_key == agent_key => Some(Some(memory_id)),
                    Ok(_) => None,
                    Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(_)) => {
                        Some(None)
                    }
                }
            }
        })
        .filter_map(move |memory_id| {
            let db_pool = summary_db_pool.clone();
            let agent_key = summary_agent_key_render.clone();
            async move {
                if let Some(memory_id) = memory_id {
                    match get_memory_record(&db_pool, &agent_key, memory_id).await {
                        Ok(Some(memory)) if memory.memory_type == "trade_execution" => {}
                        Ok(Some(_)) | Ok(None) => return None,
                        Err(error) => {
                            warn!(agent_key = %agent_key, error = ?error, "failed to inspect memory event for live summary update");
                            return None;
                        }
                    }
                }

                match render_latest_trade_execution_summary_event(&db_pool, &agent_key).await {
                    Ok(event) => Some(Ok::<Event, Infallible>(event)),
                    Err(error) => {
                        warn!(agent_key = %agent_key, error = ?error, "failed to render latest trade execution summary SSE event");
                        None
                    }
                }
            }
        });

    let stream = tokio_stream::iter(
        initial_events
            .into_iter()
            .map(Ok::<Event, Infallible>)
            .collect::<Vec<_>>(),
    )
    .chain(futures::stream::select(notifications, summary_notifications));
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

async fn render_latest_trade_execution_summary_event(
    pool: &crate::db::DbPool,
    agent_key: &str,
) -> Result<Event, AppError> {
    let summary = get_latest_agent_memory_by_type(pool, agent_key, "trade_execution")
        .await?
        .map(|memory| memory.summary);
    let html = LatestTradeExecutionSummaryPartialTemplate::render_view(summary)?;
    Ok(Event::default()
        .event("latest-trade-execution-summary")
        .data(html))
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
        agent_key: agent_key.clone(),
        created_at: now,
        updated_at: now,
        enabled: form.enabled(),
        display_name: form.display_name.trim().to_string(),
        analysis_prompt: DEFAULT_ANALYSIS_STRATEGY_PROMPT.to_string(),
        trading_prompt: DEFAULT_TRADING_STRATEGY_PROMPT.to_string(),
        wallet_address,
        environment: "live".to_string(),
        api_key: generate_api_key(),
        api_key_last_used_at: None,
        analysis_context_last_used_at: None,
        trading_context_last_used_at: None,
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

    Ok(Redirect::to(&format!("/agents/{agent_key}")).into_response())
}

fn render_new_form(form: CreateAgentForm, errors: Vec<String>) -> Response {
    let template = AgentsNewPageTemplate {
        form,
        errors,
        current_path: "/agents/new".to_string(),
    };
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
        if constraint.contains("agent_key") || constraint.contains("agents_pkey") {
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
            current_path: String::new(),
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

async fn hermes_page(State(state): State<Arc<AppState>>) -> Result<Html<String>, AppError> {
    let mut template = HermesPageTemplate {
        reachable: false,
        version: None,
        active_profile: None,
        profiles: Vec::new(),
        error: None,
        dashboard_url: None,
        current_path: "/hermes".to_string(),
    };

    if let Some(hermes) = &state.hermes {
        let health = hermes.health().await.unwrap_or(HermesHealth {
            reachable: false,
            version: None,
        });
        template.reachable = health.reachable;
        template.version = health.version;
        template.dashboard_url = Some(state.hermes_dashboard_link_url.clone());
        match hermes.list_profiles().await {
            Ok(profiles) => {
                template.profiles = profiles.into_iter().map(|p| p.name).collect();
            }
            Err(e) => {
                template.error = Some(format!("Failed to list profiles: {e}"));
            }
        }
        if template.reachable {
            match hermes.get_active_profile().await {
                Ok(Some(p)) => template.active_profile = Some(p.name),
                Ok(None) => {}
                Err(e) => {
                    template.error = Some(format!("Failed to get active profile: {e}"));
                }
            }
        }
    } else {
        template.error = Some("Hermes is not configured".to_string());
    }

    Ok(Html(template.render()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt as _;
    use rust_decimal::Decimal;
    use tower::util::ServiceExt;

    use crate::{
        agents::{
            crypto::EncryptionKey,
            prompts::{DEFAULT_ANALYSIS_STRATEGY_PROMPT, DEFAULT_TRADING_STRATEGY_PROMPT},
        },
        memory::CreateMemory,
        test_db,
        web::ui_events::UiEventHub,
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
            ui_events: Arc::new(UiEventHub::new()),
            hermes: None,
            hermes_dashboard_link_url: "http://127.0.0.1:19119".to_string(),
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
        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_millis(timeout_ms);

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

    async fn response_text(response: Response) -> String {
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    async fn seed_ledger_event(
        state: &Arc<AppState>,
        wallet_address: &str,
        hash: &str,
        event_time: chrono::DateTime<Utc>,
        usdc: Decimal,
    ) {
        sqlx::query(
            "INSERT INTO hyperliquid.ledger_events
                (hash, account_address, environment, event_time, event_type,
                 source_stream, ledger_type, usdc, ingest_source, inserted_at)
             VALUES ($1, $2, 'live', $3, 'ledger', 'test', 'deposit', $4, 'test', NOW())",
        )
        .bind(hash)
        .bind(wallet_address)
        .bind(event_time)
        .bind(usdc)
        .execute(&state.db_pool)
        .await
        .expect("insert ledger event");
    }

    async fn seed_memory(
        state: &Arc<AppState>,
        agent_key: &str,
        summary: &str,
        content: &str,
    ) -> crate::memory::MemoryRecord {
        seed_memory_with_type(state, agent_key, "plan", summary, content).await
    }

    async fn seed_memory_with_type(
        state: &Arc<AppState>,
        agent_key: &str,
        memory_type: &str,
        summary: &str,
        content: &str,
    ) -> crate::memory::MemoryRecord {
        crate::memory::insert_memory(
            &state.db_pool,
            agent_key,
            &CreateMemory {
                symbol: "BTC".to_string(),
                timeframe: Some("1h".to_string()),
                memory_type: memory_type.to_string(),
                summary: summary.to_string(),
                content: content.to_string(),
                metadata: Some(serde_json::json!({ "confidence": 0.8 })),
            },
        )
        .await
        .expect("insert memory")
    }

    async fn seed_sync_state(state: &Arc<AppState>, wallet_address: &str) {
        sqlx::query(
            "INSERT INTO hyperliquid.sync_state
                (account_address, environment, stream_name, status, metadata, last_event_key, last_synced_at)
             VALUES ($1, 'live', 'fills', $2, '{}'::jsonb, 'abc123', NOW())",
        )
        .bind(wallet_address)
        .bind(crate::hyperliquid::sync_state::SyncStatus::Healthy.as_str())
        .execute(&state.db_pool)
        .await
        .expect("insert sync state");
    }

    async fn set_job_context_timestamps(
        state: &Arc<AppState>,
        agent_key: &str,
        analysis_context_last_used_at: Option<chrono::DateTime<Utc>>,
        trading_context_last_used_at: Option<chrono::DateTime<Utc>>,
    ) {
        sqlx::query(
            "UPDATE agents
                SET analysis_context_last_used_at = $2,
                    trading_context_last_used_at = $3
              WHERE agent_key = $1",
        )
        .bind(agent_key)
        .bind(analysis_context_last_used_at)
        .bind(trading_context_last_used_at)
        .execute(&state.db_pool)
        .await
        .expect("update job context timestamps");
    }

    #[tokio::test]
    async fn hermes_page_renders_not_configured_state() {
        let state = test_state().await;

        let app = router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/hermes")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body();
        let bytes = http_body_util::BodyExt::collect(body)
            .await
            .unwrap()
            .to_bytes();
        let text = String::from_utf8_lossy(&bytes).into_owned();
        assert!(text.contains("Hermes"));
        assert!(text.contains("Hermes is not configured"));
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
        insert_test_agent_with_text(state, String::new(), String::new()).await
    }

    async fn insert_test_agent_with_text(
        state: &Arc<AppState>,
        analysis_prompt: String,
        trading_prompt: String,
    ) -> Option<(String, String)> {
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
            analysis_prompt,
            trading_prompt,
            wallet_address: wallet_address.clone(),
            environment: "live".to_string(),
            api_key: format!("balance-stream-test-{timestamp}"),
            api_key_last_used_at: None,
            analysis_context_last_used_at: None,
            trading_context_last_used_at: None,
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
    async fn post_agents_creates_agent_with_default_strategy_prompts() {
        let state = test_state().await;
        let pool = state.db_pool.clone();

        let app = router(state);
        let timestamp = chrono::Utc::now().timestamp_millis();
        let display_name = format!("SoulTest{}", timestamp);
        let agent_key = slugify_agent_key(&display_name);
        let private_key = random_private_key();
        let body = format!(
            "display_name={}&hyperliquid_private_key={}",
            display_name, private_key
        );

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
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        let expected_location = format!("/agents/{agent_key}");
        assert_eq!(
            response
                .headers()
                .get("location")
                .and_then(|value| value.to_str().ok()),
            Some(expected_location.as_str())
        );

        let stored = get_agent(&pool, &agent_key)
            .await
            .expect("get agent")
            .expect("agent present");
        assert_eq!(stored.analysis_prompt, DEFAULT_ANALYSIS_STRATEGY_PROMPT);
        assert_eq!(stored.trading_prompt, DEFAULT_TRADING_STRATEGY_PROMPT);
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
    async fn agent_chat_route_is_not_registered() {
        let state = test_state().await;
        let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");

        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{agent_key}/chat"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn agent_detail_renders_setup_alert_when_both_checkins_missing() {
        let state = test_state().await;
        let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");

        let response = router(Arc::clone(&state))
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{agent_key}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let text = response_text(response).await;
        assert!(text.contains("Hermes cron setup required"));
        assert!(text.contains("Analysis loop has not checked in."));
        assert!(text.contains("Trading loop has not checked in."));
        assert!(text.contains(&format!("Hermes profile: {agent_key}")));
    }

    #[tokio::test]
    async fn agent_detail_renders_only_analysis_missing_when_trading_checked_in_recently() {
        let state = test_state().await;
        let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
        set_job_context_timestamps(&state, &agent_key, None, Some(Utc::now())).await;

        let response = router(Arc::clone(&state))
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{agent_key}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let text = response_text(response).await;
        assert!(text.contains("Hermes cron setup required"));
        assert!(text.contains("Analysis loop has not checked in."));
        assert!(!text.contains("Trading loop has not checked in."));
    }

    #[tokio::test]
    async fn agent_detail_renders_only_trading_missing_when_analysis_checked_in_recently() {
        let state = test_state().await;
        let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
        set_job_context_timestamps(&state, &agent_key, Some(Utc::now()), None).await;

        let response = router(Arc::clone(&state))
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{agent_key}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let text = response_text(response).await;
        assert!(text.contains("Hermes cron setup required"));
        assert!(!text.contains("Analysis loop has not checked in."));
        assert!(text.contains("Trading loop has not checked in."));
    }

    #[tokio::test]
    async fn agent_detail_renders_stale_warning_for_analysis_checkin() {
        let state = test_state().await;
        let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
        set_job_context_timestamps(
            &state,
            &agent_key,
            Some(Utc::now() - chrono::Duration::minutes(31)),
            Some(Utc::now()),
        )
        .await;

        let response = router(Arc::clone(&state))
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{agent_key}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let text = response_text(response).await;
        assert!(text.contains("Hermes cron check-in is stale"));
        assert!(text.contains("Analysis loop: last checked in"));
        assert!(text.contains("expected within 30 minutes."));
        assert!(!text.contains("Trading loop: last checked in"));
    }

    #[tokio::test]
    async fn agent_detail_renders_stale_warning_for_trading_checkin() {
        let state = test_state().await;
        let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
        set_job_context_timestamps(
            &state,
            &agent_key,
            Some(Utc::now()),
            Some(Utc::now() - chrono::Duration::minutes(4)),
        )
        .await;

        let response = router(Arc::clone(&state))
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{agent_key}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let text = response_text(response).await;
        assert!(text.contains("Hermes cron check-in is stale"));
        assert!(!text.contains("Analysis loop: last checked in"));
        assert!(text.contains("Trading loop: last checked in"));
        assert!(text.contains("expected within 3 minutes."));
    }

    #[tokio::test]
    async fn agent_detail_hides_cron_alerts_when_both_checkins_are_recent() {
        let state = test_state().await;
        let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
        set_job_context_timestamps(&state, &agent_key, Some(Utc::now()), Some(Utc::now())).await;

        let response = router(Arc::clone(&state))
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{agent_key}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let text = response_text(response).await;
        assert!(!text.contains("Hermes cron setup required"));
        assert!(!text.contains("Hermes cron check-in is stale"));
    }

    #[tokio::test]
    async fn agent_transactions_route_renders_full_timeline() {
        let state = test_state().await;
        let (agent_key, wallet_address) = insert_test_agent(&state).await.expect("insert agent");
        seed_ledger_event(
            &state,
            &wallet_address,
            &format!("tx-route-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0)),
            Utc::now(),
            Decimal::new(42, 0),
        )
        .await;

        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{agent_key}/transactions"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let text = response_text(response).await;
        assert!(text.contains("Transactions"));
        assert!(text.contains("42.0000"));
    }

    #[tokio::test]
    async fn agent_positions_route_renders_latest_trade_execution_summary_under_open_orders() {
        let state = test_state().await;
        let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
        seed_memory_with_type(
            &state,
            &agent_key,
            "trade_execution",
            "Scaled out into strength",
            "Took profit on the upper band.",
        )
        .await;
        seed_memory_with_type(
            &state,
            &agent_key,
            "plan",
            "Older plan",
            "Wait for reclaim.",
        )
        .await;

        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{agent_key}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let text = response_text(response).await;
        assert!(text.contains("Open orders"));
        assert!(text.contains("Scaled out into strength"));
        assert!(!text.contains("Older plan"));
    }

    #[tokio::test]
    async fn latest_trade_execution_summary_event_renders_latest_summary() {
        let state = test_state().await;
        let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
        seed_memory_with_type(
            &state,
            &agent_key,
            "trade_execution",
            "Scaled out into strength",
            "Took profit on the upper band.",
        )
        .await;

        let event = render_latest_trade_execution_summary_event(&state.db_pool, &agent_key)
            .await
            .expect("render latest trade execution summary event");
        let text = format!("{event:?}");

        assert!(text.contains("latest-trade-execution-summary"));
        assert!(text.contains("Scaled out into strength"));
    }

    #[tokio::test]
    async fn agent_memories_route_renders_saved_memories() {
        let state = test_state().await;
        let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
        let memory = seed_memory(
            &state,
            &agent_key,
            "Remember the breakout",
            "### Plan\n\nBTC reclaimed support.",
        )
        .await;

        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{agent_key}/memories"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let text = response_text(response).await;
        assert!(text.contains("Memories"));
        assert!(text.contains("Timeline"));
        assert!(text.contains("Remember the breakout"));
        assert!(text.contains("<h3>Plan</h3>"));
        assert!(text.contains("BTC reclaimed support."));
        assert!(text.contains(&format!("/agents/{agent_key}/memories/{}", memory.id)));
    }

    #[tokio::test]
    async fn agent_memory_detail_route_renders_partial() {
        let state = test_state().await;
        let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
        let memory = seed_memory(
            &state,
            &agent_key,
            "Remember the breakout",
            "### Plan\n\nBTC reclaimed support.",
        )
        .await;

        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{agent_key}/memories/{}", memory.id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let text = response_text(response).await;
        assert!(text.contains("id=\"memory-detail\""));
        assert!(text.contains("Remember the breakout"));
        assert!(text.contains("<h3>Plan</h3>"));
    }

    #[tokio::test]
    async fn memory_timeline_event_renders_refresh_without_selected_card() {
        let state = test_state().await;
        let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
        seed_memory(
            &state,
            &agent_key,
            "Remember the breakout",
            "### Plan\n\nBTC reclaimed support.",
        )
        .await;

        let event = render_memory_timeline_event(&state.db_pool, &agent_key, None, None, None)
            .await
            .expect("render event");
        let text = format!("{event:?}");

        assert!(text.contains("memories-timeline"));
        assert!(text.contains("Remember the breakout"));
        let rows = list_agent_memories(&state.db_pool, &agent_key, None, None)
            .await
            .expect("list memories");
        let timeline = crate::web::templates::build_memory_timeline_for_sse(&agent_key, &rows);
        assert!(timeline.iter().all(|item| !item.selected));
    }

    #[tokio::test]
    async fn agent_prompts_route_renders_prompt_fields() {
        let state = test_state().await;
        let (agent_key, _wallet_address) = insert_test_agent_with_text(
            &state,
            "Wait for analysis confirmation first.".to_string(),
            "Trade breakouts only after confirmation.".to_string(),
        )
        .await
        .expect("insert agent");

        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{agent_key}/prompts"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let text = response_text(response).await;
        assert!(text.contains("Wait for analysis confirmation first."));
        assert!(text.contains("Trade breakouts only after confirmation."));
    }

    #[tokio::test]
    async fn post_agent_prompts_updates_analysis_and_trading_prompt() {
        let state = test_state().await;
        let pool = state.db_pool.clone();
        let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");

        let body = "analysis_prompt=Analyze+momentum+with+market+structure.&trading_prompt=Only+place+limit+orders+near+support.";
        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/agents/{agent_key}/prompts"))
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        let expected_location = format!("/agents/{agent_key}/prompts");
        assert_eq!(
            response
                .headers()
                .get("location")
                .and_then(|value| value.to_str().ok()),
            Some(expected_location.as_str())
        );

        let stored = get_agent(&pool, &agent_key)
            .await
            .expect("get agent")
            .expect("agent present");
        assert_eq!(
            stored.analysis_prompt,
            "Analyze momentum with market structure."
        );
        assert_eq!(
            stored.trading_prompt,
            "Only place limit orders near support."
        );
    }

    #[tokio::test]
    async fn agent_settings_route_renders_sync_state() {
        let state = test_state().await;
        let (agent_key, wallet_address) = insert_test_agent(&state).await.expect("insert agent");
        seed_sync_state(&state, &wallet_address).await;

        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{agent_key}/settings"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let text = response_text(response).await;
        assert!(text.contains("Sync status"));
        assert!(text.contains("fills"));
        assert!(text.contains("abc123"));
    }

    #[tokio::test]
    async fn unknown_agent_subroute_returns_404() {
        let state = test_state().await;

        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri("/agents/does-not-exist-12345/settings")
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
