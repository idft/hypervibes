use std::sync::Arc;
use axum::{extract::{Path, State}, http::StatusCode, response::IntoResponse};
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
pub(in crate::web::routes) async fn model_catalog_logo(
    State(state): State<Arc<AppState>>,
    Path(provider): Path<String>,
) -> impl IntoResponse {
    let key = format!("{provider}.svg");
    if validate_key(&key).is_err() {
        return (
            StatusCode::BAD_REQUEST,
            [("Content-Type", "text/plain; charset=utf-8")],
            "invalid provider",
        )
            .into_response();
    }

    let body = match state
        .asset_cache
        .get_or_fetch(
            "models-dev/logos",
            &key,
            &format!("https://models.dev/logos/{provider}.svg"),
            AssetCachePolicy {
                ttl: std::time::Duration::from_secs(30 * 24 * 60 * 60),
                max_bytes: 256 * 1024,
                allowed_content_types: vec![
                    "image/svg+xml",
                    "text/plain",
                    "application/octet-stream",
                ],
            },
        )
        .await
    {
        Ok(asset) if body_looks_like_svg(&asset.bytes) => asset.bytes,
        Ok(_) | Err(_) => fallback_logo_svg(&provider).into_bytes(),
    };

    (
        StatusCode::OK,
        [
            ("Content-Type", "image/svg+xml; charset=utf-8"),
            ("Cache-Control", "public, max-age=86400"),
        ],
        body,
    )
        .into_response()
}
pub(in crate::web::routes) fn body_looks_like_svg(body: &[u8]) -> bool {
    let text = String::from_utf8_lossy(body);
    let trimmed = text.trim_start();
    trimmed.starts_with("<svg") || (trimmed.starts_with("<?xml") && trimmed.contains("<svg"))
}
pub(in crate::web::routes) fn fallback_logo_svg(provider: &str) -> String {
    let initial = provider.chars().next().unwrap_or('M').to_ascii_uppercase();
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32" role="img" aria-label="{provider}"><rect width="32" height="32" rx="8" fill="#18181b"/><text x="16" y="21" text-anchor="middle" font-family="ui-sans-serif,system-ui" font-size="14" fill="#fafafa">{initial}</text></svg>"##
    )
}
