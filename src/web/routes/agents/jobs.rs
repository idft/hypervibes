use std::sync::Arc;

use askama::Template;
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
use super::show::{render_agent_show_page, AgentJobsQuery};
use super::shared::{ModelPickerContext, load_model_picker_context, build_model_picker_view, validate_model_selection_for_agent, ModelSelectionForm, TimeoutForm, TimeoutErrorQuery, timeout_error_redirect, jobs_warning_redirect, parse_positive_schedule_seconds, WORKSPACE_MAINTENANCE_ACTIVE_WARNING, ToggleScheduleForm};
pub(in crate::web::routes) async fn agents_show_jobs(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
    Query(query): Query<AgentJobsQuery>,
) -> Result<Response, AppError> {
    render_agent_show_page(
        &state,
        &agent_key,
        AgentShowTab::Jobs,
        None,
        None,
        Some(query),
    )
    .await
}
pub(in crate::web::routes) async fn agents_show_job_detail(
    State(state): State<Arc<AppState>>,
    Path((agent_key, job_id)): Path<(String, i64)>,
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

    let Some(job) =
        crate::agentic::store::get_agent_schedule(&state.db_pool, &agent_key, job_id).await?
    else {
        return Ok((StatusCode::NOT_FOUND, "job not found").into_response());
    };

    const RUNS_LIMIT: i64 = 50;
    let mut job_runs_loaded = false;
    let job_runs = match crate::agentic::store::list_schedule_runs(
        &state.db_pool,
        &agent_key,
        job_id,
        RUNS_LIMIT,
    )
    .await
    {
        Ok(rows) => {
            job_runs_loaded = true;
            rows.iter()
                .map(crate::web::templates::AgenticRunView::from_row)
                .collect()
        }
        Err(error) => {
            warn!(
                agent_key = %agent.agent_key,
                job_id,
                error = ?error,
                "failed to list job runs for operator page"
            );
            Vec::new()
        }
    };

    let mut job_view = crate::web::templates::AgenticJobDetailView::from_row(&job);
    if let Some(error) = query.timeout_error {
        job_view.timeout_editor.error = Some(error);
    }
    match build_job_prompt_preview(&state, &agent, &job).await {
        Ok(text) => job_view.prompt_preview_text = text,
        Err(error) => {
            warn!(
                agent_key = %agent.agent_key,
                job_id,
                error = ?error,
                "failed to build prompt preview for job detail page"
            );
            job_view.prompt_preview_error = Some(format!("{error:#}"));
        }
    }

    let picker = load_model_picker_context(&state, &agent).await;
    let model_picker =
        build_model_picker_view("job-model-selection", &job_view.model_selection, picker);
    let html = AgentJobDetailPageTemplate::render_view(
        agent.clone(),
        job_view,
        model_picker,
        job_runs,
        job_runs_loaded,
    )?;
    Ok(Html(html).into_response())
}
pub(in crate::web::routes) async fn build_job_prompt_preview(
    state: &Arc<AppState>,
    agent: &crate::agents::model::AgentDetailRow,
    job: &crate::agentic::model::AgenticJobScheduleRow,
) -> anyhow::Result<String> {
    use crate::agentic::backend::DispatchRequest;

    let selected_instruments =
        crate::agents::store::list_agent_instrument_ids(&state.db_pool, &agent.agent_key).await?;
    let system_setting =
        crate::settings::store::get_setting(&state.db_pool, "opencode_system_prompt").await?;
    let system_prompt = system_setting
        .map(|s| s.value)
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| crate::agents::prompts::DEFAULT_SYSTEM_PROMPT.to_string());

    let account_snapshot = if job.job_kind == crate::agentic::model::JOB_KIND_TRADING {
        Some(live_agent_snapshot_for_dispatch(
            &agent.wallet_address,
            &agent.environment,
            &state.live_accounts,
        ))
    } else {
        None
    };

    let request = DispatchRequest {
        run_id: 0,
        schedule_id: Some(job.id),
        hook_id: None,
        agent_key: agent.agent_key.clone(),
        display_name: agent.display_name.clone(),
        job_key: job.job_key.clone(),
        job_kind: job.job_kind.clone(),
        timeframe: Some(job.timeframe.clone()),
        operator_prompt: job.operator_prompt.clone(),
        analysis_prompt: agent.analysis_prompt.clone(),
        trading_prompt: agent.trading_prompt.clone(),
        system_prompt,
        environment: agent.environment.clone(),
        selected_instruments,
        account_snapshot,
        model_provider_id: job.model_provider_id.clone(),
        model_id: job.model_id.clone(),
        timeout_seconds: job.timeout_seconds,
        runtime_base_url: String::new(),
        runtime_config: serde_json::json!({}),
        scheduled_for: job.next_run_at,
    };

    crate::agentic::prompt::build_prompt(&request)
}
#[derive(Debug, Clone, Default, Deserialize)]
pub(in crate::web::routes) struct CreateAgentScheduleForm {
    #[serde(default)]
    pub job_kind: String,
    #[serde(default)]
    pub timeframe: String,
    #[serde(default)]
    pub timeout_seconds: String,
    #[serde(default)]
    pub model_selection: String,
    #[serde(default)]
    pub operator_prompt: String,
    pub enabled: Option<String>,
}
#[derive(Debug)]
pub(in crate::web::routes) struct ValidatedCreateAgentSchedule {
    pub job_kind: String,
    pub timeframe: String,
    pub trigger_delay_seconds: i32,
    pub timeout_seconds: i32,
    pub model_selection: Option<(String, String)>,
    pub operator_prompt: String,
    pub enabled: bool,
}
impl CreateAgentScheduleForm {
    fn defaults() -> Self {
        Self {
            job_kind: JOB_KIND_ANALYSIS.to_string(),
            timeframe: "15m".to_string(),
            timeout_seconds: "900".to_string(),
            enabled: Some("on".to_string()),
            ..Self::default()
        }
    }

