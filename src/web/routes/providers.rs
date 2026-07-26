use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use askama::Template;
use axum::{
    Form,
    extract::{Path, Query, State},
    http::{HeaderValue, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
};
use serde::Deserialize;

use crate::{
    opencode::client::{
        OpenCodeOAuthCompletionMode, OpenCodeProviderAuthMethod, OpenCodeProviderAuthPrompt,
        OpenCodeProviderAuthWhenOp,
    },
    web::{
        AppState,
        auth::AuthenticatedUser,
        error::AppError,
        provider_connections::OAuthAttempt,
        templates::{
            Navbar, ProviderAuthMethodView, ProviderAuthOptionView, ProviderAuthPromptView,
            ProviderAuthWhenView, ProviderConnectionFormTemplate,
            ProviderConnectionModalStepTemplate, ProviderConnectionView,
            ProviderOAuthPendingModalStepTemplate, ProviderOAuthPendingTemplate,
            ProviderReloadStatusTemplate, ProviderReloadStatusView, ProvidersPageTemplate,
            load_navbar,
        },
    },
};

const OPENAI_BROWSER_METHOD: &str = "ChatGPT Pro/Plus (browser)";
const OPENAI_BROWSER_WARNING: &str =
    "This requires a local OpenCode TUI; use the headless device method instead.";
const SYNTHETIC_API_KEY_LABEL: &str = "API key";

fn provider_declares_api_key_env(provider: &crate::opencode::client::OpenCodeProviderInfo) -> bool {
    provider
        .env
        .iter()
        .any(|env| env.ends_with("_API_KEY") || env.ends_with("_TOKEN") || env == "API_KEY")
}

fn synthetic_api_key_method() -> OpenCodeProviderAuthMethod {
    OpenCodeProviderAuthMethod {
        auth_type: "api".to_string(),
        label: SYNTHETIC_API_KEY_LABEL.to_string(),
        prompts: Vec::new(),
    }
}

fn resolve_provider_methods<'a>(
    provider: &'a crate::opencode::client::OpenCodeProviderInfo,
    advertised: Option<&'a Vec<OpenCodeProviderAuthMethod>>,
) -> Vec<OpenCodeProviderAuthMethod> {
    if let Some(methods) = advertised.filter(|methods| !methods.is_empty()) {
        return methods.clone();
    }
    if provider_declares_api_key_env(provider) {
        vec![synthetic_api_key_method()]
    } else {
        Vec::new()
    }
}

