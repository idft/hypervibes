use std::sync::Arc;

use askama::Template;
use axum::{
    Form,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{
        Html, IntoResponse, Redirect, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use chrono::{TimeZone, Utc};
use futures::{StreamExt, stream::unfold};
use serde::Deserialize;
use std::convert::Infallible;
use tokio::sync::broadcast;
use tracing::warn;

use super::shared::{
    ModelPickerContext, ModelSelectionForm, SERVER_SHUTTING_DOWN_WARNING, TimeoutForm,
    ToggleJobForm, WORKSPACE_MAINTENANCE_ACTIVE_WARNING, build_model_picker_view, is_htmx_request,
    jobs_warning_redirect, load_model_picker_context, parse_positive_job_seconds,
    timeout_error_redirect, validate_model_selection_for_agent,
};
use super::show::{
    AgentJobsQuery, AgentShowQueries, build_agent_recent_runs_view, parse_positive_page,
    render_agent_show_page,
};
use crate::web::error::AppError;
use crate::{
    agents::{
        store::get_agent,
        strategy_prompts::{get_agent_strategy_prompt, prompt_kind_for_job_kind},
    },
    harness::{
        model::{
            JOB_KIND_ANALYSIS, JOB_KIND_ANALYSIS_CODING, JOB_KIND_MARKET_ANALYSIS,
            JOB_KIND_TRADING, TRIGGER_TYPE_ANALYSIS_BATCH_COMPLETED, TRIGGER_TYPE_CANDLE_CLOSED,
            TRIGGER_TYPE_DAILY_REVIEW_COMPLETED,
        },
        scheduler::{
            DispatchRequestInputs, build_dispatch_request, dispatch_analysis_batch_completed_event,
            dispatch_daily_review_coding_event, dispatch_request_from_job,
            dispatch_run_with_workspace_lease,
        },
        store::{self, QueuedJobRun},
        timeframe::{parse_timeframe_seconds, parse_timeout_seconds},
    },
    hyperliquid::live_state::live_agent_snapshot_for_dispatch,
    memory::get_latest_agent_memory_by_type,
    model_catalog::options::parse_model_selection,
    web::{
        AppState,
        auth::AuthenticatedUser,
        run_detail_events::RunDetailDbEvent,
        templates::{
            AgentJobDetailPageTemplate, AgentJobNewPageTemplate, AgentRecentRunsPartialTemplate,
            AgentShowTab, CreateHarnessJobFormValues, ModelPickerPartialTemplate,
            build_agent_show_tabs, load_navbar,
        },
    },
};
pub(in crate::web::routes) async fn agents_show_jobs(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path(agent_key): Path<String>,
    Query(query): Query<AgentJobsQuery>,
) -> Result<Response, AppError> {
    render_agent_show_page(
        &state,
        &user,
        &agent_key,
        AgentShowTab::Jobs,
        AgentShowQueries {
            jobs: Some(query),
            ..Default::default()
        },
    )
    .await
}

pub(in crate::web::routes) async fn agent_recent_runs_stream(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
    Query(query): Query<AgentJobsQuery>,
) -> Result<Response, AppError> {
    let requested_page = parse_positive_page(&query.page);
    let receiver = state.run_detail_events.subscribe();
    if get_agent(&state.db_pool, &agent_key).await?.is_none() {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    }

    let shutdown_rx = state.shutdown_rx.clone();
    let recent_runs_section =
        build_agent_recent_runs_view(&state, &agent_key, requested_page).await;
    let initial_html = AgentRecentRunsPartialTemplate::render_view(recent_runs_section)?;
    let stream_state = AgentRecentRunsStreamState {
        state,
        agent_key,
        requested_page,
        receiver,
        shutdown_rx,
    };
    let updates = unfold(stream_state, next_agent_recent_runs_event);
    let stream = tokio_stream::iter(vec![Ok::<Event, Infallible>(
        Event::default().event("recent-runs").data(initial_html),
    )])
    .chain(updates);

    Ok(Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(15)))
        .into_response())
}

