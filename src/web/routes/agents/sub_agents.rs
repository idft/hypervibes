use std::sync::Arc;

use askama::Template;
use axum::{
    Form,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode},
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
    ModelSelectionForm, SERVER_SHUTTING_DOWN_WARNING, TimeoutForm, ToggleJobForm,
    build_model_picker_view, is_htmx_request, load_model_picker_context,
    parse_positive_job_seconds, sub_agents_warning_redirect, timeout_error_redirect, urlencode,
    validate_model_selection_for_agent,
};
use super::show::{
    AgentShowQueries, AgentSubAgentsQuery, build_agent_recent_runs_view_for_kind,
    load_selected_agent_navbar, parse_positive_page, render_agent_show_page,
};
use crate::web::error::AppError;
use crate::{
    agents::{
        store::get_agent,
        strategy_prompts::{
            create_prompt_revision, get_agent_strategy_prompt, rollback_prompt_revision,
            seed_initial_prompt_revision,
        },
    },
    harness::{
        model::{
            CAPABILITY_REVIEW_PROMPT_UPDATE, SUB_AGENT_KIND_ANALYSIS, SUB_AGENT_KIND_REVIEW,
            SUB_AGENT_KIND_TRADING,
        },
        scheduler::{
            DispatchRequestInputs, build_dispatch_request, dispatch_request_from_job,
            dispatch_run_in_isolated_workspace_with_workspace_lease,
        },
        store::{self, QueuedSubAgentRun},
        timeframe::{parse_timeframe_seconds, parse_timeout_seconds},
    },
    hyperliquid::live_state::live_agent_snapshot_for_dispatch,
    memory::get_latest_agent_memory_by_type,
    model_catalog::options::parse_model_selection,
    notifications::store::count_notifications,
    web::{
        AppState,
        auth::AuthenticatedUser,
        run_detail_events::RunDetailDbEvent,
        templates::{
            AgentJobDetailPageTemplate, AgentJobNewPageTemplate, AgentJobPageNavigation,
            AgentRecentRunsPartialTemplate, AgentRoleEditPageTemplate, AgentShowTab,
            AgentSingletonRoleNewPageTemplate, CreateAnalysisJobFormValues,
            CreateSingletonRoleFormValues, ModelPickerPartialTemplate, build_agent_show_tabs,
        },
    },
};

pub(in crate::web::routes) async fn agents_show_analysis(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path(agent_key): Path<String>,
    Query(query): Query<AgentSubAgentsQuery>,
) -> Result<Response, AppError> {
    render_agent_show_page(
        &state,
        &user,
        &agent_key,
        AgentShowTab::Analysis,
        AgentShowQueries {
            sub_agents: Some(query),
            ..Default::default()
        },
    )
    .await
}

