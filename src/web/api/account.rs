use std::sync::Arc;

use axum::{
    Json,
    extract::State,
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Duration, Utc};
use rust_decimal::Decimal;

use crate::{
    agents::{AuthenticatedAgent, store::get_agent},
    hyperliquid::live_state::{AccountKey, AccountLiveState, LiveOpenOrder, LivePosition},
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

impl LiveAgentSnapshot {
    #[allow(dead_code)]
    pub fn to_markdown(&self) -> String {
        let available = self.account_data.available;
        let stale = self.account_data.stale;
        let as_of = self
            .account_data
            .as_of
            .map(|ts| ts.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true))
            .unwrap_or_default();

        let mut body = format!(
            "- Account: {}\n- Environment: {}\n- Available: {}\n- Stale: {}\n- As of: {}\n",
            self.account_address, self.environment, available, stale, as_of,
        );

        if let Some(b) = &self.balance {
            body.push_str(&format!(
                "- Total equity USD: {}\n- Available to trade USD: {}\n- Margin used USD: {}\n- Unrealized PnL USD: {}\n",
                b.total_equity_usd,
                b.available_to_trade_usd,
                b.margin_used_usd,
                b.unrealized_pnl_usd,
            ));
        }

        body.push_str("\n### Open positions\n");
        if self.open_positions.is_empty() {
            body.push_str("None\n");
        } else {
            for position in &self.open_positions {
                body.push_str(&format!(
                    "- {}: size {}, unrealized_pnl {}\n",
                    position.coin,
                    position
                        .szi
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "-".to_string()),
                    position
                        .unrealized_pnl
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "-".to_string()),
                ));
            }
        }

        body.push_str("\n### Open orders\n");
        if self.open_orders.is_empty() {
            body.push_str("None\n");
        } else {
            for order in &self.open_orders {
                body.push_str(&format!(
                    "- {} {} {} @ {}\n",
                    order.side.as_deref().unwrap_or("-"),
                    order
                        .sz
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "-".to_string()),
                    order.coin,
                    order
                        .limit_px
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "-".to_string()),
                ));
            }
        }

        body
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub(super) struct AccountDataStatus {
    available: bool,
    as_of: Option<DateTime<Utc>>,
    stale: bool,
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

pub(super) const ACCOUNT_DATA_MAX_AGE: Duration = Duration::minutes(2);
pub(super) const COLLATERAL_ASSETS: &[&str] = &["USDC", "USDE", "USDT0", "USDH"];

pub(super) fn live_agent_snapshot(
    state: &AppState,
    row: &crate::agents::model::AgentDetailRow,
) -> LiveAgentSnapshot {
    let key = AccountKey::new(&row.wallet_address, &row.environment);
    let snapshot = state.live_accounts.get(&key);
    let Some(snapshot) = snapshot else {
        return unavailable_live_agent_snapshot(row);
    };
    let Some(updated_at) = snapshot.updated_at else {
        return unavailable_live_agent_snapshot(row);
    };
    if Utc::now() - updated_at > ACCOUNT_DATA_MAX_AGE {
        return unavailable_live_agent_snapshot(row);
    }

    LiveAgentSnapshot {
        agent_key: row.agent_key.clone(),
        account_address: row.wallet_address.clone(),
        environment: row.environment.clone(),
        account_data: AccountDataStatus {
            available: true,
            as_of: Some(updated_at),
            stale: false,
        },
        balance: Some(account_balance_from_live_state(&snapshot)),
        open_positions: snapshot.open_positions.clone(),
        open_orders: snapshot.open_orders.clone(),
    }
}

pub(super) fn unavailable_live_agent_snapshot(
    row: &crate::agents::model::AgentDetailRow,
) -> LiveAgentSnapshot {
    LiveAgentSnapshot {
        agent_key: row.agent_key.clone(),
        account_address: row.wallet_address.clone(),
        environment: row.environment.clone(),
        account_data: AccountDataStatus {
            available: false,
            as_of: None,
            stale: true,
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
