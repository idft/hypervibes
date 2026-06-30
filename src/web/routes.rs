use std::{convert::Infallible, sync::Arc};

use askama::Template;
use axum::{
    Form, Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{
        Html, IntoResponse, Redirect, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use chrono::Utc;
use futures::StreamExt;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde::Serialize;
use tokio_stream::wrappers::BroadcastStream;
use tracing::{error, warn};

use crate::{
    agentic::{
        model::{
            HOOK_EVENT_ANALYSIS_BATCH_COMPLETED, JOB_KIND_ANALYSIS, JOB_KIND_MARKET_ANALYSIS,
            JOB_KIND_TRADING,
        },
        scheduler::{
            build_hook_dispatch_request, dispatch_analysis_batch_completed_hook,
            dispatch_request_from_schedule, dispatch_run,
        },
        store::{QueuedHookRun, QueuedScheduleRun},
        timeframe::parse_timeframe_seconds,
    },
    agents::{
        crypto::{encrypt, generate_api_key},
        keys::derive_wallet_address,
        model::{
            AgentRegistryRow, AgentRuntimeRow, BACKEND_KIND_OPENCODE, CreateAgentForm,
            CreateAgentRuntimeForm, slugify_agent_key,
        },
        prompts::{DEFAULT_ANALYSIS_STRATEGY_PROMPT, DEFAULT_TRADING_STRATEGY_PROMPT},
        store::{
            delete_agent as delete_agent_in_store, get_agent, insert_agent, insert_agent_runtime,
            list_agent_instrument_options, list_agent_runtimes, list_agents,
            list_enabled_agent_runtimes, replace_agent_instruments, runtime_matches_backend,
            update_agent_analysis_prompt, update_agent_runtime_config, update_agent_trading_prompt,
        },
    },
    hermes::HermesHealth,
    hyperliquid::{
        live_state::{
            AccountKey, AccountLiveState, LiveConnectionStatus, live_agent_snapshot_for_dispatch,
        },
        queries::{
            AccountTransactionRow, BalanceSeriesBucket, fetch_balance_series,
            list_account_sync_state, list_all_account_transactions,
        },
    },
    memory::{
        get_latest_agent_memory_by_type, get_memory as get_memory_record, list_agent_memories,
        memory_expires_at,
    },
    opencode::workspace::{
        OpenCodeWorkspaceAgent, OpenCodeWorkspaceRuntimeConfig, generate_agent_workspace,
        runtime_config_for_generated_workspace,
    },
    web::{
        AppState,
        templates::{
            AccountBalancePartialTemplate, AccountBalanceView, AgentHookDetailPageTemplate,
            AgentHookNewPageTemplate, AgentJobDetailPageTemplate, AgentListEntry,
            AgentMemoryDetailPageTemplate, AgentMemoryDetailPartialTemplate,
            AgentMemoryTimelinePartialTemplate, AgentRunDetailPageTemplate,
            AgentScheduleNewPageTemplate, AgentShowTab, AgentsNewPageTemplate, AgentsPageTemplate,
            AgentsShowPageTemplate, BackendsNewPageTemplate, BackendsPageTemplate,
            BalanceSparklinesPartialTemplate, CreateAgentHookFormValues,
            CreateAgentScheduleFormValues, HermesPageTemplate,
            LatestAnalysisSummaryPartialTemplate, LatestTradeExecutionSummaryPartialTemplate,
            MemoryView, OpenCodeWorkspaceSettingsView, OpenOrdersPartialTemplate, OpenOrdersView,
            OpenPositionsPartialTemplate, OpenPositionsView, ServerErrorPageTemplate,
            SettingsPageTemplate, SparklineView, SyncStateView, TransactionView,
        },
        ui_events::UiEvent,
    },
};

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(root))
        .route("/healthz", get(healthz))
        .route("/agents", get(agents_index).post(create_agent))
        .route("/agents/new", get(agents_new))
        .route("/backends", get(backends_index).post(create_backend))
        .route("/backends/new", get(backends_new))
        .route("/agents/{agent_key}", get(agents_show))
        .route(
            "/agents/{agent_key}/transactions",
            get(agents_show_transactions),
        )
        .route("/agents/{agent_key}/memories", get(agents_show_memories))
        .route(
            "/agents/{agent_key}/memories/stream",
            get(agent_memories_stream),
        )
        .route(
            "/agents/{agent_key}/memories/{memory_id}",
            get(agents_show_memory_detail),
        )
        .route("/agents/{agent_key}/prompts", get(agents_show_prompts))
        .route(
            "/agents/{agent_key}/prompts/analysis",
            post(agents_update_analysis_prompt),
        )
        .route(
            "/agents/{agent_key}/prompts/trading",
            post(agents_update_trading_prompt),
        )
        .route("/agents/{agent_key}/settings", get(agents_show_settings))
        .route(
            "/agents/{agent_key}/settings/instruments",
            post(agents_update_instruments),
        )
        .route(
            "/agents/{agent_key}/settings/regenerate-workspace",
            post(agents_regenerate_workspace),
        )
        .route(
            "/agents/{agent_key}/jobs",
            get(agents_show_jobs).post(agents_create_job),
        )
        .route("/agents/{agent_key}/jobs/new", get(agents_new_job))
        .route(
            "/agents/{agent_key}/jobs/{job_id}",
            get(agents_show_job_detail),
        )
        .route(
            "/agents/{agent_key}/hooks/{hook_id}",
            get(agents_show_hook_detail),
        )
        .route(
            "/agents/{agent_key}/jobs/{job_id}/toggle",
            post(agents_toggle_job),
        )
        .route(
            "/agents/{agent_key}/jobs/{job_id}/delete",
            post(agents_delete_job),
        )
        .route(
            "/agents/{agent_key}/jobs/{job_id}/run",
            post(agents_run_job_now),
        )
        .route("/agents/{agent_key}/hooks/new", get(agents_new_hook))
        .route("/agents/{agent_key}/hooks", post(agents_create_hook))
        .route(
            "/agents/{agent_key}/hooks/{hook_id}/run",
            post(agents_run_hook_now),
        )
        .route(
            "/agents/{agent_key}/hooks/{hook_id}/toggle",
            post(agents_toggle_hook),
        )
        .route(
            "/agents/{agent_key}/hooks/{hook_id}/delete",
            post(agents_delete_hook),
        )
        .route(
            "/agents/{agent_key}/runs/{run_id}",
            get(agents_show_run_detail),
        )
        .route("/agents/{agent_key}/delete", post(delete_agent))
        .route("/agents/{agent_key}/live/stream", get(agent_live_stream))
        .route("/hermes", get(hermes_page))
        .route("/settings", get(settings_index).post(settings_update))
        .with_state(state)
}

async fn root() -> Redirect {
    Redirect::to("/agents")
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
}

async fn healthz(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let _ = &state.db_pool;
    (StatusCode::OK, Json(HealthResponse { status: "ok" }))
}

async fn agents_index(State(state): State<Arc<AppState>>) -> Result<Html<String>, AppError> {
    let agents = list_agents(&state.db_pool).await?;

    let entries: Vec<AgentListEntry> = agents
        .into_iter()
        .map(|row| {
            let account_key = AccountKey::new(&row.wallet_address, &row.environment);
            let snapshot =
                state
                    .live_accounts
                    .get(&account_key)
                    .unwrap_or_else(|| AccountLiveState {
                        account_address: account_key.account_address.clone(),
                        environment: account_key.environment.clone(),
                        status: LiveConnectionStatus::Starting,
                        ..Default::default()
                    });
            let account_balance = AccountBalanceView::from_live_state(snapshot);
            let api_key_last_used_iso = row
                .api_key_last_used_at
                .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true));
            AgentListEntry {
                row,
                account_balance,
                api_key_last_used_iso,
            }
        })
        .collect();

    let template = AgentsPageTemplate {
        agents: entries,
        current_path: "/agents".to_string(),
    };

    Ok(Html(template.render()?))
}

async fn agents_new(State(state): State<Arc<AppState>>) -> Result<Html<String>, AppError> {
    let runtimes = list_enabled_agent_runtimes(&state.db_pool).await?;
    let template = AgentsNewPageTemplate {
        form: CreateAgentForm {
            enabled: Some("on".to_string()),
            ..Default::default()
        },
        runtimes,
        errors: Vec::new(),
        current_path: "/agents/new".to_string(),
    };
    Ok(Html(template.render()?))
}

async fn backends_index(State(state): State<Arc<AppState>>) -> Result<Html<String>, AppError> {
    let template = BackendsPageTemplate {
        runtimes: list_agent_runtimes(&state.db_pool).await?,
        current_path: "/backends".to_string(),
    };
    Ok(Html(template.render()?))
}

async fn backends_new() -> Result<Html<String>, AppError> {
    let template = BackendsNewPageTemplate {
        form: CreateAgentRuntimeForm {
            backend_kind: crate::agents::model::BACKEND_KIND_HERMES.to_string(),
            enabled: Some("on".to_string()),
            ..Default::default()
        },
        errors: Vec::new(),
        current_path: "/backends/new".to_string(),
    };
    Ok(Html(template.render()?))
}

async fn create_backend(
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

async fn agents_show(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    render_agent_show_page(&state, &agent_key, AgentShowTab::Positions, None).await
}

async fn agents_show_transactions(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    render_agent_show_page(&state, &agent_key, AgentShowTab::Transactions, None).await
}

#[derive(Debug, Default, Deserialize)]
struct AgentMemoriesQuery {
    #[serde(default)]
    date: String,
}

async fn agents_show_memories(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
    Query(query): Query<AgentMemoriesQuery>,
) -> Result<Response, AppError> {
    render_agent_show_page(&state, &agent_key, AgentShowTab::Memories, Some(query)).await
}

async fn agents_show_memory_detail(
    State(state): State<Arc<AppState>>,
    Path((agent_key, memory_id)): Path<(String, uuid::Uuid)>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    tracing::info!(agent_key = %agent.agent_key, "opened memories SSE stream");

    let Some(memory) = get_memory_record(&state.db_pool, &agent.agent_key, memory_id).await? else {
        return Ok((StatusCode::NOT_FOUND, "memory not found").into_response());
    };

    let memory_view = MemoryView::from_record(memory);

    // HTMX in-place swap (from the Memories tab) only needs the bare partial.
    // Direct browser navigation gets a full styled page so the user sees the
    // agent context and a back link instead of unstyled HTML.
    if is_htmx_request(&headers) {
        let html = AgentMemoryDetailPartialTemplate::render_view(memory_view)?;
        return Ok(Html(html).into_response());
    }

    let memory_detail_html = AgentMemoryDetailPartialTemplate::render_view(memory_view.clone())?;
    let html =
        AgentMemoryDetailPageTemplate::render_view(agent.clone(), memory_view, memory_detail_html)?;
    Ok(Html(html).into_response())
}

fn is_htmx_request(headers: &HeaderMap) -> bool {
    headers
        .get("HX-Request")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

async fn agent_memories_stream(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
    Query(query): Query<AgentMemoriesQuery>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };

    let (_, selected_date_text, _, since, until) = parse_memory_date_filter(&query.date);
    let db_pool = state.db_pool.clone();
    let agent_key = agent.agent_key.clone();
    let agent_key_for_render = agent_key.clone();
    let selected_date_text_filter = selected_date_text.clone();

    let notifications = BroadcastStream::new(state.ui_events.subscribe())
        .filter_map(move |item| {
            let agent_key = agent_key.clone();
            async move {
                match item {
                    Ok(UiEvent::MemoryCreated { agent_key: event_agent_key, memory_id })
                        if event_agent_key == agent_key =>
                    {
                        tracing::info!(agent_key = %agent_key, memory_id = %memory_id, "memories SSE received memory event");
                        Some(())
                    }
                    Ok(_) => None,
                    Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(_)) => {
                        Some(())
                    }
                }
            }
        })
        .filter_map(move |_| {
            let db_pool = db_pool.clone();
            let agent_key = agent_key_for_render.clone();
            let selected_date_text = selected_date_text_filter.clone();
            async move {
                match render_memory_timeline_event(&db_pool, &agent_key, since, until, selected_date_text)
                    .await
                {
                    Ok(event) => Some(Ok::<Event, Infallible>(event)),
                    Err(e) => {
                        warn!(agent_key = %agent_key, error = ?e, "failed to render memories SSE event");
                        None
                    }
                }
            }
        });

    let sse = Sse::new(notifications)
        .keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(15)));
    Ok(sse.into_response())
}

async fn render_memory_timeline_event(
    pool: &crate::db::DbPool,
    agent_key: &str,
    since: Option<chrono::DateTime<Utc>>,
    until: Option<chrono::DateTime<Utc>>,
    selected_date_text: Option<String>,
) -> Result<Event, AppError> {
    let rows = list_agent_memories(pool, agent_key, since, until).await?;
    let timeline = crate::web::templates::build_memory_timeline_for_sse(agent_key, &rows);
    let html =
        AgentMemoryTimelinePartialTemplate::render_view(timeline, rows.len(), selected_date_text)?;
    Ok(Event::default().event("memories-timeline").data(html))
}

async fn agents_show_prompts(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    render_agent_show_page(&state, &agent_key, AgentShowTab::Prompts, None).await
}

#[derive(Debug, Default, Deserialize)]
struct UpdateAgentPromptForm {
    #[serde(default)]
    prompt: String,
}

async fn agents_update_analysis_prompt(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
    Form(form): Form<UpdateAgentPromptForm>,
) -> Result<Response, AppError> {
    let updated =
        update_agent_analysis_prompt(&state.db_pool, &agent_key, form.prompt.trim()).await?;

    if !updated {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    }

    Ok(Redirect::to(&format!("/agents/{agent_key}/prompts")).into_response())
}

async fn agents_update_trading_prompt(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
    Form(form): Form<UpdateAgentPromptForm>,
) -> Result<Response, AppError> {
    let updated =
        update_agent_trading_prompt(&state.db_pool, &agent_key, form.prompt.trim()).await?;

    if !updated {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    }

    Ok(Redirect::to(&format!("/agents/{agent_key}/prompts")).into_response())
}

async fn agents_show_settings(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    render_agent_show_page(&state, &agent_key, AgentShowTab::Settings, None).await
}

async fn agents_regenerate_workspace(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    if agent.backend_kind != BACKEND_KIND_OPENCODE {
        return Ok((
            StatusCode::NOT_FOUND,
            "workspace regeneration is only available for OpenCode agents",
        )
            .into_response());
    }

    let updated = generate_and_persist_opencode_workspace(
        &state,
        &OpenCodeWorkspaceAgent {
            agent_key: agent.agent_key.clone(),
            display_name: agent.display_name.clone(),
            api_key: agent.api_key.clone(),
        },
    )
    .await?;

    if !updated {
        error!(agent_key = %agent.agent_key, "agent disappeared before OpenCode workspace metadata update");
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    }

    Ok(Redirect::to(&format!("/agents/{agent_key}/settings")).into_response())
}

async fn agents_show_jobs(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    render_agent_show_page(&state, &agent_key, AgentShowTab::Jobs, None).await
}

