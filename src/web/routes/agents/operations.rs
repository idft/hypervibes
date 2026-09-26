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
    hyperliquid::orders::{
        gateway::{
            CancelAllSummary, ExchangeClient, cancel_all_exchange_orders, close_exchange_position,
        },
        model::OrderResult,
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

/// Stop one agent without touching its position. Disabling under the execution
/// lock waits for already-started gateway submissions and is committed before
/// any best-effort cleanup begins.
pub(in crate::web::routes) async fn agents_emergency_stop(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    if !set_agent_enabled(&state.db_pool, &agent_key, false).await? {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    }

    let mut aborted_runs = 0_usize;
    let mut failures = Vec::new();
    match list_active_agent_runs(&state.db_pool, &agent_key).await {
        Ok(active_runs) => {
            for run in active_runs {
                let run_workspace_container_path = Some(format!(
                    "{}/runs/{}/{}/workspace",
                    state
                        .opencode_container_workspaces_root
                        .trim_end_matches('/'),
                    agent_key,
                    run.id
                ));
                let abort_result = match run.backend_run_ref.as_deref() {
                    Some(session_id) => {
                        crate::harness::backend::abort_and_confirm_session_terminated(
                            &state.harness_backend,
                            &state.opencode_base_url,
                            session_id,
                            run_workspace_container_path.as_deref(),
                        )
                        .await
                    }
                    None => Ok(true),
                };
                match abort_result {
                    Ok(true) => {
                        if let Err(error) = mark_run_aborted(
                            &state.db_pool,
                            run.id,
                            "aborted by emergency stop",
                            None,
                        )
                        .await
                        {
                            failures.push(format!("run {} could not be marked aborted", run.id));
                            warn!(agent_key = %agent_key, run_id = run.id, error = ?error, "emergency stop could not persist aborted run state");
                            continue;
                        }
                        aborted_runs += 1;
                        if let Err(error) =
                            crate::harness::scheduler::terminalize_run_workspace_artifact(
                                &state.db_pool,
                                &state.harness_backend,
                                &state.workspace_controller,
                                &state.opencode_base_url,
                                run_workspace_container_path
                                    .as_deref()
                                    .expect("run workspace path was constructed"),
                                &agent_key,
                                run.id,
                            )
                            .await
                        {
                            failures.push(format!("run {} workspace cleanup failed", run.id));
                            warn!(agent_key = %agent_key, run_id = run.id, error = ?error, "emergency stop failed to clean up run workspace");
                        }
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
        }
        Err(error) => {
            failures.push("active runs could not be loaded".to_string());
            warn!(agent_key = %agent_key, error = ?error, "emergency stop failed to load active runs");
        }
    }

    let mut cancelled_orders = 0;
    match agent.trading_account_address.as_deref() {
        Some(account_address) => match build_exchange_for_agent(&state, &agent_key).await {
            Ok(exchange) => {
                match cancel_all_exchange_orders(
                    &state.db_pool,
                    &exchange,
                    &agent_key,
                    account_address,
                    &agent.environment,
                    None,
                )
                .await
                {
                    Ok(summary) => {
                        let (successful, cancellation_failures) = inspect_cancel_summary(&summary);
                        cancelled_orders = successful;
                        for failure in cancellation_failures {
                            warn!(agent_key = %agent_key, failure = %failure, "emergency stop exchange cancellation was not successful");
                            failures.push(failure);
                        }
                    }
                    Err(error) => {
                        failures.push("open-order cancellation failed".to_string());
                        warn!(agent_key = %agent_key, error = %error, "emergency stop failed to cancel all exchange orders");
                    }
                }

                match exchange.open_orders(account_address).await {
                    Ok(remaining) if !remaining.is_empty() => {
                        failures.push(format!("{} exchange order(s) remain open", remaining.len()));
                        warn!(agent_key = %agent_key, remaining_orders = remaining.len(), "emergency stop left exchange orders open");
                    }
                    Ok(_) => {}
                    Err(error) => {
                        failures.push("remaining open orders could not be verified".to_string());
                        warn!(agent_key = %agent_key, error = %error, "emergency stop could not verify open orders");
                    }
                }
            }
            Err(error) => {
                failures.push("trading signer unavailable; order cancellation skipped".to_string());
                warn!(agent_key = %agent_key, error = ?error, "emergency stop could not build exchange client");
            }
        },
        None => {
            failures.push("agent has no trading account; order cancellation skipped".to_string());
            warn!(agent_key = %agent_key, "emergency stop skipped order cancellation because the agent has no trading account");
        }
    }

    let notice = if failures.is_empty() {
        format!(
            "Emergency stop complete: aborted {aborted_runs} run(s) and confirmed {cancelled_orders} cancellation(s)."
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
    let notice = close_positions(&state, &agent_key, Some(symbol.as_str())).await?;
    Ok(Redirect::to(&format!(
        "/agents/{agent_key}?notice={}",
        urlencode(&notice)
    ))
    .into_response())
}

pub(in crate::web::routes) async fn agents_close_all_positions(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    let notice = close_positions(&state, &agent_key, None).await?;
    Ok(Redirect::to(&format!(
        "/agents/{agent_key}?notice={}",
        urlencode(&notice)
    ))
    .into_response())
}

async fn close_positions(
    state: &Arc<AppState>,
    agent_key: &str,
    symbol: Option<&str>,
) -> Result<String, AppError> {
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

    let mut failures = Vec::new();
    let mut cancelled_orders = 0;
    match cancel_all_exchange_orders(
        &state.db_pool,
        &exchange,
        agent_key,
        account_address,
        &agent.environment,
        symbol,
    )
    .await
    {
        Ok(summary) => {
            let (successful, cancellation_failures) = inspect_cancel_summary(&summary);
            cancelled_orders = successful;
            failures.extend(cancellation_failures);
        }
        Err(error) => failures.push(format!("open-order cancellation failed: {error}")),
    }

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
    let mut closed_positions = 0;
    for symbol in symbols {
        match close_exchange_position(
            &state.db_pool,
            &exchange,
            &state.builder_fee_cache,
            agent_key,
            account_address,
            &agent.environment,
            &symbol,
        )
        .await
        {
            Ok(Some(outcomes)) => {
                let outcome_failures = inspect_close_outcomes(&symbol, &outcomes);
                if outcome_failures.is_empty() {
                    closed_positions += 1;
                } else {
                    failures.extend(outcome_failures);
                }
            }
            Ok(None) => {}
            Err(error) => failures.push(format!("{symbol} close failed: {error}")),
        }
    }

    match exchange.open_orders(account_address).await {
        Ok(orders) => {
            let remaining: Vec<_> = orders
                .into_iter()
                .filter(|order| symbol.is_none_or(|requested| requested == order.symbol))
                .collect();
            if !remaining.is_empty() {
                failures.push(format!("{} exchange order(s) remain open", remaining.len()));
            }
        }
        Err(error) => failures.push(format!(
            "remaining open orders could not be verified: {error}"
        )),
    }

    match exchange.positions(account_address).await {
        Ok(positions) => {
            let remaining: Vec<_> = positions
                .into_iter()
                .filter(|position| {
                    !position.szi.is_zero()
                        && symbol.is_none_or(|requested| requested == position.symbol)
                })
                .collect();
            if !remaining.is_empty() {
                let symbols = remaining
                    .iter()
                    .map(|position| position.symbol.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                failures.push(format!(
                    "{} position(s) remain open: {symbols}",
                    remaining.len()
                ));
            }
        }
        Err(error) => failures.push(format!(
            "remaining positions could not be verified: {error}"
        )),
    }

    tx.rollback().await?;
    if failures.is_empty() {
        Ok(format!(
            "Position close complete: closed {closed_positions} position(s) and canceled {cancelled_orders} order(s)."
        ))
    } else {
        Ok(format!(
            "Position close incomplete: {}.",
            failures.join("; ")
        ))
    }
}

pub(super) fn inspect_cancel_summary(summary: &CancelAllSummary) -> (usize, Vec<String>) {
    let mut successful = 0;
    let mut failures = Vec::new();
    for outcome in &summary.outcomes {
        if outcome.status == "canceled" && outcome.error.is_none() {
            successful += 1;
        } else {
            let detail = outcome
                .error
                .as_deref()
                .map(|error| format!(": {error}"))
                .unwrap_or_default();
            failures.push(format!(
                "{} order {} cancellation returned {}{detail}",
                outcome.symbol, outcome.oid, outcome.status
            ));
        }
    }
    if summary.outcomes.len() != summary.considered {
        failures.push(format!(
            "exchange returned {} cancellation outcome(s) for {} open order(s)",
            summary.outcomes.len(),
            summary.considered
        ));
    }
    (successful, failures)
}

pub(super) fn inspect_close_outcomes(symbol: &str, outcomes: &[OrderResult]) -> Vec<String> {
    if outcomes.is_empty() {
        return vec![format!("{symbol} close returned no exchange outcome")];
    }

    outcomes
        .iter()
        .filter(|outcome| outcome.status != "filled" || outcome.error.is_some())
        .map(|outcome| {
            let detail = outcome
                .error
                .as_deref()
                .map(|error| format!(": {error}"))
                .unwrap_or_default();
            format!("{symbol} close returned {}{detail}", outcome.status)
        })
        .collect()
}
