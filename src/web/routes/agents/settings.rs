use std::sync::Arc;
use axum::{
    Form,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
};
use serde::Deserialize;
use tracing::warn;

use crate::web::error::AppError;
use crate::{
    agentic::{model::{HOOK_EVENT_ANALYSIS_BATCH_COMPLETED, JOB_KIND_ANALYSIS, JOB_KIND_MARKET_ANALYSIS, JOB_KIND_TRADING}, scheduler::{build_hook_dispatch_request, dispatch_analysis_batch_completed_hook, dispatch_request_from_schedule, dispatch_run}, store::{InsertWorkspaceMaintenanceTaskOutcome, QueuedHookRun, QueuedScheduleRun}, timeframe::{parse_timeframe_seconds, parse_timeout_seconds}},
    agents::{crypto::{encrypt, generate_api_key}, keys::derive_wallet_address, model::{AgentRegistryRow, AgentRuntimeRow, BACKEND_KIND_OPENCODE, CreateAgentForm, CreateAgentRuntimeForm, slugify_agent_key}, prompts::{DEFAULT_ANALYSIS_STRATEGY_PROMPT, DEFAULT_TRADING_STRATEGY_PROMPT}, store::{delete_agent as delete_agent_in_store, get_agent, insert_agent, insert_agent_runtime, list_agent_instrument_options, list_agent_runtimes, list_agents, list_enabled_agent_runtimes, replace_agent_instruments, update_agent_analysis_prompt, update_agent_runtime_config, update_agent_trading_prompt}},
    cache::asset::{AssetCachePolicy, validate_key},
    hyperliquid::{live_state::{AccountKey, AccountLiveState, LiveConnectionStatus, live_agent_snapshot_for_dispatch}, queries::{AccountTransactionRow, BalanceSeriesBucket, fetch_balance_series, list_account_sync_state, list_all_account_transactions}},
    memory::{get_latest_agent_memory_by_type, get_memory as get_memory_record, list_agent_memories, memory_expires_at},
    model_catalog::options::{ModelPickerOption, build_model_picker_options, parse_model_selection, selection_exists_in_options},
    opencode::workspace::{OpenCodeWorkspaceAgent, OpenCodeWorkspaceRuntimeConfig, WorkspaceGenerationMode, agent_workspace_host_path, delete_agent_workspace, diff_agent_workspace_from_template, generate_agent_workspace, runtime_config_for_generated_workspace},
    web::{AppState, templates::{AccountBalancePartialTemplate, AccountBalanceView, AgentHookDetailPageTemplate, AgentHookNewPageTemplate, AgentJobDetailPageTemplate, AgentListEntry, AgentMemoryDetailPageTemplate, AgentMemoryDetailPartialTemplate, AgentMemoryTimelinePartialTemplate, AgentRunDetailPageTemplate, AgentScheduleNewPageTemplate, AgentShowTab, AgentsNewPageTemplate, AgentsPageTemplate, AgentsShowPageTemplate, BackendsNewPageTemplate, BackendsPageTemplate, BalanceSparklinesPartialTemplate, CreateAgentHookFormValues, CreateAgentScheduleFormValues, LatestAnalysisSummaryPartialTemplate, LatestTradeExecutionSummaryPartialTemplate, MemoryView, ModelPickerView, OpenCodeWorkspaceMaintenanceStatusTemplate, OpenCodeWorkspaceMaintenanceView, OpenCodeWorkspaceSectionTemplate, OpenCodeWorkspaceSettingsView, OpenOrdersPartialTemplate, OpenOrdersView, OpenPositionsPartialTemplate, OpenPositionsView, ServerErrorPageTemplate, SettingsPageTemplate, SparklineView, SyncStateView, TransactionView}, ui_events::UiEvent},
};
use super::show::{render_agent_show_page, AgentSettingsQuery};
use super::shared::{WORKSPACE_MAINTENANCE_DUPLICATE_WARNING, urlencode};
#[derive(Debug, Default, Deserialize)]
pub(in crate::web::routes) struct RegenerateWorkspaceForm {
    #[serde(default)]
    pub hard_reset: Option<String>,
}
impl RegenerateWorkspaceForm {
    fn hard_reset(&self) -> bool {
        self.hard_reset.is_some()
    }
}
pub(in crate::web::routes) async fn agents_show_settings(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
    Query(query): Query<AgentSettingsQuery>,
) -> Result<Response, AppError> {
    render_agent_show_page(
        &state,
        &agent_key,
        AgentShowTab::Settings,
        None,
        Some(query),
        None,
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
    if agent.backend_kind != BACKEND_KIND_OPENCODE {
        return Ok((
            StatusCode::NOT_FOUND,
            "workspace regeneration is only available for OpenCode agents",
        )
            .into_response());
    }

    let redirect_url = format!("/agents/{agent_key}/settings");
    match crate::agentic::store::insert_workspace_regenerate_task(
        &state.db_pool,
        &agent.agent_key,
        form.hard_reset(),
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
    if agent.backend_kind != BACKEND_KIND_OPENCODE {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    }

    let opencode_workspace = build_opencode_workspace_settings_view(&state, &agent).await;
    let html = OpenCodeWorkspaceSectionTemplate::render_view(agent, opencode_workspace, None)?;
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
) -> Result<OpenCodeWorkspaceMaintenanceView, AppError> {
    let task = crate::agentic::store::get_latest_workspace_regenerate_task(pool, agent_key).await?;
    Ok(task
        .map(|task| OpenCodeWorkspaceMaintenanceView::from_task(agent_key, task))
        .unwrap_or_else(|| OpenCodeWorkspaceMaintenanceView::idle(agent_key)))
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
    let workspace_host_path = agent_workspace_host_path(
        &state.opencode_workspace_config,
        &agent.agent_key,
    )
    .inspect_err(|error| {
        warn!(agent_key = %agent.agent_key, error = ?error, "failed to derive OpenCode workspace path for settings page");
    })
    .ok();
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
    let maintenance = load_workspace_maintenance_view(&state.db_pool, &agent.agent_key)
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

    OpenCodeWorkspaceRuntimeConfig::from_value(&agent.runtime_config).map(|workspace| {
        OpenCodeWorkspaceSettingsView {
            env_exists: workspace_host_path
                .as_deref()
                .map(|path| path.join(".env").is_file())
                .unwrap_or(false),
            workspace_host_path: workspace.workspace_host_path,
            workspace_container_path: workspace.workspace_container_path,
            profile_source: workspace.profile_source,
            template_drift,
            maintenance_html,
            maintenance,
        }
    })
}
