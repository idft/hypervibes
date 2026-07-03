use std::sync::Arc;

use askama::Template;
use axum::{
    Form,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
};
use chrono::Utc;
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
use super::shared::{ModelPickerContext, load_model_picker_context, build_model_picker_view, validate_model_selection_for_agent, ModelSelectionForm, TimeoutForm, TimeoutErrorQuery, timeout_error_redirect, jobs_warning_redirect, parse_positive_schedule_seconds, WORKSPACE_MAINTENANCE_ACTIVE_WARNING};
pub(in crate::web::routes) async fn agents_show_hook_detail(
    State(state): State<Arc<AppState>>,
    Path((agent_key, hook_id)): Path<(String, i64)>,
    Query(query): Query<TimeoutErrorQuery>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    if agent.backend_kind != BACKEND_KIND_OPENCODE {
        return Ok((
            StatusCode::NOT_FOUND,
            "jobs are only available for OpenCode agents",
        )
            .into_response());
    }

    let Some(hook) =
        crate::agentic::store::get_agent_hook(&state.db_pool, &agent_key, hook_id).await?
    else {
        return Ok((StatusCode::NOT_FOUND, "hook not found").into_response());
    };

    const RUNS_LIMIT: i64 = 50;
    let mut hook_runs_loaded = false;
    let hook_runs = match crate::agentic::store::list_hook_runs(
        &state.db_pool,
        &agent_key,
        hook_id,
        RUNS_LIMIT,
    )
    .await
    {
        Ok(rows) => {
            hook_runs_loaded = true;
            rows.iter()
                .map(crate::web::templates::AgenticRunView::from_row)
                .collect()
        }
        Err(error) => {
            warn!(
                agent_key = %agent.agent_key,
                hook_id,
                error = ?error,
                "failed to list hook runs for operator page"
            );
            Vec::new()
        }
    };

    let mut hook_view = crate::web::templates::AgenticHookDetailView::from_row(&hook);
    if let Some(error) = query.timeout_error {
        hook_view.timeout_editor.error = Some(error);
    }
    match build_hook_prompt_preview(&state, &agent_key, hook_id).await {
        Ok(text) => hook_view.prompt_preview_text = text,
        Err(error) => {
            warn!(
                agent_key = %agent.agent_key,
                hook_id,
                error = ?error,
                "failed to build prompt preview for hook detail page"
            );
            hook_view.prompt_preview_error = Some(format!("{error:#}"));
        }
    }

    let picker = load_model_picker_context(&state, &agent).await;
    let model_picker =
        build_model_picker_view("hook-model-selection", &hook_view.model_selection, picker);
    let html = AgentHookDetailPageTemplate::render_view(
        agent.clone(),
        hook_view,
        model_picker,
        hook_runs,
        hook_runs_loaded,
    )?;
    Ok(Html(html).into_response())
}
pub(in crate::web::routes) async fn build_hook_prompt_preview(
    state: &Arc<AppState>,
    agent_key: &str,
    hook_id: i64,
) -> anyhow::Result<String> {
    let hook =
        crate::agentic::store::get_opencode_hook_for_dispatch(&state.db_pool, agent_key, hook_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("hook dispatch metadata unavailable"))?;

    let request = build_hook_dispatch_request(&state.db_pool, &hook, 0, Utc::now())
        .await?
        .ok_or_else(|| {
            anyhow::anyhow!("prompt preview unavailable: no currencies selected for agent")
        })?;

    crate::agentic::prompt::build_prompt(&request)
}
#[derive(Debug, Clone, Default, Deserialize)]
pub(in crate::web::routes) struct CreateAgentHookForm {
    #[serde(default)]
    pub timeout_seconds: String,
    #[serde(default)]
    pub model_selection: String,
    #[serde(default)]
    pub operator_prompt: String,
    pub enabled: Option<String>,
}
#[derive(Debug)]
pub(in crate::web::routes) struct ValidatedCreateAgentHook {
    pub timeout_seconds: i32,
    pub model_selection: Option<(String, String)>,
    pub operator_prompt: String,
    pub enabled: bool,
}
impl CreateAgentHookForm {
    fn defaults() -> Self {
        Self {
            timeout_seconds: "900".to_string(),
            enabled: Some("on".to_string()),
            ..Self::default()
        }
    }

