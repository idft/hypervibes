use std::sync::Arc;
use axum::{http::HeaderMap, response::{IntoResponse, Redirect, Response}};
use serde::Deserialize;
use tracing::warn;
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
pub(in crate::web::routes) const WORKSPACE_MAINTENANCE_ACTIVE_WARNING: &str = "Workspace maintenance is queued or running for this agent. Run now is unavailable until it completes.";
pub(in crate::web::routes) const WORKSPACE_MAINTENANCE_DUPLICATE_WARNING: &str =
    "A workspace maintenance task is already queued or running for this agent.";
#[derive(Debug, Clone)]
pub(in crate::web::routes) struct ModelPickerContext {
    pub options: Vec<ModelPickerOption>,
    pub warning: Option<String>,
}
pub(in crate::web::routes) fn is_htmx_request(headers: &HeaderMap) -> bool {
    headers
        .get("HX-Request")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}
#[derive(Debug, Default, Deserialize)]
pub(in crate::web::routes) struct ToggleScheduleForm {
    pub enabled: Option<String>,
}
#[derive(Debug, Default, Deserialize)]
pub(in crate::web::routes) struct TimeoutForm {
    #[serde(default)]
    pub timeout: String,
}
#[derive(Debug, Default, Deserialize)]
pub(in crate::web::routes) struct ModelSelectionForm {
    #[serde(default)]
    pub model_selection: String,
}
#[derive(Debug, Default, Deserialize)]
pub(in crate::web::routes) struct TimeoutErrorQuery {
    #[serde(default)]
    pub timeout_error: Option<String>,
}
pub(in crate::web::routes) fn timeout_error_redirect(detail_url: &str, message: String) -> Response {
    let encoded = urlencode(&message);
    Redirect::to(&format!("{detail_url}?timeout_error={encoded}")).into_response()
}
pub(in crate::web::routes) fn urlencode(value: &str) -> String {
    value
        .bytes()
        .flat_map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
                vec![byte as char].into_iter().collect::<Vec<_>>()
            } else {
                format!("%{byte:02X}").chars().collect::<Vec<_>>()
            }
            .into_iter()
        })
        .collect()
}
pub(in crate::web::routes) fn jobs_warning_redirect(agent_key: &str, message: &str) -> Response {
    Redirect::to(&format!(
        "/agents/{agent_key}/jobs?warning={}",
        urlencode(message)
    ))
    .into_response()
}
pub(in crate::web::routes) async fn load_model_picker_context(
    state: &Arc<AppState>,
    agent: &crate::agents::model::AgentDetailRow,
) -> ModelPickerContext {
    match build_model_picker_options(agent, &state.opencode_client, &state.model_catalog).await {
        Ok(options) => ModelPickerContext {
            options,
            warning: None,
        },
        Err(error) => {
            warn!(agent_key = %agent.agent_key, error = ?error, "failed to load model picker options");
            ModelPickerContext {
                options: Vec::new(),
                warning: Some(
                    "Could not load configured OpenCode models. You can still use OpenCode default."
                        .to_string(),
                ),
            }
        }
    }
}
pub(in crate::web::routes) fn selected_model_label(selected: &str, options: &[ModelPickerOption]) -> String {
    if selected.trim().is_empty() {
        return "OpenCode default".to_string();
    }

    options
        .iter()
        .find(|option| option.value == selected)
        .map(|option| format!("{} / {}", option.provider_name, option.model_name))
        .unwrap_or_else(|| selected.to_string())
}
pub(in crate::web::routes) fn build_model_picker_view(
    input_id: &str,
    selected_value: &str,
    picker: ModelPickerContext,
) -> ModelPickerView {
    ModelPickerView {
        input_id: input_id.to_string(),
        input_name: "model_selection".to_string(),
        selected_value: selected_value.to_string(),
        selected_label: selected_model_label(selected_value, &picker.options),
        options: picker.options,
        warning: picker.warning,
    }
}
pub(in crate::web::routes) async fn validate_model_selection_for_agent(
    state: &Arc<AppState>,
    agent: &crate::agents::model::AgentDetailRow,
    selection: Option<(String, String)>,
) -> Result<Option<(String, String)>, String> {
    let Some(selection) = selection else {
        return Ok(None);
    };

    let options = build_model_picker_options(agent, &state.opencode_client, &state.model_catalog)
        .await
        .map_err(|_| {
            "Could not load configured OpenCode models. Try again or use OpenCode default."
                .to_string()
        })?;
    if selection_exists_in_options(&options, &selection) {
        Ok(Some(selection))
    } else {
        Err("Select a valid model.".to_string())
    }
}
pub(in crate::web::routes) fn parse_positive_schedule_seconds(
    raw_value: &str,
    field_name: &str,
    errors: &mut Vec<String>,
) -> Option<i32> {
    match raw_value.trim().parse::<i32>() {
        Ok(value) if value > 0 => Some(value),
        _ => {
            errors.push(format!(
                "{field_name} must be a positive number of seconds."
            ));
            None
        }
    }
}
