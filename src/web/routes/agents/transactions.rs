use std::sync::Arc;
use axum::{extract::{Path, State}, response::Response};
use rust_decimal::Decimal;

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
use super::show::render_agent_show_page;
pub(in crate::web::routes) async fn agents_show_transactions(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    render_agent_show_page(
        &state,
        &agent_key,
        AgentShowTab::Transactions,
        None,
        None,
        None,
    )
    .await
}
pub(in crate::web::routes) fn apply_live_cash_balance_anchor(
    state: &Arc<AppState>,
    agent: &crate::agents::model::AgentDetailRow,
    rows: &mut [AccountTransactionRow],
) {
    let Some(latest_running_balance) = rows.first().and_then(|row| row.running_balance) else {
        return;
    };
    let account_key = AccountKey::new(&agent.wallet_address, &agent.environment);
    let Some(snapshot) = state.live_accounts.get(&account_key) else {
        return;
    };
    let Some(live_cash_balance) = live_cash_balance(&snapshot) else {
        return;
    };

    let adjustment = live_cash_balance - latest_running_balance;
    for row in rows {
        if let Some(running_balance) = row.running_balance {
            row.running_balance = Some(running_balance + adjustment);
        }
    }
}
pub(in crate::web::routes) fn live_cash_balance(state: &AccountLiveState) -> Option<Decimal> {
    let perps_account_value = state
        .margin
        .as_ref()
        .and_then(|margin| margin.account_value)
        .filter(|value| !value.is_sign_negative());
    let unrealized_pnl = state
        .open_positions
        .iter()
        .filter_map(|position| position.unrealized_pnl)
        .fold(Decimal::ZERO, |acc, value| acc + value);
    let perps_cash = perps_account_value.map(|value| value - unrealized_pnl);

    let spot_usdc = state
        .spot_balances
        .iter()
        .find(|balance| balance.coin.eq_ignore_ascii_case("USDC"));
    let spot_usdc_available = spot_usdc
        .and_then(|balance| balance.available)
        .filter(|value| !value.is_sign_negative());
    let spot_usdc_total = spot_usdc
        .and_then(|balance| balance.total)
        .filter(|value| !value.is_sign_negative());

    match (perps_cash, spot_usdc_available) {
        (Some(perps), Some(spot_available)) => Some(perps + spot_available),
        (Some(perps), None) => Some(perps),
        (None, Some(spot_available)) => Some(spot_available),
        (None, None) => spot_usdc_total,
    }
}
