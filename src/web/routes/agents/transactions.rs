use axum::{
    extract::{Path, Query, State},
    response::Response,
};
use rust_decimal::Decimal;
use std::sync::Arc;

use super::show::{AgentTransactionsQuery, render_agent_show_page};
use crate::{
    hyperliquid::{
        live_state::{AccountKey, AccountLiveState},
        queries::AccountTransactionRow,
    },
    web::{
        error::AppError,
        AppState,
        templates::AgentShowTab,
    },
};
pub(in crate::web::routes) async fn agents_show_transactions(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
    Query(query): Query<AgentTransactionsQuery>,
) -> Result<Response, AppError> {
    render_agent_show_page(
        &state,
        &agent_key,
        AgentShowTab::Transactions,
        Some(query),
        None,
        None,
        None,
    )
    .await
}
/// Re-anchor every displayed `running_balance` to the live wallet cash
/// balance, so the transactions table shows the same cash figure as the
/// Positions tab regardless of which historical page is being viewed.
///
/// `latest_running_balance` is the cumulative net USDC flow across the
/// account's **full** history — i.e. the running balance immediately after
/// the most recent journaled event — supplied by the caller so this helper can
/// re-anchor correctly even when only a paginated slice of rows is loaded
/// (where `rows.first()` is *not* the global newest row).
pub(in crate::web::routes) fn apply_live_cash_balance_anchor(
    state: &Arc<AppState>,
    agent: &crate::agents::model::AgentDetailRow,
    latest_running_balance: Option<Decimal>,
    rows: &mut [AccountTransactionRow],
) {
    let Some(latest_running_balance) = latest_running_balance else {
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
pub(in crate::web::routes) fn live_cash_balance(state: &AccountLiveState) -> Option<Decimal> {
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