#[derive(Debug, Default, Deserialize)]
pub(in crate::web::routes) struct ProvidersQuery {
    pub result: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub(in crate::web::routes) struct ProviderMethodQuery {
    pub method: Option<usize>,
    pub modal: bool,
}

pub(in crate::web::routes) async fn providers_index(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Query(query): Query<ProvidersQuery>,
) -> Result<Response, AppError> {
    let navbar = load_navbar(&state.db_pool, user.id).await?;
    let result = query.result.as_deref();
    let (notice, error) = match result {
        Some("reload_queued") => (
            Some("Config reload queued. It will run when agent sessions go idle.".to_string()),
            None,
        ),
        Some("failed") => (
            None,
            Some("The provider operation failed. Try again.".to_string()),
        ),
        _ => (None, None),
    };

    let directory = &state.opencode_container_workspaces_root;
    let provider_result = state
        .opencode_client
        .list_providers(&state.opencode_base_url, directory)
        .await;
    let auth_result = state
        .opencode_client
        .list_provider_auth_methods(&state.opencode_base_url, directory)
        .await;
    let (providers, load_error) = match (provider_result, auth_result) {
        (Ok(providers), Ok(methods)) => (build_provider_views(&providers, &methods), None),
        _ => (
            Vec::new(),
            Some("OpenCode provider information is temporarily unavailable.".to_string()),
        ),
    };
    let reload_task = crate::agentic::store::get_latest_provider_config_reload_task(&state.db_pool)
        .await
        .unwrap_or(None);
    let reload_status = ProviderReloadStatusView::from_task(reload_task);
    let template = ProvidersPageTemplate {
        current_path: "/providers".to_string(),
        connected_count: providers
            .iter()
            .filter(|provider| provider.connected)
            .count(),
        providers,
        notice,
        error: error.or(load_error),
        reload_status,
        navbar,
    };
    Ok(Html(template.render()?).into_response())
}

pub(in crate::web::routes) async fn provider_connect_form(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path(provider_id): Path<String>,
    Query(query): Query<ProviderMethodQuery>,
) -> Result<Response, AppError> {
    let method_index = query
        .method
        .ok_or_else(|| anyhow::anyhow!("provider method is required"))?;
    let (provider_name, method) = load_selected_method(&state, &provider_id, method_index).await?;
    let navbar = load_navbar(&state.db_pool, user.id).await?;
    Ok(render_connect_form(
        &provider_id,
        &provider_name,
        method,
        method_index,
        None,
        navbar,
        query.modal,
    ))
}

pub(in crate::web::routes) async fn provider_connect(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path(provider_id): Path<String>,
    Query(query): Query<ProviderMethodQuery>,
    Form(pairs): Form<Vec<(String, String)>>,
) -> Result<Response, AppError> {
    let modal = query.modal;
    let method_index = single_usize(&pairs, "method").map_err(validation_response)?;
    let (provider_name, method) = load_selected_method(&state, &provider_id, method_index).await?;
    let answers = validate_prompt_answers(&method, &pairs).map_err(validation_response)?;
    let navbar = load_navbar(&state.db_pool, user.id).await?;
    if provider_id == "openai" && method.label == OPENAI_BROWSER_METHOD {
        return Ok(render_connect_form(
            &provider_id,
            &provider_name,
            method,
            method_index,
            Some(OPENAI_BROWSER_WARNING.to_string()),
            navbar,
            modal,
        ));
    }
    match method.auth_type.as_str() {
        "api" => {
            let api_key = match single_value(&pairs, "api_key") {
                Ok(api_key) => api_key,
                Err(error) => {
                    return Ok(render_connect_form(
                        &provider_id,
                        &provider_name,
                        method,
                        method_index,
                        Some(error),
                        navbar,
                        modal,
                    ));
                }
            };
            if api_key.is_empty() {
                return Ok(render_connect_form(
                    &provider_id,
                    &provider_name,
                    method,
                    method_index,
                    Some("Enter an API key.".to_string()),
                    navbar,
                    modal,
                ));
            }
            state
                .opencode_client
                .set_provider_api_auth(&state.opencode_base_url, &provider_id, api_key, answers)
                .await
                .map_err(|_| AppError(anyhow::anyhow!("API credential storage failed")))?;
            state.provider_connections.remove(&provider_id).await;
            state.opencode_client.invalidate_provider_cache().await;
            queue_provider_config_reload(
                &state,
                "Provider credentials were saved, but the OpenCode config reload could not be queued. Return to Providers and select Reload config.",
            )
            .await?;
            Ok(provider_result_redirect(
                "/providers?result=connected",
                modal,
            ))
        }
        "oauth" => {
            if !state
                .provider_connections
                .reserve(&provider_id, user.id)
                .await
            {
                return Ok(render_connect_form(
                    &provider_id,
                    &provider_name,
                    method,
                    method_index,
                    Some(
                        "A connection is already in progress for this provider. Finish it or wait for it to expire."
                            .to_string(),
                    ),
                    navbar,
                    modal,
                ));
            }
            let directory = state.opencode_container_workspaces_root.clone();
            let authorization = match state
                .opencode_client
                .authorize_provider_oauth(
                    &state.opencode_base_url,
                    &directory,
                    &provider_id,
                    method_index,
                    answers,
                )
                .await
            {
                Ok(authorization) => authorization,
                Err(_) => {
                    state
                        .provider_connections
                        .release_reservation(&provider_id, user.id)
                        .await;
                    return Err(AppError(anyhow::anyhow!("OAuth authorization failed")));
                }
            };
            let attempt = OAuthAttempt::new(
                user.id,
                provider_id.clone(),
                method_index,
                directory,
                authorization.completion_mode,
                authorization.url,
                authorization.instructions,
            );
            if state
                .provider_connections
                .complete_reservation(attempt.clone())
                .await
                .is_err()
            {
                return Ok(render_connect_form(
                    &provider_id,
                    &provider_name,
                    method,
                    method_index,
                    Some("The OAuth connection expired. Start it again.".to_string()),
                    navbar,
                    modal,
                ));
            }
            if modal {
                Ok(render_pending(&attempt, &provider_name, None, navbar, true))
            } else {
                Ok(
                    Redirect::to(&format!("/providers/{provider_id}/connect/pending"))
                        .into_response(),
                )
            }
        }
        _ => Ok(render_connect_form(
            &provider_id,
            &provider_name,
            method,
            method_index,
            Some("OpenCode advertised an unsupported connection method.".to_string()),
            navbar,
            modal,
        )),
    }
}

pub(in crate::web::routes) async fn provider_pending(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path(provider_id): Path<String>,
    Query(query): Query<ProviderMethodQuery>,
) -> Result<Response, AppError> {
    let Some(attempt) = state.provider_connections.get(&provider_id).await else {
        return Ok(Redirect::to("/providers?result=failed").into_response());
    };
    if attempt.user_id != user.id {
        return Ok(Redirect::to("/providers?result=failed").into_response());
    }
    let provider_name = provider_name(&state, &provider_id).await;
    let navbar = load_navbar(&state.db_pool, user.id).await?;
    Ok(render_pending(
        &attempt,
        &provider_name,
        None,
        navbar,
        query.modal,
    ))
}

pub(in crate::web::routes) async fn provider_callback(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path(provider_id): Path<String>,
    Query(query): Query<ProviderMethodQuery>,
    Form(pairs): Form<Vec<(String, String)>>,
) -> Result<Response, AppError> {
    let modal = query.modal;
    let Some(attempt) = state.provider_connections.get(&provider_id).await else {
        return Ok(Redirect::to("/providers?result=failed").into_response());
    };
    let method = match single_usize(&pairs, "method") {
        Ok(method) => method,
        Err(_) => return Ok(Redirect::to("/providers?result=failed").into_response()),
    };
    let completion_mode = match single_value(&pairs, "completion_mode") {
        Ok(completion_mode) => completion_mode,
        Err(_) => return Ok(Redirect::to("/providers?result=failed").into_response()),
    };
    if attempt.user_id != user.id
        || attempt.provider_id != provider_id
        || attempt.method != method
        || attempt.directory != state.opencode_container_workspaces_root
        || completion_mode != completion_mode_name(attempt.completion_mode)
    {
        return Ok(Redirect::to("/providers?result=failed").into_response());
    }
    if pairs.iter().any(|(key, _)| {
        !matches!(
            key.as_str(),
            "csrf_token" | "method" | "completion_mode" | "code"
        )
    }) {
        return Ok(Redirect::to("/providers?result=failed").into_response());
    }
    let code = match attempt.completion_mode {
        OpenCodeOAuthCompletionMode::Auto => {
            if pairs.iter().any(|(key, _)| key == "code") {
                return Ok(Redirect::to("/providers?result=failed").into_response());
            }
            None
        }
        OpenCodeOAuthCompletionMode::Code => {
            let code = match single_value(&pairs, "code") {
                Ok(code) => code,
                Err(_) => {
                    let navbar = load_navbar(&state.db_pool, user.id).await?;
                    let provider_name = provider_name(&state, &provider_id).await;
                    return Ok(render_pending(
                        &attempt,
                        &provider_name,
                        Some("Enter the completion code.".to_string()),
                        navbar,
                        modal,
                    ));
                }
            };
            Some(code)
        }
    };
    if code.is_some_and(str::is_empty) {
        let navbar = load_navbar(&state.db_pool, user.id).await?;
        let provider_name = provider_name(&state, &provider_id).await;
        return Ok(render_pending(
            &attempt,
            &provider_name,
            Some("Enter the completion code.".to_string()),
            navbar,
            modal,
        ));
    }

    let callback = state
        .opencode_client
        .complete_provider_oauth(
            &state.opencode_base_url,
            &attempt.directory,
            &provider_id,
            attempt.method,
            code,
        )
        .await;
    if callback.is_err() {
        let navbar = load_navbar(&state.db_pool, user.id).await?;
        let provider_name = provider_name(&state, &provider_id).await;
        return Ok(render_pending(
            &attempt,
            &provider_name,
            Some("OAuth callback failed. Try the completion action again.".to_string()),
            navbar,
            modal,
        ));
    }
    state.opencode_client.invalidate_provider_cache().await;
    queue_provider_config_reload(
        &state,
        "Provider credentials were saved, but the OpenCode config reload could not be queued. Return to Providers and select Reload config.",
    )
    .await?;
    state
        .provider_connections
        .remove_if_matches(&provider_id, &attempt)
        .await;
    Ok(provider_result_redirect(
        "/providers?result=connected",
        modal,
    ))
}

pub(in crate::web::routes) async fn provider_cancel(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path(provider_id): Path<String>,
) -> StatusCode {
    state
        .provider_connections
        .cancel_for_user(&provider_id, user.id)
        .await;
    StatusCode::NO_CONTENT
}

pub(in crate::web::routes) async fn provider_reload_config(
    State(state): State<Arc<AppState>>,
    _user: AuthenticatedUser,
) -> Result<Response, AppError> {
    queue_provider_config_reload(
        &state,
        "The OpenCode config reload could not be queued. Try again.",
    )
    .await?;
    Ok(Redirect::to("/providers?result=reload_queued").into_response())
}

pub(in crate::web::routes) async fn provider_reload_status(
    State(state): State<Arc<AppState>>,
    _user: AuthenticatedUser,
) -> Result<Response, AppError> {
    let task = crate::agentic::store::get_latest_provider_config_reload_task(&state.db_pool)
        .await
        .unwrap_or(None);
    let reload_completed = task
        .as_ref()
        .is_some_and(|task| task.status == crate::agentic::model::MAINTENANCE_STATUS_SUCCEEDED);
    let reload_status = ProviderReloadStatusView::from_task(task);
    let view = ProviderReloadStatusTemplate { reload_status };
    let mut response = Html(
        view.render()
            .unwrap_or_else(|_| "Reload status unavailable.".to_string()),
    )
    .into_response();
    if reload_completed {
        // The provider table was rendered from OpenCode's pre-dispose cache.
        // Refresh it once disposal has completed and OpenCode rebuilds that cache.
        response
            .headers_mut()
            .insert("HX-Refresh", HeaderValue::from_static("true"));
    }
    Ok(response)
}

pub(in crate::web::routes) async fn provider_disconnect(
    State(state): State<Arc<AppState>>,
    _user: AuthenticatedUser,
    Path(provider_id): Path<String>,
) -> Result<Response, AppError> {
    state
        .opencode_client
        .remove_provider_auth(&state.opencode_base_url, &provider_id)
        .await
        .map_err(|_| AppError(anyhow::anyhow!("credential removal failed")))?;
    state.provider_connections.remove(&provider_id).await;
    state.opencode_client.invalidate_provider_cache().await;
    queue_provider_config_reload(
        &state,
        "Provider credentials were removed, but the OpenCode config reload could not be queued. Return to Providers and select Reload config.",
    )
    .await?;
    Ok(Redirect::to("/providers?result=removed").into_response())
}

async fn queue_provider_config_reload(
    state: &AppState,
    failure_message: &str,
) -> Result<(), AppError> {
    crate::agentic::store::insert_provider_config_reload_task(&state.db_pool)
        .await
        .map(|_| ())
        .map_err(|error| AppError(error.context(failure_message.to_string())))
}

fn build_provider_views(
    response: &crate::opencode::client::OpenCodeProvidersResponse,
    methods: &HashMap<String, Vec<OpenCodeProviderAuthMethod>>,
) -> Vec<ProviderConnectionView> {
    let mut providers: Vec<ProviderConnectionView> = response
        .all
        .iter()
        .map(|provider| ProviderConnectionView {
            id: provider.id.clone(),
            name: provider.name.clone().unwrap_or_else(|| provider.id.clone()),
            connected: response.connected.iter().any(|id| id == &provider.id),
            models: provider
                .models
                .iter()
                .map(|(id, info)| info.name.clone().unwrap_or_else(|| id.clone()))
                .collect(),
            methods: resolve_provider_methods(provider, methods.get(&provider.id))
                .iter()
                .enumerate()
                .map(|(index, method)| method_view(&provider.id, index, method))
                .collect(),
        })
        .collect();
    providers.sort_by(|left, right| {
        left.name
            .to_ascii_lowercase()
            .cmp(&right.name.to_ascii_lowercase())
            .then_with(|| left.id.cmp(&right.id))
    });
    providers
}

async fn load_selected_method(
    state: &Arc<AppState>,
    provider_id: &str,
    method_index: usize,
) -> Result<(String, OpenCodeProviderAuthMethod), AppError> {
    let directory = &state.opencode_container_workspaces_root;
    let providers = state
        .opencode_client
        .list_providers(&state.opencode_base_url, directory)
        .await?;
    let Some(provider) = providers
        .all
        .iter()
        .find(|provider| provider.id == provider_id)
    else {
        return Err(AppError(anyhow::anyhow!(
            "provider was not advertised by OpenCode"
        )));
    };
    let methods = state
        .opencode_client
        .list_provider_auth_methods(&state.opencode_base_url, directory)
        .await?;
    let resolved = resolve_provider_methods(provider, methods.get(provider_id));
    let Some(method) = resolved.get(method_index).cloned() else {
        return Err(AppError(anyhow::anyhow!(
            "provider connection method is no longer available"
        )));
    };
    Ok((
        provider.name.clone().unwrap_or_else(|| provider.id.clone()),
        method,
    ))
}

fn method_view(
    provider_id: &str,
    index: usize,
    method: &OpenCodeProviderAuthMethod,
) -> ProviderAuthMethodView {
    let disabled_reason = (provider_id == "openai" && method.label == OPENAI_BROWSER_METHOD)
        .then(|| OPENAI_BROWSER_WARNING.to_string());
    ProviderAuthMethodView {
        index,
        auth_type: method.auth_type.clone(),
        label: method.label.clone(),
        prompts: method.prompts.iter().map(prompt_view).collect(),
        disabled_reason,
    }
}

fn prompt_view(prompt: &OpenCodeProviderAuthPrompt) -> ProviderAuthPromptView {
    match prompt {
        OpenCodeProviderAuthPrompt::Text {
            key,
            message,
            placeholder,
            when,
        } => ProviderAuthPromptView {
            key: key.clone(),
            message: message.clone(),
            placeholder: placeholder.clone(),
            select: false,
            options: Vec::new(),
            when: when.as_ref().map(when_view),
        },
        OpenCodeProviderAuthPrompt::Select {
            key,
            message,
            options,
            when,
        } => ProviderAuthPromptView {
            key: key.clone(),
            message: message.clone(),
            placeholder: None,
            select: true,
            options: options
                .iter()
                .map(|option| ProviderAuthOptionView {
                    label: option.label.clone(),
                    value: option.value.clone(),
                    hint: option.hint.clone(),
                })
                .collect(),
            when: when.as_ref().map(when_view),
        },
    }
}

fn when_view(when: &crate::opencode::client::OpenCodeProviderAuthWhen) -> ProviderAuthWhenView {
    ProviderAuthWhenView {
        key: when.key.clone(),
        op: match when.op {
            OpenCodeProviderAuthWhenOp::Eq => "eq",
            OpenCodeProviderAuthWhenOp::Neq => "neq",
        }
        .to_string(),
        value: when.value.clone(),
    }
}

fn validate_prompt_answers(
    method: &OpenCodeProviderAuthMethod,
    pairs: &[(String, String)],
) -> Result<BTreeMap<String, String>, String> {
    let mut values = BTreeMap::new();
    for (key, value) in pairs {
        if key == "csrf_token" || key == "method" || (key == "api_key" && method.auth_type == "api")
        {
            continue;
        }
        if values.insert(key.clone(), value.clone()).is_some() {
            return Err("Each provider prompt may be submitted only once.".to_string());
        }
    }
    for prompt in &method.prompts {
        let (key, when, is_select, options) = match prompt {
            OpenCodeProviderAuthPrompt::Text { key, when, .. } => (key, when, false, &[][..]),
            OpenCodeProviderAuthPrompt::Select {
                key, when, options, ..
            } => (key, when, true, options.as_slice()),
        };
        let active = when.as_ref().is_none_or(|when| {
            values.get(&when.key).is_some_and(|answer| match when.op {
                OpenCodeProviderAuthWhenOp::Eq => answer == &when.value,
                OpenCodeProviderAuthWhenOp::Neq => answer != &when.value,
            })
        });
        let answer = values.get(key);
        if !active {
            if answer.is_some() {
                return Err("An inactive provider prompt was submitted.".to_string());
            }
            continue;
        }
        let Some(answer) = answer.filter(|answer| !answer.is_empty()) else {
            return Err("Complete all required provider prompts.".to_string());
        };
        if is_select && !options.iter().any(|option| option.value == *answer) {
            return Err("Choose an advertised provider option.".to_string());
        }
    }
    let prompt_keys: std::collections::HashSet<&str> = method
        .prompts
        .iter()
        .map(|prompt| match prompt {
            OpenCodeProviderAuthPrompt::Text { key, .. }
            | OpenCodeProviderAuthPrompt::Select { key, .. } => key.as_str(),
        })
        .collect();
    if values.keys().any(|key| !prompt_keys.contains(key.as_str())) {
        return Err("An unknown provider prompt was submitted.".to_string());
    }
    Ok(values)
}

fn single_value<'a>(pairs: &'a [(String, String)], key: &str) -> Result<&'a str, String> {
    let values: Vec<&str> = pairs
        .iter()
        .filter(|(name, _)| name == key)
        .map(|(_, value)| value.as_str())
        .collect();
    match values.as_slice() {
        [value] => Ok(value),
        [] => Err(format!("Missing {key}.")),
        _ => Err(format!("Duplicate {key}.")),
    }
}

