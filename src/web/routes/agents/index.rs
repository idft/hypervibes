use std::sync::Arc;

use askama::Template;
use axum::{
    Form,
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
};
use chrono::Utc;
use tracing::error;

use super::super::shared::unique_violation_message;
use crate::web::error::AppError;
use crate::{
    agents::{
        crypto::{encrypt, generate_api_key},
        keys::derive_wallet_address,
        model::{
            AgentRegistryRow, AgentRuntimeRow, BACKEND_KIND_OPENCODE, CreateAgentForm,
            slugify_agent_key,
        },
        prompts::{DEFAULT_ANALYSIS_STRATEGY_PROMPT, DEFAULT_TRADING_STRATEGY_PROMPT},
        store::{
            delete_agent as delete_agent_in_store, get_agent, insert_agent, list_agents,
            list_enabled_agent_runtimes,
        },
    },
    hyperliquid::live_state::{AccountKey, AccountLiveState, LiveConnectionStatus},
    opencode::workspace::{
        OpenCodeWorkspaceAgent, WorkspaceGenerationMode, delete_agent_workspace,
        generate_agent_workspace, runtime_config_for_generated_workspace,
    },
    web::{
        AppState,
        templates::{
            AccountBalanceView, AgentListEntry, AgentsNewPageTemplate, AgentsPageTemplate,
        },
    },
};
pub(in crate::web::routes) async fn agents_index(
    State(state): State<Arc<AppState>>,
) -> Result<Html<String>, AppError> {
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
pub(in crate::web::routes) async fn agents_new(
    State(state): State<Arc<AppState>>,
) -> Result<Html<String>, AppError> {
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
pub(in crate::web::routes) async fn delete_agent(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };

    let account_key = AccountKey::new(&agent.wallet_address, &agent.environment);
    let deleted = delete_agent_in_store(&state.db_pool, &agent_key).await?;
    if !deleted {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    }

    state.live_accounts.remove(&account_key);

    if agent.backend_kind == BACKEND_KIND_OPENCODE {
        delete_agent_workspace(&state.opencode_workspace_config, &agent.agent_key)
            .inspect_err(|error| {
                error!(agent_key = %agent.agent_key, error = ?error, "failed to delete OpenCode workspace after deleting agent");
            })?;
    }

    Ok(Redirect::to("/agents").into_response())
}
pub(in crate::web::routes) async fn create_agent(
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

    let Some(runtime) = runtimes
        .iter()
        .find(|runtime| runtime.id == form.runtime_id.trim())
        .cloned()
    else {
        return Ok(render_new_form(
            form,
            runtimes,
            vec!["Selected runtime must exist and be enabled.".to_string()],
        ));
    };

    let now = Utc::now();
    let agent_key = slugify_agent_key(&form.display_name);
    let api_key = generate_api_key();

    let opencode_workspace_runtime_config = if runtime.backend_kind == BACKEND_KIND_OPENCODE {
        let generated = generate_agent_workspace(
            &state.opencode_workspace_config,
            &OpenCodeWorkspaceAgent {
                agent_key: agent_key.clone(),
                display_name: form.display_name.trim().to_string(),
                api_key: api_key.clone(),
            },
            WorkspaceGenerationMode::CreateNew,
        )
        .map_err(|error| {
            error!(agent_key = %agent_key, error = ?error, "failed to create OpenCode workspace for new agent");
            error
        });

        match generated {
            Ok(generated) => Some(runtime_config_for_generated_workspace(&generated).into_value()),
            Err(error) => {
                return Ok(render_new_form(
                    form,
                    runtimes,
                    vec![format!("Failed to create OpenCode workspace: {error}")],
                ));
            }
        }
    } else {
        None
    };

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
        api_key: api_key.clone(),
        api_key_last_used_at: None,
        backend_kind: runtime.backend_kind.clone(),
        runtime_id: runtime.id.clone(),
        runtime_config: opencode_workspace_runtime_config.unwrap_or_else(|| serde_json::json!({})),
        hyperliquid_private_key_ciphertext: ciphertext,
        hyperliquid_private_key_key_id: state.encryption_key.key_id.clone(),
    };

    if let Err(e) = insert_agent(&state.db_pool, &row).await {
        if row.backend_kind == BACKEND_KIND_OPENCODE {
            if let Err(error) =
                delete_agent_workspace(&state.opencode_workspace_config, &row.agent_key)
            {
                error!(agent_key = %row.agent_key, error = ?error, "failed to clean up newly created workspace after agent insert failure");
            }
        }

        let errors = match unique_violation_message(&e) {
            Some(msg) => vec![msg],
            None => {
                return Err(AppError(e));
            }
        };
        return Ok(render_new_form(form, runtimes, errors));
    }

    if row.backend_kind == BACKEND_KIND_OPENCODE {
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
pub(in crate::web::routes) fn render_new_form(
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