async fn agents_show_job_detail(
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

    let html = AgentJobDetailPageTemplate::render_view(
        agent.clone(),
        job_view,
        job_runs,
        job_runs_loaded,
    )?;
    Ok(Html(html).into_response())
}

async fn agents_show_hook_detail(
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

    let Some(hook) = crate::agentic::store::get_agent_hook(&state.db_pool, &agent_key, hook_id).await?
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

    let html = AgentHookDetailPageTemplate::render_view(
        agent.clone(),
        hook_view,
        hook_runs,
        hook_runs_loaded,
    )?;
    Ok(Html(html).into_response())
}

async fn build_job_prompt_preview(
    state: &Arc<AppState>,
    agent: &crate::agents::model::AgentDetailRow,
    job: &crate::agentic::model::AgenticJobScheduleRow,
) -> anyhow::Result<String> {
    use crate::agentic::backend::DispatchRequest;

    let selected_instruments =
        crate::agents::store::list_agent_instrument_ids(&state.db_pool, &agent.agent_key).await?;
    let system_setting =
        crate::settings::store::get_setting(&state.db_pool, "opencode_system_prompt").await?;
    let system_prompt = system_setting.map(|s| s.value).unwrap_or_default();

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

async fn build_hook_prompt_preview(
    state: &Arc<AppState>,
    agent_key: &str,
    hook_id: i64,
) -> anyhow::Result<String> {
    let hook = crate::agentic::store::get_opencode_hook_for_dispatch(&state.db_pool, agent_key, hook_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("hook dispatch metadata unavailable"))?;

    let request = build_hook_dispatch_request(&state.db_pool, &hook, 0, Utc::now())
        .await?
        .ok_or_else(|| anyhow::anyhow!("prompt preview unavailable: no currencies selected for agent"))?;

    crate::agentic::prompt::build_prompt(&request)
}

async fn agents_show_run_detail(
    State(state): State<Arc<AppState>>,
    Path((agent_key, run_id)): Path<(String, i64)>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    if agent.backend_kind != BACKEND_KIND_OPENCODE {
        return Ok((
            StatusCode::NOT_FOUND,
            "run details are only available for OpenCode agents",
        )
            .into_response());
    }

    let Some(run) = crate::agentic::store::get_run(&state.db_pool, run_id).await? else {
        return Ok((StatusCode::NOT_FOUND, "run not found").into_response());
    };
    if run.agent_key != agent_key {
        return Ok((StatusCode::NOT_FOUND, "run not found").into_response());
    }

    let run_view = crate::web::templates::AgenticRunDetailView::from_row(&run);
    let session_lookup_attempted = !run_view.backend_run_ref.is_empty();
    let session = if session_lookup_attempted {
        crate::opencode::store::get_session_detail(&state.db_pool, &run_view.backend_run_ref)
            .await?
            .as_ref()
            .map(crate::web::templates::OpenCodeSessionView::from_detail)
    } else {
        None
    };

    let html = AgentRunDetailPageTemplate::render_view(
        agent.clone(),
        run_view,
        session,
        session_lookup_attempted,
    )?;
    Ok(Html(html).into_response())
}

#[derive(Debug, Clone, Default, Deserialize)]
struct CreateAgentScheduleForm {
    #[serde(default)]
    job_kind: String,
    #[serde(default)]
    timeframe: String,
    #[serde(default)]
    timeout_seconds: String,
    #[serde(default)]
    model_provider_id: String,
    #[serde(default)]
    model_id: String,
    #[serde(default)]
    operator_prompt: String,
    enabled: Option<String>,
}

#[derive(Debug)]
struct ValidatedCreateAgentSchedule {
    job_kind: String,
    timeframe: String,
    trigger_delay_seconds: i32,
    timeout_seconds: i32,
    model_provider_id: Option<String>,
    model_id: Option<String>,
    operator_prompt: String,
    enabled: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct CreateAgentHookForm {
    #[serde(default)]
    timeout_seconds: String,
    #[serde(default)]
    model_provider_id: String,
    #[serde(default)]
    model_id: String,
    #[serde(default)]
    operator_prompt: String,
    enabled: Option<String>,
}

#[derive(Debug)]
struct ValidatedCreateAgentHook {
    timeout_seconds: i32,
    model_provider_id: Option<String>,
    model_id: Option<String>,
    operator_prompt: String,
    enabled: bool,
}

impl CreateAgentScheduleForm {
    fn defaults() -> Self {
        Self {
            job_kind: JOB_KIND_ANALYSIS.to_string(),
            timeframe: "15m".to_string(),
            timeout_seconds: "600".to_string(),
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
            model_provider_id: self.model_provider_id.clone(),
            model_id: self.model_id.clone(),
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

        let model_provider_id = trim_optional_field(&self.model_provider_id);
        let model_id = trim_optional_field(&self.model_id);
        if model_provider_id.is_some() != model_id.is_some() {
            errors.push(
                "Model provider ID and model ID must either both be set or both be empty."
                    .to_string(),
            );
        }

        if errors.is_empty() {
            Ok(ValidatedCreateAgentSchedule {
                job_kind: job_kind.to_string(),
                timeframe,
                trigger_delay_seconds: 1,
                timeout_seconds: timeout_seconds.expect("validated timeout seconds"),
                model_provider_id,
                model_id,
                operator_prompt: self.operator_prompt.trim().to_string(),
                enabled: self.enabled(),
            })
        } else {
            Err(errors)
        }
    }
}

impl CreateAgentHookForm {
    fn defaults() -> Self {
        Self {
            timeout_seconds: "600".to_string(),
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
            model_provider_id: self.model_provider_id.clone(),
            model_id: self.model_id.clone(),
            operator_prompt: self.operator_prompt.clone(),
            enabled: self.enabled(),
        }
    }

    fn validate(&self) -> Result<ValidatedCreateAgentHook, Vec<String>> {
        let mut errors = Vec::new();
        let timeout_seconds =
            parse_positive_schedule_seconds(&self.timeout_seconds, "Timeout", &mut errors);
        let model_provider_id = trim_optional_field(&self.model_provider_id);
        let model_id = trim_optional_field(&self.model_id);
        if model_provider_id.is_some() != model_id.is_some() {
            errors.push(
                "Model provider ID and model ID must either both be set or both be empty."
                    .to_string(),
            );
        }

        if errors.is_empty() {
            Ok(ValidatedCreateAgentHook {
                timeout_seconds: timeout_seconds.expect("validated timeout seconds"),
                model_provider_id,
                model_id,
                operator_prompt: self.operator_prompt.trim().to_string(),
                enabled: self.enabled(),
            })
        } else {
            Err(errors)
        }
    }
}

async fn agents_new_job(
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

    Ok(render_new_job_form(
        agent,
        CreateAgentScheduleForm::defaults().as_template_values(),
        Vec::new(),
        StatusCode::OK,
    ))
}

async fn agents_new_hook(
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

    Ok(render_new_hook_form(
        agent,
        CreateAgentHookForm::defaults().as_template_values(),
        Vec::new(),
        StatusCode::OK,
    ))
}

async fn agents_create_job(
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
            return Ok(render_new_job_form(
                agent,
                form.as_template_values(),
                errors,
                StatusCode::UNPROCESSABLE_ENTITY,
            ));
        }
    };

    if let Err(error) = crate::agentic::store::insert_agent_schedule(
        &state.db_pool,
        &agent_key,
        &validated.job_kind,
        validated.enabled,
        &validated.timeframe,
        validated.trigger_delay_seconds,
        validated.model_provider_id.as_deref(),
        validated.model_id.as_deref(),
        validated.timeout_seconds,
        &validated.operator_prompt,
    )
    .await
    {
        let errors = match schedule_unique_violation_message(&error) {
            Some(message) => vec![message],
            None => return Err(AppError(error)),
        };
        return Ok(render_new_job_form(
            agent,
            form.as_template_values(),
            errors,
            StatusCode::UNPROCESSABLE_ENTITY,
        ));
    }

    Ok(Redirect::to(&format!("/agents/{agent_key}/jobs")).into_response())
}

#[derive(Debug, Default, Deserialize)]
struct ToggleScheduleForm {
    enabled: Option<String>,
}

async fn agents_toggle_job(
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

async fn agents_delete_job(
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

async fn agents_run_job_now(
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
            let system_prompt = system_setting.map(|s| s.value).unwrap_or_default();
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
    }

    Ok(Redirect::to(&format!("/agents/{agent_key}/jobs")).into_response())
}

#[derive(Debug, Default, Deserialize)]
struct ToggleHookForm {
    enabled: Option<String>,
}

async fn agents_create_hook(
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
            return Ok(render_new_hook_form(
                agent,
                form.as_template_values(),
                errors,
                StatusCode::UNPROCESSABLE_ENTITY,
            ));
        }
    };

    if let Err(error) = crate::agentic::store::insert_agent_hook(
        &state.db_pool,
        &agent_key,
        JOB_KIND_MARKET_ANALYSIS,
        HOOK_EVENT_ANALYSIS_BATCH_COMPLETED,
        validated.enabled,
        validated.model_provider_id.as_deref(),
        validated.model_id.as_deref(),
        validated.timeout_seconds,
        &validated.operator_prompt,
    )
    .await
    {
        let errors = match hook_unique_violation_message(&error) {
            Some(message) => vec![message],
            None => return Err(AppError(error)),
        };
        return Ok(render_new_hook_form(
            agent,
            form.as_template_values(),
            errors,
            StatusCode::UNPROCESSABLE_ENTITY,
        ));
    }

    Ok(Redirect::to(&format!("/agents/{agent_key}/jobs")).into_response())
}

async fn agents_run_hook_now(
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
    }

    Ok(Redirect::to(&format!("/agents/{agent_key}/jobs")).into_response())
}