fn single_usize(pairs: &[(String, String)], key: &str) -> Result<usize, String> {
    single_value(pairs, key)?
        .parse()
        .map_err(|_| "Invalid provider method.".to_string())
}

fn validation_response(message: String) -> AppError {
    AppError(anyhow::anyhow!(message))
}

fn render_connect_form(
    provider_id: &str,
    provider_name: &str,
    method: OpenCodeProviderAuthMethod,
    method_index: usize,
    error: Option<String>,
    navbar: Navbar,
    modal: bool,
) -> Response {
    let method = method_view(provider_id, method_index, &method);
    if modal {
        let view = ProviderConnectionModalStepTemplate {
            provider_id: provider_id.to_string(),
            method,
            error,
        };
        return Html(
            view.render()
                .unwrap_or_else(|_| "Provider connection form unavailable.".to_string()),
        )
        .into_response();
    }
    let view = ProviderConnectionFormTemplate {
        current_path: "/providers".to_string(),
        provider_id: provider_id.to_string(),
        provider_name: provider_name.to_string(),
        method,
        error,
        navbar,
    };
    let response = Html(
        view.render()
            .unwrap_or_else(|_| "Provider connection form unavailable.".to_string()),
    )
    .into_response();
    if view.error.is_some() {
        (StatusCode::UNPROCESSABLE_ENTITY, response).into_response()
    } else {
        response
    }
}

