use std::sync::Arc;

use askama::Template;
use axum::{Form, extract::State, response::{Html, IntoResponse, Redirect, Response}};
use serde::Deserialize;

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
#[derive(Debug, Clone, Default, Deserialize)]
pub(in crate::web::routes) struct SettingsUpdateForm {
    #[serde(default)]
    pub system_prompt: String,
}
pub(in crate::web::routes) async fn settings_index(State(state): State<Arc<AppState>>) -> Result<Response, AppError> {
    let row = crate::settings::store::get_setting(&state.db_pool, "opencode_system_prompt").await?;
    let system_prompt = row
        .map(|r| r.value)
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| crate::agents::prompts::DEFAULT_SYSTEM_PROMPT.to_string());
    let html = SettingsPageTemplate {
        system_prompt,
        current_path: "/settings".to_string(),
    }
    .render()?;
    Ok(Html(html).into_response())
}
pub(in crate::web::routes) async fn settings_update(
    State(state): State<Arc<AppState>>,
    Form(form): Form<SettingsUpdateForm>,
) -> Result<Response, AppError> {
    crate::settings::store::upsert_setting(
        &state.db_pool,
        "opencode_system_prompt",
        &form.system_prompt,
        Some("Base system prompt prepended to every OpenCode agent job prompt."),
    )
    .await?;
    Ok(Redirect::to("/settings").into_response())
}