async fn agents_toggle_hook(
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

async fn agents_delete_hook(
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

async fn agents_update_instruments(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
    Form(form_pairs): Form<Vec<(String, String)>>,
) -> Result<Response, AppError> {
    let instrument_ids: Vec<String> = form_pairs
        .into_iter()
        .filter_map(|(key, value)| (key == "instrument_id").then_some(value))
        .collect();
    let updated = replace_agent_instruments(&state.db_pool, &agent_key, &instrument_ids).await?;

    if !updated {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    }

    Ok(Redirect::to(&format!("/agents/{agent_key}/settings")).into_response())
}

async fn generate_and_persist_opencode_workspace(
    state: &Arc<AppState>,
    agent: &OpenCodeWorkspaceAgent,
) -> Result<bool, AppError> {
    let generated = generate_agent_workspace(&state.opencode_workspace_config, agent)
        .inspect_err(|error| {
            error!(agent_key = %agent.agent_key, error = ?error, "failed to generate OpenCode workspace");
        })?;

    let runtime_config = runtime_config_for_generated_workspace(&generated).into_value();
    update_agent_runtime_config(&state.db_pool, &agent.agent_key, runtime_config)
        .await
        .inspect_err(|error| {
            error!(agent_key = %agent.agent_key, error = ?error, "failed to persist OpenCode workspace metadata");
        })
        .map_err(AppError)
}

async fn render_agent_show_page(
    state: &Arc<AppState>,
    agent_key: &str,
    active_tab: AgentShowTab,
    memories_query: Option<AgentMemoriesQuery>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };

    let mut template = AgentsShowPageTemplate::new(agent.clone(), active_tab);

    match active_tab {
        AgentShowTab::Positions => {
            populate_positions_tab(state, &agent, &mut template).await?;
        }
        AgentShowTab::Transactions => {
            template.transactions = match list_all_account_transactions(
                &state.db_pool,
                &agent.wallet_address,
                &agent.environment,
            )
            .await
            {
                Ok(mut rows) => {
                    apply_live_cash_balance_anchor(state, &agent, &mut rows);
                    rows.into_iter().map(TransactionView::from_row).collect()
                }
                Err(error) => {
                    warn!(
                        agent_key = %agent.agent_key,
                        wallet_address = %agent.wallet_address,
                        environment = %agent.environment,
                        error = ?error,
                        "failed to list full account transactions for agent page"
                    );
                    Vec::new()
                }
            };
        }
        AgentShowTab::Memories => {
            let memory_query = memories_query.unwrap_or_default();
            let (filter_date_value, selected_date_text, filter_error_text, since, until) =
                parse_memory_date_filter(&memory_query.date);

            match list_agent_memories(&state.db_pool, &agent.agent_key, since, until).await {
                Ok(rows) => template.set_memories(
                    rows,
                    filter_date_value,
                    selected_date_text,
                    filter_error_text,
                ),
                Err(error) => {
                    warn!(
                        agent_key = %agent.agent_key,
                        error = ?error,
                        "failed to list agent memories for operator page"
                    );
                }
            }
        }
        AgentShowTab::Prompts => {}
        AgentShowTab::Settings => {
            if agent.backend_kind == BACKEND_KIND_OPENCODE {
                template.opencode_workspace = OpenCodeWorkspaceRuntimeConfig::from_value(
                    &agent.runtime_config,
                )
                .map(|workspace| OpenCodeWorkspaceSettingsView {
                    env_exists: std::path::Path::new(&workspace.workspace_host_path)
                        .join(".env")
                        .is_file(),
                    workspace_host_path: workspace.workspace_host_path,
                    workspace_container_path: workspace.workspace_container_path,
                    profile_source: workspace.profile_source,
                });
            }
            template.sync_state = match list_account_sync_state(
                &state.db_pool,
                &agent.wallet_address,
                &agent.environment,
            )
            .await
            {
                Ok(rows) => rows.into_iter().map(SyncStateView::from_row).collect(),
                Err(error) => {
                    warn!(
                        agent_key = %agent.agent_key,
                        wallet_address = %agent.wallet_address,
                        environment = %agent.environment,
                        error = ?error,
                        "failed to list account sync state for agent settings page"
                    );
                    Vec::new()
                }
            };
            match list_agent_instrument_options(&state.db_pool, &agent.agent_key).await {
                Ok(rows) => {
                    template.instrument_options_loaded = true;
                    template.has_selected_instruments = rows.iter().any(|row| row.selected);
                    template.instrument_options = rows;
                }
                Err(error) => {
                    warn!(
                        agent_key = %agent.agent_key,
                        error = ?error,
                        "failed to list agent instrument options for settings page"
                    );
                }
            }
        }
        AgentShowTab::Jobs => {
            if agent.backend_kind != BACKEND_KIND_OPENCODE {
                return Ok((
                    StatusCode::NOT_FOUND,
                    "jobs are only available for OpenCode agents",
                )
                    .into_response());
            }
            populate_jobs_tab(
                state,
                &agent,
                &mut template,
            )
            .await;
        }
    }

    Ok(Html(template.render()?).into_response())
}

async fn populate_jobs_tab(
    state: &Arc<AppState>,
    agent: &crate::agents::model::AgentDetailRow,
    template: &mut AgentsShowPageTemplate,
) {
    match crate::agentic::store::list_agent_schedules(&state.db_pool, &agent.agent_key).await {
        Ok(rows) => {
            template.jobs_loaded = true;
            template.jobs = rows
                .iter()
                .map(crate::web::templates::AgenticJobScheduleView::from_row)
                .collect();
        }
        Err(error) => {
            warn!(
                agent_key = %agent.agent_key,
                error = ?error,
                "failed to list agent jobs for operator page"
            );
        }
    }

    match crate::agentic::store::list_agent_hooks(&state.db_pool, &agent.agent_key).await {
        Ok(rows) => {
            template.hooks_loaded = true;
            template.hooks = rows
                .iter()
                .map(crate::web::templates::AgenticJobHookView::from_row)
                .collect();
        }
        Err(error) => {
            warn!(
                agent_key = %agent.agent_key,
                error = ?error,
                "failed to list agent hooks for operator page"
            );
        }
    }

    const RUNS_LIMIT: i64 = 50;
    match crate::agentic::store::list_agent_runs(&state.db_pool, &agent.agent_key, RUNS_LIMIT).await
    {
        Ok(rows) => {
            template.recent_runs_loaded = true;
            template.recent_runs = rows
                .iter()
                .map(crate::web::templates::AgenticRunView::from_row)
                .collect();
        }
        Err(error) => {
            warn!(
                agent_key = %agent.agent_key,
                error = ?error,
                "failed to list recent agent runs for jobs page"
            );
        }
    }
}

fn render_new_hook_form(
    agent: crate::agents::model::AgentDetailRow,
    form: CreateAgentHookFormValues,
    errors: Vec<String>,
    status: StatusCode,
) -> Response {
    let current_path = format!("/agents/{}/hooks/new", agent.agent_key);
    let template = AgentHookNewPageTemplate {
        agent,
        form,
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

fn apply_live_cash_balance_anchor(
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

fn live_cash_balance(state: &AccountLiveState) -> Option<Decimal> {
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

fn parse_memory_date_filter(
    raw: &str,
) -> (
    String,
    Option<String>,
    Option<String>,
    Option<chrono::DateTime<Utc>>,
    Option<chrono::DateTime<Utc>>,
) {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return (String::new(), None, None, None, None);
    }

    let Ok(date) = chrono::NaiveDate::parse_from_str(trimmed, "%Y-%m-%d") else {
        return (
            trimmed.to_string(),
            None,
            Some("Use YYYY-MM-DD to filter memories by UTC date.".to_string()),
            None,
            None,
        );
    };

    let Some(start_of_day) = date.and_hms_opt(0, 0, 0) else {
        return (
            trimmed.to_string(),
            None,
            Some("That date could not be parsed.".to_string()),
            None,
            None,
        );
    };
    let Some(next_day) = date.succ_opt() else {
        return (
            trimmed.to_string(),
            None,
            Some("That date is out of range.".to_string()),
            None,
            None,
        );
    };
    let Some(end_of_day) = next_day.and_hms_opt(0, 0, 0) else {
        return (
            trimmed.to_string(),
            None,
            Some("That date is out of range.".to_string()),
            None,
            None,
        );
    };

    (
        trimmed.to_string(),
        Some(date.format("%A, %B %-d, %Y").to_string()),
        None,
        Some(chrono::DateTime::<Utc>::from_naive_utc_and_offset(
            start_of_day,
            Utc,
        )),
        Some(chrono::DateTime::<Utc>::from_naive_utc_and_offset(
            end_of_day, Utc,
        )),
    )
}

async fn populate_positions_tab(
    state: &Arc<AppState>,
    agent: &crate::agents::model::AgentDetailRow,
    template: &mut AgentsShowPageTemplate,
) -> Result<(), AppError> {
    let account_key = AccountKey::new(&agent.wallet_address, &agent.environment);
    let live_snapshot = state
        .live_accounts
        .get(&account_key)
        .unwrap_or_else(|| AccountLiveState {
            account_address: account_key.account_address.clone(),
            environment: account_key.environment.clone(),
            status: LiveConnectionStatus::Starting,
            ..Default::default()
        });

    let account_balance_view = AccountBalanceView::from_live_state(live_snapshot.clone());
    template.account_balance_html =
        AccountBalancePartialTemplate::render_view(account_balance_view.clone())
            .map_err(anyhow::Error::from)?;

    let open_positions_view = OpenPositionsView::from_live_state(live_snapshot.clone());
    template.open_positions_html = OpenPositionsPartialTemplate::render_view(open_positions_view)
        .map_err(anyhow::Error::from)?;

    let open_orders_view = OpenOrdersView::from_live_state(live_snapshot.clone());
    template.open_orders_html =
        OpenOrdersPartialTemplate::render_view(open_orders_view).map_err(anyhow::Error::from)?;

    let latest_trade_execution =
        get_latest_agent_memory_by_type(&state.db_pool, &agent.agent_key, "trade_execution")
            .await?;
    template.latest_trade_execution_summary_html =
        LatestTradeExecutionSummaryPartialTemplate::render_view(
            latest_trade_execution
                .as_ref()
                .map(|memory| memory.summary.clone()),
            latest_trade_execution
                .as_ref()
                .map(|memory| memory.created_at),
        )
        .map_err(anyhow::Error::from)?;

    let latest_analysis =
        get_latest_agent_memory_by_type(&state.db_pool, &agent.agent_key, "analysis").await?;
    let analysis_detail_url = latest_analysis
        .as_ref()
        .map(|memory| format!("/agents/{}/memories/{}", agent.agent_key, memory.id));
    template.latest_analysis_summary_html = LatestAnalysisSummaryPartialTemplate::render_view(
        latest_analysis
            .as_ref()
            .map(|memory| memory.summary.clone()),
        analysis_detail_url,
        latest_analysis.as_ref().map(|memory| memory.created_at),
        latest_analysis.as_ref().and_then(memory_expires_at),
    )
    .map_err(anyhow::Error::from)?;

    let now = Utc::now();
    let since_24h = now - chrono::Duration::hours(24);
    let since_30d = now - chrono::Duration::days(30);
    let series_24h = match fetch_balance_series(
        &state.db_pool,
        &agent.wallet_address,
        &agent.environment,
        since_24h,
        BalanceSeriesBucket::Hour,
    )
    .await
    {
        Ok(points) => points,
        Err(error) => {
            warn!(
                agent_key = %agent.agent_key,
                wallet_address = %agent.wallet_address,
                environment = %agent.environment,
                error = ?error,
                "failed to fetch 24h balance series for agent page"
            );
            Vec::new()
        }
    };
    let series_30d = match fetch_balance_series(
        &state.db_pool,
        &agent.wallet_address,
        &agent.environment,
        since_30d,
        BalanceSeriesBucket::Day,
    )
    .await
    {
        Ok(points) => points,
        Err(error) => {
            warn!(
                agent_key = %agent.agent_key,
                wallet_address = %agent.wallet_address,
                environment = %agent.environment,
                error = ?error,
                "failed to fetch 30d balance series for agent page"
            );
            Vec::new()
        }
    };
    let sparklines = vec![
        SparklineView::from_series("24 hours", &series_24h, 240, 48),
        SparklineView::from_series("30 days", &series_30d, 240, 48),
    ];
    template.sparklines_html =
        BalanceSparklinesPartialTemplate::render_view(sparklines).map_err(anyhow::Error::from)?;

    Ok(())
}

async fn delete_agent(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    let deleted = delete_agent_in_store(&state.db_pool, &agent_key).await?;
    if deleted {
        Ok(Redirect::to("/agents").into_response())
    } else {
        Ok((StatusCode::NOT_FOUND, "agent not found").into_response())
    }
}

/// Stream all live account views for `agent_key` over a single Server-Sent
/// Events connection.
///
/// Using one connection (rather than one per card) keeps the agent detail
/// page well under the browser's per-host HTTP/1.1 connection limit, which
/// previously starved ordinary navigation requests when three separate SSE
/// streams were held open simultaneously.
///
/// Each account mutation emits three named events on this single stream:
/// `balance`, `positions`, and `orders`. Two companion memory-driven events,
/// `latest-trade-execution-summary` and `latest-analysis-summary`, refresh
/// the Open Orders subheader and the Analysis section when new
/// `trade_execution` or `analysis` memories arrive. The `data` field of each
/// is the freshly rendered partial for that section, which the HTMX SSE
/// extension routes to the matching `sse-swap="..."` element. The stream begins with
/// an initial snapshot and then re-emits updates for the matching account.
#[derive(Debug)]
enum MemoryNotification {
    Some(uuid::Uuid),
    Lagged,
}

async fn agent_live_stream(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    let agent = match get_agent(&state.db_pool, &agent_key).await? {
        Some(agent) => agent,
        None => return Ok((StatusCode::NOT_FOUND, "agent not found").into_response()),
    };

    let account_key = AccountKey::new(&agent.wallet_address, &agent.environment);
    let live_accounts = Arc::clone(&state.live_accounts);

    // Emit a snapshot up-front so the UI never sits on the initial-render
    // placeholder if the orchestrator already produced a value before the
    // SSE connection opened. If no snapshot exists yet, fall back to a
    // `Starting`-status placeholder so the UI can still render the
    // connection status / "Loading…" caption.
    let initial_snapshot = live_accounts
        .get(&account_key)
        .unwrap_or_else(|| AccountLiveState {
            account_address: account_key.account_address.clone(),
            environment: account_key.environment.clone(),
            status: LiveConnectionStatus::Starting,
            ..Default::default()
        });
    let mut initial_events = render_live_events(&initial_snapshot)?;
    initial_events
        .push(render_latest_trade_execution_summary_event(&state.db_pool, &agent.agent_key).await?);
    initial_events
        .push(render_latest_analysis_summary_event(&state.db_pool, &agent.agent_key).await?);

    let account_key_filter = account_key.clone();
    let live_accounts_filter = Arc::clone(&live_accounts);
    let notifications = BroadcastStream::new(live_accounts.subscribe())
        .filter_map(move |item| {
            let account_key = account_key_filter.clone();
            async move {
                match item {
                    Ok(key) if key == account_key => Some(key),
                    Ok(_) => None,
                    Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(_)) => {
                        // Slow consumers can drop intermediate
                        // notifications; treat a lagged notification as a
                        // request to re-emit the current state so the UI
                        // catches up.
                        Some(account_key.clone())
                    }
                }
            }
        })
        .flat_map(move |_key| {
            let live_accounts = Arc::clone(&live_accounts_filter);
            let key = account_key.clone();
            let events = match live_accounts.get(&key) {
                Some(snapshot) => match render_live_events(&snapshot) {
                    Ok(events) => events,
                    Err(e) => {
                        warn!(error = ?e, "failed to render live SSE events");
                        Vec::new()
                    }
                },
                None => Vec::new(),
            };
            tokio_stream::iter(events.into_iter().map(Ok::<Event, Infallible>))
        });

    let summary_db_pool = state.db_pool.clone();
    let summary_agent_key_filter = agent.agent_key.clone();
    let summary_agent_key_render = agent.agent_key.clone();
    let summary_notifications = BroadcastStream::new(state.ui_events.subscribe())
        .filter_map(move |item| {
            let agent_key = summary_agent_key_filter.clone();
            async move {
                match item {
                    Ok(UiEvent::MemoryCreated {
                        agent_key: event_agent_key,
                        memory_id,
                    }) if event_agent_key == agent_key => Some(MemoryNotification::Some(memory_id)),
                    Ok(_) => None,
                    Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(_)) => {
                        Some(MemoryNotification::Lagged)
                    }
                }
            }
        })
        .then(move |notification| {
            let db_pool = summary_db_pool.clone();
            let agent_key = summary_agent_key_render.clone();
            async move {
                let (needs_trade_execution, needs_analysis) = match notification {
                    MemoryNotification::Some(memory_id) => {
                        match get_memory_record(&db_pool, &agent_key, memory_id).await {
                            Ok(Some(memory)) => match memory.memory_type.as_str() {
                                "trade_execution" => (true, false),
                                "analysis" => (false, true),
                                _ => (false, false),
                            },
                            Ok(None) => (false, false),
                            Err(error) => {
                                warn!(agent_key = %agent_key, error = ?error, "failed to inspect memory event for live summary update");
                                (false, false)
                            }
                        }
                    }
                    MemoryNotification::Lagged => {
                        // Lagged notification: we may have missed a memory
                        // of either type, so re-render both summaries to
                        // catch up.
                        (true, true)
                    }
                };

                let mut events = Vec::new();
                if needs_trade_execution {
                    match render_latest_trade_execution_summary_event(&db_pool, &agent_key).await {
                        Ok(event) => events.push(Ok::<Event, Infallible>(event)),
                        Err(error) => {
                            warn!(agent_key = %agent_key, error = ?error, "failed to render latest trade execution summary SSE event");
                        }
                    }
                }
                if needs_analysis {
                    match render_latest_analysis_summary_event(&db_pool, &agent_key).await {
                        Ok(event) => events.push(Ok::<Event, Infallible>(event)),
                        Err(error) => {
                            warn!(agent_key = %agent_key, error = ?error, "failed to render latest analysis summary SSE event");
                        }
                    }
                }
                events
            }
        })
        .flat_map(|events| tokio_stream::iter(events));

    let stream = tokio_stream::iter(
        initial_events
            .into_iter()
            .map(Ok::<Event, Infallible>)
            .collect::<Vec<_>>(),
    )
    .chain(futures::stream::select(
        notifications,
        summary_notifications,
    ));
    let sse =
        Sse::new(stream).keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(15)));
    Ok(sse.into_response())
}

/// Render the `balance`, `positions`, and `orders` SSE events for a single
/// live-state snapshot.
fn render_live_events(state: &AccountLiveState) -> Result<Vec<Event>, AppError> {
    Ok(vec![
        render_account_balance_event(state)?,
        render_open_positions_event(state)?,
        render_open_orders_event(state)?,
    ])
}

fn render_account_balance_event(state: &AccountLiveState) -> Result<Event, AppError> {
    let view = AccountBalanceView::from_live_state(state.clone());
    let html = AccountBalancePartialTemplate::render_view(view)?;
    Ok(Event::default().event("balance").data(html))
}

fn render_open_positions_event(state: &AccountLiveState) -> Result<Event, AppError> {
    let view = OpenPositionsView::from_live_state(state.clone());
    let html = OpenPositionsPartialTemplate::render_view(view)?;
    Ok(Event::default().event("positions").data(html))
}

fn render_open_orders_event(state: &AccountLiveState) -> Result<Event, AppError> {
    let view = OpenOrdersView::from_live_state(state.clone());
    let html = OpenOrdersPartialTemplate::render_view(view)?;
    Ok(Event::default().event("orders").data(html))
}

async fn render_latest_trade_execution_summary_event(
    pool: &crate::db::DbPool,
    agent_key: &str,
) -> Result<Event, AppError> {
    let latest = get_latest_agent_memory_by_type(pool, agent_key, "trade_execution").await?;
    let summary = latest.as_ref().map(|memory| memory.summary.clone());
    let created_at = latest.as_ref().map(|memory| memory.created_at);
    let html = LatestTradeExecutionSummaryPartialTemplate::render_view(summary, created_at)?;
    Ok(Event::default()
        .event("latest-trade-execution-summary")
        .data(html))
}

