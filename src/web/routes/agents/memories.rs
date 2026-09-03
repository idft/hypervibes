use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{
        Html, IntoResponse, Redirect, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use chrono::{DateTime, Utc};
use futures::StreamExt;
use serde::Deserialize;
use std::{convert::Infallible, sync::Arc};
use tokio_stream::wrappers::BroadcastStream;
use tracing::warn;

use super::shared::is_htmx_request;
use super::show::{AgentShowQueries, load_selected_agent_navbar, render_agent_show_page};
use crate::web::error::AppError;
use crate::{
    agents::store::get_agent,
    memory::{
        AGENT_MEMORY_TIMELINE_PAGE_SIZE, MemoryTimelineRecord, get_memory as get_memory_record,
        list_agent_memory_timeline,
    },
    notifications::store::count_notifications,
    web::{
        AppState,
        auth::AuthenticatedUser,
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
    #[serde(default)]
    pub before_us: String,
    #[serde(default)]
    pub before_id: String,
}
pub(in crate::web::routes) async fn agents_show_memories(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path(agent_key): Path<String>,
    Query(query): Query<AgentMemoriesQuery>,
) -> Result<Response, AppError> {
    render_agent_show_page(
        &state,
        &user,
        &agent_key,
        AgentShowTab::Memories,
        AgentShowQueries {
            memories: Some(query),
            ..Default::default()
        },
    )
    .await
}
pub(in crate::web::routes) async fn agents_show_memory_detail(
    State(state): State<Arc<AppState>>,
    Path((agent_key, memory_id)): Path<(String, uuid::Uuid)>,
    headers: HeaderMap,
    user: AuthenticatedUser,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let Some(memory) = get_memory_record(&state.db_pool, &agent.agent_key, memory_id).await? else {
        return Ok((StatusCode::NOT_FOUND, "memory not found").into_response());
    };

    let memory_view = MemoryView::from_record(memory);

    // HTMX in-place swap (from the Memories tab) only needs the bare partial.
    // Direct browser navigation gets a full styled page so the user sees the
    // agent context and a back link instead of unstyled HTML.
    if is_htmx_request(&headers)
        && !headers
            .get("HX-Boosted")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.eq_ignore_ascii_case("true"))
    {
        let html = AgentMemoryDetailPartialTemplate::render_view(memory_view)?;
        return Ok(Html(html).into_response());
    }

    let memory_detail_html = AgentMemoryDetailPartialTemplate::render_page_view(
        memory_view.clone(),
        format!("/agents/{}/memories", agent.agent_key),
    )?;
    let navbar = load_selected_agent_navbar(&state, user.id, &agent).await?;
    let notification_count = count_notifications(&state.db_pool, &agent.agent_key).await?;
    let html = AgentMemoryDetailPageTemplate::render_view(
        agent.clone(),
        memory_view,
        memory_detail_html,
        notification_count,
        navbar,
    )?;
    Ok(Html(html).into_response())
}
pub(in crate::web::routes) async fn agents_delete_memory(
    State(state): State<Arc<AppState>>,
    Path((agent_key, memory_id)): Path<(String, uuid::Uuid)>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };

    let deleted = crate::memory::delete_memory(&state.db_pool, &agent.agent_key, memory_id).await?;
    if !deleted {
        return Ok((StatusCode::NOT_FOUND, "memory not found").into_response());
    }

    Ok(Redirect::to(&format!("/agents/{agent_key}/memories")).into_response())
}
pub(in crate::web::routes) async fn agent_memory_timeline_page(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
    Query(query): Query<AgentMemoriesQuery>,
) -> Result<Response, AppError> {
    let (_, selected_date_text, _, since, until) = parse_memory_date_filter(&query.date);
    let Ok(before) = parse_memory_cursor(&query.before_us, &query.before_id) else {
        return Ok((StatusCode::BAD_REQUEST, "invalid memory timeline cursor").into_response());
    };
    let rows = list_agent_memory_timeline(&state.db_pool, &agent_key, since, until, before).await?;
    let (rows, next_page_url) = prepare_memory_timeline_page(
        &agent_key,
        rows,
        selected_date_text.as_ref().map(|_| query.date.as_str()),
    );
    let timeline = crate::web::templates::build_memory_timeline_for_sse(&agent_key, &rows);
    let html = crate::web::templates::AgentMemoryTimelineItemsPartialTemplate::render_view(
        timeline,
        next_page_url,
    )?;
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
    let date_filter_value = query.date.clone();

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
            let date_filter_value = date_filter_value.clone();
            async move {
                match render_memory_timeline_event(
                    &db_pool,
                    &agent_key,
                    since,
                    until,
                    selected_date_text,
                    date_filter_value,
                )
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
    date_filter_value: String,
) -> Result<Event, AppError> {
    let rows = list_agent_memory_timeline(pool, agent_key, since, until, None).await?;
    let (rows, next_page_url) = prepare_memory_timeline_page(
        agent_key,
        rows,
        selected_date_text
            .as_ref()
            .map(|_| date_filter_value.as_str()),
    );
    let timeline = crate::web::templates::build_memory_timeline_for_sse(agent_key, &rows);
    let html = AgentMemoryTimelinePartialTemplate::render_view(
        timeline,
        rows.len(),
        selected_date_text,
        next_page_url,
    )?;
    Ok(Event::default().event("memories-timeline").data(html))
}

pub(in crate::web::routes) fn prepare_memory_timeline_page(
    agent_key: &str,
    mut rows: Vec<MemoryTimelineRecord>,
    date: Option<&str>,
) -> (Vec<MemoryTimelineRecord>, Option<String>) {
    let has_more = rows.len() > AGENT_MEMORY_TIMELINE_PAGE_SIZE as usize;
    rows.truncate(AGENT_MEMORY_TIMELINE_PAGE_SIZE as usize);
    let next_page_url = has_more.then(|| {
        let last = rows.last().expect("a full timeline page has a last row");
        let mut url = format!(
            "/agents/{agent_key}/memories/timeline?before_us={}&before_id={}",
            last.created_at.timestamp_micros(),
            last.id
        );
        if let Some(date) = date {
            url.push_str("&date=");
            url.push_str(date);
        }
        url
    });
    (rows, next_page_url)
}

fn parse_memory_cursor(
    before_us: &str,
    before_id: &str,
) -> Result<Option<(DateTime<Utc>, uuid::Uuid)>, ()> {
    if before_us.is_empty() && before_id.is_empty() {
        return Ok(None);
    }
    let timestamp = before_us
        .parse::<i64>()
        .ok()
        .and_then(DateTime::<Utc>::from_timestamp_micros);
    let id = uuid::Uuid::parse_str(before_id).ok();
    match (timestamp, id) {
        (Some(timestamp), Some(id)) => Ok(Some((timestamp, id))),
        _ => Err(()),
    }
}
type MemoryDateFilter = (
    String,
    Option<String>,
    Option<String>,
    Option<chrono::DateTime<Utc>>,
    Option<chrono::DateTime<Utc>>,
);

pub(in crate::web::routes) fn parse_memory_date_filter(raw: &str) -> MemoryDateFilter {
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