fn render_pending(
    attempt: &OAuthAttempt,
    provider_name: &str,
    error: Option<String>,
    navbar: Navbar,
    modal: bool,
) -> Response {
    if modal {
        let auto_detect =
            error.is_none() && matches!(attempt.completion_mode, OpenCodeOAuthCompletionMode::Auto);
        let view = ProviderOAuthPendingModalStepTemplate {
            provider_id: attempt.provider_id.clone(),
            method: attempt.method,
            completion_mode: completion_mode_name(attempt.completion_mode).to_string(),
            authorization_url: attempt.authorization_url.clone(),
            instructions: attempt.instructions.clone(),
            device_code: device_code(&attempt.instructions).map(ToOwned::to_owned),
            auto_detect,
            error,
        };
        return Html(
            view.render()
                .unwrap_or_else(|_| "OAuth connection state unavailable.".to_string()),
        )
        .into_response();
    }
    let view = ProviderOAuthPendingTemplate {
        current_path: "/providers".to_string(),
        provider_id: attempt.provider_id.clone(),
        provider_name: provider_name.to_string(),
        method: attempt.method,
        completion_mode: completion_mode_name(attempt.completion_mode).to_string(),
        authorization_url: attempt.authorization_url.clone(),
        instructions: attempt.instructions.clone(),
        error,
        navbar,
    };
    Html(
        view.render()
            .unwrap_or_else(|_| "OAuth connection state unavailable.".to_string()),
    )
    .into_response()
}