struct AgentRecentRunsStreamState {
    state: Arc<AppState>,
    agent_key: String,
    requested_page: usize,
    receiver: broadcast::Receiver<RunDetailDbEvent>,
    shutdown_rx: tokio::sync::watch::Receiver<bool>,
}

async fn next_agent_recent_runs_event(
    stream_state: AgentRecentRunsStreamState,
) -> Option<(Result<Event, Infallible>, AgentRecentRunsStreamState)> {
    let mut stream_state = stream_state;
    loop {
        if *stream_state.shutdown_rx.borrow() {
            return None;
        }

        let notification = tokio::select! {
            result = stream_state.receiver.recv() => result,
            changed = stream_state.shutdown_rx.changed() => {
                if changed.is_err() || *stream_state.shutdown_rx.borrow() {
                    return None;
                }
                continue;
            }
        };
        let event = match notification {
            Ok(event) => event,
            Err(broadcast::error::RecvError::Lagged(_)) => RunDetailDbEvent::Resync,
            Err(broadcast::error::RecvError::Closed) => return None,
        };

        let matches = match event {
            RunDetailDbEvent::RunChanged { run_id } => {
                match crate::harness::store::get_run(&stream_state.state.db_pool, run_id).await {
                    Ok(Some(run)) => run.agent_key == stream_state.agent_key,
                    Ok(None) => false,
                    Err(error) => {
                        warn!(
                            agent_key = %stream_state.agent_key,
                            run_id,
                            error = ?error,
                            "failed to inspect agent run for recent-runs SSE update"
                        );
                        false
                    }
                }
            }
            RunDetailDbEvent::SessionChanged { .. } => false,
            RunDetailDbEvent::Resync => true,
            RunDetailDbEvent::ConversationChanged { .. } => false,
        };
        if !matches {
            continue;
        }

        let recent_runs_section = build_agent_recent_runs_view(
            &stream_state.state,
            &stream_state.agent_key,
            stream_state.requested_page,
        )
        .await;
        match AgentRecentRunsPartialTemplate::render_view(recent_runs_section) {
            Ok(html) => {
                return Some((
                    Ok(Event::default().event("recent-runs").data(html)),
                    stream_state,
                ));
            }
            Err(error) => {
                warn!(
                    agent_key = %stream_state.agent_key,
                    error = ?error,
                    "failed to render recent-runs SSE snapshot"
                );
            }
        }
    }
}

