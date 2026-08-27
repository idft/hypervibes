use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{
        Html, IntoResponse, Redirect, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use futures::StreamExt;
use futures::stream::unfold;
use std::{collections::VecDeque, convert::Infallible, sync::Arc};
use tokio::sync::broadcast;
use tracing::warn;

use super::show::load_selected_agent_navbar;
use crate::{
    agents::store::get_agent,
    notifications::store::count_notifications,
    web::{
        AppState,
        auth::AuthenticatedUser,
        error::AppError,
        run_detail_events::RunDetailDbEvent,
        templates::{
            AgentRunDetailPageTemplate, AgentRunDetailSummaryPartialTemplate,
            AgentRunDetailTranscriptPartialTemplate, HarnessSubAgentRunDetailView,
            OpenCodeSessionView,
        },
    },
};

struct RunDetailSnapshot {
    agent: crate::agents::model::AgentDetailRow,
    run: HarnessSubAgentRunDetailView,
    session: Option<OpenCodeSessionView>,
    session_lookup_attempted: bool,
}

async fn load_run_detail_snapshot(
    state: &AppState,
    agent_key: &str,
    run_id: i64,
) -> Result<Option<RunDetailSnapshot>, AppError> {
    let Some(agent) = get_agent(&state.db_pool, agent_key).await? else {
        return Ok(None);
    };
    let Some(run) = crate::harness::store::get_run(&state.db_pool, run_id).await? else {
        return Ok(None);
    };
    if run.agent_key != agent_key {
        return Ok(None);
    }

    let mut run = HarnessSubAgentRunDetailView::from_row(&run);
    let session_lookup_attempted = !run.backend_run_ref.is_empty();
    let session = if session_lookup_attempted {
        crate::opencode::store::get_session_detail(&state.db_pool, &run.backend_run_ref)
            .await?
            .as_ref()
            .map(OpenCodeSessionView::from_detail)
    } else {
        None
    };
    if let Some(session) = session.as_ref() {
        prefer_session_error(&mut run, session);
    }

    Ok(Some(RunDetailSnapshot {
        agent,
        run,
        session,
        session_lookup_attempted,
    }))
}

fn prefer_session_error(run: &mut HarnessSubAgentRunDetailView, session: &OpenCodeSessionView) {
    let Some(error) = session
        .session_errors
        .iter()
        .rev()
        .find(|error| !error.error_message.trim().is_empty())
    else {
        return;
    };
    run.error_summary = if error.error_type.trim().is_empty() {
        error.error_message.clone()
    } else {
        format!("{}: {}", error.error_type, error.error_message)
    };
}

fn render_run_detail_events(snapshot: &RunDetailSnapshot) -> Result<Vec<Event>, AppError> {
    let summary = AgentRunDetailSummaryPartialTemplate::render_view(
        snapshot.run.clone(),
        snapshot.session.clone(),
    )?;
    let transcript = AgentRunDetailTranscriptPartialTemplate::render_view(
        snapshot.run.clone(),
        snapshot.session.clone(),
        snapshot.session_lookup_attempted,
    )?;
    Ok(vec![
        Event::default().event("run-summary").data(summary),
        Event::default().event("run-transcript").data(transcript),
    ])
}

pub(in crate::web::routes) async fn agents_show_run_detail(
    State(state): State<Arc<AppState>>,
    Path((agent_key, run_id)): Path<(String, i64)>,
    user: AuthenticatedUser,
) -> Result<Response, AppError> {
    let Some(snapshot) = load_run_detail_snapshot(&state, &agent_key, run_id).await? else {
        return Ok((StatusCode::NOT_FOUND, "run not found").into_response());
    };

    let navbar = load_selected_agent_navbar(&state, user.id, &snapshot.agent)
        .await?
        .0;
    let notification_count = count_notifications(&state.db_pool, &snapshot.agent.agent_key).await?;
    let html = AgentRunDetailPageTemplate::render_view(
        snapshot.agent,
        snapshot.run,
        snapshot.session,
        snapshot.session_lookup_attempted,
        notification_count,
        navbar,
    )?;
    Ok(Html(html).into_response())
}

pub(in crate::web::routes) async fn agents_cancel_run(
    State(state): State<Arc<AppState>>,
    Path((agent_key, run_id)): Path<(String, i64)>,
) -> Result<Response, AppError> {
    let Some(run) = crate::harness::store::get_run(&state.db_pool, run_id).await? else {
        return Ok((StatusCode::NOT_FOUND, "run not found").into_response());
    };
    if run.agent_key != agent_key {
        return Ok((StatusCode::NOT_FOUND, "run not found").into_response());
    }
    if run.status != crate::harness::model::RUN_STATUS_RUNNING {
        return Ok((StatusCode::CONFLICT, "run is no longer running").into_response());
    }
    let Some(session_id) = run.backend_run_ref.as_deref() else {
        return Ok((
            StatusCode::CONFLICT,
            "run has not created an OpenCode session yet",
        )
            .into_response());
    };

    match state
        .harness_backend
        .abort_session(&state.opencode_base_url, session_id)
        .await
    {
        Ok(true) => {
            crate::harness::store::mark_run_aborted(
                &state.db_pool,
                run_id,
                "cancelled by operator",
                None,
            )
            .await?;
            Ok(Redirect::to(&format!("/agents/{agent_key}/runs/{run_id}")).into_response())
        }
        Ok(false) => {
            Ok((StatusCode::CONFLICT, "OpenCode could not cancel this run").into_response())
        }
        Err(error) => {
            warn!(agent_key, run_id, error = ?error, "failed to cancel OpenCode run");
            Ok((StatusCode::BAD_GATEWAY, "failed to cancel OpenCode run").into_response())
        }
    }
}

pub(in crate::web::routes) async fn agents_retry_run(
    State(state): State<Arc<AppState>>,
    Path((agent_key, run_id)): Path<(String, i64)>,
) -> Result<Response, AppError> {
    let Some(run) = crate::harness::store::get_run(&state.db_pool, run_id).await? else {
        return Ok((StatusCode::NOT_FOUND, "run not found").into_response());
    };
    if run.agent_key != agent_key {
        return Ok((StatusCode::NOT_FOUND, "run not found").into_response());
    }
    if run.status != crate::harness::model::RUN_STATUS_FAILED {
        return Ok((StatusCode::CONFLICT, "only failed runs can be retried").into_response());
    }

    let queued = if run.timeframe.is_some() {
        crate::harness::store::insert_queued_manual_run(
            &state.db_pool,
            &agent_key,
            run.sub_agent_id,
        )
        .await?
    } else {
        crate::harness::store::insert_queued_event_run(&state.db_pool, &agent_key, run.sub_agent_id)
            .await?
    };

    match queued {
        crate::harness::store::QueuedSubAgentRun::Dispatch { run_id, .. }
        | crate::harness::store::QueuedSubAgentRun::Skipped { run_id } => {
            Ok(Redirect::to(&format!("/agents/{agent_key}/runs/{run_id}")).into_response())
        }
        crate::harness::store::QueuedSubAgentRun::Missing => {
            Ok((StatusCode::NOT_FOUND, "sub-agent not found").into_response())
        }
        crate::harness::store::QueuedSubAgentRun::BlockedByMaintenance => Ok((
            StatusCode::CONFLICT,
            "workspace maintenance is active; retry is unavailable",
        )
            .into_response()),
    }
}

pub(in crate::web::routes) async fn agent_run_detail_stream(
    State(state): State<Arc<AppState>>,
    Path((agent_key, run_id)): Path<(String, i64)>,
) -> Result<Response, AppError> {
    let receiver = state.run_detail_events.subscribe();
    let Some(snapshot) = load_run_detail_snapshot(&state, &agent_key, run_id).await? else {
        return Ok((StatusCode::NOT_FOUND, "run not found").into_response());
    };
    let initial_events = render_run_detail_events(&snapshot)?;
    let shutdown_rx = state.shutdown_rx.clone();
    let stream_state = RunDetailStreamState {
        state,
        agent_key,
        run_id,
        backend_run_ref: snapshot.run.backend_run_ref.clone(),
        receiver,
        shutdown_rx,
        pending: VecDeque::new(),
    };
    let updates = unfold(stream_state, next_run_detail_event);
    let stream =
        tokio_stream::iter(initial_events.into_iter().map(Ok::<Event, Infallible>)).chain(updates);
    Ok(Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(15)))
        .into_response())
}

