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

use super::shared::{
    ModelPickerContext, ModelSelectionForm, TimeoutErrorQuery, TimeoutForm,
    WORKSPACE_MAINTENANCE_ACTIVE_WARNING, build_model_picker_view, jobs_warning_redirect,
    load_model_picker_context, load_model_picker_context_cached, parse_positive_schedule_seconds,
    timeout_error_redirect, validate_model_selection_for_agent,
};
use crate::web::error::AppError;
use crate::{
    agentic::{
        model::{
            HOOK_EVENT_ANALYSIS_BATCH_COMPLETED, JOB_KIND_ANALYSIS_CODING, JOB_KIND_MARKET_ANALYSIS,
        },
        scheduler::{build_hook_dispatch_request, dispatch_run_with_workspace_lease},
        store::QueuedHookRun,
        timeframe::parse_timeout_seconds,
    },
    agents::store::get_agent,
    model_catalog::options::parse_model_selection,
    web::{
        AppState,
        templates::{
            AgentHookDetailPageTemplate, AgentHookNewPageTemplate, AgentShowTab,
            CreateAgentHookFormValues, ModelPickerPartialTemplate, build_agent_show_tabs,
        },
    },
};
pub(in crate::web::routes) async fn agents_show_hook_detail(
    State(state): State<Arc<AppState>>,
    Path((agent_key, hook_id)): Path<(String, i64)>,
    Query(query): Query<TimeoutErrorQuery>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
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

    let mut model_picker = build_model_picker_view(
        "hook-model-selection",
        &hook_view.model_selection,
        ModelPickerContext {
            options: Vec::new(),
            warning: None,
        },
    );
    model_picker.show_label = false;
    model_picker.use_modal = true;
    model_picker.lazy_options_url =
        Some(format!("/agents/{agent_key}/hooks/{hook_id}/model-picker"));
    let html = AgentHookDetailPageTemplate::render_view(
        agent.clone(),
        hook_view,
        model_picker,
        hook_runs,
        hook_runs_loaded,
    )?;
    Ok(Html(html).into_response())
}
pub(in crate::web::routes) async fn agents_hook_model_picker(
    State(state): State<Arc<AppState>>,
    Path((agent_key, hook_id)): Path<(String, i64)>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let Some(hook) =
        crate::agentic::store::get_agent_hook(&state.db_pool, &agent_key, hook_id).await?
    else {
        return Ok((StatusCode::NOT_FOUND, "hook not found").into_response());
    };

    let selected = match (hook.model_provider_id.as_deref(), hook.model_id.as_deref()) {
        (Some(provider), Some(model)) => format!("{provider}/{model}"),
        _ => String::new(),
    };
    let picker = load_model_picker_context_cached(&state, &agent).await;
    let mut model_picker = build_model_picker_view("hook-model-selection", &selected, picker);
    model_picker.show_label = false;
    model_picker.use_modal = true;
    let html = ModelPickerPartialTemplate::render_view(model_picker)?;
    Ok(Html(html).into_response())
}
pub(in crate::web::routes) async fn build_hook_prompt_preview(
    state: &Arc<AppState>,
    agent_key: &str,
    hook_id: i64,
) -> anyhow::Result<String> {
    let hook = crate::agentic::store::get_opencode_hook_for_dispatch(
        &state.db_pool,
        agent_key,
        hook_id,
        &state.opencode_base_url,
    )
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

        if self.enabled() && model_selection.is_none() {
            errors.push("A model is required to enable a hook job.".to_string());
        }

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
    user: crate::web::auth::AuthenticatedUser,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let picker = load_model_picker_context(&state, &agent).await;
    let navbar = crate::web::templates::load_navbar(&state.db_pool, user.id)
        .await
        .unwrap_or_default();
    Ok(render_new_hook_form(
        agent,
        CreateAgentHookForm::defaults().as_template_values(),
        picker,
        Vec::new(),
        StatusCode::OK,
        navbar,
    ))
}
#[derive(Debug, Default, Deserialize)]
pub(in crate::web::routes) struct ToggleHookForm {
    pub enabled: Option<String>,
}
pub(in crate::web::routes) async fn agents_create_hook(
    State(state): State<Arc<AppState>>,
    user: crate::web::auth::AuthenticatedUser,
    Path(agent_key): Path<String>,
    Form(form): Form<CreateAgentHookForm>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let navbar = || async {
        crate::web::templates::load_navbar(&state.db_pool, user.id)
            .await
            .unwrap_or_default()
    };

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
                navbar().await,
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
                    navbar().await,
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
            navbar().await,
        ));
    }

    Ok(Redirect::to(&format!("/agents/{agent_key}/jobs")).into_response())
}
pub(in crate::web::routes) async fn agents_run_hook_now(
    State(state): State<Arc<AppState>>,
    Path((agent_key, hook_id)): Path<(String, i64)>,
) -> Result<Response, AppError> {
    let Some(_agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let Some(hook) = crate::agentic::store::get_opencode_hook_for_dispatch(
        &state.db_pool,
        &agent_key,
        hook_id,
        &state.opencode_base_url,
    )
    .await?
    else {
        return Ok((StatusCode::NOT_FOUND, "hook not found").into_response());
    };

    if hook.model_provider_id.is_none() || hook.model_id.is_none() {
        return Ok(jobs_warning_redirect(&agent_key, "No model set"));
    }

    if hook.job_kind == JOB_KIND_ANALYSIS_CODING {
        let outcome = crate::agentic::store::insert_analysis_coding_task_and_run(
            &state.db_pool,
            &agent_key,
            hook_id,
            crate::agentic::store::CodingTriggerMode::Manual,
            None,
            None,
            Some(&hook.operator_prompt),
            None,
        )
        .await
        .map_err(AppError)?;
        if let crate::agentic::store::InsertAnalysisCodingTaskOutcome::Inserted { run_id, .. } =
            outcome
        {
            return Ok(Redirect::to(&format!("/agents/{agent_key}/runs/{run_id}")).into_response());
        }
        return Ok(Redirect::to(&format!("/agents/{agent_key}/jobs")).into_response());
    }

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
                let workspace_leases = state.workspace_leases.clone();
                // Hook jobs are intentionally allowed to start after
                // shutdown has been initiated; the tracker guard below
                // makes sure the dispatch is awaited on the way out so
                // we don't exit with a half-completed run row.
                let in_flight = state.in_flight.clone();
                tokio::spawn(async move {
                    let _guard = in_flight.track();
                    let _ = dispatch_run_with_workspace_lease(
                        pool,
                        backend,
                        request,
                        &workspace_leases,
                    )
                    .await;
                });
                return Ok(
                    Redirect::to(&format!("/agents/{agent_key}/runs/{run_id}")).into_response()
                );
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
        QueuedHookRun::Skipped { run_id } => {
            return Ok(Redirect::to(&format!("/agents/{agent_key}/runs/{run_id}")).into_response());
        }
        QueuedHookRun::Missing => {
            return Ok((StatusCode::NOT_FOUND, "hook not found").into_response());
        }
        QueuedHookRun::BlockedByMaintenance => {
            return Ok(jobs_warning_redirect(
                &agent_key,
                WORKSPACE_MAINTENANCE_ACTIVE_WARNING,
            ));
        }
    };

    Ok(Redirect::to(&format!("/agents/{agent_key}/jobs")).into_response())
}

