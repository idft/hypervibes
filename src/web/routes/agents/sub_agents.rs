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
    ModelPickerContext, ModelSelectionForm, SERVER_SHUTTING_DOWN_WARNING, TimeoutForm,
    ToggleJobForm, WORKSPACE_MAINTENANCE_ACTIVE_WARNING, build_model_picker_view, is_htmx_request,
    load_model_picker_context, parse_positive_job_seconds, sub_agents_warning_redirect,
    timeout_error_redirect, validate_model_selection_for_agent,
};
use super::show::{
    AgentShowQueries, AgentSubAgentsQuery, build_agent_recent_runs_view,
    load_selected_agent_navbar, parse_positive_page, render_agent_show_page,
};
use crate::web::error::AppError;
use crate::{
    agents::{
        store::get_agent,
        strategy_prompts::{get_agent_strategy_prompt, prompt_kind_for_sub_agent_kind},
    },
    harness::{
        model::{
            SUB_AGENT_KIND_ANALYSIS, SUB_AGENT_KIND_ANALYSIS_CODING, SUB_AGENT_KIND_DAILY_REVIEW,
            SUB_AGENT_KIND_MARKET_ANALYSIS, SUB_AGENT_KIND_TRADING,
        },
        scheduler::{
            DispatchRequestInputs, build_dispatch_request, dispatch_analysis_batch_completed_event,
            dispatch_daily_review_coding_event, dispatch_request_from_job,
            dispatch_run_with_workspace_lease,
        },
        store::{self, QueuedSubAgentRun},
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
            AgentShowTab, CreateHarnessSubAgentFormValues, ModelPickerPartialTemplate,
            build_agent_show_tabs,
        },
    },
};
pub(in crate::web::routes) async fn agents_show_sub_agents(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path(agent_key): Path<String>,
    Query(query): Query<AgentSubAgentsQuery>,
) -> Result<Response, AppError> {
    render_agent_show_page(
        &state,
        &user,
        &agent_key,
        AgentShowTab::SubAgents,
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
    model_picker.use_modal = true;
    let navbar = load_selected_agent_navbar(&state, user.id, &agent).await?.0;
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
        navbar,
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
    model_picker.use_modal = true;
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
        SUB_AGENT_KIND_ANALYSIS
            | SUB_AGENT_KIND_MARKET_ANALYSIS
            | crate::harness::model::SUB_AGENT_KIND_TRADING
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

async fn load_strategy_prompt(
    state: &Arc<AppState>,
    agent_key: &str,
    sub_agent_kind: &str,
) -> anyhow::Result<String> {
    let prompt_kind = prompt_kind_for_sub_agent_kind(sub_agent_kind)
        .ok_or_else(|| anyhow::anyhow!("unknown job kind {sub_agent_kind}"))?;
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
pub(in crate::web::routes) struct CreateHarnessSubAgentForm {
    #[serde(default)]
    pub sub_agent_kind: String,
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
pub(in crate::web::routes) struct ValidatedCreateHarnessSubAgent {
    pub sub_agent_kind: String,
    pub timeframe: String,
    pub trigger_delay_seconds: i32,
    pub timeout_seconds: i32,
    pub model_selection: Option<(String, String)>,
    pub model_variant: String,
    pub operator_prompt: String,
    pub enabled: bool,
}
impl CreateHarnessSubAgentForm {
    fn defaults() -> Self {
        Self {
            sub_agent_kind: SUB_AGENT_KIND_ANALYSIS.to_string(),
            timeframe: "15m".to_string(),
            timeout_seconds: "900".to_string(),
            enabled: Some("on".to_string()),
            ..Self::default()
        }
    }

    fn enabled(&self) -> bool {
        self.enabled.is_some()
    }

    fn as_template_values(&self) -> CreateHarnessSubAgentFormValues {
        CreateHarnessSubAgentFormValues {
            sub_agent_kind: self.sub_agent_kind.clone(),
            timeframe: self.timeframe.clone(),
            timeout_seconds: self.timeout_seconds.clone(),
            model_selection: self.model_selection.clone(),
            model_variant: self.model_variant.clone(),
            operator_prompt: self.operator_prompt.clone(),
            enabled: self.enabled(),
        }
    }

    fn validate(&self) -> Result<ValidatedCreateHarnessSubAgent, Vec<String>> {
        let mut errors = Vec::new();

        let sub_agent_kind = self.sub_agent_kind.trim();
        if !is_supported_sub_agent_kind(sub_agent_kind) {
            errors.push("Choose a supported sub-agent type.".to_string());
            return Err(errors);
        }
        let candle = is_candle_sub_agent_kind(sub_agent_kind);

        let timeframe = if candle {
            self.timeframe.trim().to_string()
        } else {
            String::new()
        };
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
            errors.push("A model is required to enable a sub-agent.".to_string());
        }

        if errors.is_empty() {
            Ok(ValidatedCreateHarnessSubAgent {
                sub_agent_kind: sub_agent_kind.to_string(),
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

fn is_supported_sub_agent_kind(sub_agent_kind: &str) -> bool {
    matches!(
        sub_agent_kind,
        SUB_AGENT_KIND_ANALYSIS
            | SUB_AGENT_KIND_TRADING
            | SUB_AGENT_KIND_DAILY_REVIEW
            | SUB_AGENT_KIND_MARKET_ANALYSIS
            | SUB_AGENT_KIND_ANALYSIS_CODING
    )
}

fn is_candle_sub_agent_kind(sub_agent_kind: &str) -> bool {
    matches!(
        sub_agent_kind,
        SUB_AGENT_KIND_ANALYSIS | SUB_AGENT_KIND_TRADING | SUB_AGENT_KIND_DAILY_REVIEW
    )
}

#[derive(Debug, Clone, Copy)]
pub(in crate::web::routes) struct NewJobKindAvailability {
    market_analysis: bool,
    analysis_coding: bool,
}

impl NewJobKindAvailability {
    fn singleton_exists(self, sub_agent_kind: &str) -> bool {
        match sub_agent_kind {
            SUB_AGENT_KIND_MARKET_ANALYSIS => !self.market_analysis,
            SUB_AGENT_KIND_ANALYSIS_CODING => !self.analysis_coding,
            _ => false,
        }
    }
}

async fn load_new_sub_agent_kind_availability(
    state: &Arc<AppState>,
    agent_key: &str,
) -> anyhow::Result<NewJobKindAvailability> {
    let jobs = store::list_agent_sub_agents(&state.db_pool, agent_key).await?;
    Ok(NewJobKindAvailability {
        market_analysis: !jobs
            .iter()
            .any(|job| job.sub_agent_kind == SUB_AGENT_KIND_MARKET_ANALYSIS),
        analysis_coding: !jobs
            .iter()
            .any(|job| job.sub_agent_kind == SUB_AGENT_KIND_ANALYSIS_CODING),
    })
}

fn singleton_job_exists_message(sub_agent_kind: &str) -> Option<String> {
    match sub_agent_kind {
        SUB_AGENT_KIND_MARKET_ANALYSIS => {
            Some("A market analysis sub-agent already exists for this agent.".to_string())
        }
        SUB_AGENT_KIND_ANALYSIS_CODING => {
            Some("An analysis coding sub-agent already exists for this agent.".to_string())
        }
        _ => None,
    }
}

pub(in crate::web::routes) async fn agents_new_sub_agent(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let picker = load_model_picker_context(&state, &agent).await;
    let availability = load_new_sub_agent_kind_availability(&state, &agent_key).await?;
    let navbar = load_selected_agent_navbar(&state, user.id, &agent).await?.0;
    Ok(render_new_job_form(
        agent,
        CreateHarnessSubAgentForm::defaults().as_template_values(),
        picker,
        availability,
        Vec::new(),
        StatusCode::OK,
        navbar,
    ))
}
pub(in crate::web::routes) async fn agents_create_sub_agent(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path(agent_key): Path<String>,
    Form(form): Form<CreateHarnessSubAgentForm>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let navbar = load_selected_agent_navbar(&state, user.id, &agent).await?.0;
    let availability = load_new_sub_agent_kind_availability(&state, &agent_key).await?;
    let validated = match form.validate() {
        Ok(validated) => validated,
        Err(errors) => {
            let picker = load_model_picker_context(&state, &agent).await;
            return Ok(render_new_job_form(
                agent,
                form.as_template_values(),
                picker,
                availability,
                errors,
                StatusCode::UNPROCESSABLE_ENTITY,
                navbar.clone(),
            ));
        }
    };

    if availability.singleton_exists(&validated.sub_agent_kind) {
        let picker = load_model_picker_context(&state, &agent).await;
        return Ok(render_new_job_form(
            agent,
            form.as_template_values(),
            picker,
            availability,
            vec![
                singleton_job_exists_message(&validated.sub_agent_kind)
                    .expect("only singleton jobs can be unavailable"),
            ],
            StatusCode::UNPROCESSABLE_ENTITY,
            navbar.clone(),
        ));
    }

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
                availability,
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
    let result = if is_candle_sub_agent_kind(&validated.sub_agent_kind) {
        crate::harness::store::insert_candle_sub_agent_with_model_variant(
            &state.db_pool,
            &agent_key,
            &validated.sub_agent_kind,
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
        crate::harness::store::insert_unscheduled_sub_agent_with_model_variant(
            &state.db_pool,
            &agent_key,
            &validated.sub_agent_kind,
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
        let singleton_message = singleton_job_exists_message(&validated.sub_agent_kind);
        let errors = match job_unique_violation_message(&error) {
            Some(_) if singleton_message.is_some() => {
                vec![singleton_message.expect("singleton conflict message must exist")]
            }
            Some(message) => vec![message],
            None => return Err(AppError(error)),
        };
        let picker = load_model_picker_context(&state, &agent).await;
        return Ok(render_new_job_form(
            agent,
            form.as_template_values(),
            picker,
            availability,
            errors,
            StatusCode::UNPROCESSABLE_ENTITY,
            navbar,
        ));
    }

    Ok(Redirect::to(&format!("/agents/{agent_key}/sub-agents")).into_response())
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

    Ok(Redirect::to(&format!("/agents/{agent_key}/sub-agents")).into_response())
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

    Ok(Redirect::to(&format!("/agents/{agent_key}/sub-agents")).into_response())
}
pub(in crate::web::routes) async fn agents_delete_sub_agent(
    State(state): State<Arc<AppState>>,
    Path((agent_key, sub_agent_id)): Path<(String, i64)>,
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
        sub_agent_id,
    )
    .await?
    {
        crate::harness::service::DeleteJobOutcome::Deleted => {}
        crate::harness::service::DeleteJobOutcome::Missing => {
            return Ok((StatusCode::NOT_FOUND, "sub-agent not found").into_response());
        }
    }

    Ok(Redirect::to(&format!("/agents/{agent_key}/sub-agents")).into_response())
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

    if job.sub_agent_kind == crate::harness::model::SUB_AGENT_KIND_ANALYSIS_CODING {
        return match store::insert_analysis_coding_task_and_run(
            &state.db_pool,
            store::AnalysisCodingTaskRequest {
                agent_key: &agent_key,
                sub_agent_id,
                trigger_mode: store::CodingTriggerMode::Manual,
                request_origin: "manual",
                source_sub_agent_run_id: None,
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
            store::InsertAnalysisCodingTaskOutcome::AlreadyQueued => Ok(
                sub_agents_warning_redirect(&agent_key, "Analysis coding is already queued."),
            ),
            store::InsertAnalysisCodingTaskOutcome::BlockedByMaintenance => Ok(
                sub_agents_warning_redirect(&agent_key, WORKSPACE_MAINTENANCE_ACTIVE_WARNING),
            ),
        };
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
            let account_snapshot =
                if job.sub_agent_kind == crate::harness::model::SUB_AGENT_KIND_TRADING {
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
                    strategy_prompt: load_strategy_prompt(&state, &agent_key, &job.sub_agent_kind)
                        .await?,
                    accumulated_learnings: load_accumulated_learnings(&state, &agent_key).await?,
                    system_prompt,
                },
                account_snapshot,
            );
            if job.sub_agent_kind == crate::harness::model::SUB_AGENT_KIND_DAILY_REVIEW {
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
            let trigger_analysis_event = job.sub_agent_kind == SUB_AGENT_KIND_ANALYSIS;
            let trigger_coding_event =
                job.sub_agent_kind == crate::harness::model::SUB_AGENT_KIND_DAILY_REVIEW;
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
        QueuedSubAgentRun::Missing => {
            Ok((StatusCode::NOT_FOUND, "sub-agent not found").into_response())
        }
        QueuedSubAgentRun::Skipped { run_id } => {
            Ok(Redirect::to(&format!("/agents/{agent_key}/runs/{run_id}")).into_response())
        }
        QueuedSubAgentRun::BlockedByMaintenance => Ok(sub_agents_warning_redirect(
            &agent_key,
            WORKSPACE_MAINTENANCE_ACTIVE_WARNING,
        )),
    }
}
pub(in crate::web::routes) async fn agents_update_sub_agent_model(
    State(state): State<Arc<AppState>>,
    Path((agent_key, sub_agent_id)): Path<(String, i64)>,
    headers: HeaderMap,
    Form(form): Form<ModelSelectionForm>,
) -> Result<Response, AppError> {
    let detail_url = format!("/agents/{agent_key}/sub-agents/{sub_agent_id}");
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
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
        match validate_model_selection_for_agent(&state, &agent, parsed, &form.model_variant).await
        {
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
    let location = format!(
        "{detail_url}?model_error={}",
        super::shared::urlencode(message)
    );
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
pub(in crate::web::routes) struct OperatorPromptForm {
    #[serde(default)]
    pub operator_prompt: String,
}

pub(in crate::web::routes) async fn agents_update_sub_agent_operator_prompt(
    State(state): State<Arc<AppState>>,
    Path((agent_key, sub_agent_id)): Path<(String, i64)>,
    Form(form): Form<OperatorPromptForm>,
) -> Result<Response, AppError> {
    let detail_url = format!("/agents/{agent_key}/sub-agents/{sub_agent_id}");
    if !crate::harness::store::set_sub_agent_operator_prompt(
        &state.db_pool,
        &agent_key,
        sub_agent_id,
        form.operator_prompt.trim(),
    )
    .await?
    {
        return Ok((StatusCode::NOT_FOUND, "sub-agent not found").into_response());
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
    pub page: String,
    #[serde(default)]
    pub timeout_error: Option<String>,
    #[serde(default)]
    pub timeframe_error: Option<String>,
    #[serde(default)]
    pub model_error: Option<String>,
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
        super::shared::urlencode(&message)
    ))
    .into_response()
}
pub(in crate::web::routes) fn render_new_job_form(
    agent: crate::agents::model::AgentDetailRow,
    form: CreateHarnessSubAgentFormValues,
    picker: ModelPickerContext,
    availability: NewJobKindAvailability,
    errors: Vec<String>,
    status: StatusCode,
    navbar: crate::web::templates::Navbar,
) -> Response {
    let current_path = format!("/agents/{}/sub-agents/new", agent.agent_key);
    let mut model_picker = build_model_picker_view(
        "sub-agent-model-selection",
        &form.model_selection,
        (!form.model_variant.trim().is_empty()).then_some(form.model_variant.as_str()),
        picker,
    );
    model_picker.use_modal = true;
    model_picker.submit_on_save = false;
    let tabs = build_agent_show_tabs(&agent, AgentShowTab::SubAgents);
    let navbar = navbar.with_selected_agent(
        agent.agent_key.clone(),
        agent.display_name.clone(),
        agent.enabled,
    );
    let show_timeframe = is_candle_sub_agent_kind(&form.sub_agent_kind);
    let template = AgentJobNewPageTemplate {
        agent,
        tabs,
        agent_tabs_use_htmx: false,
        form,
        model_picker,
        market_analysis_available: availability.market_analysis,
        analysis_coding_available: availability.analysis_coding,
        show_timeframe,
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
    if constraint.contains("harness_sub_agents") || constraint.contains("sub_agent_key") {
        Some("A sub-agent with this type and timeframe already exists for this agent.".to_string())
    } else {
        Some("This sub-agent conflicts with an existing row.".to_string())
    }
}