fn provider_result_redirect(location: &'static str, modal: bool) -> Response {
    if !modal {
        return Redirect::to(location).into_response();
    }
    let mut response = StatusCode::OK.into_response();
    response
        .headers_mut()
        .insert("HX-Redirect", HeaderValue::from_static(location));
    response
}

async fn provider_name(state: &Arc<AppState>, provider_id: &str) -> String {
    state
        .opencode_client
        .list_providers(
            &state.opencode_base_url,
            &state.opencode_container_workspaces_root,
        )
        .await
        .ok()
        .and_then(|response| {
            response
                .all
                .into_iter()
                .find(|provider| provider.id == provider_id)
        })
        .and_then(|provider| provider.name)
        .unwrap_or_else(|| provider_id.to_string())
}

fn completion_mode_name(mode: OpenCodeOAuthCompletionMode) -> &'static str {
    match mode {
        OpenCodeOAuthCompletionMode::Auto => "auto",
        OpenCodeOAuthCompletionMode::Code => "code",
    }
}

fn device_code(instructions: &str) -> Option<&str> {
    let code = instructions.strip_prefix("Enter code: ")?;
    (!code.is_empty() && !code.contains('\n')).then_some(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn method() -> OpenCodeProviderAuthMethod {
        OpenCodeProviderAuthMethod {
            auth_type: "api".to_string(),
            label: "API key".to_string(),
            prompts: vec![
                OpenCodeProviderAuthPrompt::Select {
                    key: "mode".to_string(),
                    message: "Mode".to_string(),
                    options: vec![crate::opencode::client::OpenCodeProviderAuthOption {
                        label: "Team".to_string(),
                        value: "team".to_string(),
                        hint: None,
                    }],
                    when: None,
                },
                OpenCodeProviderAuthPrompt::Text {
                    key: "account".to_string(),
                    message: "Account".to_string(),
                    placeholder: None,
                    when: Some(crate::opencode::client::OpenCodeProviderAuthWhen {
                        key: "mode".to_string(),
                        op: OpenCodeProviderAuthWhenOp::Eq,
                        value: "team".to_string(),
                    }),
                },
            ],
        }
    }

    #[test]
    fn dynamic_prompt_validation_rejects_unknown_and_inactive_fields() {
        let method = method();
        assert!(
            validate_prompt_answers(
                &method,
                &[
                    ("unknown".to_string(), "value".to_string()),
                    ("api_key".to_string(), "secret".to_string()),
                ]
            )
            .is_err()
        );
        assert!(
            validate_prompt_answers(
                &method,
                &[
                    ("mode".to_string(), "team".to_string()),
                    ("account".to_string(), "acct".to_string()),
                    ("api_key".to_string(), "secret".to_string()),
                ]
            )
            .is_ok()
        );
        assert!(
            validate_prompt_answers(
                &method,
                &[
                    ("mode".to_string(), "team".to_string()),
                    ("account".to_string(), "acct".to_string()),
                    ("account".to_string(), "duplicate".to_string()),
                ]
            )
            .is_err()
        );
    }

    fn provider_info(id: &str, env: &[&str]) -> crate::opencode::client::OpenCodeProviderInfo {
        crate::opencode::client::OpenCodeProviderInfo {
            id: id.to_string(),
            name: Some(id.to_string()),
            models: BTreeMap::new(),
            env: env.iter().map(|env| env.to_string()).collect(),
        }
    }

    #[test]
    fn resolve_methods_synthesizes_api_key_for_env_key_provider_without_advertised() {
        let provider = provider_info("ollama-cloud", &["OLLAMA_API_KEY"]);
        let resolved = resolve_provider_methods(&provider, None);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].auth_type, "api");
        assert_eq!(resolved[0].label, SYNTHETIC_API_KEY_LABEL);
        assert!(resolved[0].prompts.is_empty());

        let empty: Vec<OpenCodeProviderAuthMethod> = Vec::new();
        let resolved = resolve_provider_methods(&provider, Some(&empty));
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].label, SYNTHETIC_API_KEY_LABEL);
    }

    #[test]
    fn resolve_methods_keeps_advertised_methods_when_present() {
        let provider = provider_info("openai", &["OPENAI_API_KEY"]);
        let advertised = vec![OpenCodeProviderAuthMethod {
            auth_type: "oauth".to_string(),
            label: "ChatGPT Pro/Plus (browser)".to_string(),
            prompts: Vec::new(),
        }];
        let resolved = resolve_provider_methods(&provider, Some(&advertised));
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].label, "ChatGPT Pro/Plus (browser)");
    }

    #[test]
    fn resolve_methods_synthesizes_nothing_for_provider_without_env_key() {
        let provider = provider_info("no-env", &[]);
        let resolved = resolve_provider_methods(&provider, None);
        assert!(resolved.is_empty());
    }

    #[test]
    fn build_provider_views_offers_synthetic_method_for_env_key_provider() {
        let response = crate::opencode::client::OpenCodeProvidersResponse {
            all: vec![provider_info("ollama-cloud", &["OLLAMA_API_KEY"])],
            connected: Vec::new(),
        };
        let mut methods = HashMap::new();
        methods.insert("ollama-cloud".to_string(), Vec::new());
        let views = build_provider_views(&response, &methods);
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].methods.len(), 1);
        assert_eq!(views[0].methods[0].label, SYNTHETIC_API_KEY_LABEL);
        assert_eq!(views[0].methods[0].auth_type, "api");
    }

    #[test]
    fn device_code_only_extracts_a_single_line_code_instruction() {
        assert_eq!(device_code("Enter code: ABCD-1234"), Some("ABCD-1234"));
        assert_eq!(device_code("Enter code: "), None);
        assert_eq!(device_code("Use code: ABCD-1234"), None);
        assert_eq!(device_code("Enter code: ABCD-1234\nThen continue"), None);
    }
}
