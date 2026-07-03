use std::sync::Arc;

use askama::Template;
use axum::{
    Form,
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
};

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
use super::shared::unique_violation_message;
pub(in crate::web::routes) async fn backends_index(State(state): State<Arc<AppState>>) -> Result<Html<String>, AppError> {
    let template = BackendsPageTemplate {
        runtimes: list_agent_runtimes(&state.db_pool).await?,
        current_path: "/backends".to_string(),
    };
    Ok(Html(template.render()?))
}
pub(in crate::web::routes) async fn backends_new() -> Result<Html<String>, AppError> {
    let template = BackendsNewPageTemplate {
        form: CreateAgentRuntimeForm {
            backend_kind: crate::agents::model::BACKEND_KIND_OPENCODE.to_string(),
            enabled: Some("on".to_string()),
            ..Default::default()
        },
        errors: Vec::new(),
        current_path: "/backends/new".to_string(),
    };
    Ok(Html(template.render()?))
}
pub(in crate::web::routes) async fn create_backend(
    State(state): State<Arc<AppState>>,
    Form(form): Form<CreateAgentRuntimeForm>,
) -> Result<Response, AppError> {
    if let Err(errors) = form.validate() {
        return Ok(render_backend_form(form, errors));
    }

    if let Err(error) = insert_agent_runtime(&state.db_pool, &form).await {
        let errors = match unique_violation_message(&error) {
            Some(message) => vec![message],
            None => return Err(AppError(error)),
        };
        return Ok(render_backend_form(form, errors));
    }

    Ok(Redirect::to("/backends").into_response())
}
pub(in crate::web::routes) fn render_backend_form(form: CreateAgentRuntimeForm, errors: Vec<String>) -> Response {
    let template = BackendsNewPageTemplate {
        form,
        errors,
        current_path: "/backends/new".to_string(),
    };
    match template.render() {
        Ok(body) => (StatusCode::UNPROCESSABLE_ENTITY, Html(body)).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("template error: {error}"),
        )
            .into_response(),
    }
}