struct RunDetailStreamState {
    state: Arc<AppState>,
    agent_key: String,
    run_id: i64,
    backend_run_ref: String,
    receiver: broadcast::Receiver<RunDetailDbEvent>,
    shutdown_rx: tokio::sync::watch::Receiver<bool>,
    pending: VecDeque<Event>,
}

async fn next_run_detail_event(
    mut stream_state: RunDetailStreamState,
) -> Option<(Result<Event, Infallible>, RunDetailStreamState)> {
    loop {
        if let Some(event) = stream_state.pending.pop_front() {
            return Some((Ok(event), stream_state));
        }

        let notification = tokio::select! {
            result = stream_state.receiver.recv() => result,
            changed = stream_state.shutdown_rx.changed() => {
                if changed.is_err() || *stream_state.shutdown_rx.borrow() {
                    return None;
                }
                continue;
            }
        };
        let event = match notification {
            Ok(event) => event,
            Err(broadcast::error::RecvError::Lagged(_)) => RunDetailDbEvent::Resync,
            Err(broadcast::error::RecvError::Closed) => return None,
        };

        let matches = match &event {
            RunDetailDbEvent::RunChanged { run_id } => *run_id == stream_state.run_id,
            RunDetailDbEvent::SessionChanged { session_id } => {
                session_id == &stream_state.backend_run_ref
            }
            RunDetailDbEvent::ConversationChanged { .. } => false,
            RunDetailDbEvent::Resync => true,
        };
        if !matches {
            continue;
        }

        let snapshot = match load_run_detail_snapshot(
            &stream_state.state,
            &stream_state.agent_key,
            stream_state.run_id,
        )
        .await
        {
            Ok(Some(snapshot)) => snapshot,
            Ok(None) => return None,
            Err(error) => {
                warn!(
                    agent_key = %stream_state.agent_key,
                    run_id = stream_state.run_id,
                    error = ?error,
                    "failed to refresh run-detail SSE snapshot"
                );
                continue;
            }
        };
        stream_state.backend_run_ref = snapshot.run.backend_run_ref.clone();
        match render_run_detail_events(&snapshot) {
            Ok(events) => stream_state.pending.extend(events),
            Err(error) => warn!(
                agent_key = %stream_state.agent_key,
                run_id = stream_state.run_id,
                error = ?error,
                "failed to render run-detail SSE snapshot"
            ),
        }
    }
}