pub(in crate::web::routes) async fn agents_show_job_detail(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path((agent_key, job_id)): Path<(String, i64)>,
    Query(query): Query<JobDetailQuery>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let Some(job) =
        crate::harness::store::get_agent_job(&state.db_pool, &agent_key, job_id).await?
    else {
        return Ok((StatusCode::NOT_FOUND, "job not found").into_response());
    };

    const RUNS_LIMIT: i64 = 50;
    let mut job_runs_loaded = false;
    let job_runs =
        match crate::harness::store::list_job_runs(&state.db_pool, &agent_key, job_id, RUNS_LIMIT)
            .await
        {
            Ok(rows) => {
                job_runs_loaded = true;
                rows.iter()
                    .map(crate::web::templates::HarnessRunView::from_row)
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

    let mut job_view = crate::web::templates::HarnessJobDetailView::from_row(&job);
    if let Some(error) = query.timeout_error {
        job_view.timeout_editor.error = Some(error);
    }
    if let Some(error) = query.timeframe_error {
        job_view.candle_trigger_editor.error = Some(error);
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
    let mut model_picker = build_model_picker_view(
        "job-model-selection",
        &job_view.model_selection,
        job.model_variant.as_deref(),
        picker,
    );
    model_picker.show_label = false;
    model_picker.use_modal = true;
    let navbar = load_navbar(&state.db_pool, user.id).await?;
    let html = AgentJobDetailPageTemplate::render_view(
        agent.clone(),
        job_view,
        model_picker,
        job_runs,
        job_runs_loaded,
        navbar,
    )?;
    Ok(Html(html).into_response())
}
pub(in crate::web::routes) async fn agents_job_model_picker(
    State(state): State<Arc<AppState>>,
    Path((agent_key, job_id)): Path<(String, i64)>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let Some(job) = store::get_agent_job(&state.db_pool, &agent_key, job_id).await? else {
        return Ok((StatusCode::NOT_FOUND, "job not found").into_response());
    };

    let selected = match (job.model_provider_id.as_deref(), job.model_id.as_deref()) {
        (Some(provider), Some(model)) => format!("{provider}/{model}"),
        _ => String::new(),
    };
    let picker = load_model_picker_context(&state, &agent).await;
    let mut model_picker = build_model_picker_view(
        "job-model-selection",
        &selected,
        job.model_variant.as_deref(),
        picker,
    );
    model_picker.show_label = false;
    model_picker.use_modal = true;
    let html = ModelPickerPartialTemplate::render_view(model_picker)?;
    Ok(Html(html).into_response())
}
pub(in crate::web::routes) async fn build_job_prompt_preview(
    state: &Arc<AppState>,
    agent: &crate::agents::model::AgentDetailRow,
    job: &crate::harness::model::HarnessJobRow,
) -> anyhow::Result<String> {
    let Some(dispatch_job) = store::get_dispatch_job(
        &state.db_pool,
        &agent.agent_key,
        job.id,
        &state.opencode_base_url,
    )
    .await?
    else {
        anyhow::bail!("Job dispatch context is unavailable.");
    };
    let requires_instruments = matches!(
        job.job_kind.as_str(),
        JOB_KIND_ANALYSIS | JOB_KIND_MARKET_ANALYSIS | crate::harness::model::JOB_KIND_TRADING
    );
    if requires_instruments
        && crate::agents::store::list_agent_instrument_ids(&state.db_pool, &agent.agent_key)
            .await?
            .is_empty()
    {
        anyhow::bail!("Prompt preview requires at least one selected instrument for this job.");
    }
    let Some(request) = build_dispatch_request(
        &state.db_pool,
        &state.live_accounts,
        &dispatch_job,
        0,
        job.next_run_at.unwrap_or_else(Utc::now),
    )
    .await?
    else {
        anyhow::bail!("Prompt preview dispatch context is unavailable.");
    };

    crate::harness::prompt::build_prompt(&request)
}

async fn load_strategy_prompt(
    state: &Arc<AppState>,
    agent_key: &str,
    job_kind: &str,
) -> anyhow::Result<String> {
    let prompt_kind = prompt_kind_for_job_kind(job_kind)
        .ok_or_else(|| anyhow::anyhow!("unknown job kind {job_kind}"))?;
    Ok(
        get_agent_strategy_prompt(&state.db_pool, agent_key, prompt_kind)
            .await?
            .map(|row| row.prompt)
            .unwrap_or_default(),
    )
}

async fn load_accumulated_learnings(
    state: &Arc<AppState>,
    agent_key: &str,
) -> anyhow::Result<Option<String>> {
    Ok(
        get_latest_agent_memory_by_type(&state.db_pool, agent_key, "agent_learnings")
            .await?
            .map(|memory| {
                format!(
                    "Summary: {}\nCreated at: {}\nContent: {}",
                    memory.summary,
                    memory
                        .created_at
                        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                    memory.content
                )
            }),
    )
}
#[derive(Debug, Clone, Default, Deserialize)]
pub(in crate::web::routes) struct CreateHarnessJobForm {
    #[serde(default)]
    pub trigger_type: String,
    #[serde(default)]
    pub job_kind: String,
    #[serde(default)]
    pub timeframe: String,
    #[serde(default)]
    pub timeout_seconds: String,
    #[serde(default)]
    pub model_selection: String,
    #[serde(default)]
    pub model_variant: String,
    #[serde(default)]
    pub operator_prompt: String,
    pub enabled: Option<String>,
}
#[derive(Debug)]
pub(in crate::web::routes) struct ValidatedCreateHarnessJob {
    pub trigger_type: String,
    pub job_kind: String,
    pub timeframe: String,
    pub trigger_delay_seconds: i32,
    pub timeout_seconds: i32,
    pub model_selection: Option<(String, String)>,
    pub model_variant: String,
    pub operator_prompt: String,
    pub enabled: bool,
}
impl CreateHarnessJobForm {
    fn defaults() -> Self {
        Self {
            job_kind: JOB_KIND_ANALYSIS.to_string(),
            trigger_type: TRIGGER_TYPE_CANDLE_CLOSED.to_string(),
            timeframe: "15m".to_string(),
            timeout_seconds: "900".to_string(),
            enabled: Some("on".to_string()),
            ..Self::default()
        }
    }

    fn enabled(&self) -> bool {
        self.enabled.is_some()
    }

    fn as_template_values(&self) -> CreateHarnessJobFormValues {
        CreateHarnessJobFormValues {
            trigger_type: self.trigger_type.clone(),
            job_kind: self.job_kind.clone(),
            timeframe: self.timeframe.clone(),
            timeout_seconds: self.timeout_seconds.clone(),
            model_selection: self.model_selection.clone(),
            model_variant: self.model_variant.clone(),
            operator_prompt: self.operator_prompt.clone(),
            enabled: self.enabled(),
        }
    }

    fn validate(&self) -> Result<ValidatedCreateHarnessJob, Vec<String>> {
        let mut errors = Vec::new();

        let job_kind = self.job_kind.trim();
        let trigger_type = self.trigger_type.trim();
        let candle = matches!(
            job_kind,
            JOB_KIND_ANALYSIS | JOB_KIND_TRADING | crate::harness::model::JOB_KIND_DAILY_REVIEW
        ) && trigger_type == TRIGGER_TYPE_CANDLE_CLOSED;
        let event = matches!(
            (job_kind, trigger_type),
            (
                JOB_KIND_MARKET_ANALYSIS,
                TRIGGER_TYPE_ANALYSIS_BATCH_COMPLETED
            ) | (
                JOB_KIND_ANALYSIS_CODING,
                TRIGGER_TYPE_DAILY_REVIEW_COMPLETED
            )
        );
        if !candle && !event {
            errors
                .push("Choose one of the supported job kind and trigger combinations.".to_string());
        }

        let timeframe = self.timeframe.trim().to_string();
        if candle && parse_timeframe_seconds(&timeframe).is_err() {
            errors.push(
                "Timeframe must be a positive integer with unit m, h, or d (e.g. 15m, 1h, 1d)."
                    .to_string(),
            );
        }

        let timeout_seconds =
            parse_positive_job_seconds(&self.timeout_seconds, "Timeout", &mut errors);

        let model_selection = match parse_model_selection(&self.model_selection) {
            Ok(selection) => selection,
            Err(error) => {
                errors.push(error);
                None
            }
        };

        if self.enabled() && model_selection.is_none() {
            errors.push("A model is required to enable a job.".to_string());
        }

        if errors.is_empty() {
            Ok(ValidatedCreateHarnessJob {
                trigger_type: trigger_type.to_string(),
                job_kind: job_kind.to_string(),
                timeframe,
                trigger_delay_seconds: 1,
                timeout_seconds: timeout_seconds.expect("validated timeout seconds"),
                model_selection,
                model_variant: self.model_variant.clone(),
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
    user: AuthenticatedUser,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let picker = load_model_picker_context(&state, &agent).await;
    let navbar = load_navbar(&state.db_pool, user.id).await?;
    Ok(render_new_job_form(
        agent,
        CreateHarnessJobForm::defaults().as_template_values(),
        picker,
        Vec::new(),
        StatusCode::OK,
        navbar,
    ))
}
pub(in crate::web::routes) async fn agents_create_job(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path(agent_key): Path<String>,
    Form(form): Form<CreateHarnessJobForm>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let navbar = load_navbar(&state.db_pool, user.id).await?;
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
                navbar.clone(),
            ));
        }
    };

    let validated_model_selection = match validate_model_selection_for_agent(
        &state,
        &agent,
        validated.model_selection.clone(),
        &validated.model_variant,
    )
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
                navbar.clone(),
            ));
        }
    };

    let model_provider_id = validated_model_selection
        .as_ref()
        .map(|(provider, _, _)| provider.as_str());
    let model_id = validated_model_selection
        .as_ref()
        .map(|(_, model, _)| model.as_str());
    let model_variant = validated_model_selection
        .as_ref()
        .and_then(|(_, _, variant)| variant.as_deref());
    let result = if validated.trigger_type == TRIGGER_TYPE_CANDLE_CLOSED {
        crate::harness::store::insert_candle_job_with_model_variant(
            &state.db_pool,
            &agent_key,
            &validated.job_kind,
            validated.enabled,
            &validated.timeframe,
            validated.trigger_delay_seconds,
            model_provider_id,
            model_id,
            model_variant,
            validated.timeout_seconds,
            &validated.operator_prompt,
        )
        .await
    } else {
        crate::harness::store::insert_event_job_with_model_variant(
            &state.db_pool,
            &agent_key,
            &validated.job_kind,
            &validated.trigger_type,
            validated.enabled,
            model_provider_id,
            model_id,
            model_variant,
            validated.timeout_seconds,
            &validated.operator_prompt,
        )
        .await
    };
    if let Err(error) = result {
        let errors = match job_unique_violation_message(&error) {
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
            navbar,
        ));
    }

    Ok(Redirect::to(&format!("/agents/{agent_key}/jobs")).into_response())
}
pub(in crate::web::routes) async fn agents_toggle_job(
    State(state): State<Arc<AppState>>,
    Path((agent_key, job_id)): Path<(String, i64)>,
    Form(form): Form<ToggleJobForm>,
) -> Result<Response, AppError> {
    let Some(_agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    // Checkbox presence: if `enabled=on` was submitted, the new state is
    // enabled. Otherwise (only `enabled=off` was submitted), the new state
    // is disabled. This is the same shape used by the agent create form.
    let enable = matches!(form.enabled.as_deref(), Some("on"));
    if enable {
        let Some(job) =
            crate::harness::store::get_agent_job(&state.db_pool, &agent_key, job_id).await?
        else {
            return Ok((StatusCode::NOT_FOUND, "job not found").into_response());
        };
        if job.model_provider_id.is_none() || job.model_id.is_none() {
            return Ok(jobs_warning_redirect(&agent_key, "No model set"));
        }
    }
    let updated =
        crate::harness::store::set_job_enabled(&state.db_pool, &agent_key, job_id, enable).await?;

    if !updated {
        return Ok((StatusCode::NOT_FOUND, "job not found").into_response());
    }

    Ok(Redirect::to(&format!("/agents/{agent_key}/jobs")).into_response())
}
pub(in crate::web::routes) async fn agents_toggle_all_jobs(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
    Form(form): Form<ToggleJobForm>,
) -> Result<Response, AppError> {
    let Some(_agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let enable = matches!(form.enabled.as_deref(), Some("on"));
    crate::harness::store::set_all_agent_jobs_enabled(&state.db_pool, &agent_key, enable).await?;

    Ok(Redirect::to(&format!("/agents/{agent_key}/jobs")).into_response())
}
pub(in crate::web::routes) async fn agents_delete_job(
    State(state): State<Arc<AppState>>,
    Path((agent_key, job_id)): Path<(String, i64)>,
) -> Result<Response, AppError> {
    let Some(_agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    match crate::harness::service::delete_idle_job(
        &state.db_pool,
        &state.opencode_client,
        &state.workspace_leases,
        &state.opencode_base_url,
        &agent_key,
        job_id,
    )
    .await?
    {
        crate::harness::service::DeleteJobOutcome::Deleted => {}
        crate::harness::service::DeleteJobOutcome::Missing => {
            return Ok((StatusCode::NOT_FOUND, "job not found").into_response());
        }
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
    if !agent.enabled {
        return Ok(jobs_warning_redirect(
            &agent_key,
            "Agent is disabled; Run now is unavailable.",
        ));
    }
    let Some(job) = crate::harness::store::get_dispatch_job(
        &state.db_pool,
        &agent_key,
        job_id,
        &state.opencode_base_url,
    )
    .await?
    else {
        return Ok((StatusCode::NOT_FOUND, "job not found").into_response());
    };
    if job.model_provider_id.is_none() || job.model_id.is_none() {
        return Ok(jobs_warning_redirect(&agent_key, "No model set"));
    }

    if job.job_kind == crate::harness::model::JOB_KIND_ANALYSIS_CODING {
        return match store::insert_analysis_coding_task_and_run(
            &state.db_pool,
            store::AnalysisCodingTaskRequest {
                agent_key: &agent_key,
                job_id,
                trigger_mode: store::CodingTriggerMode::Manual,
                source_run_id: None,
                source_memory_id: None,
                operator_prompt: None,
                requested_mode: None,
            },
        )
        .await?
        {
            store::InsertAnalysisCodingTaskOutcome::Inserted { run_id, .. } => {
                Ok(Redirect::to(&format!("/agents/{agent_key}/runs/{run_id}")).into_response())
            }
            store::InsertAnalysisCodingTaskOutcome::AlreadyQueued => Ok(jobs_warning_redirect(
                &agent_key,
                "Analysis coding is already queued.",
            )),
            store::InsertAnalysisCodingTaskOutcome::BlockedByMaintenance => Ok(
                jobs_warning_redirect(&agent_key, WORKSPACE_MAINTENANCE_ACTIVE_WARNING),
            ),
        };
    }

    let queued_run = if job.trigger_type == TRIGGER_TYPE_CANDLE_CLOSED {
        crate::harness::store::insert_queued_manual_run(&state.db_pool, &agent_key, job_id).await?
    } else {
        crate::harness::store::insert_queued_event_run(&state.db_pool, &agent_key, job_id).await?
    };

    match queued_run {
        QueuedJobRun::Dispatch {
            run_id,
            scheduled_for,
            wait_for_lane,
        } => {
            if *state.shutdown_rx.borrow() {
                let _ = store::mark_run_failed(
                    &state.db_pool,
                    run_id,
                    "server is shutting down; Run now was rejected",
                    None,
                )
                .await;
                return Ok(jobs_warning_redirect(
                    &agent_key,
                    SERVER_SHUTTING_DOWN_WARNING,
                ));
            }
            let agent = get_agent(&state.db_pool, &agent_key)
                .await?
                .ok_or_else(|| AppError(anyhow::anyhow!("agent not found")))?;
            let selected_instruments =
                crate::agents::store::list_agent_instrument_ids(&state.db_pool, &agent_key).await?;
            let system_prompt = crate::agents::prompts::SYSTEM_PROMPT.to_string();
            let account_snapshot = if job.job_kind == crate::harness::model::JOB_KIND_TRADING {
                Some(live_agent_snapshot_for_dispatch(
                    agent.trading_account_address.as_deref().unwrap_or_default(),
                    &agent.environment,
                    &state.live_accounts,
                ))
            } else {
                None
            };
            let mut request = dispatch_request_from_job(
                &job,
                DispatchRequestInputs {
                    run_id,
                    scheduled_for,
                    agent,
                    selected_instruments,
                    strategy_prompt: load_strategy_prompt(&state, &agent_key, &job.job_kind)
                        .await?,
                    accumulated_learnings: load_accumulated_learnings(&state, &agent_key).await?,
                    system_prompt,
                },
                account_snapshot,
            );
            if job.job_kind == crate::harness::model::JOB_KIND_DAILY_REVIEW {
                let review_window_end = Utc::now();
                let review_window_start = Utc.from_utc_datetime(
                    &review_window_end
                        .date_naive()
                        .and_hms_opt(0, 0, 0)
                        .expect("UTC midnight is valid"),
                );
                request.review_window_start = Some(review_window_start);
                request.review_window_end = Some(review_window_end);
            }
            let pool = state.db_pool.clone();
            let backend = state.harness_backend.clone();
            let live_accounts = state.live_accounts.clone();
            let workspace_leases = state.workspace_leases.clone();
            let trigger_analysis_event = job.job_kind == JOB_KIND_ANALYSIS;
            let trigger_coding_event = job.job_kind == crate::harness::model::JOB_KIND_DAILY_REVIEW;
            let dispatch_agent_key = agent_key.clone();
            let in_flight = state.in_flight.clone();
            tokio::spawn(async move {
                let _guard = in_flight.track();
                if wait_for_lane {
                    loop {
                        match store::has_prior_active_run_in_lane(
                            &pool,
                            &dispatch_agent_key,
                            &job.job_kind,
                            run_id,
                        )
                        .await
                        {
                            Ok(false) => break,
                            Ok(true) => {}
                            Err(error) => {
                                let summary = format!("manual run queue check failed: {error:#}");
                                let _ = store::mark_run_failed(&pool, run_id, &summary, None).await;
                                return;
                            }
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                    }
                }
                let _workspace_lease = workspace_leases
                    .acquire_live_read(&dispatch_agent_key)
                    .await;
                let result = dispatch_run_with_workspace_lease(
                    pool.clone(),
                    backend.clone(),
                    request,
                    &workspace_leases,
                )
                .await;
                if trigger_analysis_event && result.succeeded {
                    let _ = dispatch_analysis_batch_completed_event(
                        &pool,
                        &backend,
                        &live_accounts,
                        &dispatch_agent_key,
                        &workspace_leases,
                        &state.opencode_base_url,
                    )
                    .await;
                }
                if trigger_coding_event && result.succeeded {
                    let _ = dispatch_daily_review_coding_event(&pool, &dispatch_agent_key, run_id)
                        .await;
                }
            });
            Ok(Redirect::to(&format!("/agents/{agent_key}/runs/{run_id}")).into_response())
        }
        QueuedJobRun::Missing => Ok((StatusCode::NOT_FOUND, "job not found").into_response()),
        QueuedJobRun::Skipped { run_id } => {
            Ok(Redirect::to(&format!("/agents/{agent_key}/runs/{run_id}")).into_response())
        }
        QueuedJobRun::BlockedByMaintenance => Ok(jobs_warning_redirect(
            &agent_key,
            WORKSPACE_MAINTENANCE_ACTIVE_WARNING,
        )),
    }
}
pub(in crate::web::routes) async fn agents_update_job_model(
    State(state): State<Arc<AppState>>,
    Path((agent_key, job_id)): Path<(String, i64)>,
    headers: HeaderMap,
    Form(form): Form<ModelSelectionForm>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let Some(_job) =
        crate::harness::store::get_agent_job(&state.db_pool, &agent_key, job_id).await?
    else {
        return Ok((StatusCode::NOT_FOUND, "job not found").into_response());
    };

    let parsed = parse_model_selection(&form.model_selection)
        .map_err(|message| AppError(anyhow::anyhow!(message)))?;
    let validated = validate_model_selection_for_agent(&state, &agent, parsed, &form.model_variant)
        .await
        .map_err(|message| AppError(anyhow::anyhow!(message)))?;
    let model_provider_id = validated.as_ref().map(|(provider, _, _)| provider.as_str());
    let model_id = validated.as_ref().map(|(_, model, _)| model.as_str());
    let model_variant = validated
        .as_ref()
        .and_then(|(_, _, variant)| variant.as_deref());

    if !crate::harness::store::set_job_model_with_variant(
        &state.db_pool,
        &agent_key,
        job_id,
        model_provider_id,
        model_id,
        model_variant,
    )
    .await?
    {
        return Ok((StatusCode::NOT_FOUND, "job not found").into_response());
    }

    if is_htmx_request(&headers) {
        return Ok(StatusCode::NO_CONTENT.into_response());
    }

    Ok(Redirect::to(&format!("/agents/{agent_key}/jobs/{job_id}")).into_response())
}
pub(in crate::web::routes) async fn agents_update_job_timeout(
    State(state): State<Arc<AppState>>,
    Path((agent_key, job_id)): Path<(String, i64)>,
    Form(form): Form<TimeoutForm>,
) -> Result<Response, AppError> {
    let detail_url = format!("/agents/{agent_key}/jobs/{job_id}");

    if crate::harness::store::get_agent_job(&state.db_pool, &agent_key, job_id)
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

    if !crate::harness::store::set_job_timeout(&state.db_pool, &agent_key, job_id, timeout_i32)
        .await?
    {
        return Ok((StatusCode::NOT_FOUND, "job not found").into_response());
    }

    Ok(Redirect::to(&detail_url).into_response())
}
#[derive(Debug, Default, Deserialize)]
pub(in crate::web::routes) struct TimeframeForm {
    #[serde(default)]
    pub timeframe: String,
}

#[derive(Debug, Default, Deserialize)]
pub(in crate::web::routes) struct JobDetailQuery {
    #[serde(default)]
    pub timeout_error: Option<String>,
    #[serde(default)]
    pub timeframe_error: Option<String>,
}

pub(in crate::web::routes) async fn agents_update_job_timeframe(
    State(state): State<Arc<AppState>>,
    Path((agent_key, job_id)): Path<(String, i64)>,
    Form(form): Form<TimeframeForm>,
) -> Result<Response, AppError> {
    let detail_url = format!("/agents/{agent_key}/jobs/{job_id}");
    let timeframe = form.timeframe.trim();
    if let Err(error) = parse_timeframe_seconds(timeframe) {
        return Ok(timeframe_error_redirect(
            &detail_url,
            format!("Invalid timeframe: {error}"),
        ));
    }

    match crate::harness::store::set_candle_job_timeframe(
        &state.db_pool,
        &agent_key,
        job_id,
        timeframe,
    )
    .await
    {
        Ok(true) => Ok(Redirect::to(&detail_url).into_response()),
        Ok(false) => Ok((StatusCode::NOT_FOUND, "job not found").into_response()),
        Err(error) => match job_unique_violation_message(&error) {
            Some(message) => Ok(timeframe_error_redirect(&detail_url, message)),
            None => Err(AppError(error)),
        },
    }
}

fn timeframe_error_redirect(detail_url: &str, message: String) -> Response {
    Redirect::to(&format!(
        "{detail_url}?timeframe_error={}",
        super::shared::urlencode(&message)
    ))
    .into_response()
}
pub(in crate::web::routes) fn render_new_job_form(
    agent: crate::agents::model::AgentDetailRow,
    form: CreateHarnessJobFormValues,
    picker: ModelPickerContext,
    errors: Vec<String>,
    status: StatusCode,
    navbar: crate::web::templates::Navbar,
) -> Response {
    let current_path = format!("/agents/{}/jobs/new", agent.agent_key);
    let model_picker = build_model_picker_view(
        "job-model-selection",
        &form.model_selection,
        (!form.model_variant.trim().is_empty()).then_some(form.model_variant.as_str()),
        picker,
    );
    let tabs = build_agent_show_tabs(&agent, AgentShowTab::Jobs);
    let navbar = navbar.with_selected_agent(
        agent.agent_key.clone(),
        agent.display_name.clone(),
        agent.enabled,
    );
    let template = AgentJobNewPageTemplate {
        agent,
        tabs,
        agent_tabs_use_htmx: false,
        form,
        model_picker,
        errors,
        current_path,
        navbar,
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
pub(in crate::web::routes) fn job_unique_violation_message(
    error: &anyhow::Error,
) -> Option<String> {
    let db_err = error.downcast_ref::<sqlx::Error>()?.as_database_error()?;
    if !db_err.is_unique_violation() {
        return None;
    }

    let constraint = db_err.constraint().unwrap_or("unknown");
    if constraint.contains("harness_jobs") || constraint.contains("job_key") {
        Some("A job with this kind and timeframe already exists for this agent.".to_string())
    } else {
        Some("This job conflicts with an existing row.".to_string())
    }
}