async fn render_latest_analysis_summary_event(
    pool: &crate::db::DbPool,
    agent_key: &str,
) -> Result<Event, AppError> {
    let latest = get_latest_agent_memory_by_type(pool, agent_key, "analysis").await?;
    let detail_url = latest
        .as_ref()
        .map(|memory| format!("/agents/{agent_key}/memories/{}", memory.id));
    let summary = latest.as_ref().map(|memory| memory.summary.clone());
    let created_at = latest.as_ref().map(|memory| memory.created_at);
    let expires_at = latest.as_ref().and_then(memory_expires_at);
    let html = LatestAnalysisSummaryPartialTemplate::render_view(
        summary, detail_url, created_at, expires_at,
    )?;
    Ok(Event::default().event("latest-analysis-summary").data(html))
}

async fn create_agent(
    State(state): State<Arc<AppState>>,
    Form(form): Form<CreateAgentForm>,
) -> Result<Response, AppError> {
    let runtimes = list_enabled_agent_runtimes(&state.db_pool).await?;

    if let Err(errors) = form.validate() {
        return Ok(render_new_form(form, runtimes, errors));
    }

    let wallet_address = match derive_wallet_address(&form.hyperliquid_private_key) {
        Ok(addr) => addr,
        Err(e) => {
            return Ok(render_new_form(
                form,
                runtimes,
                vec![format!("Hyperliquid private key is invalid: {e}")],
            ));
        }
    };

    let ciphertext = match encrypt(&state.encryption_key, &form.hyperliquid_private_key) {
        Ok(ct) => ct,
        Err(e) => {
            return Ok(render_new_form(
                form,
                runtimes,
                vec![format!("Failed to encrypt private key: {e}")],
            ));
        }
    };

    if !runtime_matches_backend(
        &state.db_pool,
        form.runtime_id.trim(),
        form.backend_kind.trim(),
    )
    .await?
    {
        return Ok(render_new_form(
            form,
            runtimes,
            vec![
                "Selected runtime must exist, be enabled, and match the selected backend kind."
                    .to_string(),
            ],
        ));
    }

    let now = Utc::now();
    let agent_key = slugify_agent_key(&form.display_name);

    let row = AgentRegistryRow {
        agent_key: agent_key.clone(),
        created_at: now,
        updated_at: now,
        enabled: form.enabled(),
        display_name: form.display_name.trim().to_string(),
        analysis_prompt: DEFAULT_ANALYSIS_STRATEGY_PROMPT.to_string(),
        trading_prompt: DEFAULT_TRADING_STRATEGY_PROMPT.to_string(),
        wallet_address,
        environment: "live".to_string(),
        api_key: generate_api_key(),
        api_key_last_used_at: None,
        backend_kind: form.backend_kind.trim().to_string(),
        runtime_id: form.runtime_id.trim().to_string(),
        runtime_config: serde_json::json!({}),
        analysis_context_last_used_at: None,
        trading_context_last_used_at: None,
        hyperliquid_private_key_ciphertext: ciphertext,
        hyperliquid_private_key_key_id: state.encryption_key.key_id.clone(),
    };

    if let Err(e) = insert_agent(&state.db_pool, &row).await {
        let errors = match unique_violation_message(&e) {
            Some(msg) => vec![msg],
            None => {
                return Err(AppError(e));
            }
        };
        return Ok(render_new_form(form, runtimes, errors));
    }

    if row.backend_kind == BACKEND_KIND_OPENCODE {
        let updated = generate_and_persist_opencode_workspace(
            &state,
            &OpenCodeWorkspaceAgent {
                agent_key: row.agent_key.clone(),
                display_name: row.display_name.clone(),
                api_key: row.api_key.clone(),
            },
        )
        .await?;

        if !updated {
            error!(agent_key = %row.agent_key, "agent disappeared before OpenCode workspace metadata update");
        }

        if let Err(error) =
            crate::agentic::store::insert_default_opencode_schedules(&state.db_pool, &row.agent_key)
                .await
        {
            error!(
                agent_key = %row.agent_key,
                error = ?error,
                "failed to insert default OpenCode schedules"
            );
            return Err(AppError(error));
        }
    }

    Ok(Redirect::to(&format!("/agents/{agent_key}")).into_response())
}

fn render_new_form(
    form: CreateAgentForm,
    runtimes: Vec<AgentRuntimeRow>,
    errors: Vec<String>,
) -> Response {
    let template = AgentsNewPageTemplate {
        form,
        runtimes,
        errors,
        current_path: "/agents/new".to_string(),
    };
    match template.render() {
        Ok(body) => (StatusCode::UNPROCESSABLE_ENTITY, Html(body)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("template error: {e}"),
        )
            .into_response(),
    }
}