pub(in crate::web::routes) async fn agent_sub_agent_recent_runs_stream(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
    Query(query): Query<AgentSubAgentsQuery>,
) -> Result<Response, AppError> {
    let requested_page = parse_positive_page(&query.page);
    let sub_agent_kind = match query.kind.as_str() {
        "trading" => SUB_AGENT_KIND_TRADING,
        "review" => SUB_AGENT_KIND_REVIEW,
        _ => SUB_AGENT_KIND_ANALYSIS,
    };
    let receiver = state.run_detail_events.subscribe();
    let Some(_agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };

    let shutdown_rx = state.shutdown_rx.clone();
    let page_path = match sub_agent_kind {
        SUB_AGENT_KIND_TRADING => format!("/agents/{agent_key}/trading"),
        SUB_AGENT_KIND_REVIEW => format!("/agents/{agent_key}/review"),
        _ => format!("/agents/{agent_key}/analysis"),
    };
    let recent_runs_section = build_agent_recent_runs_view_for_kind(
        &state,
        &agent_key,
        sub_agent_kind,
        &page_path,
        requested_page,
    )
    .await;
    let initial_html = AgentRecentRunsPartialTemplate::render_view(recent_runs_section)?;
    let stream_state = AgentRecentRunsStreamState {
        state,
        agent_key,
        sub_agent_kind: sub_agent_kind.to_string(),
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
    sub_agent_kind: String,
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
                    Ok(Some(run)) => {
                        run.agent_key == stream_state.agent_key
                            && run.sub_agent_kind == stream_state.sub_agent_kind
                    }
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

        let page_path = match stream_state.sub_agent_kind.as_str() {
            SUB_AGENT_KIND_TRADING => format!("/agents/{}/trading", stream_state.agent_key),
            SUB_AGENT_KIND_REVIEW => format!("/agents/{}/review", stream_state.agent_key),
            _ => format!("/agents/{}/analysis", stream_state.agent_key),
        };
        let recent_runs_section = build_agent_recent_runs_view_for_kind(
            &stream_state.state,
            &stream_state.agent_key,
            &stream_state.sub_agent_kind,
            &page_path,
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

pub(in crate::web::routes) async fn agents_show_sub_agent_detail(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path((agent_key, sub_agent_id)): Path<(String, i64)>,
    Query(query): Query<JobDetailQuery>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let Some(job) =
        crate::harness::store::get_agent_sub_agent(&state.db_pool, &agent_key, sub_agent_id)
            .await?
    else {
        return Ok((StatusCode::NOT_FOUND, "sub-agent not found").into_response());
    };

    const RUNS_PER_PAGE: usize = 10;
    let requested_runs_page = parse_positive_page(&query.page);
    let mut job_runs_loaded = false;
    let mut job_runs = Vec::new();
    let mut job_runs_page = requested_runs_page;
    let mut job_runs_total_pages = 0;
    let mut job_runs_total_count = 0;
    let mut job_runs_range_start = 0;
    let mut job_runs_range_end = 0;
    let mut job_runs_previous_page_url = None;
    let mut job_runs_next_page_url = None;

    match crate::harness::store::count_sub_agent_runs(&state.db_pool, &agent_key, sub_agent_id)
        .await
    {
        Ok(total_count) => {
            job_runs_total_count = total_count as usize;
            job_runs_total_pages = if job_runs_total_count == 0 {
                0
            } else {
                job_runs_total_count.div_ceil(RUNS_PER_PAGE)
            };
            job_runs_page = if job_runs_total_pages == 0 {
                1
            } else {
                requested_runs_page.min(job_runs_total_pages)
            };
            job_runs_previous_page_url = (job_runs_page > 1).then(|| {
                format!(
                    "/agents/{agent_key}/sub-agents/{sub_agent_id}?page={}",
                    job_runs_page - 1
                )
            });
            job_runs_next_page_url = (job_runs_page < job_runs_total_pages).then(|| {
                format!(
                    "/agents/{agent_key}/sub-agents/{sub_agent_id}?page={}",
                    job_runs_page + 1
                )
            });

            if job_runs_total_count == 0 {
                job_runs_loaded = true;
            } else {
                let offset = ((job_runs_page - 1) * RUNS_PER_PAGE) as i64;
                match crate::harness::store::list_sub_agent_runs_page(
                    &state.db_pool,
                    &agent_key,
                    sub_agent_id,
                    RUNS_PER_PAGE as i64,
                    offset,
                )
                .await
                {
                    Ok(rows) => {
                        let run_count = rows.len();
                        job_runs_loaded = true;
                        job_runs = rows
                            .iter()
                            .map(crate::web::templates::HarnessSubAgentRunView::from_row)
                            .collect();
                        job_runs_range_start = offset as usize + 1;
                        job_runs_range_end = offset as usize + run_count;
                    }
                    Err(error) => {
                        warn!(
                            agent_key = %agent.agent_key,
                            sub_agent_id,
                            error = ?error,
                            "failed to list sub-agent runs for operator page"
                        );
                    }
                }
            }
        }
        Err(error) => {
            warn!(
                agent_key = %agent.agent_key,
                sub_agent_id,
                error = ?error,
                "failed to count sub-agent runs for operator page"
            );
        }
    }

    let mut job_view = crate::web::templates::HarnessSubAgentDetailView::from_row(&job);
    job_view.highlight_model_selector = query.setup && !job_view.has_model;
    job_view.prompt_error = query.prompt_error;
    if let Some(prompt) =
        get_agent_strategy_prompt(&state.db_pool, &agent_key, sub_agent_id).await?
    {
        job_view.strategy_prompt = prompt.prompt;
        job_view.strategy_prompt_revision = prompt.revision_id;
    }
    if let Some(error) = query.timeout_error {
        job_view.timeout_editor.error = Some(error);
    }
    if let Some(error) = query.timeframe_error {
        job_view.candle_trigger_editor.error = Some(error);
    }
    if let Some(error) = query.model_error {
        job_view.model_error = Some(error);
    }
    match build_job_prompt_preview(&state, &agent, &job).await {
        Ok(text) => job_view.prompt_preview_text = text,
        Err(error) => {
            warn!(
                agent_key = %agent.agent_key,
                sub_agent_id,
                error = ?error,
                "failed to build prompt preview for sub-agent detail page"
            );
            job_view.prompt_preview_error = Some(format!("{error:#}"));
        }
    }

    let picker = load_model_picker_context(&state, &agent).await;
    let mut model_picker = build_model_picker_view(
        "sub-agent-model-selection",
        &job_view.model_selection,
        job.model_variant.as_deref(),
        picker,
    );
    model_picker.show_label = false;
    let navbar = load_selected_agent_navbar(&state, user.id, &agent).await?;
    let notification_count = count_notifications(&state.db_pool, &agent.agent_key).await?;
    let html = AgentJobDetailPageTemplate::render_view(
        agent.clone(),
        job_view,
        model_picker,
        job_runs,
        job_runs_loaded,
        crate::web::templates::HarnessSubAgentRunsPagination {
            page: job_runs_page,
            total_pages: job_runs_total_pages,
            total_count: job_runs_total_count,
            range_start: job_runs_range_start,
            range_end: job_runs_range_end,
            previous_page_url: job_runs_previous_page_url,
            next_page_url: job_runs_next_page_url,
        },
        AgentJobPageNavigation {
            notification_count,
            navbar,
        },
    )?;
    Ok(Html(html).into_response())
}
pub(in crate::web::routes) async fn agents_sub_agent_model_picker(
    State(state): State<Arc<AppState>>,
    Path((agent_key, sub_agent_id)): Path<(String, i64)>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let Some(job) = store::get_agent_sub_agent(&state.db_pool, &agent_key, sub_agent_id).await?
    else {
        return Ok((StatusCode::NOT_FOUND, "sub-agent not found").into_response());
    };

    let selected = match (job.model_provider_id.as_deref(), job.model_id.as_deref()) {
        (Some(provider), Some(model)) => format!("{provider}/{model}"),
        _ => String::new(),
    };
    let picker = load_model_picker_context(&state, &agent).await;
    let mut model_picker = build_model_picker_view(
        "sub-agent-model-selection",
        &selected,
        job.model_variant.as_deref(),
        picker,
    );
    model_picker.show_label = false;
    let html = ModelPickerPartialTemplate::render_view(model_picker)?;
    Ok(Html(html).into_response())
}
pub(in crate::web::routes) async fn build_job_prompt_preview(
    state: &Arc<AppState>,
    agent: &crate::agents::model::AgentDetailRow,
    job: &crate::harness::model::HarnessSubAgentRow,
) -> anyhow::Result<String> {
    let Some(dispatch_job) = store::get_dispatch_sub_agent(
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
        job.sub_agent_kind.as_str(),
        SUB_AGENT_KIND_ANALYSIS | SUB_AGENT_KIND_TRADING
    );
    if requires_instruments
        && crate::agents::store::list_agent_instrument_ids(&state.db_pool, &agent.agent_key)
            .await?
            .is_empty()
    {
        anyhow::bail!(
            "Prompt preview requires at least one selected instrument for this sub-agent."
        );
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

async fn load_accumulated_learnings(
    state: &Arc<AppState>,
    agent_key: &str,
) -> anyhow::Result<(Option<String>, Option<uuid::Uuid>)> {
    Ok(
        get_latest_agent_memory_by_type(&state.db_pool, agent_key, "agent_learnings")
            .await?
            .map(|memory| {
                (
                    format!(
                        "Summary: {}\nCreated at: {}\nContent: {}",
                        memory.summary,
                        memory
                            .created_at
                            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                        memory.content
                    ),
                    memory.id,
                )
            })
            .map_or((None, None), |(content, id)| (Some(content), Some(id))),
    )
}
#[derive(Debug, Clone, Default, Deserialize)]
pub(in crate::web::routes) struct CreateAnalysisJobForm {
    #[serde(default)]
    pub sub_agent_key: String,
    #[serde(default)]
    pub timeframe: String,
    #[serde(default)]
    pub timeout_seconds: String,
    #[serde(default)]
    pub model_selection: String,
    #[serde(default)]
    pub model_variant: String,
    #[serde(default)]
    pub prompt: String,
    pub enabled: Option<String>,
}
#[derive(Debug)]
pub(in crate::web::routes) struct ValidatedCreateAnalysisJob {
    pub sub_agent_key: String,
    pub timeframe: String,
    pub trigger_delay_seconds: i32,
    pub timeout_seconds: i32,
    pub model_selection: Option<(String, String)>,
    pub model_variant: String,
    pub prompt: String,
    pub enabled: bool,
}
impl CreateAnalysisJobForm {
    fn defaults() -> Self {
        Self {
            timeframe: "15m".to_string(),
            timeout_seconds: "900".to_string(),
            prompt: crate::agents::strategy_prompts::default_prompt_for_role(
                SUB_AGENT_KIND_ANALYSIS,
            )
            .to_string(),
            enabled: Some("on".to_string()),
            ..Self::default()
        }
    }

    fn enabled(&self) -> bool {
        self.enabled.is_some()
    }

    fn as_template_values(&self) -> CreateAnalysisJobFormValues {
        CreateAnalysisJobFormValues {
            sub_agent_key: self.sub_agent_key.clone(),
            timeframe: self.timeframe.clone(),
            timeout_seconds: self.timeout_seconds.clone(),
            model_selection: self.model_selection.clone(),
            model_variant: self.model_variant.clone(),
            prompt: self.prompt.clone(),
            enabled: self.enabled(),
        }
    }

    fn validate(&self) -> Result<ValidatedCreateAnalysisJob, Vec<String>> {
        let mut errors = Vec::new();

        let sub_agent_key = self.sub_agent_key.trim().to_string();
        if !crate::harness::sub_agent_key::is_valid_user_sub_agent_key(&sub_agent_key) {
            errors.push(
                "Sub-agent key is required and must contain only letters, digits, hyphens, and underscores (max 64 characters)."
                    .to_string(),
            );
        }

        let timeframe = self.timeframe.trim().to_string();
        if parse_timeframe_seconds(&timeframe).is_err() {
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
            errors.push("A model is required to enable a sub-agent.".to_string());
        }

        let prompt = self.prompt.trim().to_string();
        if prompt.is_empty() {
            errors.push("Prompt must not be empty.".to_string());
        }

        if errors.is_empty() {
            Ok(ValidatedCreateAnalysisJob {
                sub_agent_key,
                timeframe,
                trigger_delay_seconds: 1,
                timeout_seconds: timeout_seconds.expect("validated timeout seconds"),
                model_selection,
                model_variant: self.model_variant.clone(),
                prompt,
                enabled: self.enabled(),
            })
        } else {
            Err(errors)
        }
    }
}

pub(in crate::web::routes) async fn agents_new_analysis_job(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let picker = load_model_picker_context(&state, &agent).await;
    let navbar = load_selected_agent_navbar(&state, user.id, &agent).await?;
    let notification_count = count_notifications(&state.db_pool, &agent.agent_key).await?;
    Ok(render_new_analysis_job_form(
        agent,
        CreateAnalysisJobForm::defaults().as_template_values(),
        picker,
        Vec::new(),
        StatusCode::OK,
        AgentJobPageNavigation {
            notification_count,
            navbar,
        },
    ))
}

fn render_new_analysis_job_form(
    agent: crate::agents::model::AgentDetailRow,
    form: CreateAnalysisJobFormValues,
    picker: super::shared::ModelPickerContext,
    errors: Vec<String>,
    status: StatusCode,
    navigation: AgentJobPageNavigation,
) -> Response {
    let model_picker = build_model_picker_view(
        "sub-agent-model-selection",
        &form.model_selection,
        (!form.model_variant.is_empty()).then_some(form.model_variant.as_str()),
        picker,
    );
    let html = AgentJobNewPageTemplate {
        tabs: build_agent_show_tabs(
            &agent,
            AgentShowTab::Analysis,
            navigation.notification_count,
        ),
        agent_tabs_use_htmx: false,
        current_path: format!("/agents/{}/analysis/new", agent.agent_key),
        agent,
        form,
        model_picker,
        errors,
        navbar: navigation.navbar,
    }
    .render()
    .expect("analysis-job form template must render");
    (status, Html(html)).into_response()
}

pub(in crate::web::routes) async fn agents_create_analysis_job(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path(agent_key): Path<String>,
    Form(form): Form<CreateAnalysisJobForm>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let notification_count = count_notifications(&state.db_pool, &agent.agent_key).await?;
    let navbar = load_selected_agent_navbar(&state, user.id, &agent).await?;
    let validated = match form.validate() {
        Ok(validated) => validated,
        Err(errors) => {
            let picker = load_model_picker_context(&state, &agent).await;
            return Ok(render_new_analysis_job_form(
                agent,
                form.as_template_values(),
                picker,
                errors,
                StatusCode::UNPROCESSABLE_ENTITY,
                AgentJobPageNavigation {
                    notification_count,
                    navbar,
                },
            ));
        }
    };

    let validated_model_selection = match validate_model_selection_for_agent(
        &state,
        validated.model_selection.clone(),
        &validated.model_variant,
    )
    .await
    {
        Ok(selection) => selection,
        Err(error) => {
            let picker = load_model_picker_context(&state, &agent).await;
            return Ok(render_new_analysis_job_form(
                agent,
                form.as_template_values(),
                picker,
                vec![error],
                StatusCode::UNPROCESSABLE_ENTITY,
                AgentJobPageNavigation {
                    notification_count,
                    navbar,
                },
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
    let result = crate::harness::store::insert_analysis_sub_agent_with_model_variant(
        &state.db_pool,
        &agent_key,
        &validated.sub_agent_key,
        validated.enabled,
        &validated.timeframe,
        validated.trigger_delay_seconds,
        model_provider_id,
        model_id,
        model_variant,
        validated.timeout_seconds,
    )
    .await;
    let inserted_id = match result {
        Ok(id) => id,
        Err(error) => {
            let errors = match job_unique_violation_message(&error) {
                Some(message) => vec![message],
                None => return Err(AppError(error)),
            };
            let picker = load_model_picker_context(&state, &agent).await;
            return Ok(render_new_analysis_job_form(
                agent,
                form.as_template_values(),
                picker,
                errors,
                StatusCode::UNPROCESSABLE_ENTITY,
                AgentJobPageNavigation {
                    notification_count,
                    navbar,
                },
            ));
        }
    };
    // Seed the initial prompt revision for the new Analysis job.
    if let Err(error) = crate::agents::strategy_prompts::seed_initial_prompt_revision(
        &state.db_pool,
        &agent_key,
        inserted_id,
        &validated.sub_agent_key,
        &validated.prompt,
    )
    .await
    {
        warn!(
            agent_key = %agent_key,
            sub_agent_id = inserted_id,
            error = ?error,
            "failed to seed initial prompt for new analysis job"
        );
    }

    Ok(Redirect::to(&format!("/agents/{agent_key}/analysis")).into_response())
}

pub(in crate::web::routes) async fn agents_toggle_sub_agent(
    State(state): State<Arc<AppState>>,
    Path((agent_key, sub_agent_id)): Path<(String, i64)>,
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
            crate::harness::store::get_agent_sub_agent(&state.db_pool, &agent_key, sub_agent_id)
                .await?
        else {
            return Ok((StatusCode::NOT_FOUND, "sub-agent not found").into_response());
        };
        if job.model_provider_id.is_none() || job.model_id.is_none() {
            return Ok(sub_agents_warning_redirect(&agent_key, "No model set"));
        }
    }
    let updated = crate::harness::store::set_sub_agent_enabled(
        &state.db_pool,
        &agent_key,
        sub_agent_id,
        enable,
    )
    .await?;

    if !updated {
        return Ok((StatusCode::NOT_FOUND, "sub-agent not found").into_response());
    }

    Ok(Redirect::to(&role_list_redirect(&state, &agent_key, sub_agent_id).await).into_response())
}
pub(in crate::web::routes) async fn agents_toggle_all_sub_agents(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
    Form(form): Form<ToggleJobForm>,
) -> Result<Response, AppError> {
    let Some(_agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let enable = matches!(form.enabled.as_deref(), Some("on"));
    crate::harness::store::set_all_agent_sub_agents_enabled(&state.db_pool, &agent_key, enable)
        .await?;

    Ok(Redirect::to(&format!("/agents/{agent_key}/analysis")).into_response())
}
pub(in crate::web::routes) async fn agents_delete_sub_agent(
    State(state): State<Arc<AppState>>,
    Path((agent_key, sub_agent_id)): Path<(String, i64)>,
) -> Result<Response, AppError> {
    let Some(_agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let Some(job) =
        crate::harness::store::get_agent_sub_agent(&state.db_pool, &agent_key, sub_agent_id)
            .await?
    else {
        return Ok((StatusCode::NOT_FOUND, "sub-agent not found").into_response());
    };
    let redirect_url = role_page_url(&agent_key, &job.sub_agent_kind);
    match crate::harness::service::delete_idle_job(
        &state.db_pool,
        &state.opencode_client,
        &state.workspace_leases,
        &state.opencode_base_url,
        &state.opencode_container_workspaces_root,
        &agent_key,
        sub_agent_id,
    )
    .await?
    {
        crate::harness::service::DeleteJobOutcome::Deleted => {}
        crate::harness::service::DeleteJobOutcome::Missing => {
            return Ok((StatusCode::NOT_FOUND, "sub-agent not found").into_response());
        }
    }

    Ok(Redirect::to(&redirect_url).into_response())
}

/// Resolve the role page a mutation on `sub_agent_id` should return to.
async fn role_list_redirect(state: &Arc<AppState>, agent_key: &str, sub_agent_id: i64) -> String {
    let role = crate::harness::store::get_agent_sub_agent(&state.db_pool, agent_key, sub_agent_id)
        .await
        .ok()
        .flatten()
        .map(|job| job.sub_agent_kind);
    let role_page = match role.as_deref() {
        Some(SUB_AGENT_KIND_TRADING) => "trading",
        Some(SUB_AGENT_KIND_REVIEW) => "review",
        _ => "analysis",
    };
    format!("/agents/{agent_key}/{role_page}")
}
pub(in crate::web::routes) async fn agents_run_sub_agent_now(
    State(state): State<Arc<AppState>>,
    Path((agent_key, sub_agent_id)): Path<(String, i64)>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    if !agent.enabled {
        return Ok(sub_agents_warning_redirect(
            &agent_key,
            "Agent is disabled; Run now is unavailable.",
        ));
    }
    let Some(job) = crate::harness::store::get_dispatch_sub_agent(
        &state.db_pool,
        &agent_key,
        sub_agent_id,
        &state.opencode_base_url,
    )
    .await?
    else {
        return Ok((StatusCode::NOT_FOUND, "sub-agent not found").into_response());
    };
    if job.model_provider_id.is_none() || job.model_id.is_none() {
        return Ok(sub_agents_warning_redirect(&agent_key, "No model set"));
    }

    let queued_run = if job.timeframe.is_some() {
        crate::harness::store::insert_queued_manual_run(&state.db_pool, &agent_key, sub_agent_id)
            .await?
    } else {
        crate::harness::store::insert_queued_event_run(&state.db_pool, &agent_key, sub_agent_id)
            .await?
    };

    match queued_run {
        QueuedSubAgentRun::Dispatch {
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
                return Ok(sub_agents_warning_redirect(
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
            let account_snapshot = if job.sub_agent_kind == SUB_AGENT_KIND_TRADING {
                Some(live_agent_snapshot_for_dispatch(
                    agent.trading_account_address.as_deref().unwrap_or_default(),
                    &agent.environment,
                    &state.live_accounts,
                ))
            } else {
                None
            };
            let (strategy_prompt, strategy_prompt_revision) =
                load_strategy_prompt(&state, &agent_key, sub_agent_id).await?;
            let (accumulated_learnings, accumulated_learning_memory_id) =
                load_accumulated_learnings(&state, &agent_key).await?;
            let mut request = dispatch_request_from_job(
                &job,
                DispatchRequestInputs {
                    run_id,
                    scheduled_for,
                    agent,
                    selected_instruments,
                    strategy_prompt,
                    strategy_prompt_revision,
                    accumulated_learnings,
                    accumulated_learning_memory_id,
                    system_prompt,
                },
                account_snapshot,
            );
            if job.sub_agent_kind == SUB_AGENT_KIND_REVIEW {
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
            let workspace_controller = state.workspace_controller.clone();
            let agent_api_base_url = state.hypervibes_agent_api_base_url.clone();
            let workspace_leases = state.workspace_leases.clone();
            let dispatch_agent_key = agent_key.clone();
            let in_flight = state.in_flight.clone();
            tokio::spawn(async move {
                let _guard = in_flight.track();
                if wait_for_lane {
                    loop {
                        match store::has_prior_active_run_in_lane(
                            &pool,
                            &dispatch_agent_key,
                            &job.sub_agent_kind,
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
                dispatch_run_in_isolated_workspace_with_workspace_lease(
                    pool.clone(),
                    backend.clone(),
                    workspace_controller.clone(),
                    agent_api_base_url.clone(),
                    request,
                    &workspace_leases,
                )
                .await;
            });
            Ok(Redirect::to(&format!("/agents/{agent_key}/runs/{run_id}")).into_response())
        }
        QueuedSubAgentRun::Missing => {
            Ok((StatusCode::NOT_FOUND, "sub-agent not found").into_response())
        }
        QueuedSubAgentRun::Skipped { run_id } => {
            Ok(Redirect::to(&format!("/agents/{agent_key}/runs/{run_id}")).into_response())
        }
    }
}
pub(in crate::web::routes) async fn agents_update_sub_agent_model(
    State(state): State<Arc<AppState>>,
    Path((agent_key, sub_agent_id)): Path<(String, i64)>,
    headers: HeaderMap,
    Form(form): Form<ModelSelectionForm>,
) -> Result<Response, AppError> {
    let detail_url = format!("/agents/{agent_key}/sub-agents/{sub_agent_id}");
    let Some(_agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let Some(_job) =
        crate::harness::store::get_agent_sub_agent(&state.db_pool, &agent_key, sub_agent_id)
            .await?
    else {
        return Ok((StatusCode::NOT_FOUND, "sub-agent not found").into_response());
    };

    let parsed = match parse_model_selection(&form.model_selection) {
        Ok(parsed) => parsed,
        Err(message) => return Ok(model_error_response(&headers, &detail_url, &message)),
    };
    let validated =
        match validate_model_selection_for_agent(&state, parsed, &form.model_variant).await {
            Ok(validated) => validated,
            Err(message) => return Ok(model_error_response(&headers, &detail_url, &message)),
        };
    let model_provider_id = validated.as_ref().map(|(provider, _, _)| provider.as_str());
    let model_id = validated.as_ref().map(|(_, model, _)| model.as_str());
    let model_variant = validated
        .as_ref()
        .and_then(|(_, _, variant)| variant.as_deref());

    if !crate::harness::store::set_sub_agent_model_with_variant(
        &state.db_pool,
        &agent_key,
        sub_agent_id,
        model_provider_id,
        model_id,
        model_variant,
    )
    .await?
    {
        return Ok((StatusCode::NOT_FOUND, "sub-agent not found").into_response());
    }

    if is_htmx_request(&headers) {
        return Ok(htmx_redirect(&detail_url));
    }

    Ok(Redirect::to(&detail_url).into_response())
}

fn model_error_response(headers: &HeaderMap, detail_url: &str, message: &str) -> Response {
    let location = format!("{detail_url}?model_error={}", urlencode(message));
    if is_htmx_request(headers) {
        htmx_redirect(&location)
    } else {
        Redirect::to(&location).into_response()
    }
}

fn htmx_redirect(location: &str) -> Response {
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        HeaderName::from_static("hx-redirect"),
        HeaderValue::from_str(location)
            .expect("sub-agent detail redirect locations are valid headers"),
    );
    response
}
pub(in crate::web::routes) async fn agents_update_sub_agent_timeout(
    State(state): State<Arc<AppState>>,
    Path((agent_key, sub_agent_id)): Path<(String, i64)>,
    Form(form): Form<TimeoutForm>,
) -> Result<Response, AppError> {
    let detail_url = format!("/agents/{agent_key}/sub-agents/{sub_agent_id}");

    if crate::harness::store::get_agent_sub_agent(&state.db_pool, &agent_key, sub_agent_id)
        .await?
        .is_none()
    {
        return Ok((StatusCode::NOT_FOUND, "sub-agent not found").into_response());
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

    if !crate::harness::store::set_sub_agent_timeout(
        &state.db_pool,
        &agent_key,
        sub_agent_id,
        timeout_i32,
    )
    .await?
    {
        return Ok((StatusCode::NOT_FOUND, "sub-agent not found").into_response());
    }

    Ok(Redirect::to(&detail_url).into_response())
}

#[derive(Debug, Default, Deserialize)]
pub(in crate::web::routes) struct NotificationCapabilityForm {
    pub enabled: Option<String>,
}

pub(in crate::web::routes) async fn agents_update_sub_agent_notification_capability(
    State(state): State<Arc<AppState>>,
    Path((agent_key, sub_agent_id)): Path<(String, i64)>,
    Form(form): Form<NotificationCapabilityForm>,
) -> Result<Response, AppError> {
    let updated = crate::harness::store::set_sub_agent_notification_send_enabled(
        &state.db_pool,
        &agent_key,
        sub_agent_id,
        form.enabled.is_some(),
    )
    .await?;
    if !updated {
        return Ok((StatusCode::NOT_FOUND, "sub-agent not found").into_response());
    }
    let redirect_url =
        match crate::harness::store::get_agent_sub_agent(&state.db_pool, &agent_key, sub_agent_id)
            .await?
        {
            Some(job) => prompt_page_url(&agent_key, sub_agent_id, &job.sub_agent_kind),
            None => format!("/agents/{agent_key}/sub-agents/{sub_agent_id}"),
        };
    Ok(Redirect::to(&redirect_url).into_response())
}

#[derive(Debug, Default, Deserialize)]
pub(in crate::web::routes) struct ReviewPromptUpdateForm {
    pub enabled: Option<String>,
}

/// Toggle an Analysis job's opt-in to Review-driven prompt updates. The
/// capability lives in `enabled_capabilities`; only Analysis jobs accept it.
pub(in crate::web::routes) async fn agents_update_sub_agent_review_prompt_update(
    State(state): State<Arc<AppState>>,
    Path((agent_key, sub_agent_id)): Path<(String, i64)>,
    Form(form): Form<ReviewPromptUpdateForm>,
) -> Result<Response, AppError> {
    let Some(job) =
        crate::harness::store::get_agent_sub_agent(&state.db_pool, &agent_key, sub_agent_id)
            .await?
    else {
        return Ok((StatusCode::NOT_FOUND, "sub-agent not found").into_response());
    };
    if job.sub_agent_kind != SUB_AGENT_KIND_ANALYSIS {
        return Err(AppError(anyhow::anyhow!(
            "only analysis jobs can opt in to review prompt updates"
        )));
    }
    let mut capabilities = job.enabled_capabilities;
    capabilities.retain(|capability| capability != CAPABILITY_REVIEW_PROMPT_UPDATE);
    if form.enabled.is_some() {
        capabilities.push(CAPABILITY_REVIEW_PROMPT_UPDATE.to_string());
    }
    let updated = crate::harness::store::set_sub_agent_capabilities(
        &state.db_pool,
        &agent_key,
        sub_agent_id,
        capabilities,
    )
    .await?;
    if !updated {
        return Ok((StatusCode::NOT_FOUND, "sub-agent not found").into_response());
    }
    Ok(Redirect::to(&prompt_page_url(
        &agent_key,
        sub_agent_id,
        &job.sub_agent_kind,
    ))
    .into_response())
}

#[derive(Debug, Default, Deserialize)]
pub(in crate::web::routes) struct TimeframeForm {
    #[serde(default)]
    pub timeframe: String,
}

#[derive(Debug, Default, Deserialize)]
pub(in crate::web::routes) struct JobDetailQuery {
    #[serde(default)]
    pub page: String,
    #[serde(default)]
    pub timeout_error: Option<String>,
    #[serde(default)]
    pub timeframe_error: Option<String>,
    #[serde(default)]
    pub model_error: Option<String>,
    #[serde(default)]
    pub prompt_error: Option<String>,
    #[serde(default)]
    pub setup: bool,
}

pub(in crate::web::routes) async fn agents_update_sub_agent_timeframe(
    State(state): State<Arc<AppState>>,
    Path((agent_key, sub_agent_id)): Path<(String, i64)>,
    Form(form): Form<TimeframeForm>,
) -> Result<Response, AppError> {
    let detail_url = format!("/agents/{agent_key}/sub-agents/{sub_agent_id}");
    let timeframe = form.timeframe.trim();
    if let Err(error) = parse_timeframe_seconds(timeframe) {
        return Ok(timeframe_error_redirect(
            &detail_url,
            format!("Invalid timeframe: {error}"),
        ));
    }

    match crate::harness::store::set_candle_sub_agent_timeframe(
        &state.db_pool,
        &agent_key,
        sub_agent_id,
        timeframe,
    )
    .await
    {
        Ok(true) => Ok(Redirect::to(&detail_url).into_response()),
        Ok(false) => Ok((StatusCode::NOT_FOUND, "sub-agent not found").into_response()),
        Err(error) => match job_unique_violation_message(&error) {
            Some(message) => Ok(timeframe_error_redirect(&detail_url, message)),
            None => Err(AppError(error)),
        },
    }
}

fn timeframe_error_redirect(detail_url: &str, message: String) -> Response {
    Redirect::to(&format!(
        "{detail_url}?timeframe_error={}",
        urlencode(&message)
    ))
    .into_response()
}

#[derive(Debug, Default, Deserialize)]
pub(in crate::web::routes) struct SubAgentPromptForm {
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub base_revision_id: String,
}

/// Update a sub-agent's strategy prompt from its role page.
pub(in crate::web::routes) async fn agents_update_sub_agent_prompt(
    State(state): State<Arc<AppState>>,
    Path((agent_key, sub_agent_id)): Path<(String, i64)>,
    Form(form): Form<SubAgentPromptForm>,
) -> Result<Response, AppError> {
    let Some(_agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let Some(current) = get_agent_strategy_prompt(&state.db_pool, &agent_key, sub_agent_id)
        .await
        .map_err(AppError)?
    else {
        return Ok((StatusCode::NOT_FOUND, "strategy prompt not found").into_response());
    };
    let prompt = form.prompt.trim();
    if prompt.is_empty() {
        return Ok(prompt_error_redirect(
            &state,
            &agent_key,
            sub_agent_id,
            "Prompt must not be empty.".to_string(),
        )
        .await);
    }
    if prompt == current.prompt {
        return Ok(Redirect::to(&prompt_page_url(
            &agent_key,
            sub_agent_id,
            &current.target_sub_agent_kind,
        ))
        .into_response());
    }
    let base_revision_id: i64 = match form.base_revision_id.trim().parse() {
        Ok(value) if value > 0 => value,
        _ => {
            return Ok(prompt_error_redirect(
                &state,
                &agent_key,
                sub_agent_id,
                "Invalid base revision; reload the page and try again.".to_string(),
            )
            .await);
        }
    };
    if base_revision_id != current.revision_id {
        return Ok(prompt_error_redirect(
            &state,
            &agent_key,
            sub_agent_id,
            "The prompt changed since you loaded it. Review the latest revision and retry."
                .to_string(),
        )
        .await);
    }
    if let Err(error) = create_prompt_revision(
        &state.db_pool,
        &agent_key,
        sub_agent_id,
        prompt,
        base_revision_id,
        "manual",
        "",
    )
    .await
    {
        return Ok(
            prompt_error_redirect(&state, &agent_key, sub_agent_id, format!("{error:#}")).await,
        );
    }
    Ok(Redirect::to(&prompt_page_url(
        &agent_key,
        sub_agent_id,
        &current.target_sub_agent_kind,
    ))
    .into_response())
}

#[derive(Debug, Default, Deserialize)]
pub(in crate::web::routes) struct SubAgentPromptRollbackForm {
    #[serde(default)]
    pub revision_id: String,
}

pub(in crate::web::routes) async fn agents_rollback_sub_agent_prompt(
    State(state): State<Arc<AppState>>,
    Path((agent_key, sub_agent_id)): Path<(String, i64)>,
    Form(form): Form<SubAgentPromptRollbackForm>,
) -> Result<Response, AppError> {
    let Some(_agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let Ok(revision_id) = form.revision_id.trim().parse::<i64>() else {
        return Ok((StatusCode::NOT_FOUND, "revision not found").into_response());
    };
    if revision_id <= 0 {
        return Ok((StatusCode::NOT_FOUND, "revision not found").into_response());
    }
    let rolled_back = rollback_prompt_revision(&state.db_pool, &agent_key, revision_id).await?;
    if !rolled_back {
        return Ok((StatusCode::NOT_FOUND, "revision not found").into_response());
    }
    let Some(current) = get_agent_strategy_prompt(&state.db_pool, &agent_key, sub_agent_id)
        .await
        .map_err(AppError)?
    else {
        return Ok((StatusCode::NOT_FOUND, "strategy prompt not found").into_response());
    };
    Ok(Redirect::to(&prompt_page_url(
        &agent_key,
        sub_agent_id,
        &current.target_sub_agent_kind,
    ))
    .into_response())
}

async fn prompt_error_redirect(
    state: &Arc<AppState>,
    agent_key: &str,
    sub_agent_id: i64,
    message: String,
) -> Response {
    let prompt_page =
        match crate::harness::store::get_agent_sub_agent(&state.db_pool, agent_key, sub_agent_id)
            .await
            .ok()
            .flatten()
        {
            Some(job) => prompt_page_url(agent_key, sub_agent_id, &job.sub_agent_kind),
            None => role_list_redirect(state, agent_key, sub_agent_id).await,
        };
    Redirect::to(&format!(
        "{prompt_page}?prompt_error={}",
        urlencode(&message)
    ))
    .into_response()
}

fn prompt_page_url(agent_key: &str, sub_agent_id: i64, kind: &str) -> String {
    if kind == SUB_AGENT_KIND_ANALYSIS {
        format!("/agents/{agent_key}/sub-agents/{sub_agent_id}")
    } else {
        singleton_edit_page_url(agent_key, kind)
    }
}

fn singleton_edit_page_url(agent_key: &str, kind: &str) -> String {
    format!("{}/edit", role_page_url(agent_key, kind))
}

fn role_page_url(agent_key: &str, kind: &str) -> String {
    let role_page = match kind {
        SUB_AGENT_KIND_TRADING => "trading",
        SUB_AGENT_KIND_REVIEW => "review",
        _ => "analysis",
    };
    format!("/agents/{agent_key}/{role_page}")
}
pub(in crate::web::routes) fn job_unique_violation_message(
    error: &anyhow::Error,
) -> Option<String> {
    let db_err = error.downcast_ref::<sqlx::Error>()?.as_database_error()?;
    if !db_err.is_unique_violation() {
        return None;
    }

    let constraint = db_err.constraint().unwrap_or("unknown");
    if constraint.contains("harness_sub_agents_singleton_kind_idx") {
        Some("This sub-agent role already exists for this agent.".to_string())
    } else if constraint.contains("harness_sub_agents") || constraint.contains("sub_agent_key") {
        Some("A sub-agent with this key already exists for this agent.".to_string())
    } else {
        Some("This sub-agent conflicts with an existing row.".to_string())
    }
}

async fn load_strategy_prompt(
    state: &Arc<AppState>,
    agent_key: &str,
    sub_agent_id: i64,
) -> anyhow::Result<(String, i64)> {
    let prompt = get_agent_strategy_prompt(&state.db_pool, agent_key, sub_agent_id).await?;
    let stored_prompt = prompt
        .as_ref()
        .map(|row| row.prompt.clone())
        .unwrap_or_default();
    let revision = prompt.as_ref().map(|row| row.revision_id).unwrap_or(1);
    Ok((stored_prompt, revision))
}

#[derive(Debug, Default, Deserialize)]
pub(in crate::web::routes) struct RolePageQuery {
    #[serde(default)]
    pub page: String,
    #[serde(default)]
    pub prompt_error: Option<String>,
    #[serde(default)]
    pub timeout_error: Option<String>,
    #[serde(default)]
    pub model_error: Option<String>,
}

struct RolePageContext {
    role_label: &'static str,
    role_page_path: &'static str,
    role_description: &'static str,
    default_timeframe: &'static str,
    sub_agent_kind: &'static str,
    active_tab: AgentShowTab,
}

const TRADING_ROLE: RolePageContext = RolePageContext {
    role_label: "Trading",
    role_page_path: "/trading",
    role_description: "Reads the latest research context, records a trading decision for every evaluated instrument, and manages orders.",
    default_timeframe: store::DEFAULT_TRADING_TIMEFRAME,
    sub_agent_kind: SUB_AGENT_KIND_TRADING,
    active_tab: AgentShowTab::Trading,
};

const REVIEW_ROLE: RolePageContext = RolePageContext {
    role_label: "Review",
    role_page_path: "/review",
    role_description: "Reviews outcomes, records durable learnings, and may revise the prompts that opted in.",
    default_timeframe: store::DEFAULT_REVIEW_TIMEFRAME,
    sub_agent_kind: SUB_AGENT_KIND_REVIEW,
    active_tab: AgentShowTab::Review,
};

#[derive(Debug, Clone, Default, Deserialize)]
pub(in crate::web::routes) struct CreateSingletonRoleForm {
    #[serde(default)]
    pub timeframe: String,
    #[serde(default)]
    pub timeout_seconds: String,
    #[serde(default)]
    pub model_selection: String,
    #[serde(default)]
    pub model_variant: String,
    #[serde(default)]
    pub prompt: String,
    pub enabled: Option<String>,
}

#[derive(Debug)]
struct ValidatedCreateSingletonRole {
    timeframe: String,
    timeout_seconds: i32,
    model_selection: Option<(String, String)>,
    model_variant: String,
    prompt: String,
    enabled: bool,
}

impl CreateSingletonRoleForm {
    fn defaults(sub_agent_kind: &str, timeframe: &str) -> Self {
        Self {
            timeframe: timeframe.to_string(),
            timeout_seconds: "900".to_string(),
            prompt: crate::agents::strategy_prompts::default_prompt_for_role(sub_agent_kind)
                .to_string(),
            enabled: Some("on".to_string()),
            ..Self::default()
        }
    }

    fn enabled(&self) -> bool {
        self.enabled.is_some()
    }

    fn as_template_values(&self) -> CreateSingletonRoleFormValues {
        CreateSingletonRoleFormValues {
            timeframe: self.timeframe.clone(),
            timeout_seconds: self.timeout_seconds.clone(),
            model_selection: self.model_selection.clone(),
            model_variant: self.model_variant.clone(),
            prompt: self.prompt.clone(),
            enabled: self.enabled(),
        }
    }

    fn validate(&self) -> Result<ValidatedCreateSingletonRole, Vec<String>> {
        let mut errors = Vec::new();
        let timeframe = self.timeframe.trim().to_string();
        if parse_timeframe_seconds(&timeframe).is_err() {
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
            errors.push("A model is required to enable a sub-agent.".to_string());
        }
        let prompt = self.prompt.trim().to_string();
        if prompt.is_empty() {
            errors.push("Prompt must not be empty.".to_string());
        }

        if errors.is_empty() {
            Ok(ValidatedCreateSingletonRole {
                timeframe,
                timeout_seconds: timeout_seconds.expect("validated timeout seconds"),
                model_selection,
                model_variant: self.model_variant.clone(),
                prompt,
                enabled: self.enabled(),
            })
        } else {
            Err(errors)
        }
    }
}

fn render_new_singleton_role_form(
    agent: crate::agents::model::AgentDetailRow,
    context: &RolePageContext,
    form: CreateSingletonRoleFormValues,
    picker: super::shared::ModelPickerContext,
    errors: Vec<String>,
    status: StatusCode,
    navigation: AgentJobPageNavigation,
) -> Response {
    let mut model_picker = build_model_picker_view(
        "sub-agent-model-selection",
        &form.model_selection,
        (!form.model_variant.is_empty()).then_some(form.model_variant.as_str()),
        picker,
    );
    model_picker.submit_on_save = false;
    let role_page_path = format!("/agents/{}{}", agent.agent_key, context.role_page_path);
    let html = AgentSingletonRoleNewPageTemplate {
        tabs: build_agent_show_tabs(&agent, context.active_tab, navigation.notification_count),
        agent_tabs_use_htmx: false,
        current_path: format!("{role_page_path}/new"),
        agent,
        role_label: context.role_label,
        role_description: context.role_description,
        role_page_path,
        form,
        model_picker,
        errors,
        navbar: navigation.navbar,
    }
    .render()
    .expect("singleton role form template must render");
    (status, Html(html)).into_response()
}

async fn agents_new_singleton_role(
    state: Arc<AppState>,
    user: AuthenticatedUser,
    agent_key: String,
    context: &RolePageContext,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    if store::get_singleton_sub_agent(&state.db_pool, &agent_key, context.sub_agent_kind)
        .await?
        .is_some()
    {
        return Ok(
            Redirect::to(&format!("/agents/{agent_key}{}", context.role_page_path)).into_response(),
        );
    }
    let picker = load_model_picker_context(&state, &agent).await;
    let navbar = load_selected_agent_navbar(&state, user.id, &agent).await?;
    let notification_count = count_notifications(&state.db_pool, &agent.agent_key).await?;
    Ok(render_new_singleton_role_form(
        agent,
        context,
        CreateSingletonRoleForm::defaults(context.sub_agent_kind, context.default_timeframe)
            .as_template_values(),
        picker,
        Vec::new(),
        StatusCode::OK,
        AgentJobPageNavigation {
            notification_count,
            navbar,
        },
    ))
}

async fn agents_create_singleton_role(
    state: Arc<AppState>,
    user: AuthenticatedUser,
    agent_key: String,
    context: &RolePageContext,
    form: CreateSingletonRoleForm,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let navigation = AgentJobPageNavigation {
        notification_count: count_notifications(&state.db_pool, &agent.agent_key).await?,
        navbar: load_selected_agent_navbar(&state, user.id, &agent).await?,
    };
    let validated = match form.validate() {
        Ok(validated) => validated,
        Err(errors) => {
            let picker = load_model_picker_context(&state, &agent).await;
            return Ok(render_new_singleton_role_form(
                agent,
                context,
                form.as_template_values(),
                picker,
                errors,
                StatusCode::UNPROCESSABLE_ENTITY,
                navigation,
            ));
        }
    };
    let validated_model_selection = match validate_model_selection_for_agent(
        &state,
        validated.model_selection.clone(),
        &validated.model_variant,
    )
    .await
    {
        Ok(selection) => selection,
        Err(error) => {
            let picker = load_model_picker_context(&state, &agent).await;
            return Ok(render_new_singleton_role_form(
                agent,
                context,
                form.as_template_values(),
                picker,
                vec![error],
                StatusCode::UNPROCESSABLE_ENTITY,
                navigation,
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
    let inserted = match store::insert_singleton_sub_agent(
        &state.db_pool,
        &agent_key,
        context.sub_agent_kind,
        store::SingletonSubAgentConfig {
            enabled: validated.enabled,
            timeframe: &validated.timeframe,
            model_provider_id,
            model_id,
            model_variant,
            timeout_seconds: validated.timeout_seconds,
        },
    )
    .await
    {
        Ok(id) => id,
        Err(error) => {
            let errors = match job_unique_violation_message(&error) {
                Some(message) => vec![message],
                None => return Err(AppError(error)),
            };
            let picker = load_model_picker_context(&state, &agent).await;
            return Ok(render_new_singleton_role_form(
                agent,
                context,
                form.as_template_values(),
                picker,
                errors,
                StatusCode::UNPROCESSABLE_ENTITY,
                navigation,
            ));
        }
    };
    let (sub_agent_key,): (String,) =
        sqlx::query_as("SELECT sub_agent_key FROM harness_sub_agents WHERE id = $1")
            .bind(inserted)
            .fetch_one(&state.db_pool)
            .await?;
    if let Err(error) = seed_initial_prompt_revision(
        &state.db_pool,
        &agent_key,
        inserted,
        &sub_agent_key,
        &validated.prompt,
    )
    .await
    {
        warn!(
            agent_key,
            sub_agent_id = inserted,
            error = ?error,
            "failed to seed initial prompt for singleton role"
        );
    }
    Ok(Redirect::to(&format!("/agents/{agent_key}{}", context.role_page_path)).into_response())
}

async fn render_role_page(
    state: &Arc<AppState>,
    user: &AuthenticatedUser,
    agent_key: &str,
    context: &RolePageContext,
    query: RolePageQuery,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let job =
        store::get_singleton_sub_agent(&state.db_pool, agent_key, context.sub_agent_kind).await?;
    let notification_count = count_notifications(&state.db_pool, &agent.agent_key).await?;
    let navbar = load_selected_agent_navbar(state, user.id, &agent).await?;
    let tabs = build_agent_show_tabs(&agent, context.active_tab, notification_count);
    let role_page_path = format!("/agents/{agent_key}{}", context.role_page_path);

    let Some(job) = job else {
        let html = crate::web::templates::AgentRolePageTemplate {
            current_path: role_page_path.clone(),
            agent,
            tabs,
            agent_tabs_use_htmx: true,
            active_role_label: context.role_label,
            role_page_path: role_page_path.clone(),
            job: None,
            recent_runs_section: build_agent_recent_runs_view_for_kind(
                state,
                agent_key,
                context.sub_agent_kind,
                &role_page_path,
                1,
            )
            .await,
            navbar,
        }
        .render()?;
        return Ok(Html(html).into_response());
    };

    let mut job_view = crate::web::templates::HarnessSubAgentDetailView::from_row(&job);
    if let Some(error) = query.timeout_error {
        job_view.timeout_editor.error = Some(error);
    }
    if let Some(error) = query.model_error {
        job_view.model_error = Some(error);
    }

    let requested_runs_page = parse_positive_page(&query.page);
    let recent_runs_section = build_agent_recent_runs_view_for_kind(
        state,
        agent_key,
        context.sub_agent_kind,
        &role_page_path,
        requested_runs_page,
    )
    .await;

    let html = crate::web::templates::AgentRolePageTemplate {
        current_path: role_page_path.clone(),
        agent,
        tabs,
        agent_tabs_use_htmx: true,
        active_role_label: context.role_label,
        role_page_path,
        job: Some(job_view),
        recent_runs_section,
        navbar,
    }
    .render()?;
    Ok(Html(html).into_response())
}

async fn render_role_edit_page(
    state: &Arc<AppState>,
    user: &AuthenticatedUser,
    agent_key: &str,
    context: &RolePageContext,
    query: RolePageQuery,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let Some(job) =
        store::get_singleton_sub_agent(&state.db_pool, agent_key, context.sub_agent_kind).await?
    else {
        return Ok(
            Redirect::to(&format!("/agents/{agent_key}{}", context.role_page_path)).into_response(),
        );
    };

    let prompt_view = match get_agent_strategy_prompt(&state.db_pool, agent_key, job.id).await {
        Ok(Some(row)) => crate::web::templates::RolePromptView {
            revision_id: row.revision_id,
            prompt: row.prompt,
            prompt_error: query.prompt_error,
        },
        Ok(None) => crate::web::templates::RolePromptView {
            prompt_error: query.prompt_error,
            ..Default::default()
        },
        Err(error) => {
            warn!(
                agent_key,
                error = ?error,
                "failed to load singleton strategy prompt for edit page"
            );
            crate::web::templates::RolePromptView {
                prompt_error: Some("Strategy prompt could not be loaded.".to_string()),
                ..Default::default()
            }
        }
    };
    let notification_count = count_notifications(&state.db_pool, &agent.agent_key).await?;
    let navbar = load_selected_agent_navbar(state, user.id, &agent).await?;
    let role_page_path = format!("/agents/{agent_key}{}", context.role_page_path);
    let tabs = build_agent_show_tabs(&agent, context.active_tab, notification_count);
    let html = AgentRoleEditPageTemplate {
        current_path: format!("{role_page_path}/edit"),
        agent,
        tabs,
        agent_tabs_use_htmx: false,
        active_role_label: context.role_label,
        role_page_path,
        job: crate::web::templates::HarnessSubAgentDetailView::from_row(&job),
        prompt_view,
        navbar,
    }
    .render()?;
    Ok(Html(html).into_response())
}

pub(in crate::web::routes) async fn agents_show_trading(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path(agent_key): Path<String>,
    Query(query): Query<RolePageQuery>,
) -> Result<Response, AppError> {
    render_role_page(&state, &user, &agent_key, &TRADING_ROLE, query).await
}

pub(in crate::web::routes) async fn agents_show_review(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path(agent_key): Path<String>,
    Query(query): Query<RolePageQuery>,
) -> Result<Response, AppError> {
    render_role_page(&state, &user, &agent_key, &REVIEW_ROLE, query).await
}

pub(in crate::web::routes) async fn agents_edit_trading_singleton(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path(agent_key): Path<String>,
    Query(query): Query<RolePageQuery>,
) -> Result<Response, AppError> {
    render_role_edit_page(&state, &user, &agent_key, &TRADING_ROLE, query).await
}

pub(in crate::web::routes) async fn agents_edit_review_singleton(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path(agent_key): Path<String>,
    Query(query): Query<RolePageQuery>,
) -> Result<Response, AppError> {
    render_role_edit_page(&state, &user, &agent_key, &REVIEW_ROLE, query).await
}

pub(in crate::web::routes) async fn agents_new_trading_singleton(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    agents_new_singleton_role(state, user, agent_key, &TRADING_ROLE).await
}

pub(in crate::web::routes) async fn agents_new_review_singleton(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    agents_new_singleton_role(state, user, agent_key, &REVIEW_ROLE).await
}

pub(in crate::web::routes) async fn agents_create_trading_singleton(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path(agent_key): Path<String>,
    Form(form): Form<CreateSingletonRoleForm>,
) -> Result<Response, AppError> {
    agents_create_singleton_role(state, user, agent_key, &TRADING_ROLE, form).await
}

pub(in crate::web::routes) async fn agents_create_review_singleton(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path(agent_key): Path<String>,
    Form(form): Form<CreateSingletonRoleForm>,
) -> Result<Response, AppError> {
    agents_create_singleton_role(state, user, agent_key, &REVIEW_ROLE, form).await
}