pub(in crate::web::routes) async fn agents_toggle_hook(
    State(state): State<Arc<AppState>>,
    Path((agent_key, hook_id)): Path<(String, i64)>,
    Form(form): Form<ToggleHookForm>,
) -> Result<Response, AppError> {
    let Some(_agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let enable = matches!(form.enabled.as_deref(), Some("on"));
    let Some(hook) =
        crate::agentic::store::get_agent_hook(&state.db_pool, &agent_key, hook_id).await?
    else {
        return Ok((StatusCode::NOT_FOUND, "hook not found").into_response());
    };
    if enable && (hook.model_provider_id.is_none() || hook.model_id.is_none()) {
        let message = if hook.job_kind == JOB_KIND_ANALYSIS_CODING {
            "Select an explicit provider and model before enabling analysis coding."
        } else {
            "No model set"
        };
        return Ok(jobs_warning_redirect(&agent_key, message));
    }
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
    let Some(_agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
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
    navbar: crate::web::templates::Navbar,
) -> Response {
    let current_path = format!("/agents/{}/hooks/new", agent.agent_key);
    let model_picker =
        build_model_picker_view("hook-model-selection", &form.model_selection, picker);
    let template = AgentHookNewPageTemplate {
        tabs: build_agent_show_tabs(&agent, AgentShowTab::Jobs),
        agent_tabs_use_htmx: false,
        agent,
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
pub(in crate::web::routes) fn hook_unique_violation_message(
    error: &anyhow::Error,
) -> Option<String> {
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