    fn enabled(&self) -> bool {
        self.enabled.is_some()
    }

    fn as_template_values(&self) -> CreateAgentScheduleFormValues {
        CreateAgentScheduleFormValues {
            job_kind: self.job_kind.clone(),
            timeframe: self.timeframe.clone(),
            timeout_seconds: self.timeout_seconds.clone(),
            model_selection: self.model_selection.clone(),
            operator_prompt: self.operator_prompt.clone(),
            enabled: self.enabled(),
        }
    }

    fn validate(&self) -> Result<ValidatedCreateAgentSchedule, Vec<String>> {
        let mut errors = Vec::new();

        let job_kind = self.job_kind.trim();
        if !matches!(job_kind, JOB_KIND_ANALYSIS | JOB_KIND_TRADING) {
            errors.push("Job kind must be analysis or trading.".to_string());
        }

        let timeframe = self.timeframe.trim().to_string();
        if parse_timeframe_seconds(&timeframe).is_err() {
            errors.push(
                "Timeframe must be a positive integer with unit m, h, or d (e.g. 15m, 1h, 1d)."
                    .to_string(),
            );
        }

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
            Ok(ValidatedCreateAgentSchedule {
                job_kind: job_kind.to_string(),
                timeframe,
                trigger_delay_seconds: 1,
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
pub(in crate::web::routes) async fn agents_new_job(
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
    Ok(render_new_job_form(
        agent,
        CreateAgentScheduleForm::defaults().as_template_values(),
        picker,
        Vec::new(),
        StatusCode::OK,
    ))
}
pub(in crate::web::routes) async fn agents_create_job(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
    Form(form): Form<CreateAgentScheduleForm>,
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
            return Ok(render_new_job_form(
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
                return Ok(render_new_job_form(
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
    if let Err(error) = crate::agentic::store::insert_agent_schedule(
        &state.db_pool,
        &agent_key,
        &validated.job_kind,
        validated.enabled,
        &validated.timeframe,
        validated.trigger_delay_seconds,
        model_provider_id,
        model_id,
        validated.timeout_seconds,
        &validated.operator_prompt,
    )
    .await
    {
        let errors = match schedule_unique_violation_message(&error) {
            Some(message) => vec![message],
            None => return Err(AppError(error)),
        };
        let picker = load_model_picker_context(&state, &agent).await;
        return Ok(render_new_job_form(
            agent,
            form.as_template_values(),
            picker,
            errors,
            StatusCode::UNPROCESSABLE_ENTITY,
        ));
    }

    Ok(Redirect::to(&format!("/agents/{agent_key}/jobs")).into_response())
}
pub(in crate::web::routes) async fn agents_toggle_job(
    State(state): State<Arc<AppState>>,
    Path((agent_key, job_id)): Path<(String, i64)>,
    Form(form): Form<ToggleScheduleForm>,
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

    // Checkbox presence: if `enabled=on` was submitted, the new state is
    // enabled. Otherwise (only `enabled=off` was submitted), the new state
    // is disabled. This is the same shape used by the agent create form.
    let enable = matches!(form.enabled.as_deref(), Some("on"));
    let updated =
        crate::agentic::store::set_schedule_enabled(&state.db_pool, &agent_key, job_id, enable)
            .await?;

    if !updated {
        return Ok((StatusCode::NOT_FOUND, "job not found").into_response());
    }

    Ok(Redirect::to(&format!("/agents/{agent_key}/jobs")).into_response())
}
pub(in crate::web::routes) async fn agents_toggle_all_jobs(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
    Form(form): Form<ToggleScheduleForm>,
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
    crate::agentic::store::set_all_agent_jobs_enabled(&state.db_pool, &agent_key, enable).await?;

    Ok(Redirect::to(&format!("/agents/{agent_key}/jobs")).into_response())
}
pub(in crate::web::routes) async fn agents_delete_job(
    State(state): State<Arc<AppState>>,
    Path((agent_key, job_id)): Path<(String, i64)>,
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
        crate::agentic::store::delete_agent_schedule(&state.db_pool, &agent_key, job_id).await?;
    if !deleted {
        return Ok((StatusCode::NOT_FOUND, "job not found").into_response());
    }

    Ok(Redirect::to(&format!("/agents/{agent_key}/jobs")).into_response())
}
pub(in crate::web::routes) async fn agents_run_job_now(
    State(state): State<Arc<AppState>>,
    Path((agent_key, job_id)): Path<(String, i64)>,
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

    let Some(schedule) = crate::agentic::store::get_opencode_schedule_for_dispatch(
        &state.db_pool,
        &agent_key,
        job_id,
    )
    .await?
    else {
        return Ok((StatusCode::NOT_FOUND, "job not found").into_response());
    };

    match crate::agentic::store::insert_queued_run(&state.db_pool, &agent_key, job_id).await? {
        QueuedScheduleRun::Dispatch {
            run_id,
            scheduled_for,
        } => {
            let agent = get_agent(&state.db_pool, &agent_key)
                .await?
                .ok_or_else(|| AppError(anyhow::anyhow!("agent not found")))?;
            let selected_instruments =
                crate::agents::store::list_agent_instrument_ids(&state.db_pool, &agent_key).await?;
            let system_setting =
                crate::settings::store::get_setting(&state.db_pool, "opencode_system_prompt")
                    .await?;
            let system_prompt = system_setting
                .map(|s| s.value)
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| crate::agents::prompts::DEFAULT_SYSTEM_PROMPT.to_string());
            let account_snapshot = if schedule.job_kind == crate::agentic::model::JOB_KIND_TRADING {
                Some(live_agent_snapshot_for_dispatch(
                    &agent.wallet_address,
                    &agent.environment,
                    &state.live_accounts,
                ))
            } else {
                None
            };
            let request = dispatch_request_from_schedule(
                &schedule,
                run_id,
                scheduled_for,
                &agent,
                selected_instruments,
                system_prompt,
                account_snapshot,
            );
            let pool = state.db_pool.clone();
            let backend = state.agentic_backend.clone();
            let live_accounts = state.live_accounts.clone();
            let trigger_hook = schedule.job_kind == JOB_KIND_ANALYSIS;
            let hook_agent_key = agent_key.clone();
            tokio::spawn(async move {
                let result = dispatch_run(pool.clone(), backend.clone(), request).await;
                if trigger_hook && result.succeeded {
                    let _ = dispatch_analysis_batch_completed_hook(
                        &pool,
                        &backend,
                        &live_accounts,
                        &hook_agent_key,
                    )
                    .await;
                }
            });
        }
        QueuedScheduleRun::Skipped { .. } => {}
        QueuedScheduleRun::Missing => {
            return Ok((StatusCode::NOT_FOUND, "job not found").into_response());
        }
        QueuedScheduleRun::BlockedByMaintenance => {
            return Ok(jobs_warning_redirect(
                &agent_key,
                WORKSPACE_MAINTENANCE_ACTIVE_WARNING,
            ));
        }
    }

    Ok(Redirect::to(&format!("/agents/{agent_key}/jobs")).into_response())
}
pub(in crate::web::routes) async fn agents_update_job_model(
    State(state): State<Arc<AppState>>,
    Path((agent_key, job_id)): Path<(String, i64)>,
    Form(form): Form<ModelSelectionForm>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let Some(_job) =
        crate::agentic::store::get_agent_schedule(&state.db_pool, &agent_key, job_id).await?
    else {
        return Ok((StatusCode::NOT_FOUND, "job not found").into_response());
    };

    let parsed = parse_model_selection(&form.model_selection)
        .map_err(|message| AppError(anyhow::anyhow!(message)))?;
    let validated = validate_model_selection_for_agent(&state, &agent, parsed)
        .await
        .map_err(|message| AppError(anyhow::anyhow!(message)))?;
    let model_provider_id = validated.as_ref().map(|(provider, _)| provider.as_str());
    let model_id = validated.as_ref().map(|(_, model)| model.as_str());

    if !crate::agentic::store::set_schedule_model(
        &state.db_pool,
        &agent_key,
        job_id,
        model_provider_id,
        model_id,
    )
    .await?
    {
        return Ok((StatusCode::NOT_FOUND, "job not found").into_response());
    }

    Ok(Redirect::to(&format!("/agents/{agent_key}/jobs/{job_id}")).into_response())
}
pub(in crate::web::routes) async fn agents_update_job_timeout(
    State(state): State<Arc<AppState>>,
    Path((agent_key, job_id)): Path<(String, i64)>,
    Form(form): Form<TimeoutForm>,
) -> Result<Response, AppError> {
    let detail_url = format!("/agents/{agent_key}/jobs/{job_id}");

    if crate::agentic::store::get_agent_schedule(&state.db_pool, &agent_key, job_id)
        .await?
        .is_none()
    {
        return Ok((StatusCode::NOT_FOUND, "job not found").into_response());
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

    if !crate::agentic::store::set_schedule_timeout(&state.db_pool, &agent_key, job_id, timeout_i32)
        .await?
    {
        return Ok((StatusCode::NOT_FOUND, "job not found").into_response());
    }

    Ok(Redirect::to(&detail_url).into_response())
}
pub(in crate::web::routes) fn render_new_job_form(
    agent: crate::agents::model::AgentDetailRow,
    form: CreateAgentScheduleFormValues,
    picker: ModelPickerContext,
    errors: Vec<String>,
    status: StatusCode,
) -> Response {
    let current_path = format!("/agents/{}/jobs/new", agent.agent_key);
    let model_picker =
        build_model_picker_view("job-model-selection", &form.model_selection, picker);
    let template = AgentScheduleNewPageTemplate {
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
pub(in crate::web::routes) fn schedule_unique_violation_message(error: &anyhow::Error) -> Option<String> {
    let db_err = error.downcast_ref::<sqlx::Error>()?.as_database_error()?;
    if !db_err.is_unique_violation() {
        return None;
    }

    let constraint = db_err.constraint().unwrap_or("unknown");
    if constraint.contains("agentic_job_schedules") || constraint.contains("job_key") {
        Some("A job with this kind and timeframe already exists for this agent.".to_string())
    } else {
        Some("This job conflicts with an existing row.".to_string())
    }
}
