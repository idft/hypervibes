use std::{convert::Infallible, sync::Arc};

use super::show::{AgentShowQueries, render_agent_show_page};
use crate::{
    agents::store::get_agent,
    notifications::store::{count_notifications, delete_notification, delete_notifications},
    web::{
        AppState,
        auth::AuthenticatedUser,
        error::AppError,
        templates::{AgentNotificationCountPartialTemplate, AgentShowTab},
        ui_events::UiEvent,
    },
};
use axum::{
    Form,
    extract::{Path, State},
    http::StatusCode,
    response::{
        IntoResponse, Redirect, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use futures::StreamExt;
use tokio_stream::wrappers::BroadcastStream;
use tracing::warn;

pub(in crate::web::routes) async fn agents_show_notifications(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    render_agent_show_page(
        &state,
        &user,
        &agent_key,
        AgentShowTab::Notifications,
        AgentShowQueries::default(),
    )
    .await
}

pub(in crate::web::routes) async fn agent_notification_count_stream(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    // Subscribe before loading the initial count so a change during setup is
    // queued and rendered immediately after the initial snapshot.
    let receiver = state.ui_events.subscribe();
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let initial = render_notification_count_event(&state.db_pool, &agent.agent_key).await?;
    let db_pool = state.db_pool.clone();
    let agent_key = agent.agent_key;
    let event_agent_key = agent_key.clone();

    let updates = BroadcastStream::new(receiver)
        .filter_map(move |item| {
            let agent_key = event_agent_key.clone();
            async move {
                match item {
                    Ok(UiEvent::NotificationQueued {
                        agent_key: event_agent_key,
                        ..
                    })
                    | Ok(UiEvent::NotificationsDeleted {
                        agent_key: event_agent_key,
                    }) if event_agent_key == agent_key => Some(()),
                    Ok(_) => None,
                    Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(_)) => {
                        Some(())
                    }
                }
            }
        })
        .filter_map(move |_| {
            let db_pool = db_pool.clone();
            let agent_key = agent_key.clone();
            async move {
                match render_notification_count_event(&db_pool, &agent_key).await {
                    Ok(event) => Some(Ok::<Event, Infallible>(event)),
                    Err(error) => {
                        warn!(agent_key = %agent_key, error = ?error, "failed to render notification-count SSE update");
                        None
                    }
                }
            }
        });

    let stream = tokio_stream::iter(vec![Ok::<Event, Infallible>(initial)]).chain(updates);
    Ok(Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(15)))
        .into_response())
}

async fn render_notification_count_event(
    pool: &crate::db::DbPool,
    agent_key: &str,
) -> Result<Event, AppError> {
    let notification_count = count_notifications(pool, agent_key).await?;
    let html = AgentNotificationCountPartialTemplate::render_view(notification_count)?;
    Ok(Event::default().event("notification-count").data(html))
}

pub(in crate::web::routes) async fn agents_delete_notifications(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
    Form(form_pairs): Form<Vec<(String, String)>>,
) -> Result<Response, AppError> {
    let Some(_agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };

    let mut selected_ids = Vec::new();
    let mut delete_id = None;
    for (field, value) in form_pairs {
        match field.as_str() {
            "notification_id" => match value.parse::<uuid::Uuid>() {
                Ok(notification_id) => selected_ids.push(notification_id),
                Err(_) => {
                    return Ok((StatusCode::BAD_REQUEST, "invalid notification id").into_response());
                }
            },
            "delete_id" => match value.parse::<uuid::Uuid>() {
                Ok(notification_id) if delete_id.replace(notification_id).is_none() => {}
                Ok(_) => {
                    return Ok(
                        (StatusCode::BAD_REQUEST, "multiple delete notification ids")
                            .into_response(),
                    );
                }
                Err(_) => {
                    return Ok((StatusCode::BAD_REQUEST, "invalid notification id").into_response());
                }
            },
            _ => {}
        }
    }

    let notifications_deleted = if let Some(notification_id) = delete_id {
        if !delete_notification(&state.db_pool, &agent_key, notification_id).await? {
            return Ok((StatusCode::NOT_FOUND, "notification not found").into_response());
        }
        true
    } else {
        delete_notifications(&state.db_pool, &agent_key, &selected_ids).await? > 0
    };

    if notifications_deleted {
        state.ui_events.publish(UiEvent::NotificationsDeleted {
            agent_key: agent_key.clone(),
        });
    }

    Ok(Redirect::to(&format!("/agents/{agent_key}/notifications")).into_response())
}
