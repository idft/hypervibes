use std::sync::Arc;

use axum::{
    Json,
    extract::State,
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::{
    agents::{AuthenticatedAgent, store::get_agent},
    harness::model::RunApiScope,
    hyperliquid::live_state::{
        AccountKey, AccountLiveState, LiveAccountHealthStatus, LiveConnectionStatus,
        LiveDataStatus, LiveOpenOrder, LivePosition, account_live_health,
    },
    web::AppState,
};

use super::error::ApiError;

/// `GET /api/v1/account`
///
/// Snapshot of the calling agent's Hyperliquid live account state. The
/// operator UI uses the SSE stream at `/agents/{agent_key}/live/stream`
/// instead; this is the agent-facing JSON poll.
pub(super) async fn get_account(
    State(state): State<Arc<AppState>>,
    agent: AuthenticatedAgent,
) -> Result<Response, ApiError> {
    super::require_run_api_scope(&agent, RunApiScope::AccountRead)?;
    let row = get_agent(&state.db_pool, &agent.agent_key)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound("agent not found"))?;
    let body = live_agent_snapshot(&state, &row);
    Ok(Json(body).into_response())
}

/// Snapshot of the calling agent's live account state. Returned to the
/// agent as JSON; the operator UI's SSE stream emits the rendered view
/// types from `src/web/templates.rs` directly.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LiveAgentSnapshot {
    agent_key: String,
    account_address: String,
    environment: String,
    account_data: AccountDataStatus,
    balance: Option<AccountBalance>,
    open_positions: Vec<LivePosition>,
    open_orders: Vec<LiveOpenOrder>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub(super) struct AccountDataStatus {
    available: bool,
    as_of: Option<DateTime<Utc>>,
    stale: bool,
    status: LiveAccountHealthStatus,
    connection_status: LiveConnectionStatus,
    positions_status: LiveDataStatus,
    orders_status: LiveDataStatus,
    balance_status: LiveDataStatus,
    last_successful_update: Option<DateTime<Utc>>,
    error: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub(super) struct AccountBalance {
    exchange: &'static str,
    model: &'static str,
    total_equity_usd: Decimal,
    available_to_trade_usd: Decimal,
    available_to_withdraw_usd: Decimal,
    margin_used_usd: Decimal,
    unrealized_pnl_usd: Decimal,
    collateral_balances: Vec<CollateralBalance>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub(super) struct CollateralBalance {
    asset: String,
    total: Decimal,
    available: Decimal,
}

pub(super) const COLLATERAL_ASSETS: &[&str] = &["USDC", "USDE", "USDT0", "USDH"];

pub(super) fn live_agent_snapshot(
    state: &AppState,
    row: &crate::agents::model::AgentDetailRow,
) -> LiveAgentSnapshot {
    let Some(trading_account_address) = row.trading_account_address.as_deref() else {
        return unavailable_live_agent_snapshot(row);
    };
    let key = AccountKey::new(trading_account_address, &row.environment);
    let snapshot = state.live_accounts.get(&key);
    let Some(snapshot) = snapshot else {
        return unavailable_live_agent_snapshot(row);
    };
    let health = account_live_health(&snapshot);

    LiveAgentSnapshot {
        agent_key: row.agent_key.clone(),
        account_address: trading_account_address.to_string(),
        environment: row.environment.clone(),
        account_data: AccountDataStatus {
            available: health.status == LiveAccountHealthStatus::Healthy,
            as_of: snapshot.account_data_as_of(),
            stale: health.status != LiveAccountHealthStatus::Healthy,
            status: health.status,
            connection_status: health.connection_status,
            positions_status: health.positions,
            orders_status: health.open_orders,
            balance_status: health.balance,
            last_successful_update: health.last_successful_update,
            error: health.last_error,
        },
        balance: health
            .balance
            .is_current()
            .then(|| account_balance_from_live_state(&snapshot)),
        open_positions: if health.positions.is_current() {
            snapshot.open_positions.clone()
        } else {
            Vec::new()
        },
        open_orders: if health.open_orders.is_current() {
            snapshot.open_orders.clone()
        } else {
            Vec::new()
        },
    }
}

pub(super) fn unavailable_live_agent_snapshot(
    row: &crate::agents::model::AgentDetailRow,
) -> LiveAgentSnapshot {
    LiveAgentSnapshot {
        agent_key: row.agent_key.clone(),
        account_address: row.trading_account_address.clone().unwrap_or_default(),
        environment: row.environment.clone(),
        account_data: AccountDataStatus {
            available: false,
            as_of: None,
            stale: true,
            status: LiveAccountHealthStatus::Loading,
            connection_status: LiveConnectionStatus::Starting,
            positions_status: LiveDataStatus::Loading,
            orders_status: LiveDataStatus::Loading,
            balance_status: LiveDataStatus::Loading,
            last_successful_update: None,
            error: None,
        },
        balance: None,
        open_positions: Vec::new(),
        open_orders: Vec::new(),
    }
}

pub(super) fn account_balance_from_live_state(state: &AccountLiveState) -> AccountBalance {
    let mut collateral_balances = Vec::new();

    for asset in COLLATERAL_ASSETS {
        let total = state
            .spot_balances
            .iter()
            .filter(|balance| balance.coin == *asset)
            .filter_map(|balance| balance.total)
            .sum();
        let available = state
            .spot_balances
            .iter()
            .filter(|balance| balance.coin == *asset)
            .filter_map(|balance| balance.available)
            .sum();
        if total > Decimal::ZERO || available > Decimal::ZERO {
            collateral_balances.push(CollateralBalance {
                asset: (*asset).to_string(),
                total,
                available,
            });
        }
    }

    let collateral_total: Decimal = collateral_balances
        .iter()
        .map(|balance| balance.total)
        .sum();
    let collateral_available: Decimal = collateral_balances
        .iter()
        .map(|balance| balance.available)
        .sum();
    let margin_withdrawable = state
        .margin
        .as_ref()
        .and_then(|margin| margin.withdrawable)
        .unwrap_or(Decimal::ZERO);
    let margin_account_value = state
        .margin
        .as_ref()
        .and_then(|margin| margin.account_value)
        .unwrap_or(Decimal::ZERO);
    let margin_used_usd = state
        .margin
        .as_ref()
        .and_then(|margin| margin.total_margin_used)
        .unwrap_or(Decimal::ZERO);
    let unrealized_pnl_usd: Decimal = state
        .open_positions
        .iter()
        .filter_map(|position| position.unrealized_pnl)
        .sum();
    let available_to_trade_usd = collateral_available.max(margin_withdrawable);

    AccountBalance {
        exchange: "hyperliquid",
        model: "unified_cross_margin",
        total_equity_usd: collateral_total.max(margin_account_value),
        available_to_trade_usd,
        available_to_withdraw_usd: available_to_trade_usd,
        margin_used_usd,
        unrealized_pnl_usd,
        collateral_balances,
    }
}
