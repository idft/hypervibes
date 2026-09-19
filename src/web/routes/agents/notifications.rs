use std::sync::Arc;

use super::show::{AgentShowQueries, load_selected_agent_navbar, render_agent_show_page};
use crate::{
    agents::store::get_agent,
    notifications::store::{
        count_notifications, delete_notification, delete_notifications, get_notification,
    },
    web::{
        AppState,
        auth::AuthenticatedUser,
        error::AppError,
        templates::{
            AgentNotificationCountPartialTemplate, AgentNotificationDetailPageTemplate,
            AgentNotificationDetailView, AgentShowTab,
        },
        ui_events::UiEvent,
    },
};
use axum::{
    Form,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
};
use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize)]
pub(in crate::web::routes) struct AgentNotificationsQuery {
    #[serde(default)]
    pub page: String,
}

pub(in crate::web::routes) async fn agents_show_notifications(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path(agent_key): Path<String>,
    Query(query): Query<AgentNotificationsQuery>,
) -> Result<Response, AppError> {
    render_agent_show_page(
        &state,
        &user,
        &agent_key,
        AgentShowTab::Notifications,
        AgentShowQueries {
            notifications: Some(query),
            ..Default::default()
        },
    )
    .await
}

pub(in crate::web::routes) async fn agents_show_notification_detail(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path((agent_key, notification_id)): Path<(String, uuid::Uuid)>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let Some(notification) =
        get_notification(&state.db_pool, &agent.agent_key, notification_id).await?
    else {
        return Ok((StatusCode::NOT_FOUND, "notification not found").into_response());
    };

    let navbar = load_selected_agent_navbar(&state, user.id, &agent).await?;
    let notification_count = count_notifications(&state.db_pool, &agent.agent_key).await?;
    let html = AgentNotificationDetailPageTemplate::render_view(
        agent.clone(),
        AgentNotificationDetailView::from_row(&agent.agent_key, notification),
        notification_count,
        navbar,
    )?;
    Ok(Html(html).into_response())
}

pub(in crate::web::routes) async fn agent_notification_count(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let notification_count = count_notifications(&state.db_pool, &agent.agent_key).await?;
    let html = AgentNotificationCountPartialTemplate::render_view(notification_count)?;
    Ok(Html(html).into_response())
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