    fn enabled(&self) -> bool {
        self.enabled.is_some()
    }

    fn as_template_values(&self) -> CreateAgentHookFormValues {
        CreateAgentHookFormValues {
            timeout_seconds: self.timeout_seconds.clone(),
            model_selection: self.model_selection.clone(),
            operator_prompt: self.operator_prompt.clone(),
            enabled: self.enabled(),
        }
    }

    fn validate(&self) -> Result<ValidatedCreateAgentHook, Vec<String>> {
        let mut errors = Vec::new();
        let timeout_seconds =
            parse_positive_schedule_seconds(&self.timeout_seconds, "Timeout", &mut errors);
        let model_selection = match parse_model_selection(&self.model_selection) {
            Ok(selection) => selection,
            Err(error) => {
                errors.push(error);
                None
            }
        };

        if errors.is_empty() {
            Ok(ValidatedCreateAgentHook {
                timeout_seconds: timeout_seconds.expect("validated timeout seconds"),
                model_selection,
                operator_prompt: self.operator_prompt.trim().to_string(),
                enabled: self.enabled(),
            })
        } else {
            Err(errors)
        }
    }
}
pub(in crate::web::routes) async fn agents_new_hook(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    if agent.backend_kind != BACKEND_KIND_OPENCODE {
        return Ok((
            StatusCode::NOT_FOUND,
            "jobs are only available for OpenCode agents",
        )
            .into_response());
    }

    let picker = load_model_picker_context(&state, &agent).await;
    Ok(render_new_hook_form(
        agent,
        CreateAgentHookForm::defaults().as_template_values(),
        picker,
        Vec::new(),
        StatusCode::OK,
    ))
}
#[derive(Debug, Default, Deserialize)]
pub(in crate::web::routes) struct ToggleHookForm {
    pub enabled: Option<String>,
}
pub(in crate::web::routes) async fn agents_create_hook(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
    Form(form): Form<CreateAgentHookForm>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    if agent.backend_kind != BACKEND_KIND_OPENCODE {
        return Ok((
            StatusCode::NOT_FOUND,
            "jobs are only available for OpenCode agents",
        )
            .into_response());
    }

    let validated = match form.validate() {
        Ok(validated) => validated,
        Err(errors) => {
            let picker = load_model_picker_context(&state, &agent).await;
            return Ok(render_new_hook_form(
                agent,
                form.as_template_values(),
                picker,
                errors,
                StatusCode::UNPROCESSABLE_ENTITY,
            ));
        }
    };

    let validated_model_selection =
        match validate_model_selection_for_agent(&state, &agent, validated.model_selection.clone())
            .await
        {
            Ok(selection) => selection,
            Err(error) => {
                let picker = load_model_picker_context(&state, &agent).await;
                return Ok(render_new_hook_form(
                    agent,
                    form.as_template_values(),
                    picker,
                    vec![error],
                    StatusCode::UNPROCESSABLE_ENTITY,
                ));
            }
        };

    let model_provider_id = validated_model_selection
        .as_ref()
        .map(|(provider, _)| provider.as_str());
    let model_id = validated_model_selection
        .as_ref()
        .map(|(_, model)| model.as_str());
    if let Err(error) = crate::agentic::store::insert_agent_hook(
        &state.db_pool,
        &agent_key,
        JOB_KIND_MARKET_ANALYSIS,
        HOOK_EVENT_ANALYSIS_BATCH_COMPLETED,
        validated.enabled,
        model_provider_id,
        model_id,
        validated.timeout_seconds,
        &validated.operator_prompt,
    )
    .await
    {
        let errors = match hook_unique_violation_message(&error) {
            Some(message) => vec![message],
            None => return Err(AppError(error)),
        };
        let picker = load_model_picker_context(&state, &agent).await;
        return Ok(render_new_hook_form(
            agent,
            form.as_template_values(),
            picker,
            errors,
            StatusCode::UNPROCESSABLE_ENTITY,
        ));
    }

    Ok(Redirect::to(&format!("/agents/{agent_key}/jobs")).into_response())
}
pub(in crate::web::routes) async fn agents_run_hook_now(
    State(state): State<Arc<AppState>>,
    Path((agent_key, hook_id)): Path<(String, i64)>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    if agent.backend_kind != BACKEND_KIND_OPENCODE {
        return Ok((
            StatusCode::NOT_FOUND,
            "jobs are only available for OpenCode agents",
        )
            .into_response());
    }

    let Some(hook) =
        crate::agentic::store::get_opencode_hook_for_dispatch(&state.db_pool, &agent_key, hook_id)
            .await?
    else {
        return Ok((StatusCode::NOT_FOUND, "hook not found").into_response());
    };

    match crate::agentic::store::insert_queued_hook_run(&state.db_pool, &agent_key, hook_id).await?
    {
        QueuedHookRun::Dispatch {
            run_id,
            scheduled_for,
        } => match build_hook_dispatch_request(&state.db_pool, &hook, run_id, scheduled_for).await?
        {
            Some(request) => {
                let pool = state.db_pool.clone();
                let backend = state.agentic_backend.clone();
                tokio::spawn(async move {
                    let _ = dispatch_run(pool, backend, request).await;
                });
            }
            None => {
                let _ = crate::agentic::store::mark_run_failed(
                    &state.db_pool,
                    run_id,
                    "no currencies selected for agent; job skipped",
                    None,
                )
                .await;
            }
        },
        QueuedHookRun::Skipped { .. } => {}
        QueuedHookRun::Missing => {
            return Ok((StatusCode::NOT_FOUND, "hook not found").into_response());
        }
        QueuedHookRun::BlockedByMaintenance => {
            return Ok(jobs_warning_redirect(
                &agent_key,
                WORKSPACE_MAINTENANCE_ACTIVE_WARNING,
            ));
        }
    }

    Ok(Redirect::to(&format!("/agents/{agent_key}/jobs")).into_response())
}
pub(in crate::web::routes) async fn agents_toggle_hook(
    State(state): State<Arc<AppState>>,
    Path((agent_key, hook_id)): Path<(String, i64)>,
    Form(form): Form<ToggleHookForm>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    if agent.backend_kind != BACKEND_KIND_OPENCODE {
        return Ok((
            StatusCode::NOT_FOUND,
            "jobs are only available for OpenCode agents",
        )
            .into_response());
    }

    let enable = matches!(form.enabled.as_deref(), Some("on"));
    let updated =
        crate::agentic::store::set_hook_enabled(&state.db_pool, &agent_key, hook_id, enable)
            .await?;
    if !updated {
        return Ok((StatusCode::NOT_FOUND, "hook not found").into_response());
    }

    Ok(Redirect::to(&format!("/agents/{agent_key}/jobs")).into_response())
}
pub(in crate::web::routes) async fn agents_delete_hook(
    State(state): State<Arc<AppState>>,
    Path((agent_key, hook_id)): Path<(String, i64)>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    if agent.backend_kind != BACKEND_KIND_OPENCODE {
        return Ok((
            StatusCode::NOT_FOUND,
            "jobs are only available for OpenCode agents",
        )
            .into_response());
    }

    let deleted =
        crate::agentic::store::delete_agent_hook(&state.db_pool, &agent_key, hook_id).await?;
    if !deleted {
        return Ok((StatusCode::NOT_FOUND, "hook not found").into_response());
    }

    Ok(Redirect::to(&format!("/agents/{agent_key}/jobs")).into_response())
}
pub(in crate::web::routes) async fn agents_update_hook_model(
    State(state): State<Arc<AppState>>,
    Path((agent_key, hook_id)): Path<(String, i64)>,
    Form(form): Form<ModelSelectionForm>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let Some(_hook) =
        crate::agentic::store::get_agent_hook(&state.db_pool, &agent_key, hook_id).await?
    else {
        return Ok((StatusCode::NOT_FOUND, "hook not found").into_response());
    };

    let parsed = parse_model_selection(&form.model_selection)
        .map_err(|message| AppError(anyhow::anyhow!(message)))?;
    let validated = validate_model_selection_for_agent(&state, &agent, parsed)
        .await
        .map_err(|message| AppError(anyhow::anyhow!(message)))?;
    let model_provider_id = validated.as_ref().map(|(provider, _)| provider.as_str());
    let model_id = validated.as_ref().map(|(_, model)| model.as_str());

    if !crate::agentic::store::set_hook_model(
        &state.db_pool,
        &agent_key,
        hook_id,
        model_provider_id,
        model_id,
    )
    .await?
    {
        return Ok((StatusCode::NOT_FOUND, "hook not found").into_response());
    }

    Ok(Redirect::to(&format!("/agents/{agent_key}/hooks/{hook_id}")).into_response())
}
pub(in crate::web::routes) async fn agents_update_hook_timeout(
    State(state): State<Arc<AppState>>,
    Path((agent_key, hook_id)): Path<(String, i64)>,
    Form(form): Form<TimeoutForm>,
) -> Result<Response, AppError> {
    let detail_url = format!("/agents/{agent_key}/hooks/{hook_id}");

    if crate::agentic::store::get_agent_hook(&state.db_pool, &agent_key, hook_id)
        .await?
        .is_none()
    {
        return Ok((StatusCode::NOT_FOUND, "hook not found").into_response());
    }

    let timeout_seconds = match parse_timeout_seconds(&form.timeout) {
        Ok(value) => value,
        Err(error) => {
            return Ok(timeout_error_redirect(
                &detail_url,
                format!("Invalid timeout: {error}"),
            ));
        }
    };
    let timeout_i32 = match i32::try_from(timeout_seconds) {
        Ok(value) => value,
        Err(_) => {
            return Ok(timeout_error_redirect(
                &detail_url,
                format!(
                    "Invalid timeout: {timeout_seconds} seconds exceeds the maximum allowed value"
                ),
            ));
        }
    };

    if !crate::agentic::store::set_hook_timeout(&state.db_pool, &agent_key, hook_id, timeout_i32)
        .await?
    {
        return Ok((StatusCode::NOT_FOUND, "hook not found").into_response());
    }

    Ok(Redirect::to(&detail_url).into_response())
}
pub(in crate::web::routes) fn render_new_hook_form(
    agent: crate::agents::model::AgentDetailRow,
    form: CreateAgentHookFormValues,
    picker: ModelPickerContext,
    errors: Vec<String>,
    status: StatusCode,
) -> Response {
    let current_path = format!("/agents/{}/hooks/new", agent.agent_key);
    let model_picker =
        build_model_picker_view("hook-model-selection", &form.model_selection, picker);
    let template = AgentHookNewPageTemplate {
        agent,
        form,
        model_picker,
        errors,
        current_path,
    };
    match template.render() {
        Ok(body) => (status, Html(body)).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("template error: {error}"),
        )
            .into_response(),
    }
}
pub(in crate::web::routes) fn hook_unique_violation_message(error: &anyhow::Error) -> Option<String> {
    let db_err = error.downcast_ref::<sqlx::Error>()?.as_database_error()?;
    if !db_err.is_unique_violation() {
        return None;
    }

    let constraint = db_err.constraint().unwrap_or("unknown");
    if constraint.contains("agentic_job_hooks") || constraint.contains("job_kind") {
        Some("A market-analysis hook already exists for this agent.".to_string())
    } else {
        Some("This hook conflicts with an existing row.".to_string())
    }
}
