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
    agents::store::{get_agent, list_agent_trading_instrument_ids},
    hyperliquid::{
        live_state::{AccountKey, AccountLiveState, LiveConnectionStatus},
        queries::{BalanceSeriesBucket, fetch_balance_series},
    },
    memory::{MemoryListFilter, store::list_memories},
    web::{
        AppState,
        templates::{
            AccountBalancePartialTemplate, AccountBalanceView, AgentActivityPartialTemplate,
            BalanceSparklinesPartialTemplate, LiveAccountHealthPartialTemplate,
            LiveAccountHealthView, OpenOrdersPartialTemplate, OpenOrdersView,
            OpenPositionsPartialTemplate, OpenPositionsView, SparklineView,
        },
        ui_events::UiEvent,
    },
};
pub(in crate::web::routes) async fn agent_live_stream(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    let agent = match get_agent(&state.db_pool, &agent_key).await? {
        Some(agent) => agent,
        None => return Ok((StatusCode::NOT_FOUND, "agent not found").into_response()),
    };
    let configured_coins =
        match list_agent_trading_instrument_ids(&state.db_pool, &agent.agent_key).await {
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

    let Some(trading_account_address) = agent.trading_account_address.as_deref() else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    let account_key = AccountKey::new(trading_account_address, &agent.environment);
    let live_accounts = Arc::clone(&state.live_accounts);
    let activity_receiver = state.ui_events.subscribe();

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
    let mut initial_events = render_live_events(
        &initial_snapshot,
        &configured_coins,
        &state.market_data.snapshot(),
        &agent.agent_key,
    )?;
    initial_events.push(render_agent_activity_event(&state.db_pool, &agent.agent_key).await?);
    let mut last_sparklines_html =
        render_balance_sparklines_html(&state.db_pool, &account_key).await?;
    initial_events.push(
        Event::default()
            .event("sparklines")
            .data(&last_sparklines_html),
    );

    // Charts use the durable journal, not the in-memory account snapshot.
    // Poll independently so a fill committed after an account notification,
    // funding, or an HTTP catch-up is reflected without another exchange event.
    let sparklines_db_pool = state.db_pool.clone();
    let sparklines_account_key = account_key.clone();
    let sparklines_refresh = tokio_stream::wrappers::IntervalStream::new(tokio::time::interval(
        std::time::Duration::from_secs(5),
    ))
    .skip(1)
    .then(move |_| {
        let pool = sparklines_db_pool.clone();
        let account_key = sparklines_account_key.clone();
        async move {
            match render_balance_sparklines_html(&pool, &account_key).await {
                Ok(html) => Some(html),
                Err(error) => {
                    warn!(error = ?error, "failed to render balance sparklines SSE event");
                    None
                }
            }
        }
    })
    .filter_map(move |html| {
        let event = html
            .filter(|html| *html != last_sparklines_html)
            .map(|html| {
                let event = Event::default().event("sparklines").data(&html);
                last_sparklines_html = html;
                Ok::<Event, Infallible>(event)
            });
        futures::future::ready(event)
    });

    let account_key_filter = account_key.clone();
    let account_key_for_notifications = account_key.clone();
    let live_accounts_filter = Arc::clone(&live_accounts);
    let configured_coins_filter = configured_coins.clone();
    let agent_key_for_positions = agent.agent_key.clone();
    let market_data_for_notifications = Arc::clone(&state.market_data);
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
            let key = account_key_for_notifications.clone();
            let configured_coins = configured_coins_filter.clone();
            let market_data = Arc::clone(&market_data_for_notifications);
            let events = match live_accounts.get(&key) {
                Some(snapshot) => {
                    match render_live_events(
                        &snapshot,
                        &configured_coins,
                        &market_data.snapshot(),
                        &agent_key_for_positions,
                    ) {
                        Ok(events) => events,
                        Err(e) => {
                            warn!(error = ?e, "failed to render live SSE events");
                            Vec::new()
                        }
                    }
                }
                None => Vec::new(),
            };
            tokio_stream::iter(events.into_iter().map(Ok::<Event, Infallible>))
        });

    let activity_db_pool = state.db_pool.clone();
    let activity_agent_key_filter = agent.agent_key.clone();
    let activity_agent_key_render = agent.agent_key.clone();
    let activity_notifications = BroadcastStream::new(activity_receiver)
        .filter_map(move |item| {
            let agent_key = activity_agent_key_filter.clone();
            async move {
                match item {
                    Ok(UiEvent::MemoryCreated {
                        agent_key: event_agent_key,
                        ..
                    }) if event_agent_key == agent_key => Some(()),
                    Ok(_) => None,
                    Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(_)) => {
                        Some(())
                    }
                }
            }
        })
        .then(move |()| {
            let db_pool = activity_db_pool.clone();
            let agent_key = activity_agent_key_render.clone();
            async move {
                match render_agent_activity_event(&db_pool, &agent_key).await {
                    Ok(event) => Some(Ok::<Event, Infallible>(event)),
                    Err(error) => {
                        warn!(agent_key = %agent_key, error = ?error, "failed to render activity SSE event");
                        None
                    }
                }
            }
        })
        .filter_map(|event| async move { event });

    // Store notifications cover incoming exchange data. This timer covers the
    // opposite case: a silent connection must still visibly become stale.
    let live_accounts_refresh = Arc::clone(&live_accounts);
    let refresh_account_key = account_key.clone();
    let refresh_configured_coins = configured_coins.clone();
    let refresh_agent_key = agent.agent_key.clone();
    let initial_market_coins = configured_coins.clone();
    let initial_market_data = Arc::clone(&state.market_data);
    let initial_market_refresh = futures::stream::once(async move {
        initial_market_data.refresh(&initial_market_coins).await;
        initial_market_data.snapshot()
    });
    let periodic_market_coins = configured_coins.clone();
    let periodic_market_data = Arc::clone(&state.market_data);
    let periodic_market_refresh = tokio_stream::wrappers::IntervalStream::new(
        tokio::time::interval(std::time::Duration::from_secs(15)),
    )
    .skip(1)
    .then(move |_| {
        let market_data = Arc::clone(&periodic_market_data);
        let configured_coins = periodic_market_coins.clone();
        async move {
            market_data.refresh(&configured_coins).await;
            market_data.snapshot()
        }
    });
    // Reevaluate display grace/freshness even if the exchange is silent or a
    // market refresh is waiting on HTTP. These ticks use only cached prices.
    let freshness_market_data = Arc::clone(&state.market_data);
    let freshness_ticks = tokio_stream::wrappers::IntervalStream::new(tokio::time::interval(
        std::time::Duration::from_secs(5),
    ))
    .skip(1)
    .map(move |_| freshness_market_data.snapshot());
    let freshness_refresh = futures::stream::select(
        initial_market_refresh.chain(periodic_market_refresh),
        freshness_ticks,
    )
    .flat_map(move |market_data| {
        let events = live_accounts_refresh
            .get(&refresh_account_key)
            .map(|snapshot| {
                render_live_events(
                    &snapshot,
                    &refresh_configured_coins,
                    &market_data,
                    &refresh_agent_key,
                )
                .unwrap_or_else(|error| {
                    warn!(error = ?error, "failed to render live freshness SSE events");
                    Vec::new()
                })
            })
            .unwrap_or_default();
        tokio_stream::iter(events.into_iter().map(Ok::<Event, Infallible>))
    });

    let mut shutdown_rx = state.shutdown_rx.clone();
    let shutdown = async move {
        loop {
            if *shutdown_rx.borrow() || shutdown_rx.changed().await.is_err() {
                return;
            }
        }
    };
    let stream = tokio_stream::iter(
        initial_events
            .into_iter()
            .map(Ok::<Event, Infallible>)
            .collect::<Vec<_>>(),
    )
    .chain(futures::stream::select(
        futures::stream::select(notifications, freshness_refresh),
        futures::stream::select(activity_notifications, sparklines_refresh),
    ))
    // Ending the SSE stream drops an in-progress market-data request before
    // the runtime starts tearing down its HTTP dispatcher.
    .take_until(shutdown);
    let sse =
        Sse::new(stream).keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(15)));
    Ok(sse.into_response())
}
pub(in crate::web::routes) fn render_live_events(
    state: &AccountLiveState,
    configured_coins: &[String],
    market_data: &crate::hyperliquid::market_data::MarketDataSnapshot,
    agent_key: &str,
) -> Result<Vec<Event>, AppError> {
    Ok(vec![
        render_live_account_health_event(state)?,
        render_account_balance_event(state)?,
        render_open_positions_event(state, configured_coins, market_data, agent_key)?,
        render_open_orders_event(state)?,
    ])
}
pub(in crate::web::routes) fn render_live_account_health_event(
    state: &AccountLiveState,
) -> Result<Event, AppError> {
    let view = LiveAccountHealthView::from_live_state(state);
    let html = LiveAccountHealthPartialTemplate::render_view(view)?;
    Ok(Event::default().event("health").data(html))
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
    market_data: &crate::hyperliquid::market_data::MarketDataSnapshot,
    agent_key: &str,
) -> Result<Event, AppError> {
    let view = OpenPositionsView::from_live_state_with_configured_coins_and_market_data(
        state.clone(),
        configured_coins,
        market_data,
    )
    .with_agent_key(agent_key);
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
pub(in crate::web::routes) async fn render_agent_activity_html(
    pool: &crate::db::DbPool,
    agent_key: &str,
) -> Result<String, AppError> {
    let memories = list_memories(
        pool,
        agent_key,
        &MemoryListFilter {
            limit: Some(5),
            ..Default::default()
        },
    )
    .await?;
    Ok(AgentActivityPartialTemplate::render_view(
        agent_key, memories,
    )?)
}

pub(in crate::web::routes) async fn render_balance_sparklines_html(
    pool: &crate::db::DbPool,
    account_key: &AccountKey,
) -> Result<String, AppError> {
    let now = chrono::Utc::now();
    let mut sparklines = Vec::with_capacity(2);
    for (label, window, bucket) in [
        (
            "24 hours",
            chrono::Duration::hours(24),
            BalanceSeriesBucket::Hour,
        ),
        (
            "30 days",
            chrono::Duration::days(30),
            BalanceSeriesBucket::Day,
        ),
    ] {
        let series = match fetch_balance_series(
            pool,
            &account_key.account_address,
            &account_key.environment,
            now - window,
            bucket,
        )
        .await
        {
            Ok(points) => points,
            Err(error) => {
                warn!(
                    trading_account_address = %account_key.account_address,
                    environment = %account_key.environment,
                    window = label,
                    error = ?error,
                    "failed to fetch balance series for agent charts"
                );
                Vec::new()
            }
        };
        sparklines.push(SparklineView::from_series(label, &series, 240, 48));
    }
    Ok(BalanceSparklinesPartialTemplate::render_view(sparklines)?)
}

pub(in crate::web::routes) async fn render_agent_activity_event(
    pool: &crate::db::DbPool,
    agent_key: &str,
) -> Result<Event, AppError> {
    let html = render_agent_activity_html(pool, agent_key).await?;
    Ok(Event::default().event("activity").data(html))
}
