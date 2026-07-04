use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{
        Html, IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use chrono::Utc;
use futures::StreamExt;
use serde::Deserialize;
use std::{convert::Infallible, sync::Arc};
use tokio_stream::wrappers::BroadcastStream;
use tracing::warn;

use super::shared::is_htmx_request;
use super::show::render_agent_show_page;
use crate::web::error::AppError;
use crate::{
    agents::store::get_agent,
    memory::{
        get_memory as get_memory_record, list_agent_memories,
    },
    web::{
        AppState,
        templates::{
            AgentMemoryDetailPageTemplate, AgentMemoryDetailPartialTemplate,
            AgentMemoryTimelinePartialTemplate, AgentShowTab, MemoryView,
        },
        ui_events::UiEvent,
    },
};
#[derive(Debug, Default, Deserialize)]
pub(in crate::web::routes) struct AgentMemoriesQuery {
    #[serde(default)]
    pub date: String,
}
pub(in crate::web::routes) async fn agents_show_memories(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
    Query(query): Query<AgentMemoriesQuery>,
) -> Result<Response, AppError> {
    render_agent_show_page(
        &state,
        &agent_key,
        AgentShowTab::Memories,
        Some(query),
        None,
        None,
    )
    .await
}
pub(in crate::web::routes) async fn agents_show_memory_detail(
    State(state): State<Arc<AppState>>,
    Path((agent_key, memory_id)): Path<(String, uuid::Uuid)>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    tracing::info!(agent_key = %agent.agent_key, "opened memories SSE stream");

    let Some(memory) = get_memory_record(&state.db_pool, &agent.agent_key, memory_id).await? else {
        return Ok((StatusCode::NOT_FOUND, "memory not found").into_response());
    };

    let memory_view = MemoryView::from_record(memory);

    // HTMX in-place swap (from the Memories tab) only needs the bare partial.
    // Direct browser navigation gets a full styled page so the user sees the
    // agent context and a back link instead of unstyled HTML.
    if is_htmx_request(&headers) {
        let html = AgentMemoryDetailPartialTemplate::render_view(memory_view)?;
        return Ok(Html(html).into_response());
    }

    let memory_detail_html = AgentMemoryDetailPartialTemplate::render_view(memory_view.clone())?;
    let html =
        AgentMemoryDetailPageTemplate::render_view(agent.clone(), memory_view, memory_detail_html)?;
    Ok(Html(html).into_response())
}
pub(in crate::web::routes) async fn agent_memories_stream(
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
pub(in crate::web::routes) async fn render_memory_timeline_event(
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
pub(in crate::web::routes) fn parse_memory_date_filter(
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
