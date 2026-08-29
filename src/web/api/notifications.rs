use std::sync::Arc;

use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use tracing::info;

use crate::{
    agents::AuthenticatedAgent,
    harness::model::RunApiScope,
    notifications::model::{CreateNotification, NotificationProvenance, NotificationResponse},
    web::{AppState, ui_events::UiEvent},
};

use super::error::ApiError;

/// `POST /api/v1/notifications`
///
/// Queues a notification for the agent's configured gateway. The gateway
/// service observes the `notification_created` pg_notify event and sends the
/// message to the bound Telegram chat.
pub(super) async fn create(
    State(state): State<Arc<AppState>>,
    agent: AuthenticatedAgent,
    Json(payload): Json<CreateNotification>,
) -> Result<Response, ApiError> {
    super::require_run_api_scope(&agent, RunApiScope::NotificationSend)?;
    payload.validate().map_err(ApiError::Validation)?;
    let severity = payload.severity();
    let record = match agent.run_provenance() {
        Some((run_id, capability_schema_version)) => {
            crate::notifications::store::create_notification_with_provenance(
                &state.db_pool,
                &agent.agent_key,
                &payload.title,
                &payload.body,
                severity,
                NotificationProvenance::Run {
                    run_id,
                    capability_schema_version,
                },
            )
            .await
        }
        None => {
            crate::notifications::store::create_notification(
                &state.db_pool,
                &agent.agent_key,
                &payload.title,
                &payload.body,
                severity,
            )
            .await
        }
    }
    .map_err(ApiError::Internal)?;
    state.ui_events.publish(UiEvent::NotificationQueued {
        agent_key: record.agent_key.clone(),
        notification_id: record.id,
    });
    info!(
        agent_key = %record.agent_key,
        notification_id = %record.id,
        severity = %severity.as_str(),
        "queued notification"
    );
    Ok((
        StatusCode::CREATED,
        Json(NotificationResponse { id: record.id }),
    )
        .into_response())
}
