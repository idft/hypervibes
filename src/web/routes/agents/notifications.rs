use std::sync::Arc;

use super::show::{AgentShowQueries, render_agent_show_page};
use crate::{
    agents::store::get_agent,
    notifications::store::{delete_notification, delete_notifications},
    web::{AppState, auth::AuthenticatedUser, error::AppError, templates::AgentShowTab},
};
use axum::{
    Form,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
};

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

    if let Some(notification_id) = delete_id {
        if !delete_notification(&state.db_pool, &agent_key, notification_id).await? {
            return Ok((StatusCode::NOT_FOUND, "notification not found").into_response());
        }
    } else {
        delete_notifications(&state.db_pool, &agent_key, &selected_ids).await?;
    }

    Ok(Redirect::to(&format!("/agents/{agent_key}/notifications")).into_response())
}
