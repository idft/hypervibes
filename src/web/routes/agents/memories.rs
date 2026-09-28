use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{
        Html, IntoResponse, Redirect, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use chrono::{DateTime, Duration, LocalResult, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use futures::StreamExt;
use serde::Deserialize;
use std::{convert::Infallible, sync::Arc};
use tokio_stream::wrappers::BroadcastStream;
use tracing::warn;

use super::shared::{is_htmx_request, urlencode};
use super::show::{AgentShowQueries, load_selected_agent_navbar, render_agent_show_page};
use crate::web::error::AppError;
use crate::{
    agents::store::get_agent,
    memory::{
        AGENT_MEMORY_TIMELINE_PAGE_SIZE, MemoryTimelineRecord, MemoryTypeCount,
        get_memory as get_memory_record, list_agent_memory_timeline, list_agent_memory_type_counts,
    },
    notifications::store::count_notifications,
    web::{
        AppState,
        auth::AuthenticatedUser,
        templates::{
            AgentMemoryBrowserPartialTemplate, AgentMemoryDetailPageTemplate,
            AgentMemoryDetailPartialTemplate, AgentMemoryTimelineItemsPartialTemplate,
            AgentShowTab, MemoryBrowserOptions, MemoryTypeOption, MemoryView,
        },
        ui_events::UiEvent,
    },
};
#[derive(Debug, Clone, Default, Deserialize)]
pub(in crate::web::routes) struct AgentMemoriesQuery {
    #[serde(default)]
    pub date: String,
    #[serde(default)]
    pub range: String,
    #[serde(default)]
    pub start: String,
    #[serde(default)]
    pub end: String,
    #[serde(default)]
    pub tz: String,
    #[serde(default)]
    pub memory_type: String,
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
    let date_filter = parse_memory_time_filter(&query, Utc::now());
    let Ok(before) = parse_memory_cursor(&query.before_us, &query.before_id) else {
        return Ok((StatusCode::BAD_REQUEST, "invalid memory timeline cursor").into_response());
    };
    let memory_type = (!query.memory_type.is_empty()).then_some(query.memory_type.as_str());
    let rows = list_agent_memory_timeline(
        &state.db_pool,
        &agent_key,
        memory_type,
        date_filter.since,
        date_filter.until,
        before,
    )
    .await?;
    let (rows, next_page_url) =
        prepare_memory_timeline_page(&agent_key, rows, &date_filter, memory_type);
    let timeline = crate::web::templates::build_memory_timeline_for_sse(&agent_key, &rows);
    let html = AgentMemoryTimelineItemsPartialTemplate::render_view(timeline, next_page_url)?;
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

    let stream_query = query.clone();
    let db_pool = state.db_pool.clone();
    let agent_key = agent.agent_key.clone();
    let agent_key_for_render = agent_key.clone();
    let memory_type = query.memory_type;

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
            let date_filter = parse_memory_time_filter(&stream_query, Utc::now());
            let memory_type = memory_type.clone();
            async move {
                match render_memory_timeline_event(
                    &db_pool,
                    &agent_key,
                    &date_filter,
                    &memory_type,
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
    date_filter: &MemoryDateFilter,
    selected_type: &str,
) -> Result<Event, AppError> {
    let memory_type = (!selected_type.is_empty()).then_some(selected_type);
    let rows = list_agent_memory_timeline(
        pool,
        agent_key,
        memory_type,
        date_filter.since,
        date_filter.until,
        None,
    )
    .await?;
    let types =
        list_agent_memory_type_counts(pool, agent_key, date_filter.since, date_filter.until)
            .await?;
    let (rows, next_page_url) =
        prepare_memory_timeline_page(agent_key, rows, date_filter, memory_type);
    let timeline = crate::web::templates::build_memory_timeline_for_sse(agent_key, &rows);
    let html = AgentMemoryBrowserPartialTemplate::render_view(
        timeline,
        next_page_url,
        MemoryBrowserOptions {
            date_text: date_filter.date_text.clone(),
            memory_types: build_memory_type_options(agent_key, &types, selected_type, date_filter),
            all_url: memory_page_url(agent_key, "", date_filter),
            selected_type: selected_type.to_string(),
            has_any_memories: !types.is_empty(),
        },
    )?;
    Ok(Event::default().event("memories-browser").data(html))
}

pub(in crate::web::routes) fn prepare_memory_timeline_page(
    agent_key: &str,
    mut rows: Vec<MemoryTimelineRecord>,
    date_filter: &MemoryDateFilter,
    memory_type: Option<&str>,
) -> (Vec<MemoryTimelineRecord>, Option<String>) {
    let has_more = rows.len() > AGENT_MEMORY_TIMELINE_PAGE_SIZE as usize;
    rows.truncate(AGENT_MEMORY_TIMELINE_PAGE_SIZE as usize);
    let next_page_url = has_more.then(|| {
        let last = rows.last().expect("a full timeline page has a last row");
        format!(
            "/agents/{agent_key}/memories/timeline?before_us={}&before_id={}{}",
            last.created_at.timestamp_micros(),
            last.id,
            date_filter.query_tail(memory_type.unwrap_or_default()),
        )
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
#[derive(Debug, Clone, Default)]
pub(in crate::web::routes) struct MemoryDateFilter {
    pub range_value: String,
    pub start_value: String,
    pub end_value: String,
    pub date_value: String,
    pub tz_value: String,
    pub date_text: Option<String>,
    pub error_text: Option<String>,
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
}

impl MemoryDateFilter {
    fn query_tail(&self, memory_type: &str) -> String {
        let mut tail = String::new();
        if self.date_text.is_some() {
            if !self.date_value.is_empty() {
                tail.push_str(&format!(
                    "&date={}&tz={}",
                    urlencode(&self.date_value),
                    urlencode(&self.tz_value)
                ));
            } else if self.range_value == "custom" {
                tail.push_str(&format!(
                    "&range=custom&start={}&end={}&tz={}",
                    urlencode(&self.start_value),
                    urlencode(&self.end_value),
                    urlencode(&self.tz_value)
                ));
            }
        }
        if matches!(self.range_value.as_str(), "1h" | "6h" | "24h") {
            tail.push_str(&format!("&range={}", self.range_value));
        }
        if !memory_type.is_empty() {
            tail.push_str(&format!("&memory_type={}", urlencode(memory_type)));
        }
        tail
    }
}

pub(in crate::web::routes) fn memory_preset_url(
    agent_key: &str,
    memory_type: &str,
    range: &str,
) -> String {
    memory_page_url(
        agent_key,
        memory_type,
        &MemoryDateFilter {
            range_value: range.to_string(),
            ..Default::default()
        },
    )
}

pub(in crate::web::routes) fn parse_memory_time_filter(
    query: &AgentMemoriesQuery,
    now: DateTime<Utc>,
) -> MemoryDateFilter {
    if query.range.is_empty() && !query.date.is_empty() {
        return parse_memory_date_filter(&query.date, &query.tz);
    }
    let range = query.range.as_str();
    if let Some(hours) = match range {
        "1h" => Some(1),
        "6h" => Some(6),
        "24h" => Some(24),
        _ => None,
    } {
        return MemoryDateFilter {
            range_value: range.to_string(),
            date_text: Some(format!("Last {hours}h")),
            since: Some(now - Duration::hours(hours)),
            ..Default::default()
        };
    }
    if range == "custom" {
        return parse_memory_custom_filter(&query.start, &query.end, &query.tz);
    }
    if range.is_empty() || range == "all" {
        return MemoryDateFilter::default();
    }
    MemoryDateFilter {
        error_text: Some("Choose a valid time filter.".to_string()),
        ..Default::default()
    }
}

fn parse_memory_custom_filter(raw_start: &str, raw_end: &str, raw_tz: &str) -> MemoryDateFilter {
    let mut filter = MemoryDateFilter {
        range_value: "custom".to_string(),
        start_value: raw_start.trim().to_string(),
        end_value: raw_end.trim().to_string(),
        tz_value: raw_tz.trim().to_string(),
        ..Default::default()
    };
    let (Ok(start), Ok(end)) = (
        NaiveDate::parse_from_str(&filter.start_value, "%Y-%m-%d"),
        NaiveDate::parse_from_str(&filter.end_value, "%Y-%m-%d"),
    ) else {
        filter.error_text = Some("Choose a start and end date in YYYY-MM-DD format.".to_string());
        return filter;
    };
    if start > end {
        filter.error_text = Some("The start date must be on or before the end date.".to_string());
        return filter;
    }
    if filter.tz_value.is_empty() {
        filter.tz_value = "UTC".to_string();
    }
    let Ok(tz) = filter.tz_value.parse::<Tz>() else {
        filter.error_text = Some("That time zone is not recognized.".to_string());
        return filter;
    };
    let Some(next_day) = end.succ_opt() else {
        filter.error_text = Some("That end date is out of range.".to_string());
        return filter;
    };
    let (Some(since), Some(until)) = (local_day_start(start, tz), local_day_start(next_day, tz))
    else {
        filter.error_text =
            Some("That date range could not be resolved in this time zone.".to_string());
        return filter;
    };
    filter.date_text = Some(if start == end {
        start.format("%b %-d, %Y").to_string()
    } else {
        format!(
            "{} – {}",
            start.format("%b %-d, %Y"),
            end.format("%b %-d, %Y")
        )
    });
    filter.since = Some(since);
    filter.until = Some(until);
    filter
}

pub(in crate::web::routes) fn memory_page_url(
    agent_key: &str,
    memory_type: &str,
    date_filter: &MemoryDateFilter,
) -> String {
    memory_filter_url(
        &format!("/agents/{agent_key}/memories"),
        memory_type,
        date_filter,
    )
}

pub(in crate::web::routes) fn memory_stream_url(
    agent_key: &str,
    memory_type: &str,
    date_filter: &MemoryDateFilter,
) -> String {
    memory_filter_url(
        &format!("/agents/{agent_key}/memories/stream"),
        memory_type,
        date_filter,
    )
}

fn memory_filter_url(base: &str, memory_type: &str, date_filter: &MemoryDateFilter) -> String {
    let tail = date_filter.query_tail(memory_type);
    if tail.is_empty() {
        base.to_string()
    } else {
        format!("{base}?{}", &tail[1..])
    }
}

pub(in crate::web::routes) fn build_memory_type_options(
    agent_key: &str,
    types: &[MemoryTypeCount],
    selected_type: &str,
    date_filter: &MemoryDateFilter,
) -> Vec<MemoryTypeOption> {
    let has_time_filter = date_filter.since.is_some() || date_filter.until.is_some();
    types
        .iter()
        .filter(|row| !has_time_filter || row.count > 0)
        .map(|row| MemoryTypeOption {
            name: row.memory_type.clone(),
            count: row.count,
            url: memory_page_url(agent_key, &row.memory_type, date_filter),
            selected: selected_type == row.memory_type,
        })
        .collect()
}

pub(in crate::web::routes) fn parse_memory_date_filter(
    raw: &str,
    raw_tz: &str,
) -> MemoryDateFilter {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return MemoryDateFilter::default();
    }

    let mut filter = MemoryDateFilter {
        range_value: "custom".to_string(),
        start_value: trimmed.to_string(),
        end_value: trimmed.to_string(),
        date_value: trimmed.to_string(),
        tz_value: raw_tz.trim().to_string(),
        ..Default::default()
    };
    let Ok(date) = NaiveDate::parse_from_str(trimmed, "%Y-%m-%d") else {
        filter.error_text = Some("Use YYYY-MM-DD to filter memories by local date.".to_string());
        return filter;
    };
    // Bare legacy date URLs retain UTC semantics; the form supplies the browser's IANA zone.
    if filter.tz_value.is_empty() {
        filter.tz_value = "UTC".to_string();
    }
    let Ok(tz) = filter.tz_value.parse::<Tz>() else {
        filter.error_text = Some("That time zone is not recognized.".to_string());
        return filter;
    };
    let Some(next_day) = date.succ_opt() else {
        filter.error_text = Some("That date is out of range.".to_string());
        return filter;
    };
    let (Some(since), Some(until)) = (local_day_start(date, tz), local_day_start(next_day, tz))
    else {
        filter.error_text = Some("That date could not be resolved in this time zone.".to_string());
        return filter;
    };
    filter.date_text = Some(date.format("%A, %B %-d, %Y").to_string());
    filter.since = Some(since);
    filter.until = Some(until);
    filter
}

fn local_day_start(date: NaiveDate, tz: Tz) -> Option<DateTime<Utc>> {
    // Some zones transition at midnight: use the earliest valid minute of that day.
    (0..24 * 60).find_map(|minute| {
        let local = date.and_hms_opt(minute / 60, minute % 60, 0)?;
        match tz.from_local_datetime(&local) {
            LocalResult::Single(value) => Some(value.with_timezone(&Utc)),
            LocalResult::Ambiguous(first, second) => Some(first.min(second).with_timezone(&Utc)),
            LocalResult::None => None,
        }
    })
}
