use axum::{
    Form,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
};
use serde::Deserialize;
use std::sync::Arc;
use tracing::warn;

use super::shared::{WORKSPACE_MAINTENANCE_DUPLICATE_WARNING, urlencode};
use super::show::{AgentSettingsQuery, AgentShowQueries, render_agent_show_page};
use crate::{
    agentic::store::InsertWorkspaceMaintenanceTaskOutcome,
    agents::store::{get_agent, replace_agent_instruments},
    opencode::workspace::{
        OpenCodeWorkspaceAgent, OpenCodeWorkspaceRuntimeConfig, diff_agent_workspace_from_template,
    },
    web::{
        AppState,
        auth::AuthenticatedUser,
        error::AppError,
        templates::{
            AgentShowTab, OpenCodeWorkspaceMaintenanceStatusTemplate,
            OpenCodeWorkspaceMaintenanceStatusView, OpenCodeWorkspaceMaintenanceView,
            OpenCodeWorkspaceSectionTemplate, OpenCodeWorkspaceSettingsView,
        },
    },
};
#[derive(Debug, Default, Deserialize)]
pub(in crate::web::routes) struct RegenerateWorkspaceForm {
    #[serde(default)]
    pub hard_reset: Option<String>,
    #[serde(default)]
    pub reset_memories: Option<String>,
}
impl RegenerateWorkspaceForm {
    fn hard_reset(&self) -> bool {
        self.hard_reset.is_some()
    }

    fn reset_memories(&self) -> bool {
        self.hard_reset() && self.reset_memories.is_some()
    }
}
pub(in crate::web::routes) async fn agents_show_settings(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path(agent_key): Path<String>,
    Query(query): Query<AgentSettingsQuery>,
) -> Result<Response, AppError> {
    render_agent_show_page(
        &state,
        &user,
        &agent_key,
        AgentShowTab::Settings,
        AgentShowQueries {
            settings: Some(query),
            ..Default::default()
        },
    )
    .await
}
pub(in crate::web::routes) async fn agents_regenerate_workspace(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
    Form(form): Form<RegenerateWorkspaceForm>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let redirect_url = format!("/agents/{agent_key}/settings");
    match crate::agentic::store::insert_workspace_regenerate_task(
        &state.db_pool,
        &agent.agent_key,
        form.hard_reset(),
        form.reset_memories(),
    )
    .await?
    {
        InsertWorkspaceMaintenanceTaskOutcome::Inserted { .. } => {
            Ok(Redirect::to(&redirect_url).into_response())
        }
        InsertWorkspaceMaintenanceTaskOutcome::DuplicateActiveTask => Ok(Redirect::to(&format!(
            "{redirect_url}?workspace_warning={}",
            urlencode(WORKSPACE_MAINTENANCE_DUPLICATE_WARNING)
        ))
        .into_response()),
    }
}
pub(in crate::web::routes) async fn agents_workspace_maintenance_status(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let opencode_workspace = build_opencode_workspace_settings_view(&state, &agent).await;
    let html = OpenCodeWorkspaceSectionTemplate::render_view(opencode_workspace, None)?;
    Ok(Html(html).into_response())
}
pub(in crate::web::routes) async fn agents_update_instruments(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
    Form(form_pairs): Form<Vec<(String, String)>>,
) -> Result<Response, AppError> {
    let instrument_ids: Vec<String> = form_pairs
        .into_iter()
        .filter_map(|(key, value)| (key == "instrument_id").then_some(value))
        .collect();
    let updated = replace_agent_instruments(&state.db_pool, &agent_key, &instrument_ids).await?;

    if !updated {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    }

    Ok(Redirect::to(&format!("/agents/{agent_key}/settings")).into_response())
}
pub(in crate::web::routes) async fn load_workspace_maintenance_view(
    pool: &crate::db::DbPool,
    agent_key: &str,
    workspace_config: &crate::opencode::workspace::OpenCodeWorkspaceConfig,
) -> Result<OpenCodeWorkspaceMaintenanceView, AppError> {
    let task = crate::agentic::store::get_latest_maintenance_task(pool, agent_key).await?;
    let Some(task) = task else {
        return Ok(OpenCodeWorkspaceMaintenanceView::idle(agent_key));
    };
    let (report_summary, changed_paths) = if task.task_kind
        == crate::agentic::model::MAINTENANCE_TASK_KIND_ANALYSIS_CODING
    {
        let report =
            crate::opencode::coding_workspace::candidate_root(workspace_config, agent_key, task.id)
                .ok()
                .and_then(|root| {
                    root.parent()
                        .map(|parent| parent.join("coding-report.json"))
                })
                .and_then(|path| std::fs::read_to_string(path).ok())
                .and_then(|content| serde_json::from_str::<serde_json::Value>(&content).ok());
        (
            report
                .as_ref()
                .and_then(|value| value.get("summary"))
                .and_then(serde_json::Value::as_str)
                .map(ToString::to_string),
            report
                .as_ref()
                .and_then(|value| value.get("changed_paths"))
                .and_then(serde_json::Value::as_array)
                .map(|paths| {
                    paths
                        .iter()
                        .filter_map(|path| path.as_str().map(ToString::to_string))
                        .collect()
                })
                .unwrap_or_default(),
        )
    } else {
        (None, Vec::new())
    };
    let status = OpenCodeWorkspaceMaintenanceStatusView::from_task_with_report(
        task,
        report_summary,
        changed_paths,
    );
    Ok(OpenCodeWorkspaceMaintenanceView {
        poll_url: format!("/agents/{agent_key}/settings/workspace-maintenance-status"),
        should_poll: status.should_poll,
        status: Some(status),
    })
}
pub(in crate::web::routes) async fn build_opencode_workspace_settings_view(
    state: &Arc<AppState>,
    agent: &crate::agents::model::AgentDetailRow,
) -> Option<OpenCodeWorkspaceSettingsView> {
    let workspace_agent = OpenCodeWorkspaceAgent {
        agent_key: agent.agent_key.clone(),
        display_name: agent.display_name.clone(),
        api_key: agent.api_key.clone(),
    };
    let template_drift = diff_agent_workspace_from_template(
        &state.opencode_workspace_config,
        &workspace_agent,
    )
    .inspect_err(|error| {
        warn!(agent_key = %agent.agent_key, error = ?error, "failed to diff OpenCode workspace template for settings page");
    })
    .ok()
    .map(crate::web::templates::OpenCodeWorkspaceTemplateDriftView::from_diff)
    .unwrap_or_else(crate::web::templates::OpenCodeWorkspaceTemplateDriftView::unavailable);
    let maintenance = load_workspace_maintenance_view(
        &state.db_pool,
        &agent.agent_key,
        &state.opencode_workspace_config,
    )
        .await
        .inspect_err(|error| {
            warn!(agent_key = %agent.agent_key, error = ?error, "failed to load workspace maintenance state for settings page");
        })
        .unwrap_or_else(|_| OpenCodeWorkspaceMaintenanceView::idle(&agent.agent_key));
    let maintenance_html = OpenCodeWorkspaceMaintenanceStatusTemplate::render_view(
        maintenance.clone(),
    )
    .inspect_err(|error| {
        warn!(agent_key = %agent.agent_key, error = ?error, "failed to render workspace maintenance status partial");
    })
    .unwrap_or_default();

    OpenCodeWorkspaceRuntimeConfig::from_value(&agent.runtime_config).map(|_| {
        OpenCodeWorkspaceSettingsView {
            template_drift,
            maintenance_html,
            maintenance,
        }
    })
}