fn render_backend_form(form: CreateAgentRuntimeForm, errors: Vec<String>) -> Response {
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

fn render_new_job_form(
    agent: crate::agents::model::AgentDetailRow,
    form: CreateAgentScheduleFormValues,
    errors: Vec<String>,
    status: StatusCode,
) -> Response {
    let current_path = format!("/agents/{}/jobs/new", agent.agent_key);
    let template = AgentScheduleNewPageTemplate {
        agent,
        form,
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

fn unique_violation_message(error: &anyhow::Error) -> Option<String> {
    let db_err = error.downcast_ref::<sqlx::Error>()?.as_database_error()?;
    if db_err.is_unique_violation() {
        let constraint = db_err.constraint().unwrap_or("unknown");
        if constraint.contains("agent_key") || constraint.contains("agents_pkey") {
            Some("An agent with this agent key already exists.".to_string())
        } else if constraint.contains("agent_runtimes_pkey") {
            Some("A backend with this runtime ID already exists.".to_string())
        } else if constraint.contains("agent_runtimes_name") {
            Some("A backend with this name already exists.".to_string())
        } else if constraint.contains("wallet") {
            Some("An agent with this wallet address and environment already exists.".to_string())
        } else if constraint.contains("api_key") {
            Some("An agent with this API key already exists.".to_string())
        } else {
            Some("This agent conflicts with an existing registry entry.".to_string())
        }
    } else {
        None
    }
}

fn schedule_unique_violation_message(error: &anyhow::Error) -> Option<String> {
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

fn hook_unique_violation_message(error: &anyhow::Error) -> Option<String> {
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

fn trim_optional_field(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn parse_positive_schedule_seconds(
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

#[derive(Debug)]
struct AppError(anyhow::Error);

impl<E> From<E> for AppError
where
    E: Into<anyhow::Error>,
{
    fn from(error: E) -> Self {
        Self(error.into())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        error!(error = ?self.0, "request failed");

        let template = ServerErrorPageTemplate {
            message: format!("Internal server error: {}", self.0),
            current_path: String::new(),
        };

        match template.render() {
            Ok(body) => (StatusCode::INTERNAL_SERVER_ERROR, Html(body)).into_response(),
            Err(render_error) => {
                error!(error = ?render_error, "failed to render server error page");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("internal server error: {}", self.0),
                )
                    .into_response()
            }
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct SettingsUpdateForm {
    #[serde(default)]
    system_prompt: String,
}

async fn settings_index(State(state): State<Arc<AppState>>) -> Result<Response, AppError> {
    let row = crate::settings::store::get_setting(&state.db_pool, "opencode_system_prompt").await?;
    let system_prompt = row.map(|r| r.value).unwrap_or_default();
    let html = SettingsPageTemplate {
        system_prompt,
        current_path: "/settings".to_string(),
    }
    .render()?;
    Ok(Html(html).into_response())
}

async fn settings_update(
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

async fn hermes_page(State(state): State<Arc<AppState>>) -> Result<Html<String>, AppError> {
    let mut template = HermesPageTemplate {
        reachable: false,
        version: None,
        active_profile: None,
        profiles: Vec::new(),
        error: None,
        dashboard_url: None,
        current_path: "/hermes".to_string(),
    };

    if let Some(hermes) = &state.hermes {
        let health = hermes.health().await.unwrap_or(HermesHealth {
            reachable: false,
            version: None,
        });
        template.reachable = health.reachable;
        template.version = health.version;
        template.dashboard_url = Some(state.hermes_dashboard_link_url.clone());
        match hermes.list_profiles().await {
            Ok(profiles) => {
                template.profiles = profiles.into_iter().map(|p| p.name).collect();
            }
            Err(e) => {
                template.error = Some(format!("Failed to list profiles: {e}"));
            }
        }
        if template.reachable {
            match hermes.get_active_profile().await {
                Ok(Some(p)) => template.active_profile = Some(p.name),
                Ok(None) => {}
                Err(e) => {
                    template.error = Some(format!("Failed to get active profile: {e}"));
                }
            }
        }
    } else {
        template.error = Some("Hermes is not configured".to_string());
    }

    Ok(Html(template.render()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::{Arc, Mutex},
    };

    use anyhow::Result;
    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt as _;
    use rust_decimal::Decimal;
    use tower::util::ServiceExt;

    use crate::{
        agentic::backend::{AgenticBackend, DispatchRequest, DispatchResult},
        agents::{
            crypto::EncryptionKey,
            model::CreateAgentRuntimeForm,
            prompts::{DEFAULT_ANALYSIS_STRATEGY_PROMPT, DEFAULT_TRADING_STRATEGY_PROMPT},
            store::{insert_agent_runtime, list_agent_instrument_ids, replace_agent_instruments},
        },
        memory::CreateMemory,
        test_db,
        web::ui_events::UiEventHub,
    };

    struct NoopAgenticBackend;

    #[async_trait]
    impl AgenticBackend for NoopAgenticBackend {
        async fn dispatch(&self, _request: DispatchRequest) -> Result<DispatchResult> {
            Ok(DispatchResult {
                backend_run_ref: "ses_test".to_string(),
            })
        }
    }

    struct RecordingAgenticBackend {
        calls: Arc<Mutex<Vec<DispatchRequest>>>,
    }

    #[async_trait]
    impl AgenticBackend for RecordingAgenticBackend {
        async fn dispatch(&self, request: DispatchRequest) -> Result<DispatchResult> {
            self.calls.lock().unwrap().push(request);
            Ok(DispatchResult {
                backend_run_ref: "ses_recorded".to_string(),
            })
        }
    }

    async fn test_state() -> Arc<AppState> {
        test_state_with_backend(Arc::new(NoopAgenticBackend)).await
    }

    async fn test_state_with_backend(agentic_backend: Arc<dyn AgenticBackend>) -> Arc<AppState> {
        let pool = test_db::pool().await;
        Arc::new(AppState {
            db_pool: pool,
            agentic_backend,
            encryption_key: EncryptionKey::new(
                "test",
                [
                    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21,
                    22, 23, 24, 25, 26, 27, 28, 29, 30, 31,
                ],
            ),
            live_accounts: Arc::new(crate::hyperliquid::live_state::LiveAccountStore::new()),
            ui_events: Arc::new(UiEventHub::new()),
            hermes: None,
            hermes_dashboard_link_url: "http://127.0.0.1:19119".to_string(),
            opencode_workspace_config: crate::opencode::workspace::OpenCodeWorkspaceConfig {
                source_root: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join(crate::opencode::workspace::PROFILE_SOURCE_RELATIVE_PATH),
                host_workspaces_root: std::path::PathBuf::from("/tmp/opencode/vibetrading-routes"),
                container_workspaces_root: "/workspaces".to_string(),
                api_base_url: "http://host.containers.internal:3003".to_string(),
            },
        })
    }

    /// Read bytes from an SSE body until the timeout fires. SSE response
    /// bodies are long-lived streams that never EOF, so the naive
    /// `axum::body::to_bytes` hangs forever; we only care about the
    /// initial snapshot of events here. The body is polled frame by frame
    /// and the result is whatever data arrived before the deadline.
    async fn read_sse_chunk(body: Body, timeout_ms: u64) -> String {
        let mut body = body;
        let mut buf = Vec::<u8>::new();
        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_millis(timeout_ms);

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(remaining, body.frame()).await {
                Ok(Some(Ok(frame))) => {
                    if let Some(data) = frame.data_ref() {
                        buf.extend_from_slice(data);
                    }
                }
                Ok(Some(Err(e))) => panic!("failed to read SSE body: {e}"),
                Ok(None) => break,
                Err(_) => break,
            }
        }

        if buf.is_empty() {
            panic!("timed out reading SSE body");
        }

        String::from_utf8_lossy(&buf).into_owned()
    }

    async fn response_text(response: Response) -> String {
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    async fn seed_ledger_event(
        state: &Arc<AppState>,
        wallet_address: &str,
        hash: &str,
        event_time: chrono::DateTime<Utc>,
        usdc: Decimal,
    ) {
        sqlx::query(
            "INSERT INTO hyperliquid.ledger_events
                (hash, account_address, environment, event_time, event_type,
                 source_stream, ledger_type, usdc, ingest_source, inserted_at)
             VALUES ($1, $2, 'live', $3, 'ledger', 'test', 'deposit', $4, 'test', NOW())",
        )
        .bind(hash)
        .bind(wallet_address)
        .bind(event_time)
        .bind(usdc)
        .execute(&state.db_pool)
        .await
        .expect("insert ledger event");
    }

    async fn seed_memory(
        state: &Arc<AppState>,
        agent_key: &str,
        summary: &str,
        content: &str,
    ) -> crate::memory::MemoryRecord {
        seed_memory_with_type(state, agent_key, "plan", summary, content).await
    }

    async fn seed_memory_with_type(
        state: &Arc<AppState>,
        agent_key: &str,
        memory_type: &str,
        summary: &str,
        content: &str,
    ) -> crate::memory::MemoryRecord {
        crate::memory::insert_memory(
            &state.db_pool,
            agent_key,
            &CreateMemory {
                symbol: "BTC".to_string(),
                timeframe: Some("1h".to_string()),
                memory_type: memory_type.to_string(),
                summary: summary.to_string(),
                content: content.to_string(),
                metadata: Some(serde_json::json!({ "confidence": 0.8 })),
            },
        )
        .await
        .expect("insert memory")
    }

    async fn seed_sync_state(state: &Arc<AppState>, wallet_address: &str) {
        sqlx::query(
            "INSERT INTO hyperliquid.sync_state
                (account_address, environment, stream_name, status, metadata, last_event_key, last_synced_at)
             VALUES ($1, 'live', 'fills', $2, '{}'::jsonb, 'abc123', NOW())",
        )
        .bind(wallet_address)
        .bind(crate::hyperliquid::sync_state::SyncStatus::Healthy.as_str())
        .execute(&state.db_pool)
        .await
        .expect("insert sync state");
    }

    async fn seed_instrument(state: &Arc<AppState>, instrument_id: &str, active: bool) {
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO hyperliquid.instruments (
                instrument_id,
                name,
                market_type,
                base_asset,
                quote_asset,
                settlement_asset,
                asset_index,
                price_decimals,
                size_decimals,
                lot_size,
                max_leverage,
                is_hip3,
                active,
                created_at,
                updated_at
            ) VALUES (
                $1, $1, 'perp', $1, 'USD', 'USDC', 1, 2, 3, 0.001, 50, false, $2, $3, $3
            )
            ON CONFLICT (instrument_id) DO UPDATE
                SET market_type = EXCLUDED.market_type,
                    active = EXCLUDED.active,
                    updated_at = EXCLUDED.updated_at",
        )
        .bind(instrument_id)
        .bind(active)
        .bind(now)
        .execute(&state.db_pool)
        .await
        .expect("insert instrument");
    }

    async fn set_job_context_timestamps(
        state: &Arc<AppState>,
        agent_key: &str,
        analysis_context_last_used_at: Option<chrono::DateTime<Utc>>,
        trading_context_last_used_at: Option<chrono::DateTime<Utc>>,
    ) {
        sqlx::query(
            "UPDATE agents
                SET analysis_context_last_used_at = $2,
                    trading_context_last_used_at = $3
              WHERE agent_key = $1",
        )
        .bind(agent_key)
        .bind(analysis_context_last_used_at)
        .bind(trading_context_last_used_at)
        .execute(&state.db_pool)
        .await
        .expect("update job context timestamps");
    }

    #[tokio::test]
    async fn hermes_page_renders_not_configured_state() {
        let state = test_state().await;

        let app = router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/hermes")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body();
        let bytes = http_body_util::BodyExt::collect(body)
            .await
            .unwrap()
            .to_bytes();
        let text = String::from_utf8_lossy(&bytes).into_owned();
        assert!(text.contains("Hermes"));
        assert!(text.contains("Hermes is not configured"));
    }

    #[tokio::test]
    async fn get_agents_renders_db_data() {
        let state = test_state().await;

        let app = router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/agents")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn post_agents_with_invalid_private_key_returns_validation_error() {
        let state = test_state().await;

        let app = router(state);
        let body = "display_name=Test Agent&hyperliquid_private_key=not-a-key&backend_kind=opencode&runtime_id=opencode-local";
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/agents")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn agents_new_page_renders_backend_and_runtime_controls() {
        let state = test_state().await;

        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri("/agents/new")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let text = response_text(response).await;
        assert!(text.contains("name=\"backend_kind\""));
        assert!(text.contains("name=\"runtime_id\""));
        assert!(text.contains("OpenCode local"));
    }

    #[tokio::test]
    async fn backends_new_page_renders_create_form() {
        let state = test_state().await;

        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri("/backends/new")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let text = response_text(response).await;
        assert!(text.contains("Create backend"));
        assert!(text.contains("name=\"id\""));
        assert!(text.contains("name=\"base_url\""));
    }

    #[tokio::test]
    async fn backends_index_renders_seeded_runtime() {
        let state = test_state().await;

        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri("/backends")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let text = response_text(response).await;
        assert!(text.contains("OpenCode local"));
        assert!(text.contains("opencode-local"));
        assert!(text.contains("http://localhost:14096"));
    }

    #[tokio::test]
    async fn post_agents_rejects_runtime_backend_mismatch() {
        let state = test_state().await;

        let app = router(state);
        let private_key = random_private_key();
        let body = format!(
            "display_name=MismatchTest&hyperliquid_private_key={private_key}&backend_kind=hermes&runtime_id=opencode-local"
        );
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/agents")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let text = response_text(response).await;
        assert!(text.contains(
            "Selected runtime must exist, be enabled, and match the selected backend kind."
        ));
    }

    #[tokio::test]
    async fn account_balance_stream_returns_404_for_unknown_agent() {
        let state = test_state().await;

        let app = router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/agents/does-not-exist-12345/live/stream")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    #[ignore = "SSE body read hangs in pglite-oxide test environment; see AGENTS.md"]
    async fn account_balance_stream_emits_initial_loading_placeholder() {
        let state = test_state().await;

        let (agent_key, _wallet_address) = match insert_test_agent(&state).await {
            Some(pair) => pair,
            None => return,
        };

        let app = router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{}/live/stream", agent_key))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok()),
            Some("text/event-stream")
        );

        // Read just the first event boundary of the body so we capture the
        // initial event without waiting for the keep-alive timer.
        let text = read_sse_chunk(response.into_body(), 250).await;
        assert!(
            text.contains("event: balance"),
            "missing event line in {text}"
        );
        assert!(text.contains("Loading"), "expected placeholder in {text}");
    }

    #[tokio::test]
    #[ignore = "SSE body read hangs in pglite-oxide test environment; see AGENTS.md"]
    async fn account_balance_stream_emits_initial_value_when_state_present() {
        let state = test_state().await;

        let (agent_key, wallet_address) = match insert_test_agent(&state).await {
            Some(pair) => pair,
            None => return,
        };

        let key = AccountKey::new(&wallet_address, "live");
        state.live_accounts.replace(
            key.clone(),
            AccountLiveState {
                account_address: key.account_address.clone(),
                environment: key.environment.clone(),
                status: LiveConnectionStatus::Connected,
                margin: Some(crate::hyperliquid::live_state::LiveMarginState {
                    account_value: Some(rust_decimal::Decimal::new(123_4567, 4)),
                    ..Default::default()
                }),
                updated_at: Some(Utc::now()),
                ..Default::default()
            },
        );

        let app = router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{}/live/stream", agent_key))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let text = read_sse_chunk(response.into_body(), 250).await;
        assert!(text.contains("event: balance"));
        assert!(text.contains("123.4567"));
        assert!(text.contains("USDC"));
    }

    #[tokio::test]
    #[ignore = "SSE body read hangs in pglite-oxide test environment; see AGENTS.md"]
    async fn account_balance_stream_emits_updates_when_state_changes() {
        let state = test_state().await;

        let (agent_key, wallet_address) = match insert_test_agent(&state).await {
            Some(pair) => pair,
            None => return,
        };

        let key = AccountKey::new(&wallet_address, "live");

        let app = router(state.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{}/live/stream", agent_key))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // Update the live store; the SSE consumer should observe the
        // notification and emit a new event.
        state.live_accounts.replace(
            key.clone(),
            AccountLiveState {
                account_address: key.account_address.clone(),
                environment: key.environment.clone(),
                status: LiveConnectionStatus::Connected,
                margin: Some(crate::hyperliquid::live_state::LiveMarginState {
                    account_value: Some(rust_decimal::Decimal::new(99_0000, 4)),
                    ..Default::default()
                }),
                updated_at: Some(Utc::now()),
                ..Default::default()
            },
        );

        // Longer wait: this test expects a second event after we mutate
        // live_accounts, which only happens once a real broadcast fires.
        let text = read_sse_chunk(response.into_body(), 2000).await;
        assert!(text.contains("event: balance"));
        assert!(text.contains("99.0000"));
    }

    async fn insert_test_agent(state: &Arc<AppState>) -> Option<(String, String)> {
        insert_test_agent_with_text(state, String::new(), String::new()).await
    }

    async fn insert_test_opencode_agent(state: &Arc<AppState>) -> Option<(String, String)> {
        ensure_test_runtime(
            state,
            "opencode-local",
            crate::agents::model::BACKEND_KIND_OPENCODE,
        )
        .await;

        let timestamp = chrono::Utc::now().timestamp_millis();
        let display_name = format!("OpenCodeScheduleTest{}", timestamp);
        let agent_key = slugify_agent_key(&display_name);
        let private_key = random_private_key();
        let wallet_address = match derive_wallet_address(&private_key) {
            Ok(addr) => addr,
            Err(_) => return None,
        };
        let now = Utc::now();
        let row = crate::agents::model::AgentRegistryRow {
            agent_key: agent_key.clone(),
            created_at: now,
            updated_at: now,
            enabled: true,
            display_name,
            analysis_prompt: String::new(),
            trading_prompt: String::new(),
            wallet_address: wallet_address.clone(),
            environment: "live".to_string(),
            api_key: format!("opencode-schedule-test-{timestamp}"),
            api_key_last_used_at: None,
            backend_kind: crate::agents::model::BACKEND_KIND_OPENCODE.to_string(),
            runtime_id: "opencode-local".to_string(),
            runtime_config: serde_json::json!({}),
            analysis_context_last_used_at: None,
            trading_context_last_used_at: None,
            hyperliquid_private_key_ciphertext: Vec::new(),
            hyperliquid_private_key_key_id: "test".to_string(),
        };
        if insert_agent(&state.db_pool, &row).await.is_err() {
            return None;
        }
        crate::agentic::store::insert_default_opencode_schedules(&state.db_pool, &agent_key)
            .await
            .expect("insert default schedules");
        Some((agent_key, wallet_address))
    }

    async fn ensure_test_runtime(state: &Arc<AppState>, id: &str, backend_kind: &str) {
        let form = CreateAgentRuntimeForm {
            id: id.to_string(),
            name: format!("{backend_kind}-{id}"),
            backend_kind: backend_kind.to_string(),
            base_url: if backend_kind == crate::agents::model::BACKEND_KIND_HERMES {
                "http://localhost:19119".to_string()
            } else {
                "http://localhost:14096".to_string()
            },
            enabled: Some("on".to_string()),
        };

        let _ = insert_agent_runtime(&state.db_pool, &form).await;
    }

    async fn insert_test_agent_with_text(
        state: &Arc<AppState>,
        analysis_prompt: String,
        trading_prompt: String,
    ) -> Option<(String, String)> {
        ensure_test_runtime(
            state,
            "hermes-local",
            crate::agents::model::BACKEND_KIND_HERMES,
        )
        .await;

        let timestamp = chrono::Utc::now().timestamp_millis();
        let display_name = format!("BalanceStreamTest{}", timestamp);
        let agent_key = slugify_agent_key(&display_name);
        let private_key = random_private_key();
        let wallet_address = match derive_wallet_address(&private_key) {
            Ok(addr) => addr,
            Err(_) => return None,
        };
        let now = Utc::now();
        let row = crate::agents::model::AgentRegistryRow {
            agent_key: agent_key.clone(),
            created_at: now,
            updated_at: now,
            enabled: true,
            display_name,
            analysis_prompt,
            trading_prompt,
            wallet_address: wallet_address.clone(),
            environment: "live".to_string(),
            api_key: format!("balance-stream-test-{timestamp}"),
            api_key_last_used_at: None,
            backend_kind: crate::agents::model::BACKEND_KIND_HERMES.to_string(),
            runtime_id: "hermes-local".to_string(),
            runtime_config: serde_json::json!({}),
            analysis_context_last_used_at: None,
            trading_context_last_used_at: None,
            hyperliquid_private_key_ciphertext: Vec::new(),
            hyperliquid_private_key_key_id: "test".to_string(),
        };
        if insert_agent(&state.db_pool, &row).await.is_err() {
            return None;
        }
        Some((agent_key, wallet_address))
    }

    fn random_private_key() -> String {
        use rand::Rng;
        format!("0x{}", hex::encode(rand::thread_rng().r#gen::<[u8; 32]>()))
    }

    #[tokio::test]
    async fn post_agents_creates_agent_with_default_strategy_prompts() {
        let state = test_state().await;
        let pool = state.db_pool.clone();

        let app = router(state);
        let timestamp = chrono::Utc::now().timestamp_millis();
        let display_name = format!("SoulTest{}", timestamp);
        let agent_key = slugify_agent_key(&display_name);
        let private_key = random_private_key();
        let body = format!(
            "display_name={}&hyperliquid_private_key={}&backend_kind=opencode&runtime_id=opencode-local",
            display_name, private_key
        );

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/agents")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        let expected_location = format!("/agents/{agent_key}");
        assert_eq!(
            response
                .headers()
                .get("location")
                .and_then(|value| value.to_str().ok()),
            Some(expected_location.as_str())
        );

        let stored = get_agent(&pool, &agent_key)
            .await
            .expect("get agent")
            .expect("agent present");
        assert_eq!(stored.analysis_prompt, DEFAULT_ANALYSIS_STRATEGY_PROMPT);
        assert_eq!(stored.trading_prompt, DEFAULT_TRADING_STRATEGY_PROMPT);
        assert_eq!(
            stored.runtime_config["workspace_container_path"],
            serde_json::json!(format!("/workspaces/agents/{agent_key}"))
        );
        assert!(
            std::path::Path::new(
                stored.runtime_config["workspace_host_path"]
                    .as_str()
                    .expect("host path string")
            )
            .join(".env")
            .exists()
        );

        // Default OpenCode schedules should have been inserted.
        let schedules = crate::agentic::store::list_agent_schedules(&pool, &agent_key)
            .await
            .expect("list schedules");
        assert_eq!(schedules.len(), 2);
        let analysis = schedules
            .iter()
            .find(|row| row.job_key == "analysis-15m")
            .expect("analysis schedule present");
        assert!(analysis.enabled);
        let trading = schedules
            .iter()
            .find(|row| row.job_key == "trading-1m")
            .expect("trading schedule present");
        assert!(!trading.enabled);
    }

    #[tokio::test]
    async fn post_agents_with_hermes_runtime_does_not_generate_opencode_workspace() {
        let state = test_state().await;
        let pool = state.db_pool.clone();
        let runtime_id = format!("hermes-local-{}", chrono::Utc::now().timestamp_millis());
        insert_agent_runtime(
            &pool,
            &CreateAgentRuntimeForm {
                id: runtime_id.clone(),
                name: "Hermes local".to_string(),
                backend_kind: crate::agents::model::BACKEND_KIND_HERMES.to_string(),
                base_url: "http://localhost:19119".to_string(),
                enabled: Some("on".to_string()),
            },
        )
        .await
        .expect("insert hermes runtime");

        let app = router(state);
        let timestamp = chrono::Utc::now().timestamp_millis();
        let display_name = format!("HermesOnly{}", timestamp);
        let agent_key = slugify_agent_key(&display_name);
        let private_key = random_private_key();
        let body = format!(
            "display_name={}&hyperliquid_private_key={}&backend_kind=hermes&runtime_id={}",
            display_name, private_key, runtime_id
        );

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/agents")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER);

        let stored = get_agent(&pool, &agent_key)
            .await
            .expect("get agent")
            .expect("agent present");
        assert_eq!(stored.runtime_config, serde_json::json!({}));

        // Hermes agents should not get default OpenCode schedules.
        let schedules = crate::agentic::store::list_agent_schedules(&pool, &agent_key)
            .await
            .expect("list schedules");
        assert!(schedules.is_empty());
    }

    #[tokio::test]
    async fn post_delete_agent_removes_agent_and_redirects() {
        let state = test_state().await;

        let app = router(state);
        let timestamp = chrono::Utc::now().timestamp_millis();
        let display_name = format!("DeleteRouteTest{}", timestamp);
        let agent_key = slugify_agent_key(&display_name);
        let private_key = random_private_key();
        let body = format!(
            "display_name={}&hyperliquid_private_key={}&backend_kind=opencode&runtime_id=opencode-local&enabled=on",
            display_name, private_key
        );

        // Create the agent.
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/agents")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER);

        // Verify the agent exists.
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{}", agent_key))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // Delete the agent.
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/agents/{}/delete", agent_key))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        let location = response
            .headers()
            .get("location")
            .expect("redirect location header")
            .to_str()
            .unwrap();
        assert_eq!(location, "/agents");

        // Verify the agent is gone.
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{}", agent_key))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn post_regenerate_workspace_refreshes_template_and_preserves_user_files() {
        let state = test_state().await;
        let pool = state.db_pool.clone();
        let app = router(state);
        let timestamp = chrono::Utc::now().timestamp_millis();
        let display_name = format!("RegenerateWorkspace{}", timestamp);
        let agent_key = slugify_agent_key(&display_name);
        let private_key = random_private_key();
        let body = format!(
            "display_name={display_name}&hyperliquid_private_key={private_key}&backend_kind=opencode&runtime_id=opencode-local&enabled=on"
        );

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/agents")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER);

        let stored = get_agent(&pool, &agent_key)
            .await
            .expect("get agent")
            .expect("agent present");
        let workspace = OpenCodeWorkspaceRuntimeConfig::from_value(&stored.runtime_config)
            .expect("workspace metadata present");
        let workspace_path = std::path::Path::new(&workspace.workspace_host_path);

        let custom_file = workspace_path.join("scripts/user/custom.py");
        fs::write(&custom_file, "print('custom')\n").expect("write custom file");

        let agents_md_path = workspace_path.join("AGENTS.md");
        let original_agents_md = fs::read_to_string(&agents_md_path).expect("read AGENTS.md");
        fs::write(&agents_md_path, "user-modified agents file\n").expect("modify AGENTS.md");

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/agents/{agent_key}/settings/regenerate-workspace"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            response
                .headers()
                .get("location")
                .and_then(|value| value.to_str().ok()),
            Some(format!("/agents/{agent_key}/settings").as_str())
        );
        assert_eq!(
            fs::read_to_string(&custom_file).expect("read custom file"),
            "print('custom')\n"
        );
        assert_eq!(
            fs::read_to_string(&agents_md_path).expect("read refreshed AGENTS.md"),
            original_agents_md
        );
    }

    #[tokio::test]
    async fn post_regenerate_workspace_for_non_opencode_agent_returns_404() {
        let state = test_state().await;
        let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");

        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/agents/{agent_key}/settings/regenerate-workspace"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn agent_chat_route_is_not_registered() {
        let state = test_state().await;
        let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");

        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{agent_key}/chat"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn agent_detail_renders_setup_alert_when_both_checkins_missing() {
        let state = test_state().await;
        let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");

        let response = router(Arc::clone(&state))
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{agent_key}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let text = response_text(response).await;
        assert!(text.contains("Hermes cron setup required"));
        assert!(text.contains("Analysis loop has not checked in."));
        assert!(text.contains("Trading loop has not checked in."));
        assert!(text.contains(&format!("Hermes profile: {agent_key}")));
    }

    #[tokio::test]
    async fn agent_detail_renders_only_analysis_missing_when_trading_checked_in_recently() {
        let state = test_state().await;
        let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
        set_job_context_timestamps(&state, &agent_key, None, Some(Utc::now())).await;

        let response = router(Arc::clone(&state))
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{agent_key}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let text = response_text(response).await;
        assert!(text.contains("Hermes cron setup required"));
        assert!(text.contains("Analysis loop has not checked in."));
        assert!(!text.contains("Trading loop has not checked in."));
    }

    #[tokio::test]
    async fn agent_detail_renders_only_trading_missing_when_analysis_checked_in_recently() {
        let state = test_state().await;
        let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
        set_job_context_timestamps(&state, &agent_key, Some(Utc::now()), None).await;

        let response = router(Arc::clone(&state))
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{agent_key}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let text = response_text(response).await;
        assert!(text.contains("Hermes cron setup required"));
        assert!(!text.contains("Analysis loop has not checked in."));
        assert!(text.contains("Trading loop has not checked in."));
    }

    #[tokio::test]
    async fn agent_detail_renders_stale_warning_for_analysis_checkin() {
        let state = test_state().await;
        let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
        set_job_context_timestamps(
            &state,
            &agent_key,
            Some(Utc::now() - chrono::Duration::minutes(31)),
            Some(Utc::now()),
        )
        .await;

        let response = router(Arc::clone(&state))
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{agent_key}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let text = response_text(response).await;
        assert!(text.contains("Hermes cron check-in is stale"));
        assert!(text.contains("Analysis loop: last checked in"));
        assert!(text.contains("expected within 30 minutes."));
        assert!(!text.contains("Trading loop: last checked in"));
    }

    #[tokio::test]
    async fn agent_detail_renders_stale_warning_for_trading_checkin() {
        let state = test_state().await;
        let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
        set_job_context_timestamps(
            &state,
            &agent_key,
            Some(Utc::now()),
            Some(Utc::now() - chrono::Duration::minutes(4)),
        )
        .await;

        let response = router(Arc::clone(&state))
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{agent_key}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let text = response_text(response).await;
        assert!(text.contains("Hermes cron check-in is stale"));
        assert!(!text.contains("Analysis loop: last checked in"));
        assert!(text.contains("Trading loop: last checked in"));
        assert!(text.contains("expected within 3 minutes."));
    }

    #[tokio::test]
    async fn agent_detail_hides_cron_alerts_when_both_checkins_are_recent() {
        let state = test_state().await;
        let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
        set_job_context_timestamps(&state, &agent_key, Some(Utc::now()), Some(Utc::now())).await;

        let response = router(Arc::clone(&state))
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{agent_key}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let text = response_text(response).await;
        assert!(!text.contains("Hermes cron setup required"));
        assert!(!text.contains("Hermes cron check-in is stale"));
    }

    #[tokio::test]
    async fn agent_transactions_route_renders_full_timeline() {
        let state = test_state().await;
        let (agent_key, wallet_address) = insert_test_agent(&state).await.expect("insert agent");
        seed_ledger_event(
            &state,
            &wallet_address,
            &format!("tx-route-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0)),
            Utc::now(),
            Decimal::new(42, 0),
        )
        .await;

        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{agent_key}/transactions"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let text = response_text(response).await;
        assert!(text.contains("Transactions"));
        assert!(text.contains("42.0000"));
    }

    #[tokio::test]
    async fn agent_transactions_route_anchors_running_balance_to_live_cash_balance() {
        let state = test_state().await;
        let (agent_key, wallet_address) = insert_test_agent(&state).await.expect("insert agent");
        seed_ledger_event(
            &state,
            &wallet_address,
            &format!(
                "tx-anchor-{}",
                Utc::now().timestamp_nanos_opt().unwrap_or(0)
            ),
            Utc::now(),
            Decimal::new(42, 0),
        )
        .await;

        let account_key = AccountKey::new(&wallet_address, "live");
        state.live_accounts.replace(
            account_key,
            AccountLiveState {
                account_address: wallet_address.clone(),
                environment: "live".to_string(),
                spot_balances: vec![crate::hyperliquid::live_state::LiveSpotBalance {
                    coin: "USDC".to_string(),
                    total: Some(Decimal::new(420017, 4)),
                    available: Some(Decimal::new(420017, 4)),
                    ..Default::default()
                }],
                ..Default::default()
            },
        );

        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{agent_key}/transactions"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let text = response_text(response).await;
        assert!(text.contains("42.0000"));
        assert!(text.contains("42.0017"));
    }

    #[test]
    fn live_cash_balance_excludes_unrealized_pnl() {
        let state = AccountLiveState {
            margin: Some(crate::hyperliquid::live_state::LiveMarginState {
                account_value: Some(Decimal::new(110, 0)),
                ..Default::default()
            }),
            spot_balances: vec![crate::hyperliquid::live_state::LiveSpotBalance {
                coin: "USDC".to_string(),
                available: Some(Decimal::new(5, 0)),
                ..Default::default()
            }],
            open_positions: vec![crate::hyperliquid::live_state::LivePosition {
                unrealized_pnl: Some(Decimal::new(10, 0)),
                ..Default::default()
            }],
            ..Default::default()
        };

        assert_eq!(live_cash_balance(&state), Some(Decimal::new(105, 0)));
    }

    #[tokio::test]
    async fn agent_positions_route_renders_latest_trade_execution_summary_under_open_orders() {
        let state = test_state().await;
        let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
        seed_memory_with_type(
            &state,
            &agent_key,
            "trade_execution",
            "Scaled out into strength",
            "Took profit on the upper band.",
        )
        .await;
        seed_memory_with_type(
            &state,
            &agent_key,
            "plan",
            "Older plan",
            "Wait for reclaim.",
        )
        .await;

        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{agent_key}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let text = response_text(response).await;
        assert!(text.contains("Open orders"));
        assert!(text.contains("Scaled out into strength"));
        assert!(!text.contains("Older plan"));
    }

    #[tokio::test]
    async fn latest_trade_execution_summary_event_renders_latest_summary() {
        let state = test_state().await;
        let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
        seed_memory_with_type(
            &state,
            &agent_key,
            "trade_execution",
            "Scaled out into strength",
            "Took profit on the upper band.",
        )
        .await;

        let event = render_latest_trade_execution_summary_event(&state.db_pool, &agent_key)
            .await
            .expect("render latest trade execution summary event");
        let text = format!("{event:?}");

        assert!(text.contains("latest-trade-execution-summary"));
        assert!(text.contains("Scaled out into strength"));
        assert!(text.contains("timeago"));
        assert!(text.contains("datetime="));
        // Trade execution summaries never carry the warning treatment —
        // only the analysis section can turn amber.
        assert!(!text.contains("text-amber-400"));
    }

    #[tokio::test]
    async fn agent_positions_route_renders_latest_analysis_summary_under_open_orders() {
        let state = test_state().await;
        let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
        let analysis = seed_memory_with_type(
            &state,
            &agent_key,
            "analysis",
            "BTC bullish continuation above 67k",
            "## Thesis\nReclaimed intraday support.",
        )
        .await;
        seed_memory_with_type(
            &state,
            &agent_key,
            "plan",
            "Older plan",
            "Wait for reclaim.",
        )
        .await;

        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{agent_key}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let text = response_text(response).await;
        assert!(text.contains(">Analysis<"));
        assert!(text.contains("BTC bullish continuation above 67k"));
        assert!(!text.contains("Older plan"));
        assert!(
            text.contains(&format!("/agents/{agent_key}/memories/{}", analysis.id)),
            "analysis summary should link to the memory detail page"
        );
    }

    #[tokio::test]
    async fn agent_positions_route_renders_empty_analysis_section_when_no_analysis_memory() {
        let state = test_state().await;
        let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");

        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{agent_key}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let text = response_text(response).await;
        assert!(text.contains(">Analysis<"));
        // Empty placeholder — no link to any memory.
        assert!(!text.contains(&format!("/agents/{agent_key}/memories/")));
    }

    #[tokio::test]
    async fn latest_analysis_summary_event_renders_latest_summary() {
        let state = test_state().await;
        let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
        let analysis = seed_memory_with_type(
            &state,
            &agent_key,
            "analysis",
            "BTC bullish continuation above 67k",
            "## Thesis\nReclaimed intraday support.",
        )
        .await;

        let event = render_latest_analysis_summary_event(&state.db_pool, &agent_key)
            .await
            .expect("render latest analysis summary event");
        let text = format!("{event:?}");

        assert!(text.contains("latest-analysis-summary"));
        assert!(text.contains("BTC bullish continuation above 67k"));
        assert!(text.contains(&format!("/agents/{agent_key}/memories/{}", analysis.id)));
        assert!(text.contains("timeago"));
        assert!(text.contains("datetime="));
        // Default analysis validity is 2x the schedule interval — the row
        // we just inserted is brand new, so it should not be flagged as
        // expired or carry a warning icon.
        assert!(!text.contains("text-amber-400"));
        assert!(!text.contains("(expired"));
    }

    #[tokio::test]
    async fn latest_analysis_summary_event_marks_expired_memory_with_warning() {
        let state = test_state().await;
        let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
        // `valid_for_seconds: 1` + no sleep on a slow CI machine still
        // lands the row in the past by the time we read it back.
        let _analysis = crate::memory::insert_memory(
            &state.db_pool,
            &agent_key,
            &CreateMemory {
                symbol: "BTC".to_string(),
                timeframe: Some("1h".to_string()),
                memory_type: "analysis".to_string(),
                summary: "Stale breakout call".to_string(),
                content: "## Thesis\nBid got pulled.".to_string(),
                metadata: Some(serde_json::json!({ "valid_for_seconds": 1 })),
            },
        )
        .await
        .expect("insert memory");
        // Make sure the row's created_at is comfortably in the past so
        // `expires_at <= now` is unambiguous regardless of clock skew.
        sqlx::query("UPDATE memory.records SET created_at = NOW() - INTERVAL '5 minutes'")
            .execute(&state.db_pool)
            .await
            .expect("backdate memory");

        let event = render_latest_analysis_summary_event(&state.db_pool, &agent_key)
            .await
            .expect("render latest analysis summary event");
        let text = format!("{event:?}");

        assert!(text.contains("Stale breakout call"));
        assert!(
            text.contains("text-amber-400"),
            "expired analysis should turn the timestamp amber"
        );
        assert!(
            text.contains("(expired"),
            "expired analysis should annotate the title"
        );
        // The inline warning triangle is the visual signal that the
        // analysis has aged past `valid_for_seconds` / `stale_after`.
        assert!(
            text.contains("viewBox=\\\"0 0 20 20\\\""),
            "expired analysis should render a warning icon"
        );
    }

    #[tokio::test]
    async fn latest_analysis_summary_event_renders_empty_when_no_analysis_memory() {
        let state = test_state().await;
        let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");

        let event = render_latest_analysis_summary_event(&state.db_pool, &agent_key)
            .await
            .expect("render latest analysis summary event");
        let text = format!("{event:?}");

        assert!(text.contains("latest-analysis-summary"));
        // Empty placeholder — no summary text leaks through.
        assert!(!text.contains("Reclaimed intraday support"));
    }

    #[tokio::test]
    async fn agent_memories_route_renders_saved_memories() {
        let state = test_state().await;
        let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
        let memory = seed_memory(
            &state,
            &agent_key,
            "Remember the breakout",
            "### Plan\n\nBTC reclaimed support.",
        )
        .await;

        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{agent_key}/memories"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let text = response_text(response).await;
        assert!(text.contains("Memories"));
        assert!(text.contains("Timeline"));
        assert!(text.contains("Remember the breakout"));
        assert!(text.contains("<h3>Plan</h3>"));
        assert!(text.contains("BTC reclaimed support."));
        assert!(text.contains(&format!("/agents/{agent_key}/memories/{}", memory.id)));
    }

    #[tokio::test]
    async fn agent_memory_detail_route_renders_partial_for_htmx() {
        let state = test_state().await;
        let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
        let memory = seed_memory(
            &state,
            &agent_key,
            "Remember the breakout",
            "### Plan\n\nBTC reclaimed support.",
        )
        .await;

        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{agent_key}/memories/{}", memory.id))
                    .header("HX-Request", "true")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let text = response_text(response).await;
        assert!(text.contains("id=\"memory-detail\""));
        assert!(text.contains("Remember the breakout"));
        assert!(text.contains("<h3>Plan</h3>"));
        // Bare partial — no base layout, no nav, no back link.
        assert!(!text.contains("<!DOCTYPE html>"));
        assert!(!text.contains("Back to"));
    }

    #[tokio::test]
    async fn agent_memory_detail_route_renders_full_page_for_direct_navigation() {
        let state = test_state().await;
        let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
        let memory = seed_memory(
            &state,
            &agent_key,
            "Remember the breakout",
            "### Plan\n\nBTC reclaimed support.",
        )
        .await;

        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{agent_key}/memories/{}", memory.id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let text = response_text(response).await;
        // Full styled page so direct navigation doesn't render unstyled HTML.
        assert!(text.contains("<!DOCTYPE html>"));
        assert!(text.contains("/static/dist/app.css"));
        assert!(text.contains("Vibetrading"));
        assert!(text.contains("id=\"memory-detail\""));
        assert!(text.contains("Remember the breakout"));
        assert!(text.contains("<h3>Plan</h3>"));
        // Back link to the agent.
        assert!(text.contains("Back to"));
        assert!(text.contains(&format!("href=\"/agents/{agent_key}\"")));
    }

    #[tokio::test]
    async fn agent_memory_detail_route_returns_404_for_unknown_memory() {
        let state = test_state().await;
        let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");

        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/agents/{agent_key}/memories/{}",
                        uuid::Uuid::new_v4()
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn memory_timeline_event_renders_refresh_without_selected_card() {
        let state = test_state().await;
        let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
        seed_memory(
            &state,
            &agent_key,
            "Remember the breakout",
            "### Plan\n\nBTC reclaimed support.",
        )
        .await;

        let event = render_memory_timeline_event(&state.db_pool, &agent_key, None, None, None)
            .await
            .expect("render event");
        let text = format!("{event:?}");

        assert!(text.contains("memories-timeline"));
        assert!(text.contains("Remember the breakout"));
        let rows = list_agent_memories(&state.db_pool, &agent_key, None, None)
            .await
            .expect("list memories");
        let timeline = crate::web::templates::build_memory_timeline_for_sse(&agent_key, &rows);
        assert!(timeline.iter().all(|item| !item.selected));
    }

    #[tokio::test]
    async fn agent_prompts_route_renders_prompt_fields() {
        let state = test_state().await;
        let (agent_key, _wallet_address) = insert_test_agent_with_text(
            &state,
            "Wait for analysis confirmation first.".to_string(),
            "Trade breakouts only after confirmation.".to_string(),
        )
        .await
        .expect("insert agent");

        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{agent_key}/prompts"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let text = response_text(response).await;
        assert!(text.contains("Wait for analysis confirmation first."));
        assert!(text.contains("Trade breakouts only after confirmation."));
    }

    #[tokio::test]
    async fn post_agent_analysis_prompt_updates_only_analysis() {
        let state = test_state().await;
        let pool = state.db_pool.clone();
        let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");

        let body = "prompt=Analyze+momentum+with+market+structure.";
        let response = router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/agents/{agent_key}/prompts/analysis"))
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        let expected_location = format!("/agents/{agent_key}/prompts");
        assert_eq!(
            response
                .headers()
                .get("location")
                .and_then(|value| value.to_str().ok()),
            Some(expected_location.as_str())
        );

        let stored = get_agent(&pool, &agent_key)
            .await
            .expect("get agent")
            .expect("agent present");
        assert_eq!(
            stored.analysis_prompt,
            "Analyze momentum with market structure."
        );

        let body2 = "prompt=Original+trading+prompt.";
        let _ = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/agents/{agent_key}/prompts/trading"))
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(body2))
                    .unwrap(),
            )
            .await
            .unwrap();

        let stored = get_agent(&pool, &agent_key)
            .await
            .expect("get agent")
            .expect("agent present");
        assert_eq!(stored.trading_prompt, "Original trading prompt.");
        assert_eq!(
            stored.analysis_prompt,
            "Analyze momentum with market structure."
        );
    }

    #[tokio::test]
    async fn post_agent_trading_prompt_updates_only_trading() {
        let state = test_state().await;
        let pool = state.db_pool.clone();
        let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");

        let body = "prompt=Only+place+limit+orders+near+support.";
        let response = router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/agents/{agent_key}/prompts/trading"))
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        let expected_location = format!("/agents/{agent_key}/prompts");
        assert_eq!(
            response
                .headers()
                .get("location")
                .and_then(|value| value.to_str().ok()),
            Some(expected_location.as_str())
        );

        let stored = get_agent(&pool, &agent_key)
            .await
            .expect("get agent")
            .expect("agent present");
        assert_eq!(
            stored.trading_prompt,
            "Only place limit orders near support."
        );

        let body2 = "prompt=Original+analysis+prompt.";
        let _ = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/agents/{agent_key}/prompts/analysis"))
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(body2))
                    .unwrap(),
            )
            .await
            .unwrap();

        let stored = get_agent(&pool, &agent_key)
            .await
            .expect("get agent")
            .expect("agent present");
        assert_eq!(stored.analysis_prompt, "Original analysis prompt.");
        assert_eq!(
            stored.trading_prompt,
            "Only place limit orders near support."
        );
    }

    #[tokio::test]
    async fn agent_settings_route_renders_sync_state() {
        let state = test_state().await;
        let (agent_key, wallet_address) = insert_test_agent(&state).await.expect("insert agent");
        seed_sync_state(&state, &wallet_address).await;
        seed_instrument(&state, "BTC", true).await;
        seed_instrument(&state, "ETH", true).await;
        replace_agent_instruments(&state.db_pool, &agent_key, &["BTC".to_string()])
            .await
            .expect("seed selected instruments");

        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{agent_key}/settings"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let text = response_text(response).await;
        assert!(text.contains("Currencies"));
        assert!(text.contains("Select the Hyperliquid perps this agent should analyze and trade."));
        assert!(text.contains("name=\"instrument_id\""));
        assert!(text.contains("value=\"BTC\""));
        assert!(text.contains("value=\"ETH\""));
        assert!(text.contains("value=\"BTC\" checked"));
        assert!(text.contains("Sync status"));
        assert!(text.contains("fills"));
        assert!(text.contains("abc123"));
    }

    #[tokio::test]
    async fn opencode_agent_settings_route_renders_workspace_state() {
        let state = test_state().await;
        let app = router(Arc::clone(&state));
        let timestamp = chrono::Utc::now().timestamp_millis();
        let display_name = format!("OpenCodeSettings{}", timestamp);
        let agent_key = slugify_agent_key(&display_name);
        let private_key = random_private_key();
        let body = format!(
            "display_name={}&hyperliquid_private_key={}&backend_kind=opencode&runtime_id=opencode-local",
            display_name, private_key
        );

        let create_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/agents")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(create_response.status(), StatusCode::SEE_OTHER);

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{agent_key}/settings"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let text = response_text(response).await;
        assert!(text.contains("OpenCode workspace"));
        assert!(text.contains("Container workspace path"));
        assert!(text.contains("Profile source"));
        assert!(text.contains("Workspace .env"));
    }

    #[tokio::test]
    async fn jobs_route_renders_create_job_button_and_runs_section() {
        let state = test_state().await;
        let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
            .await
            .expect("insert opencode agent");

        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{agent_key}/jobs"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let text = response_text(response).await;
        assert!(text.contains("Create job"));
        assert!(text.contains(&format!("/agents/{agent_key}/jobs/new")));
        assert!(text.contains("Scheduled Jobs"));
        assert!(text.contains("Hook Jobs"));
        assert!(text.contains("Recent Runs"));
        assert!(text.contains("Create hook"));
        assert!(text.contains(&format!("/agents/{agent_key}/hooks/new")));
        assert!(text.contains("Run now"));
    }

    #[tokio::test]
    async fn new_hook_page_renders_for_opencode_agent() {
        let state = test_state().await;
        let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
            .await
            .expect("insert opencode agent");

        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{agent_key}/hooks/new"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let text = response_text(response).await;
        assert!(text.contains("Create market-analysis hook"));
        assert!(text.contains("analysis_batch_completed"));
        assert!(text.contains("name=\"timeout_seconds\""));
        assert!(text.contains("name=\"model_provider_id\""));
        assert!(text.contains("name=\"model_id\""));
        assert!(text.contains("name=\"operator_prompt\""));
    }

    #[tokio::test]
    async fn post_hook_create_inserts_hook_and_redirects() {
        let state = test_state().await;
        let pool = state.db_pool.clone();
        let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
            .await
            .expect("insert opencode agent");

        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/agents/{agent_key}/hooks"))
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(
                        "timeout_seconds=600&enabled=on&model_provider_id=anthropic&model_id=claude-sonnet-4&operator_prompt=Summarize+multi-timeframe+agreement",
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        let hooks = crate::agentic::store::list_agent_hooks(&pool, &agent_key)
            .await
            .expect("list hooks");
        let hook = hooks.first().expect("hook present");
        assert_eq!(hook.job_key, "market-analysis");
        assert_eq!(hook.job_kind, JOB_KIND_MARKET_ANALYSIS);
    }

    #[tokio::test]
    async fn post_job_run_now_queues_and_dispatches_run() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let backend = Arc::new(RecordingAgenticBackend {
            calls: Arc::clone(&calls),
        });
        let state = test_state_with_backend(backend).await;
        let pool = state.db_pool.clone();
        let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
            .await
            .expect("insert opencode agent");
        let schedules = crate::agentic::store::list_agent_schedules(&pool, &agent_key)
            .await
            .expect("list schedules");
        seed_instrument(&state, "BTC", true).await;
        replace_agent_instruments(&pool, &agent_key, &["BTC".to_string()])
            .await
            .expect("seed instruments");
        let schedule_id = schedules
            .iter()
            .find(|row| row.job_key == "analysis-15m")
            .map(|row| row.id)
            .expect("analysis schedule id");

        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/agents/{agent_key}/jobs/{schedule_id}/run"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            response
                .headers()
                .get("location")
                .and_then(|value| value.to_str().ok()),
            Some(format!("/agents/{agent_key}/jobs").as_str())
        );

        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(1);
        loop {
            if !calls.lock().unwrap().is_empty() {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "dispatch was not spawned"
            );
            tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
        }

        let recorded = calls.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].schedule_id, Some(schedule_id));
        assert_eq!(recorded[0].agent_key, agent_key);
        assert_eq!(recorded[0].job_key, "analysis-15m");
        drop(recorded);

        let runs = crate::agentic::store::list_agent_runs(&pool, &agent_key, 10)
            .await
            .expect("list runs");
        assert!(runs.iter().any(|run| run.schedule_id == Some(schedule_id)));
    }

    #[tokio::test]
    async fn post_analysis_job_run_now_triggers_market_analysis_hook_after_success() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let backend = Arc::new(RecordingAgenticBackend {
            calls: Arc::clone(&calls),
        });
        let state = test_state_with_backend(backend).await;
        let pool = state.db_pool.clone();
        let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
            .await
            .expect("insert opencode agent");
        seed_instrument(&state, "BTC", true).await;
        replace_agent_instruments(&pool, &agent_key, &["BTC".to_string()])
            .await
            .expect("seed instruments");
        crate::agentic::store::insert_agent_hook(
            &pool,
            &agent_key,
            JOB_KIND_MARKET_ANALYSIS,
            HOOK_EVENT_ANALYSIS_BATCH_COMPLETED,
            true,
            None,
            None,
            600,
            "",
        )
        .await
        .expect("insert hook");
        let schedule_id = crate::agentic::store::list_agent_schedules(&pool, &agent_key)
            .await
            .expect("list schedules")
            .into_iter()
            .find(|row| row.job_key == "analysis-15m")
            .map(|row| row.id)
            .expect("analysis schedule id");

        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/agents/{agent_key}/jobs/{schedule_id}/run"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SEE_OTHER);

        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(1);
        loop {
            if calls.lock().unwrap().len() >= 2 {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "analysis hook dispatch was not spawned"
            );
            tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
        }

        let recorded = calls.lock().unwrap();
        let job_keys: Vec<&str> = recorded
            .iter()
            .map(|request| request.job_key.as_str())
            .collect();
        assert!(job_keys.contains(&"analysis-15m"));
        assert!(job_keys.contains(&"market-analysis"));
    }

    #[tokio::test]
    async fn post_analysis_job_run_now_skipped_does_not_trigger_market_analysis_hook() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let backend = Arc::new(RecordingAgenticBackend {
            calls: Arc::clone(&calls),
        });
        let state = test_state_with_backend(backend).await;
        let pool = state.db_pool.clone();
        let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
            .await
            .expect("insert opencode agent");
        crate::agentic::store::insert_agent_hook(
            &pool,
            &agent_key,
            JOB_KIND_MARKET_ANALYSIS,
            HOOK_EVENT_ANALYSIS_BATCH_COMPLETED,
            true,
            None,
            None,
            600,
            "",
        )
        .await
        .expect("insert hook");
        let schedule_id = crate::agentic::store::list_agent_schedules(&pool, &agent_key)
            .await
            .expect("list schedules")
            .into_iter()
            .find(|row| row.job_key == "analysis-15m")
            .map(|row| row.id)
            .expect("analysis schedule id");
        let active_run_id = crate::agentic::store::insert_test_run(&pool, schedule_id, "running")
            .await
            .expect("insert active run");
        crate::agentic::store::mark_run_running(&pool, active_run_id, Some("ses_active"))
            .await
            .expect("mark active run running");

        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/agents/{agent_key}/jobs/{schedule_id}/run"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        assert!(calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn post_trading_job_run_now_does_not_trigger_market_analysis_hook() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let backend = Arc::new(RecordingAgenticBackend {
            calls: Arc::clone(&calls),
        });
        let state = test_state_with_backend(backend).await;
        let pool = state.db_pool.clone();
        let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
            .await
            .expect("insert opencode agent");
        seed_instrument(&state, "BTC", true).await;
        replace_agent_instruments(&pool, &agent_key, &["BTC".to_string()])
            .await
            .expect("seed instruments");
        crate::agentic::store::insert_agent_hook(
            &pool,
            &agent_key,
            JOB_KIND_MARKET_ANALYSIS,
            HOOK_EVENT_ANALYSIS_BATCH_COMPLETED,
            true,
            None,
            None,
            600,
            "",
        )
        .await
        .expect("insert hook");
        let schedule_id = crate::agentic::store::list_agent_schedules(&pool, &agent_key)
            .await
            .expect("list schedules")
            .into_iter()
            .find(|row| row.job_key == "trading-1m")
            .map(|row| row.id)
            .expect("trading schedule id");

        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/agents/{agent_key}/jobs/{schedule_id}/run"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SEE_OTHER);

        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(1);
        loop {
            if !calls.lock().unwrap().is_empty() {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "trading dispatch was not spawned"
            );
            tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        let recorded = calls.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].job_key, "trading-1m");
        assert_eq!(recorded[0].job_kind, JOB_KIND_TRADING);
    }

    #[tokio::test]
    async fn post_hook_run_now_queues_and_dispatches_run() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let backend = Arc::new(RecordingAgenticBackend {
            calls: Arc::clone(&calls),
        });
        let state = test_state_with_backend(backend).await;
        let pool = state.db_pool.clone();
        let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
            .await
            .expect("insert opencode agent");
        seed_instrument(&state, "BTC", true).await;
        replace_agent_instruments(&pool, &agent_key, &["BTC".to_string()])
            .await
            .expect("seed instruments");
        let hook_id = crate::agentic::store::insert_agent_hook(
            &pool,
            &agent_key,
            JOB_KIND_MARKET_ANALYSIS,
            HOOK_EVENT_ANALYSIS_BATCH_COMPLETED,
            true,
            None,
            None,
            600,
            "",
        )
        .await
        .expect("insert hook");

        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/agents/{agent_key}/hooks/{hook_id}/run"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SEE_OTHER);

        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(1);
        loop {
            if !calls.lock().unwrap().is_empty() {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "hook dispatch was not spawned"
            );
            tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
        }

        let recorded = calls.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].schedule_id, None);
        assert_eq!(recorded[0].hook_id, Some(hook_id));
        assert_eq!(recorded[0].job_key, "market-analysis");
    }

    #[tokio::test]
    async fn post_hook_toggle_updates_enabled_state() {
        let state = test_state().await;
        let pool = state.db_pool.clone();
        let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
            .await
            .expect("insert opencode agent");
        let hook_id = crate::agentic::store::insert_agent_hook(
            &pool,
            &agent_key,
            JOB_KIND_MARKET_ANALYSIS,
            HOOK_EVENT_ANALYSIS_BATCH_COMPLETED,
            true,
            None,
            None,
            600,
            "",
        )
        .await
        .expect("insert hook");

        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/agents/{agent_key}/hooks/{hook_id}/toggle"))
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from("enabled=off"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        let hook = crate::agentic::store::get_agent_hook(&pool, &agent_key, hook_id)
            .await
            .expect("get hook")
            .expect("hook present");
        assert!(!hook.enabled);
    }

    #[tokio::test]
    async fn post_hook_delete_removes_hook() {
        let state = test_state().await;
        let pool = state.db_pool.clone();
        let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
            .await
            .expect("insert opencode agent");
        let hook_id = crate::agentic::store::insert_agent_hook(
            &pool,
            &agent_key,
            JOB_KIND_MARKET_ANALYSIS,
            HOOK_EVENT_ANALYSIS_BATCH_COMPLETED,
            true,
            None,
            None,
            600,
            "",
        )
        .await
        .expect("insert hook");

        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/agents/{agent_key}/hooks/{hook_id}/delete"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert!(
            crate::agentic::store::get_agent_hook(&pool, &agent_key, hook_id)
                .await
                .expect("get hook")
                .is_none()
        );
    }

    #[tokio::test]
    async fn new_job_page_renders_for_opencode_agent() {
        let state = test_state().await;
        let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
            .await
            .expect("insert opencode agent");

        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{agent_key}/jobs/new"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let text = response_text(response).await;
        assert!(text.contains("Create job"));
        assert!(!text.contains("name=\"job_key\""));
        assert!(text.contains("name=\"job_kind\""));
        assert!(text.contains("name=\"timeframe\""));
        assert!(text.contains("name=\"timeout_seconds\""));
    }

    #[tokio::test]
    async fn post_job_creates_new_schedule_and_redirects() {
        let state = test_state().await;
        let pool = state.db_pool.clone();
        let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
            .await
            .expect("insert opencode agent");

        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/agents/{agent_key}/jobs"))
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(
                        "job_kind=analysis&timeframe=1h&timeout_seconds=600&enabled=on&model_provider_id=anthropic&model_id=claude-sonnet-4&operator_prompt=Check+higher+timeframe+structure",
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            response
                .headers()
                .get("location")
                .and_then(|value| value.to_str().ok()),
            Some(format!("/agents/{agent_key}/jobs").as_str())
        );

        let schedules = crate::agentic::store::list_agent_schedules(&pool, &agent_key)
            .await
            .expect("list schedules");
        let schedule = schedules
            .iter()
            .find(|row| row.job_key == "analysis-1h")
            .expect("custom schedule present");
        assert_eq!(schedule.job_kind, JOB_KIND_ANALYSIS);
        assert!(schedule.enabled);
        assert_eq!(schedule.timeframe, "1h");
        assert_eq!(schedule.timeout_seconds, 600);
        assert_eq!(schedule.model_provider_id.as_deref(), Some("anthropic"));
        assert_eq!(schedule.model_id.as_deref(), Some("claude-sonnet-4"));
        assert_eq!(schedule.operator_prompt, "Check higher timeframe structure");
    }

    #[tokio::test]
    async fn post_job_with_duplicate_job_kind_timeframe_returns_validation_error() {
        let state = test_state().await;
        let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
            .await
            .expect("insert opencode agent");

        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/agents/{agent_key}/jobs"))
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(
                        "job_kind=analysis&timeframe=15m&timeout_seconds=600&enabled=on",
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let text = response_text(response).await;
        assert!(text.contains("A job with this kind and timeframe already exists for this agent."));
    }

    #[tokio::test]
    async fn job_detail_page_renders_job_specific_runs() {
        let state = test_state().await;
        let pool = state.db_pool.clone();
        let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
            .await
            .expect("insert opencode agent");

        let schedules = crate::agentic::store::list_agent_schedules(&pool, &agent_key)
            .await
            .expect("list schedules");
        let schedule_id = schedules.first().expect("default schedule").id;
        let run_id = crate::agentic::store::insert_test_run(&pool, schedule_id, "running")
            .await
            .expect("insert run");
        crate::agentic::store::mark_run_succeeded(&pool, run_id, Some("ses_job_detail"))
            .await
            .expect("mark succeeded");

        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{agent_key}/jobs/{schedule_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let text = response_text(response).await;
        assert!(text.contains("Job details"));
        assert!(text.contains("ses_job_detail"));
        assert!(text.contains(&format!("/agents/{agent_key}/runs/{run_id}")));
    }

    #[tokio::test]
    async fn hook_detail_page_renders_hook_specific_runs() {
        let state = test_state().await;
        let pool = state.db_pool.clone();
        let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
            .await
            .expect("insert opencode agent");
        seed_instrument(&state, "BTC", true).await;
        replace_agent_instruments(&pool, &agent_key, &["BTC".to_string()])
            .await
            .expect("seed instruments");
        let hook_id = crate::agentic::store::insert_agent_hook(
            &pool,
            &agent_key,
            JOB_KIND_MARKET_ANALYSIS,
            HOOK_EVENT_ANALYSIS_BATCH_COMPLETED,
            true,
            None,
            None,
            600,
            "",
        )
        .await
        .expect("insert hook");
        let run_id = match crate::agentic::store::insert_queued_hook_run(&pool, &agent_key, hook_id)
            .await
            .expect("insert hook run")
        {
            QueuedHookRun::Dispatch { run_id, .. } => run_id,
            other => panic!("expected Dispatch, got {other:?}"),
        };
        crate::agentic::store::mark_run_succeeded(&pool, run_id, Some("ses_hook_detail"))
            .await
            .expect("mark succeeded");

        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{agent_key}/hooks/{hook_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let text = response_text(response).await;
        assert!(text.contains("Hook details"));
        assert!(text.contains("analysis_batch_completed"));
        assert!(text.contains("ses_hook_detail"));
        assert!(text.contains(&format!("/agents/{agent_key}/runs/{run_id}")));
    }

    #[tokio::test]
    async fn run_detail_page_handles_missing_opencode_session_mirror() {
        let state = test_state().await;
        let pool = state.db_pool.clone();
        let (agent_key, _wallet_address) = insert_test_opencode_agent(&state)
            .await
            .expect("insert opencode agent");

        let schedules = crate::agentic::store::list_agent_schedules(&pool, &agent_key)
            .await
            .expect("list schedules");
        let schedule_id = schedules.first().expect("default schedule").id;
        let run_id = crate::agentic::store::insert_test_run(&pool, schedule_id, "running")
            .await
            .expect("insert run");
        crate::agentic::store::mark_run_succeeded(&pool, run_id, Some("ses_ui_detail"))
            .await
            .expect("mark succeeded");

        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{agent_key}/runs/{run_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let text = response_text(response).await;
        assert!(text.contains("OpenCode session"));
        assert!(text.contains("ses_ui_detail"));
        assert!(text.contains("no matching row was found yet in the"));
    }

    #[tokio::test]
    async fn post_agent_instruments_updates_selection_and_redirects() {
        let state = test_state().await;
        let pool = state.db_pool.clone();
        let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
        seed_instrument(&state, "BTC", true).await;
        seed_instrument(&state, "ETH", true).await;

        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/agents/{agent_key}/settings/instruments"))
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from("instrument_id=BTC&instrument_id=ETH"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        let expected_location = format!("/agents/{agent_key}/settings");
        assert_eq!(
            response
                .headers()
                .get("location")
                .and_then(|value| value.to_str().ok()),
            Some(expected_location.as_str())
        );

        let selected = list_agent_instrument_ids(&pool, &agent_key)
            .await
            .expect("list selected instruments");
        assert_eq!(selected, vec!["BTC".to_string(), "ETH".to_string()]);
    }

    #[tokio::test]
    async fn post_agent_instruments_without_values_clears_selection_and_redirects() {
        let state = test_state().await;
        let pool = state.db_pool.clone();
        let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
        seed_instrument(&state, "BTC", true).await;
        replace_agent_instruments(&pool, &agent_key, &["BTC".to_string()])
            .await
            .expect("seed selected instruments");

        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/agents/{agent_key}/settings/instruments"))
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SEE_OTHER);

        let selected = list_agent_instrument_ids(&pool, &agent_key)
            .await
            .expect("list selected instruments");
        assert!(selected.is_empty());
    }

    #[tokio::test]
    async fn unknown_agent_subroute_returns_404() {
        let state = test_state().await;

        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri("/agents/does-not-exist-12345/settings")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn open_positions_stream_returns_404_for_unknown_agent() {
        let state = test_state().await;

        let app = router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/agents/does-not-exist-12345/live/stream")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    #[ignore = "SSE body read hangs in pglite-oxide test environment; see AGENTS.md"]
    async fn open_positions_stream_emits_initial_loading_placeholder() {
        let state = test_state().await;

        let (agent_key, _wallet_address) = match insert_test_agent(&state).await {
            Some(pair) => pair,
            None => return,
        };

        let app = router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{}/live/stream", agent_key))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok()),
            Some("text/event-stream")
        );

        let text = read_sse_chunk(response.into_body(), 250).await;
        assert!(
            text.contains("event: positions"),
            "missing event line in {text}"
        );
        assert!(text.contains("Loading"), "expected placeholder in {text}");
    }

    #[tokio::test]
    #[ignore = "SSE body read hangs in pglite-oxide test environment; see AGENTS.md"]
    async fn open_positions_stream_emits_initial_rows_when_state_present() {
        let state = test_state().await;

        let (agent_key, wallet_address) = match insert_test_agent(&state).await {
            Some(pair) => pair,
            None => return,
        };

        let key = AccountKey::new(&wallet_address, "live");
        state.live_accounts.replace(
            key.clone(),
            AccountLiveState {
                account_address: key.account_address.clone(),
                environment: key.environment.clone(),
                status: LiveConnectionStatus::Connected,
                updated_at: Some(Utc::now()),
                open_positions: vec![crate::hyperliquid::live_state::LivePosition {
                    coin: "BTC".to_string(),
                    szi: Some(rust_decimal::Decimal::new(1, 0)),
                    entry_px: Some(rust_decimal::Decimal::new(30000, 0)),
                    liquidation_px: Some(rust_decimal::Decimal::new(25000, 0)),
                    margin_used: Some(rust_decimal::Decimal::new(6000, 0)),
                    position_value: Some(rust_decimal::Decimal::new(30000, 0)),
                    unrealized_pnl: Some(rust_decimal::Decimal::new(1500, 0)),
                    return_on_equity: Some(rust_decimal::Decimal::new(25, 2)),
                    leverage_type: Some("cross".to_string()),
                    leverage_value: Some(5),
                    max_leverage: Some(50),
                }],
                ..Default::default()
            },
        );

        let app = router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{}/live/stream", agent_key))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let text = read_sse_chunk(response.into_body(), 250).await;
        assert!(text.contains("event: positions"));
        assert!(text.contains("BTC"));
        assert!(text.contains("long"));
        assert!(text.contains("+25.00%"));
    }

    #[tokio::test]
    async fn open_orders_stream_returns_404_for_unknown_agent() {
        let state = test_state().await;

        let app = router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/agents/does-not-exist-12345/live/stream")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    #[ignore = "SSE body read hangs in pglite-oxide test environment; see AGENTS.md"]
    async fn open_orders_stream_emits_initial_loading_placeholder() {
        let state = test_state().await;

        let (agent_key, _wallet_address) = match insert_test_agent(&state).await {
            Some(pair) => pair,
            None => return,
        };

        let app = router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{}/live/stream", agent_key))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok()),
            Some("text/event-stream")
        );

        let text = read_sse_chunk(response.into_body(), 250).await;
        assert!(
            text.contains("event: orders"),
            "missing event line in {text}"
        );
        assert!(text.contains("Loading"), "expected placeholder in {text}");
    }

    #[tokio::test]
    #[ignore = "SSE body read hangs in pglite-oxide test environment; see AGENTS.md"]
    async fn open_orders_stream_emits_initial_rows_when_state_present() {
        let state = test_state().await;

        let (agent_key, wallet_address) = match insert_test_agent(&state).await {
            Some(pair) => pair,
            None => return,
        };

        let key = AccountKey::new(&wallet_address, "live");
        state.live_accounts.replace(
            key.clone(),
            AccountLiveState {
                account_address: key.account_address.clone(),
                environment: key.environment.clone(),
                status: LiveConnectionStatus::Connected,
                updated_at: Some(Utc::now()),
                open_orders: vec![crate::hyperliquid::live_state::LiveOpenOrder {
                    coin: "ETH".to_string(),
                    side: Some("buy".to_string()),
                    limit_px: Some(rust_decimal::Decimal::new(1900, 0)),
                    sz: Some(rust_decimal::Decimal::new(1, 0)),
                    orig_sz: Some(rust_decimal::Decimal::new(1, 0)),
                    oid: Some("123".to_string()),
                    timestamp: Some(chrono::Utc::now().timestamp_millis() as u64),
                    cloid: None,
                    order_type: Some("limit".to_string()),
                    tif: Some("Gtc".to_string()),
                    reduce_only: Some(false),
                    is_trigger: Some(false),
                    trigger_px: None,
                    trigger_condition: None,
                    is_position_tpsl: Some(false),
                }],
                ..Default::default()
            },
        );

        let app = router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{}/live/stream", agent_key))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let text = read_sse_chunk(response.into_body(), 250).await;
        assert!(text.contains("event: positions"));
        assert!(text.contains("BTC"));
        assert!(text.contains("long"));
        assert!(text.contains("+25.00%"));
    }
}
