use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use std::sync::Arc;

use crate::{
    agents::{model::BACKEND_KIND_OPENCODE, store::get_agent},
    web::{AppState, error::AppError, templates::AgentRunDetailPageTemplate},
};
pub(in crate::web::routes) async fn agents_show_run_detail(
    State(state): State<Arc<AppState>>,
    Path((agent_key, run_id)): Path<(String, i64)>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    if agent.backend_kind != BACKEND_KIND_OPENCODE {
        return Ok((
            StatusCode::NOT_FOUND,
            "run details are only available for OpenCode agents",
        )
            .into_response());
    }

    let Some(run) = crate::agentic::store::get_run(&state.db_pool, run_id).await? else {
        return Ok((StatusCode::NOT_FOUND, "run not found").into_response());
    };
    if run.agent_key != agent_key {
        return Ok((StatusCode::NOT_FOUND, "run not found").into_response());
    }

    let run_view = crate::web::templates::AgenticRunDetailView::from_row(&run);
    let session_lookup_attempted = !run_view.backend_run_ref.is_empty();
    let session = if session_lookup_attempted {
        crate::opencode::store::get_session_detail(&state.db_pool, &run_view.backend_run_ref)
            .await?
            .as_ref()
            .map(crate::web::templates::OpenCodeSessionView::from_detail)
    } else {
        None
    };

    let html = AgentRunDetailPageTemplate::render_view(
        agent.clone(),
        run_view,
        session,
        session_lookup_attempted,
    )?;
    Ok(Html(html).into_response())
}
