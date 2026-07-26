use std::sync::Arc;

use askama::Template;
use axum::{
    Form,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
};
use chrono::Utc;
use tracing::error;

use super::super::account::{
    created_subaccount_address, load_trading_account_choices, selected_trading_account,
};
use super::super::shared::unique_violation_message;
use crate::web::error::AppError;
use crate::{
    agents::{
        crypto::generate_api_key,
        model::{AGENT_LIFECYCLE_ACTIVE, AgentRegistryRow, CreateAgentForm, slugify_agent_key},
        store::{
            delete_agent as delete_agent_in_store, get_agent, insert_agent,
            list_agent_instrument_options, list_agents_for_user, replace_agent_instruments,
        },
    },
    hyperliquid::live_state::{AccountKey, AccountLiveState, LiveConnectionStatus},
    opencode::{
        client::{DeleteSessionResult, SessionStatusKind},
        workspace::OpenCodeWorkspaceRuntimeConfig,
        workspace_control_client::WorkspaceAgentInput,
    },
    web::{
        AppState,
        auth::{AuthenticatedUser, get_user_api_wallet},
        templates::{
            AccountBalanceView, AgentListEntry, AgentSelectorItemsTemplate,
            AgentTradingAccountChoicesTemplate, AgentsNewPageTemplate, AgentsPageTemplate,
            load_navbar,
        },
    },
};
pub(in crate::web::routes) async fn agents_index(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
) -> Result<Response, AppError> {
    let agents = list_agents_for_user(&state.db_pool, user.id).await?;

    let entries: Vec<AgentListEntry> = agents
        .into_iter()
        .map(|row| {
            let account_key = AccountKey::new(&row.trading_account_address, &row.environment);
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

    let navbar = load_navbar(&state.db_pool, user.id).await?;
    let template = AgentsPageTemplate {
        agents: entries,
        current_path: "/agents".to_string(),
        navbar,
    };

    Ok(Html(template.render()?).into_response())
}
pub(in crate::web::routes) async fn agent_selector_items(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
) -> Result<Html<String>, AppError> {
    let agents = list_agents_for_user(&state.db_pool, user.id).await?;
    let wallet = get_user_api_wallet(&state.db_pool, user.id).await?;
    let can_create_agent = wallet.as_ref().is_some_and(|w| w.is_ready());
    Ok(Html(
        AgentSelectorItemsTemplate {
            agents,
            can_create_agent,
        }
        .render()?,
    ))
}
pub(in crate::web::routes) async fn agents_new(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
) -> Result<Response, AppError> {
    let Some(wallet) = get_user_api_wallet(&state.db_pool, user.id).await? else {
        return Ok(Redirect::to("/account").into_response());
    };
    if !wallet.is_ready() {
        return Ok(Redirect::to("/account").into_response());
    }
    let navbar = load_navbar(&state.db_pool, user.id).await?;
    let template = AgentsNewPageTemplate {
        form: CreateAgentForm::default(),
        choices: load_trading_account_choices(&state, &user).await.into(),
        selected_account: String::new(),
        errors: Vec::new(),
        current_path: "/agents/new".to_string(),
        navbar,
    };
    Ok(Html(template.render()?).into_response())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::web::routes) struct AgentAccountChoicesQuery {
    selected: Option<String>,
    created_name: Option<String>,
}

pub(in crate::web::routes) async fn agent_account_choices(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Query(query): Query<AgentAccountChoicesQuery>,
) -> Result<Html<String>, AppError> {
    let choices = load_trading_account_choices(&state, &user).await;
    let selected_account = query.selected.or_else(|| {
        query
            .created_name
            .as_deref()
            .and_then(|created_name| created_subaccount_address(created_name, &choices))
    });
    Ok(Html(
        AgentTradingAccountChoicesTemplate {
            choices: choices.into(),
            selected_account: selected_account.unwrap_or_default(),
        }
        .render()?,
    ))
}
pub(in crate::web::routes) async fn delete_agent(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let Some(trading_account_address) = agent.trading_account_address.as_deref() else {
        return Ok((StatusCode::CONFLICT, "agent has no trading account").into_response());
    };
    let workspace =
        OpenCodeWorkspaceRuntimeConfig::from_value(&agent.runtime_config).ok_or_else(|| {
            AppError(anyhow::anyhow!(
                "agent is missing OpenCode workspace metadata"
            ))
        })?;
    let conversation_sessions =
        crate::agent_conversations::store::list_agent_conversation_opencode_session_ids(
            &state.db_pool,
            &agent_key,
        )
        .await?;
    for session_id in conversation_sessions {
        let status = state
            .opencode_client
            .get_session_status_in_directory(
                &state.opencode_base_url,
                &session_id,
                Some(&workspace.workspace_container_path),
            )
            .await?;
        if status.as_ref().is_some_and(SessionStatusKind::is_active) {
            return Ok((
                StatusCode::CONFLICT,
                "stop active conversations before deleting this agent",
            )
                .into_response());
        }
        match state
            .opencode_client
            .delete_session(
                &state.opencode_base_url,
                &workspace.workspace_container_path,
                &session_id,
            )
            .await?
        {
            DeleteSessionResult::Deleted | DeleteSessionResult::NotFound => {}
        }
    }
    let account_key = AccountKey::new(trading_account_address, &agent.environment);
    let deleted = delete_agent_in_store(&state.db_pool, &agent_key).await?;
    if !deleted {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    }

    state.live_accounts.remove(&account_key);

    state.workspace_controller.delete_workspace(&agent.agent_key, &format!("delete-agent:{}", agent.agent_key)).await.inspect_err(
        |error| {
            error!(agent_key = %agent.agent_key, error = ?error, "failed to delete OpenCode workspace after deleting agent");
        },
    )?;

    Ok(Redirect::to("/agents").into_response())
}
pub(in crate::web::routes) async fn create_agent(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Form(form): Form<CreateAgentForm>,
) -> Result<Response, AppError> {
    let Some(wallet) = get_user_api_wallet(&state.db_pool, user.id).await? else {
        return Ok(Redirect::to("/account").into_response());
    };
    if !wallet.is_ready() {
        return Ok(Redirect::to("/account").into_response());
    }
    let choices = load_trading_account_choices(&state, &user).await;

    if let Err(errors) = form.validate() {
        return Ok(render_new_form(
            form,
            choices.into(),
            errors,
            load_navbar(&state.db_pool, user.id).await?,
        ));
    }

    let trading_account_address =
        match selected_trading_account(&form.trading_account_selection, &choices) {
            Ok(address) => address,
            Err(message) => {
                return Ok(render_new_form(
                    form,
                    choices.into(),
                    vec![message.to_string()],
                    load_navbar(&state.db_pool, user.id).await?,
                ));
            }
        };

    let now = Utc::now();
    let agent_key = slugify_agent_key(&form.display_name);
    let api_key = generate_api_key();

    let row = AgentRegistryRow {
        agent_key: agent_key.clone(),
        user_id: user.id,
        created_at: now,
        updated_at: now,
        enabled: true,
        lifecycle: AGENT_LIFECYCLE_ACTIVE.to_string(),
        display_name: form.display_name.trim().to_string(),
        trading_account_address: Some(trading_account_address),
        environment: "live".to_string(),
        api_key: api_key.clone(),
        api_key_last_used_at: None,
        runtime_config: serde_json::json!({}),
    };

    let generated = state
        .workspace_controller
        .create_workspace(
            WorkspaceAgentInput {
                agent_key: agent_key.clone(),
                display_name: row.display_name.clone(),
                agent_api_key: api_key.clone(),
                api_base_url: state.vibetrading_agent_api_base_url.clone(),
            },
            false,
            &format!("create-agent:{agent_key}"),
        )
        .await?;
    let mut row = row;
    row.runtime_config = OpenCodeWorkspaceRuntimeConfig {
        workspace_container_path: generated.workspace_container_path,
        profile_source: generated.profile_source,
    }
    .into_value();

    if let Err(e) = insert_agent(&state.db_pool, &row).await {
        if let Err(error) = state
            .workspace_controller
            .delete_workspace(
                &row.agent_key,
                &format!("create-agent:{}:db-cleanup", row.agent_key),
            )
            .await
        {
            error!(agent_key = %row.agent_key, error = ?error, "failed to clean up newly created workspace after agent insert failure");
        }

        let errors = match unique_violation_message(&e) {
            Some(msg) => vec![msg],
            None => {
                return Err(AppError(e));
            }
        };
        return Ok(render_new_form(
            form,
            choices.into(),
            errors,
            load_navbar(&state.db_pool, user.id).await?,
        ));
    }

    if let Err(error) = activate_new_agent(&state, &agent_key).await {
        error!(agent_key = %agent_key, error = ?error, "failed to activate newly created agent");
        if let Err(cleanup_error) = delete_agent_in_store(&state.db_pool, &agent_key).await {
            error!(agent_key = %agent_key, error = ?cleanup_error, "failed to clean up newly created agent after activation failure");
        }
        if let Err(cleanup_error) = state
            .workspace_controller
            .delete_workspace(
                &agent_key,
                &format!("create-agent:{agent_key}:activation-cleanup"),
            )
            .await
        {
            error!(agent_key = %agent_key, error = ?cleanup_error, "failed to clean up newly created workspace after activation failure");
        }
        return Err(AppError(error));
    }

    Ok(Redirect::to(&format!("/agents/{agent_key}")).into_response())
}

async fn activate_new_agent(state: &Arc<AppState>, agent_key: &str) -> Result<(), anyhow::Error> {
    let instrument_options = list_agent_instrument_options(&state.db_pool, agent_key).await?;
    if instrument_options
        .iter()
        .any(|instrument| instrument.instrument_id == "BTC")
    {
        replace_agent_instruments(&state.db_pool, agent_key, &["BTC".to_string()]).await?;
    }
    crate::agents::strategy_prompts::insert_default_strategy_prompts_for_agent(
        &state.db_pool,
        agent_key,
    )
    .await?;
    crate::agentic::store::insert_default_opencode_schedules(&state.db_pool, agent_key).await?;
    Ok(())
}
pub(in crate::web::routes) fn render_new_form(
    form: CreateAgentForm,
    choices: crate::web::templates::TradingAccountChoicesView,
    errors: Vec<String>,
    navbar: crate::web::templates::Navbar,
) -> Response {
    let template = AgentsNewPageTemplate {
        selected_account: form.trading_account_selection.clone(),
        form,
        choices,
        errors,
        current_path: "/agents/new".to_string(),
        navbar,
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
