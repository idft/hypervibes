use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use futures::StreamExt;
use std::{convert::Infallible, sync::Arc};
use tokio_stream::wrappers::BroadcastStream;
use tracing::warn;

use crate::web::error::AppError;
use crate::{
    agents::store::{get_agent, list_agent_instrument_ids},
    hyperliquid::live_state::{AccountKey, AccountLiveState, LiveConnectionStatus},
    memory::{get_latest_agent_memory_by_type, get_memory as get_memory_record, memory_expires_at},
    web::{
        AppState,
        templates::{
            AccountBalancePartialTemplate, AccountBalanceView,
            LatestAnalysisSummaryPartialTemplate, LatestTradeExecutionSummaryPartialTemplate,
            OpenOrdersPartialTemplate, OpenOrdersView, OpenPositionsPartialTemplate,
            OpenPositionsView,
        },
        ui_events::UiEvent,
    },
};
#[derive(Debug)]
pub(in crate::web::routes) enum MemoryNotification {
    Some(uuid::Uuid),
    Lagged,
}

pub(in crate::web::routes) async fn agent_live_stream(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    let agent = match get_agent(&state.db_pool, &agent_key).await? {
        Some(agent) => agent,
        None => return Ok((StatusCode::NOT_FOUND, "agent not found").into_response()),
    };
    let configured_coins = match list_agent_instrument_ids(&state.db_pool, &agent.agent_key).await {
        Ok(rows) => rows,
        Err(error) => {
            warn!(
                agent_key = %agent.agent_key,
                error = ?error,
                "failed to list configured instruments for live positions stream"
            );
            Vec::new()
        }
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
    let mut initial_events = render_live_events(&initial_snapshot, &configured_coins)?;
    initial_events
        .push(render_latest_trade_execution_summary_event(&state.db_pool, &agent.agent_key).await?);
    initial_events
        .push(render_latest_analysis_summary_event(&state.db_pool, &agent.agent_key).await?);

    let account_key_filter = account_key.clone();
    let live_accounts_filter = Arc::clone(&live_accounts);
    let configured_coins_filter = configured_coins.clone();
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
            let configured_coins = configured_coins_filter.clone();
            let events = match live_accounts.get(&key) {
                Some(snapshot) => match render_live_events(&snapshot, &configured_coins) {
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
                    }) if event_agent_key == agent_key => Some(MemoryNotification::Some(memory_id)),
                    Ok(_) => None,
                    Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(_)) => {
                        Some(MemoryNotification::Lagged)
                    }
                }
            }
        })
        .then(move |notification| {
            let db_pool = summary_db_pool.clone();
            let agent_key = summary_agent_key_render.clone();
            async move {
                let (needs_trade_execution, needs_analysis) = match notification {
                    MemoryNotification::Some(memory_id) => {
                        match get_memory_record(&db_pool, &agent_key, memory_id).await {
                            Ok(Some(memory)) => match memory.memory_type.as_str() {
                                "trade_execution" => (true, false),
                                "market_analysis" => (false, true),
                                _ => (false, false),
                            },
                            Ok(None) => (false, false),
                            Err(error) => {
                                warn!(agent_key = %agent_key, error = ?error, "failed to inspect memory event for live summary update");
                                (false, false)
                            }
                        }
                    }
                    MemoryNotification::Lagged => {
                        // Lagged notification: we may have missed a memory
                        // of either type, so re-render both summaries to
                        // catch up.
                        (true, true)
                    }
                };

                let mut events = Vec::new();
                if needs_trade_execution {
                    match render_latest_trade_execution_summary_event(&db_pool, &agent_key).await {
                        Ok(event) => events.push(Ok::<Event, Infallible>(event)),
                        Err(error) => {
                            warn!(agent_key = %agent_key, error = ?error, "failed to render latest trade execution summary SSE event");
                        }
                    }
                }
                if needs_analysis {
                    match render_latest_analysis_summary_event(&db_pool, &agent_key).await {
                        Ok(event) => events.push(Ok::<Event, Infallible>(event)),
                        Err(error) => {
                            warn!(agent_key = %agent_key, error = ?error, "failed to render latest analysis summary SSE event");
                        }
                    }
                }
                events
            }
        })
        .flat_map(|events| tokio_stream::iter(events));

    let stream = tokio_stream::iter(
        initial_events
            .into_iter()
            .map(Ok::<Event, Infallible>)
            .collect::<Vec<_>>(),
    )
    .chain(futures::stream::select(
        notifications,
        summary_notifications,
    ));
    let sse =
        Sse::new(stream).keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(15)));
    Ok(sse.into_response())
}
pub(in crate::web::routes) fn render_live_events(
    state: &AccountLiveState,
    configured_coins: &[String],
) -> Result<Vec<Event>, AppError> {
    Ok(vec![
        render_account_balance_event(state)?,
        render_open_positions_event(state, configured_coins)?,
        render_open_orders_event(state)?,
    ])
}
pub(in crate::web::routes) fn render_account_balance_event(
    state: &AccountLiveState,
) -> Result<Event, AppError> {
    let view = AccountBalanceView::from_live_state(state.clone());
    let html = AccountBalancePartialTemplate::render_view(view)?;
    Ok(Event::default().event("balance").data(html))
}
pub(in crate::web::routes) fn render_open_positions_event(
    state: &AccountLiveState,
    configured_coins: &[String],
) -> Result<Event, AppError> {
    let view =
        OpenPositionsView::from_live_state_with_configured_coins(state.clone(), configured_coins);
    let html = OpenPositionsPartialTemplate::render_view(view)?;
    Ok(Event::default().event("positions").data(html))
}
pub(in crate::web::routes) fn render_open_orders_event(
    state: &AccountLiveState,
) -> Result<Event, AppError> {
    let view = OpenOrdersView::from_live_state(state.clone());
    let html = OpenOrdersPartialTemplate::render_view(view)?;
    Ok(Event::default().event("orders").data(html))
}
pub(in crate::web::routes) async fn render_latest_trade_execution_summary_event(
    pool: &crate::db::DbPool,
    agent_key: &str,
) -> Result<Event, AppError> {
    let latest = get_latest_agent_memory_by_type(pool, agent_key, "trade_execution").await?;
    let summary = latest.as_ref().map(|memory| memory.summary.clone());
    let created_at = latest.as_ref().map(|memory| memory.created_at);
    let html = LatestTradeExecutionSummaryPartialTemplate::render_view(summary, created_at)?;
    Ok(Event::default()
        .event("latest-trade-execution-summary")
        .data(html))
}
pub(in crate::web::routes) async fn render_latest_analysis_summary_event(
    pool: &crate::db::DbPool,
    agent_key: &str,
) -> Result<Event, AppError> {
    let latest = get_latest_agent_memory_by_type(pool, agent_key, "market_analysis").await?;
    let detail_url = latest
        .as_ref()
        .map(|memory| format!("/agents/{agent_key}/memories/{}", memory.id));
    let summary = latest.as_ref().map(|memory| memory.summary.clone());
    let created_at = latest.as_ref().map(|memory| memory.created_at);
    let expires_at = latest.as_ref().and_then(memory_expires_at);
    let html = LatestAnalysisSummaryPartialTemplate::render_view(
        summary, detail_url, created_at, expires_at,
    )?;
    Ok(Event::default().event("latest-analysis-summary").data(html))
}
