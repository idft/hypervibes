use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
};
use tracing::warn;

use super::shared::urlencode;

use crate::{
    agents::store::{get_agent, lock_agent_execution_tx, set_agent_enabled},
    harness::store::{list_active_agent_runs, mark_run_aborted},
    hyperliquid::orders::gateway::{
        ExchangeClient, cancel_all_exchange_orders, close_exchange_position,
    },
    web::{AppState, api::orders::build_exchange_for_agent, error::AppError},
};

pub(in crate::web::routes) async fn agents_set_enabled(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    if !set_agent_enabled(&state.db_pool, &agent_key, !agent.enabled).await? {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    }
    Ok(Redirect::to(&format!("/agents/{agent_key}/settings")).into_response())
}

/// Stop one agent without touching its position. The execution lock stays
/// held until all already-started gateway submissions have completed, so the
/// following exchange cancellation cannot miss a newly-resting order.
pub(in crate::web::routes) async fn agents_emergency_stop(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let Some(account_address) = agent.trading_account_address.as_deref() else {
        return Ok((StatusCode::CONFLICT, "agent has no trading account").into_response());
    };
    let exchange = build_exchange_for_agent(&state, &agent_key).await?;
    let active_runs = list_active_agent_runs(&state.db_pool, &agent_key).await?;

    let mut tx = state.db_pool.begin().await?;
    if lock_agent_execution_tx(&mut tx, &agent_key)
        .await?
        .is_none()
    {
        tx.rollback().await?;
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    }
    sqlx::query("UPDATE agents SET enabled = false, updated_at = now() WHERE agent_key = $1")
        .bind(&agent_key)
        .execute(&mut *tx)
        .await?;

    let mut aborted_runs = 0_usize;
    let mut failures = Vec::new();
    for run in active_runs {
        let workspace_container_path = (run.sub_agent_kind
            != crate::harness::model::SUB_AGENT_KIND_ANALYSIS_CODING)
            .then(|| {
                format!(
                    "{}/runs/{}/{}/workspace",
                    state
                        .opencode_container_workspaces_root
                        .trim_end_matches('/'),
                    agent_key,
                    run.id
                )
            });
        let abort_result = match run.backend_run_ref.as_deref() {
            Some(session_id) => {
                crate::harness::backend::abort_and_confirm_session_terminated(
                    &state.harness_backend,
                    &state.opencode_base_url,
                    session_id,
                    workspace_container_path.as_deref(),
                )
                .await
            }
            None => Ok(true),
        };
        match abort_result {
            Ok(true) => {
                mark_run_aborted(&state.db_pool, run.id, "aborted by emergency stop", None).await?;
                if workspace_container_path.is_some() {
                    crate::harness::scheduler::terminalize_run_workspace_artifact(
                        &state.db_pool,
                        &state.workspace_controller,
                        &agent_key,
                        run.id,
                    )
                    .await?;
                }
                aborted_runs += 1;
            }
            Ok(false) => {
                failures.push(format!("run {} could not be aborted", run.id));
                warn!(agent_key = %agent_key, run_id = run.id, "emergency stop could not abort active OpenCode session");
            }
            Err(error) => {
                failures.push(format!("run {} abort failed", run.id));
                warn!(agent_key = %agent_key, run_id = run.id, error = ?error, "emergency stop failed to abort active OpenCode session");
            }
        }
    }

    let cancelled_orders = match cancel_all_exchange_orders(
        &state.db_pool,
        &exchange,
        &agent_key,
        account_address,
        &agent.environment,
        None,
    )
    .await
    {
        Ok(summary) => summary.outcomes.len(),
        Err(error) => {
            failures.push("open-order cancellation failed".to_string());
            warn!(agent_key = %agent_key, error = %error, "emergency stop failed to cancel all exchange orders");
            0
        }
    };

    tx.commit().await?;
    let notice = if failures.is_empty() {
        format!(
            "Emergency stop complete: aborted {aborted_runs} run(s) and sent {cancelled_orders} cancellation(s)."
        )
    } else {
        format!("Emergency stop applied, but {}.", failures.join("; "))
    };
    Ok(Redirect::to(&format!(
        "/agents/{agent_key}?notice={}",
        urlencode(&notice)
    ))
    .into_response())
}

pub(in crate::web::routes) async fn agents_close_position(
    State(state): State<Arc<AppState>>,
    Path((agent_key, symbol)): Path<(String, String)>,
) -> Result<Response, AppError> {
    close_positions(&state, &agent_key, Some(symbol.as_str())).await?;
    Ok(Redirect::to(&format!("/agents/{agent_key}")).into_response())
}

pub(in crate::web::routes) async fn agents_close_all_positions(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    close_positions(&state, &agent_key, None).await?;
    Ok(Redirect::to(&format!("/agents/{agent_key}")).into_response())
}

async fn close_positions(
    state: &Arc<AppState>,
    agent_key: &str,
    symbol: Option<&str>,
) -> Result<(), AppError> {
    let agent = get_agent(&state.db_pool, agent_key)
        .await?
        .ok_or_else(|| anyhow::anyhow!("agent not found"))?;
    let account_address = agent
        .trading_account_address
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("agent has no trading account"))?;
    let exchange = build_exchange_for_agent(state, agent_key).await?;
    let mut tx = state.db_pool.begin().await?;
    lock_agent_execution_tx(&mut tx, agent_key)
        .await?
        .ok_or_else(|| anyhow::anyhow!("agent not found"))?;

    cancel_all_exchange_orders(
        &state.db_pool,
        &exchange,
        agent_key,
        account_address,
        &agent.environment,
        symbol,
    )
    .await
    .map_err(|error| anyhow::anyhow!(error.to_string()))?;

    let symbols: Vec<String> = match symbol {
        Some(symbol) => vec![symbol.to_string()],
        None => exchange
            .positions(account_address)
            .await
            .map_err(|error| anyhow::anyhow!("positions failed: {error}"))?
            .into_iter()
            .filter(|position| !position.szi.is_zero())
            .map(|position| position.symbol)
            .collect(),
    };
    for symbol in symbols {
        close_exchange_position(
            &state.db_pool,
            &exchange,
            &state.builder_fee_cache,
            agent_key,
            account_address,
            &agent.environment,
            &symbol,
        )
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    }
    tx.rollback().await?;
    Ok(())
}
