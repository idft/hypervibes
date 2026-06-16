use std::time::Duration;

use askama::Template;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::{
    agents::model::{AgentDetailRow, AgentListRow, CreateAgentForm},
    hyperliquid::{
        live_state::{AccountLiveState, LiveConnectionStatus, LiveOpenOrder, LivePosition},
        queries::AccountTransactionRow,
        sync_state::SyncStateRow,
    },
};

#[derive(Debug, Clone)]
pub struct SummaryCard {
    pub label: &'static str,
    pub value: String,
    pub detail: &'static str,
}

#[derive(Debug, Clone)]
pub struct MoneyCell {
    pub value: String,
    pub color_class: &'static str,
}

pub fn format_money_text(amount: Option<Decimal>) -> String {
    match amount {
        None => "-".to_string(),
        Some(value) => {
            let formatted = format!("{:.4}", value.abs());
            if value.is_sign_negative() {
                format!("({formatted})")
            } else {
                formatted
            }
        }
    }
}

pub fn format_money_cell(amount: Option<Decimal>) -> MoneyCell {
    match amount {
        None => dash_cell(),
        Some(value) if value.is_zero() => dash_cell(),
        Some(value) => {
            let formatted = format!("{:.4}", value.abs());
            if value.is_sign_negative() {
                MoneyCell {
                    value: format!("({formatted})"),
                    color_class: "text-red-400",
                }
            } else {
                MoneyCell {
                    value: formatted,
                    color_class: "text-emerald-400",
                }
            }
        }
    }
}

fn dash_cell() -> MoneyCell {
    MoneyCell {
        value: "-".to_string(),
        color_class: "text-zinc-500",
    }
}

#[derive(Debug, Clone)]
pub struct TransactionView {
    pub row: AccountTransactionRow,
    pub fee_usdc: String,
    pub realized_pnl_usdc: MoneyCell,
    pub usdc_delta: MoneyCell,
}

impl TransactionView {
    pub fn from_row(row: AccountTransactionRow) -> Self {
        let fee_usdc = format_money_text(row.fee_usdc);
        let realized_pnl_usdc = format_money_cell(row.realized_pnl_usdc);
        let usdc_delta = format_money_cell(row.usdc_delta);
        Self {
            row,
            fee_usdc,
            realized_pnl_usdc,
            usdc_delta,
        }
    }
}

/// Row entry shown on the agents index page. Combines the durable
/// [`AgentListRow`] with the in-memory live account-balance view so the
/// page can render the current Hyperliquid total balance in a single
/// table cell.
#[derive(Debug, Clone)]
pub struct AgentListEntry {
    pub row: AgentListRow,
    pub account_balance: AccountBalanceView,
}

#[derive(Template)]
#[template(path = "agents.html")]
pub struct AgentsPageTemplate {
    pub summary_cards: Vec<SummaryCard>,
    pub agents: Vec<AgentListEntry>,
}

#[derive(Template)]
#[template(path = "agents_new.html")]
pub struct AgentsNewPageTemplate {
    pub form: CreateAgentForm,
    pub errors: Vec<String>,
}

#[derive(Template)]
#[template(path = "agents_show.html")]
#[allow(dead_code)]
pub struct AgentsShowPageTemplate {
    pub agent: AgentDetailRow,
    pub transactions: Vec<TransactionView>,
    pub sync_state: Vec<SyncStateRow>,
    pub account_balance_html: String,
    pub open_positions_html: String,
    pub open_orders_html: String,
}

#[derive(Template)]
#[template(path = "server_error.html")]
pub struct ServerErrorPageTemplate {
    pub message: String,
}

/// View-model for the live account balance card shown on the agent detail
/// page and as a column on the agents index page. The same struct drives
/// both the initial render (when the page is first loaded) and the
/// live-updating SSE swaps (when the orchestrator pushes a new value).
///
/// The value shown mirrors the "Total balance" in the Hyperliquid UI:
/// the perps `crossMarginSummary.accountValue` plus the spot USDC that is
/// *available* to use. Adding only the available spot (not the spot
/// total) avoids double-counting the USDC that has been transferred to
/// the perps account as initial margin — that money is already counted
/// inside `accountValue`. When the live state has no margin snapshot
/// yet, the spot USDC available is used on its own.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AccountBalanceView {
    pub account_address: String,
    pub environment: String,
    pub total_balance: Option<Decimal>,
    pub status: LiveConnectionStatus,
    pub updated_at: Option<DateTime<Utc>>,
}

impl AccountBalanceView {
    pub fn from_live_state(state: crate::hyperliquid::live_state::AccountLiveState) -> Self {
        let perps_account_value = state
            .margin
            .as_ref()
            .and_then(|m| m.account_value)
            .filter(|v| !v.is_sign_negative());

        let spot_usdc = state
            .spot_balances
            .iter()
            .find(|b| b.coin.eq_ignore_ascii_case("USDC"));
        let spot_usdc_available = spot_usdc
            .and_then(|b| b.available)
            .filter(|v| !v.is_sign_negative());
        let spot_usdc_total = spot_usdc
            .and_then(|b| b.total)
            .filter(|v| !v.is_sign_negative());

        let total_balance = match (perps_account_value, spot_usdc_available) {
            (Some(perps), Some(spot_available)) => Some(perps + spot_available),
            (Some(perps), None) => Some(perps),
            (None, Some(spot_available)) => Some(spot_available),
            (None, None) => spot_usdc_total,
        };

        Self {
            account_address: state.account_address,
            environment: state.environment,
            total_balance,
            status: state.status,
            updated_at: state.updated_at,
        }
    }

    /// Format the total balance as a fixed-precision USDC string.
    ///
    /// Returns `None` when the value is not yet known; the template uses
    /// this to render a `Loading…` placeholder.
    pub fn formatted_total(&self) -> Option<String> {
        self.total_balance.map(format_usdc_balance)
    }

    /// Short human-readable status label, suitable for a small caption.
    pub fn status_label(&self) -> &'static str {
        match self.status {
            LiveConnectionStatus::Starting => "starting",
            LiveConnectionStatus::StartupSyncing => "startup sync",
            LiveConnectionStatus::Connecting => "connecting",
            LiveConnectionStatus::Connected => "live",
            LiveConnectionStatus::Reconnecting => "reconnecting",
            LiveConnectionStatus::Disconnected => "disconnected",
            LiveConnectionStatus::Failed => "failed",
            LiveConnectionStatus::Stopped => "stopped",
        }
    }
}

fn format_usdc_balance(value: Decimal) -> String {
    format!("{:.4}", value)
}

#[derive(Template)]
#[template(path = "account_balance.html")]
pub struct AccountBalancePartialTemplate {
    pub view: AccountBalanceView,
}

impl AccountBalancePartialTemplate {
    pub fn render_view(view: AccountBalanceView) -> Result<String, askama::Error> {
        Self { view }.render()
    }
}

/// Per-row view of an open perpetual position for the agent detail page.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct OpenPositionView {
    pub coin: String,
    pub side: &'static str,
    pub size: String,
    pub entry_px: MoneyCell,
    pub mark_px_or_value: String,
    pub unrealized_pnl: MoneyCell,
    pub liquidation_px: MoneyCell,
    pub margin_used: MoneyCell,
    pub return_on_equity: String,
    pub roe_color_class: &'static str,
}

/// Aggregates over all positions for the summary card above the table.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct OpenPositionsSummary {
    pub position_count: usize,
    pub total_u_pnl: MoneyCell,
    pub total_notional: String,
    pub total_margin_used: String,
}

/// View-model bundle handed to the open-positions partial template.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct OpenPositionsView {
    pub positions: Vec<OpenPositionView>,
    pub summary: OpenPositionsSummary,
    pub has_any_state: bool,
}

impl OpenPositionsView {
    pub fn from_live_state(state: AccountLiveState) -> Self {
        let has_any_state = state.status != LiveConnectionStatus::Starting
            || state.updated_at.is_some()
            || !state.open_positions.is_empty()
            || !state.open_orders.is_empty()
            || state.margin.is_some()
            || !state.spot_balances.is_empty();

        let mut visible: Vec<&LivePosition> = state
            .open_positions
            .iter()
            .filter(|p| p.szi.is_some_and(|s| !s.is_zero()))
            .collect();
        visible.sort_by(|a, b| {
            let a_abs = a.szi.map(|s| s.abs()).unwrap_or_default();
            let b_abs = b.szi.map(|s| s.abs()).unwrap_or_default();
            b_abs
                .partial_cmp(&a_abs)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.coin.cmp(&b.coin))
        });

        let positions: Vec<OpenPositionView> = visible.iter().map(|p| position_view(p)).collect();

        let mut total_u_pnl = Decimal::ZERO;
        let mut total_notional = Decimal::ZERO;
        let mut total_margin = Decimal::ZERO;
        for pos in &visible {
            if let Some(v) = pos.unrealized_pnl {
                total_u_pnl += v;
            }
            if let Some(v) = pos.position_value {
                total_notional += v.abs();
            }
            if let Some(v) = pos.margin_used {
                total_margin += v;
            }
        }

        let summary = OpenPositionsSummary {
            position_count: positions.len(),
            total_u_pnl: money_cell_for_pnl(total_u_pnl),
            total_notional: format_money_text(Some(total_notional)),
            total_margin_used: format_money_text(Some(total_margin)),
        };

        Self {
            positions,
            summary,
            has_any_state,
        }
    }
}

fn position_view(pos: &LivePosition) -> OpenPositionView {
    let szi = pos.szi.unwrap_or_default();
    let abs_szi = szi.abs();
    let side: &'static str = if szi.is_sign_negative() {
        "short"
    } else {
        "long"
    };

    let roe = match pos.return_on_equity {
        Some(v) => format_signed_percent(v, 2),
        None => "-".to_string(),
    };
    let roe_color_class = match pos.return_on_equity {
        Some(v) if v.is_sign_negative() => "text-red-400",
        Some(_) => "text-emerald-400",
        None => "text-zinc-500",
    };

    OpenPositionView {
        coin: pos.coin.clone(),
        side,
        size: format_size(abs_szi),
        entry_px: format_money_cell(pos.entry_px),
        mark_px_or_value: format_money_text(pos.position_value),
        unrealized_pnl: money_cell_for_pnl(pos.unrealized_pnl.unwrap_or_default()),
        liquidation_px: format_money_cell(pos.liquidation_px),
        margin_used: format_money_cell(pos.margin_used),
        return_on_equity: roe,
        roe_color_class,
    }
}

fn money_cell_for_pnl(value: Decimal) -> MoneyCell {
    if value.is_zero() {
        dash_cell()
    } else {
        let abs = value.abs();
        let formatted = format!("{:.4}", abs);
        if value.is_sign_negative() {
            MoneyCell {
                value: format!("({formatted})"),
                color_class: "text-red-400",
            }
        } else {
            MoneyCell {
                value: formatted,
                color_class: "text-emerald-400",
            }
        }
    }
}

fn format_size(value: Decimal) -> String {
    format!("{:.4}", value)
}

fn format_signed_percent(value: Decimal, decimals: usize) -> String {
    let abs = value.abs();
    let formatted = match decimals {
        0 => format!("{:.0}", abs),
        1 => format!("{:.1}", abs),
        2 => format!("{:.2}", abs),
        3 => format!("{:.3}", abs),
        _ => format!("{:.4}", abs),
    };
    if value.is_sign_negative() {
        format!("-{formatted}%")
    } else {
        format!("+{formatted}%")
    }
}

#[derive(Template)]
#[template(path = "open_positions.html")]
pub struct OpenPositionsPartialTemplate {
    pub view: OpenPositionsView,
}

impl OpenPositionsPartialTemplate {
    pub fn render_view(view: OpenPositionsView) -> Result<String, askama::Error> {
        Self { view }.render()
    }
}

/// Per-row view of an open resting order for the agent detail page.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct OpenOrderView {
    pub coin: String,
    pub side: String,
    pub order_type: String,
    pub size: String,
    pub orig_size: String,
    pub price: MoneyCell,
    pub tif: String,
    pub reduce_only: bool,
    pub trigger_px: MoneyCell,
    pub age: String,
    pub is_trigger: bool,
    pub is_position_tpsl: bool,
}

/// View-model bundle handed to the open-orders partial template.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct OpenOrdersView {
    pub orders: Vec<OpenOrderView>,
    pub has_any_state: bool,
}

impl OpenOrdersView {
    pub fn from_live_state(state: AccountLiveState) -> Self {
        let has_any_state = state.status != LiveConnectionStatus::Starting
            || state.updated_at.is_some()
            || !state.open_positions.is_empty()
            || !state.open_orders.is_empty()
            || state.margin.is_some()
            || !state.spot_balances.is_empty();

        let mut indexed: Vec<(u64, &LiveOpenOrder)> = state
            .open_orders
            .iter()
            .map(|o| (o.timestamp.unwrap_or(0), o))
            .collect();
        // Sort newest first; missing timestamps sort to the end (treated as
        // the smallest possible value).
        indexed.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.coin.cmp(&b.1.coin)));

        let orders: Vec<OpenOrderView> = indexed
            .into_iter()
            .map(|(_, order)| order_view(order))
            .collect();

        Self {
            orders,
            has_any_state,
        }
    }
}

fn order_view(order: &LiveOpenOrder) -> OpenOrderView {
    let side = order.side.clone().unwrap_or_else(|| "-".to_string());
    let order_type = order.order_type.clone().unwrap_or_else(|| "-".to_string());
    let tif = order.tif.clone().unwrap_or_else(|| "-".to_string());
    let size = match order.sz {
        Some(v) => format!("{:.4}", v),
        None => "-".to_string(),
    };
    let orig_size = match order.orig_sz {
        Some(v) => format!("{:.4}", v),
        None => "-".to_string(),
    };
    let reduce_only = order.reduce_only.unwrap_or(false);
    let is_trigger = order.is_trigger.unwrap_or(false);
    let is_position_tpsl = order.is_position_tpsl.unwrap_or(false);
    let age = format_order_age(order.timestamp);
    OpenOrderView {
        coin: order.coin.clone(),
        side,
        order_type,
        size,
        orig_size,
        price: format_money_cell(order.limit_px),
        tif,
        reduce_only,
        trigger_px: format_money_cell(order.trigger_px),
        age,
        is_trigger,
        is_position_tpsl,
    }
}

fn format_order_age(timestamp_ms: Option<u64>) -> String {
    let Some(ts_ms) = timestamp_ms else {
        return "-".to_string();
    };
    let now_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
    let age_ms = now_ms.saturating_sub(ts_ms);
    let duration = Duration::from_millis(age_ms);
    let total_seconds = duration.as_secs();
    if total_seconds < 60 {
        return format!("{total_seconds}s");
    }
    let total_minutes = total_seconds / 60;
    if total_minutes < 60 {
        let seconds = total_seconds % 60;
        return format!("{total_minutes}m {seconds}s");
    }
    let total_hours = total_minutes / 60;
    if total_hours < 24 {
        let minutes = total_minutes % 60;
        return format!("{total_hours}h {minutes}m");
    }
    let days = total_hours / 24;
    let hours = total_hours % 24;
    format!("{days}d {hours}h")
}

#[derive(Template)]
#[template(path = "open_orders.html")]
pub struct OpenOrdersPartialTemplate {
    pub view: OpenOrdersView,
}

impl OpenOrdersPartialTemplate {
    pub fn render_view(view: OpenOrdersView) -> Result<String, askama::Error> {
        Self { view }.render()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn sample_agent_list_row() -> AgentListRow {
        AgentListRow {
            display_name: "Test Agent".to_string(),
            agent_key: "test-agent".to_string(),
            enabled: true,
            wallet_address: "0x1234567890abcdef".to_string(),
            environment: "live".to_string(),
            api_key: "vt_test_key".to_string(),
            api_key_last_used_at: None,
        }
    }

    fn sample_agent_detail_row() -> AgentDetailRow {
        let now = Utc::now();
        AgentDetailRow {
            display_name: "Test Agent".to_string(),
            agent_key: "test-agent".to_string(),
            enabled: true,
            prompt: "Beep boop.".to_string(),
            wallet_address: "0x1234567890abcdef".to_string(),
            environment: "live".to_string(),
            api_key: "vt_test_key".to_string(),
            api_key_last_used_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn sample_account_balance_view() -> AccountBalanceView {
        AccountBalanceView {
            account_address: "0x1234567890abcdef".to_string(),
            environment: "live".to_string(),
            total_balance: Some(rust_decimal::Decimal::new(232_6800, 4)),
            status: crate::hyperliquid::live_state::LiveConnectionStatus::Connected,
            updated_at: Some(Utc::now()),
        }
    }

    #[test]
    fn agents_page_renders_base_layout_and_status_box() {
        let entry = AgentListEntry {
            row: sample_agent_list_row(),
            account_balance: AccountBalanceView {
                account_address: "0x1234567890abcdef".to_string(),
                environment: "live".to_string(),
                total_balance: Some(rust_decimal::Decimal::new(232_6800, 4)),
                status: crate::hyperliquid::live_state::LiveConnectionStatus::Connected,
                updated_at: Some(Utc::now()),
            },
        };
        let template = AgentsPageTemplate {
            summary_cards: vec![],
            agents: vec![entry],
        };
        let rendered = template.render().unwrap();
        assert!(rendered.contains("<!DOCTYPE html>"));
        assert!(rendered.contains("Vibetrading Agents"));
        assert!(rendered.contains("Web server online"));
        assert!(rendered.contains("Registered agents"));
        assert!(rendered.contains("Account balance"));
        assert!(rendered.contains("232.6800"));
    }

    #[test]
    fn agents_page_renders_loading_placeholder_when_no_balance() {
        let entry = AgentListEntry {
            row: sample_agent_list_row(),
            account_balance: AccountBalanceView {
                account_address: "0x1234567890abcdef".to_string(),
                environment: "live".to_string(),
                total_balance: None,
                status: crate::hyperliquid::live_state::LiveConnectionStatus::Starting,
                updated_at: None,
            },
        };
        let template = AgentsPageTemplate {
            summary_cards: vec![],
            agents: vec![entry],
        };
        let rendered = template.render().unwrap();
        assert!(rendered.contains("Loading"));
        assert!(!rendered.contains("232.6800"));
    }

    #[test]
    fn agents_show_page_renders_base_layout_and_delete_modal() {
        let view = sample_account_balance_view();
        let account_balance_html = AccountBalancePartialTemplate::render_view(view).unwrap();
        let positions_view = OpenPositionsView::from_live_state(AccountLiveState {
            account_address: "0x1234567890abcdef".to_string(),
            environment: "live".to_string(),
            ..Default::default()
        });
        let open_positions_html =
            OpenPositionsPartialTemplate::render_view(positions_view).unwrap();
        let orders_view = OpenOrdersView::from_live_state(AccountLiveState {
            account_address: "0x1234567890abcdef".to_string(),
            environment: "live".to_string(),
            ..Default::default()
        });
        let open_orders_html = OpenOrdersPartialTemplate::render_view(orders_view).unwrap();
        let template = AgentsShowPageTemplate {
            agent: sample_agent_detail_row(),
            transactions: vec![],
            sync_state: vec![],
            account_balance_html,
            open_positions_html,
            open_orders_html,
        };
        let rendered = template.render().unwrap();
        assert!(rendered.contains("<!DOCTYPE html>"));
        assert!(rendered.contains("Test Agent · Vibetrading"));
        assert!(rendered.contains("delete-modal"));
        assert!(rendered.contains("Delete agent"));
        assert!(rendered.contains("Total balance"));
    }

    #[test]
    fn account_balance_partial_renders_loading_state_when_value_missing() {
        let view = AccountBalanceView {
            account_address: "0xabc".to_string(),
            environment: "live".to_string(),
            total_balance: None,
            status: crate::hyperliquid::live_state::LiveConnectionStatus::Starting,
            updated_at: None,
        };
        let html = AccountBalancePartialTemplate::render_view(view).unwrap();
        assert!(html.contains("Total balance"));
        assert!(html.contains("Loading"));
        assert!(html.contains("starting"));
        assert!(!html.contains("USDC"));
    }

    #[test]
    fn account_balance_partial_renders_value_with_status() {
        let view = sample_account_balance_view();
        let html = AccountBalancePartialTemplate::render_view(view).unwrap();
        assert!(html.contains("Total balance"));
        assert!(html.contains("232.6800"));
        assert!(html.contains("USDC"));
        assert!(html.contains("live"));
        assert!(html.contains("Updated"));
    }

    #[test]
    fn balance_view_sums_perps_and_spot_available() {
        use crate::hyperliquid::live_state::{AccountLiveState, LiveMarginState, LiveSpotBalance};
        let state = AccountLiveState {
            account_address: "0xtest".to_string(),
            environment: "live".to_string(),
            status: LiveConnectionStatus::Connected,
            margin: Some(LiveMarginState {
                account_value: Some(rust_decimal::Decimal::new(50_0700, 4)),
                withdrawable: Some(rust_decimal::Decimal::new(420, 4)),
                ..Default::default()
            }),
            spot_balances: vec![LiveSpotBalance {
                coin: "USDC".to_string(),
                total: Some(rust_decimal::Decimal::new(232_6700, 4)),
                hold: Some(rust_decimal::Decimal::new(50_0600, 4)),
                available: Some(rust_decimal::Decimal::new(182_6100, 4)),
                ..Default::default()
            }],
            ..Default::default()
        };
        let view = AccountBalanceView::from_live_state(state);
        // perps (50.07) + spot available (182.61) = 232.68
        assert_eq!(
            view.total_balance,
            Some(rust_decimal::Decimal::new(232_6800, 4))
        );
    }

    #[test]
    fn balance_view_uses_perps_only_when_no_spot_state() {
        use crate::hyperliquid::live_state::{AccountLiveState, LiveMarginState};
        let state = AccountLiveState {
            account_address: "0xtest".to_string(),
            environment: "live".to_string(),
            status: LiveConnectionStatus::Connected,
            margin: Some(LiveMarginState {
                account_value: Some(rust_decimal::Decimal::new(1000, 0)),
                withdrawable: Some(rust_decimal::Decimal::new(900, 0)),
                ..Default::default()
            }),
            ..Default::default()
        };
        let view = AccountBalanceView::from_live_state(state);
        assert_eq!(
            view.total_balance,
            Some(rust_decimal::Decimal::new(1000, 0))
        );
    }

    #[test]
    fn balance_view_uses_spot_total_when_no_perps_margin_state() {
        use crate::hyperliquid::live_state::{AccountLiveState, LiveSpotBalance};
        let state = AccountLiveState {
            account_address: "0xtest".to_string(),
            environment: "live".to_string(),
            status: LiveConnectionStatus::Connected,
            spot_balances: vec![LiveSpotBalance {
                coin: "USDC".to_string(),
                total: Some(rust_decimal::Decimal::new(500, 0)),
                hold: Some(rust_decimal::Decimal::new(50, 0)),
                available: Some(rust_decimal::Decimal::new(450, 0)),
                ..Default::default()
            }],
            ..Default::default()
        };
        let view = AccountBalanceView::from_live_state(state);
        // No margin snapshot yet -> use spot USDC available.
        assert_eq!(view.total_balance, Some(rust_decimal::Decimal::new(450, 0)));
    }

    #[test]
    fn agents_new_page_renders_base_layout_and_form() {
        let template = AgentsNewPageTemplate {
            form: CreateAgentForm::default(),
            errors: vec![],
        };
        let rendered = template.render().unwrap();
        assert!(rendered.contains("<!DOCTYPE html>"));
        assert!(rendered.contains("Create agent · Vibetrading"));
        assert!(rendered.contains("display_name"));
    }

    #[test]
    fn server_error_page_renders_base_layout() {
        let template = ServerErrorPageTemplate {
            message: "Internal server error: boom".to_string(),
        };
        let rendered = template.render().unwrap();
        assert!(rendered.contains("<!DOCTYPE html>"));
        assert!(rendered.contains("500 Server Error"));
        assert!(rendered.contains("Internal server error: boom"));
    }

    #[test]
    fn open_positions_view_filters_zero_szi_and_sign_based_side() {
        use crate::hyperliquid::live_state::{AccountLiveState, LivePosition};
        let state = AccountLiveState {
            account_address: "0xtest".to_string(),
            environment: "live".to_string(),
            status: LiveConnectionStatus::Connected,
            open_positions: vec![
                LivePosition {
                    coin: "BTC".to_string(),
                    szi: Some(rust_decimal::Decimal::new(1, 0)),
                    entry_px: Some(rust_decimal::Decimal::new(30000, 0)),
                    liquidation_px: None,
                    margin_used: Some(rust_decimal::Decimal::new(600, 0)),
                    position_value: Some(rust_decimal::Decimal::new(30000, 0)),
                    unrealized_pnl: Some(rust_decimal::Decimal::new(100, 0)),
                    return_on_equity: Some(rust_decimal::Decimal::new(5, 1)),
                    leverage_type: Some("cross".to_string()),
                    leverage_value: Some(5),
                    max_leverage: Some(50),
                },
                LivePosition {
                    coin: "ETH".to_string(),
                    szi: Some(rust_decimal::Decimal::new(-3, 0)),
                    entry_px: Some(rust_decimal::Decimal::new(2000, 0)),
                    liquidation_px: Some(rust_decimal::Decimal::new(2500, 0)),
                    margin_used: Some(rust_decimal::Decimal::new(200, 0)),
                    position_value: Some(rust_decimal::Decimal::new(6000, 0)),
                    unrealized_pnl: Some(rust_decimal::Decimal::new(-150, 0)),
                    return_on_equity: Some(rust_decimal::Decimal::new(-7, 1)),
                    leverage_type: Some("isolated".to_string()),
                    leverage_value: Some(3),
                    max_leverage: Some(50),
                },
                LivePosition {
                    coin: "DUST".to_string(),
                    szi: Some(rust_decimal::Decimal::new(0, 0)),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let view = OpenPositionsView::from_live_state(state);
        assert_eq!(view.positions.len(), 2);
        // Sorted by |szi| desc: ETH (3) before BTC (1).
        assert_eq!(view.positions[0].coin, "ETH");
        assert_eq!(view.positions[0].side, "short");
        assert_eq!(view.positions[1].coin, "BTC");
        assert_eq!(view.positions[1].side, "long");
        // Aggregates.
        assert_eq!(view.summary.position_count, 2);
        // uPnL: 100 + (-150) = -50 → red
        assert_eq!(view.summary.total_u_pnl.value, "(50.0000)");
        assert_eq!(view.summary.total_u_pnl.color_class, "text-red-400");
        // Notional: 30000 + 6000 = 36000
        assert_eq!(view.summary.total_notional, "36000.0000");
        // Margin: 600 + 200 = 800
        assert_eq!(view.summary.total_margin_used, "800.0000");
    }

    #[test]
    fn open_positions_view_handles_no_positions() {
        use crate::hyperliquid::live_state::AccountLiveState;
        let state = AccountLiveState {
            account_address: "0xtest".to_string(),
            environment: "live".to_string(),
            status: LiveConnectionStatus::Connected,
            ..Default::default()
        };
        let view = OpenPositionsView::from_live_state(state);
        assert!(view.has_any_state);
        assert!(view.positions.is_empty());
        assert_eq!(view.summary.position_count, 0);
        // Zero totals still formatted as zero.
        assert_eq!(view.summary.total_u_pnl.value, "-");
        assert_eq!(view.summary.total_notional, "0.0000");
    }

    #[test]
    fn open_positions_view_has_no_state_when_only_starting() {
        use crate::hyperliquid::live_state::AccountLiveState;
        let state = AccountLiveState {
            account_address: "0xtest".to_string(),
            environment: "live".to_string(),
            status: LiveConnectionStatus::Starting,
            ..Default::default()
        };
        let view = OpenPositionsView::from_live_state(state);
        assert!(!view.has_any_state);
    }

    #[test]
    fn open_positions_partial_renders_loading_when_no_state() {
        use crate::hyperliquid::live_state::AccountLiveState;
        let state = AccountLiveState {
            account_address: "0xtest".to_string(),
            environment: "live".to_string(),
            status: LiveConnectionStatus::Starting,
            ..Default::default()
        };
        let view = OpenPositionsView::from_live_state(state);
        let html = OpenPositionsPartialTemplate::render_view(view).unwrap();
        assert!(html.contains("Loading"));
        assert!(!html.contains("No open positions"));
    }

    #[test]
    fn open_positions_partial_renders_empty_state() {
        use crate::hyperliquid::live_state::AccountLiveState;
        let state = AccountLiveState {
            account_address: "0xtest".to_string(),
            environment: "live".to_string(),
            status: LiveConnectionStatus::Connected,
            ..Default::default()
        };
        let view = OpenPositionsView::from_live_state(state);
        let html = OpenPositionsPartialTemplate::render_view(view).unwrap();
        assert!(html.contains("No open positions"));
    }

    #[test]
    fn open_positions_partial_renders_rows_and_pills() {
        use crate::hyperliquid::live_state::{AccountLiveState, LivePosition};
        let state = AccountLiveState {
            account_address: "0xtest".to_string(),
            environment: "live".to_string(),
            status: LiveConnectionStatus::Connected,
            updated_at: Some(Utc::now()),
            open_positions: vec![
                LivePosition {
                    coin: "BTC".to_string(),
                    szi: Some(rust_decimal::Decimal::new(1, 0)),
                    entry_px: Some(rust_decimal::Decimal::new(30000, 0)),
                    liquidation_px: Some(rust_decimal::Decimal::new(25000, 0)),
                    margin_used: Some(rust_decimal::Decimal::new(6000, 0)),
                    position_value: Some(rust_decimal::Decimal::new(30000, 0)),
                    unrealized_pnl: Some(rust_decimal::Decimal::new(1500, 0)),
                    return_on_equity: Some(rust_decimal::Decimal::new(2500, 2)),
                    leverage_type: Some("cross".to_string()),
                    leverage_value: Some(5),
                    max_leverage: Some(50),
                },
                LivePosition {
                    coin: "ETH".to_string(),
                    szi: Some(rust_decimal::Decimal::new(-2, 0)),
                    entry_px: Some(rust_decimal::Decimal::new(2000, 0)),
                    liquidation_px: None,
                    margin_used: Some(rust_decimal::Decimal::new(400, 0)),
                    position_value: Some(rust_decimal::Decimal::new(4000, 0)),
                    unrealized_pnl: Some(rust_decimal::Decimal::new(-100, 0)),
                    return_on_equity: Some(rust_decimal::Decimal::new(-2500, 2)),
                    leverage_type: Some("isolated".to_string()),
                    leverage_value: Some(10),
                    max_leverage: Some(50),
                },
            ],
            ..Default::default()
        };
        let view = OpenPositionsView::from_live_state(state);
        let html = OpenPositionsPartialTemplate::render_view(view).unwrap();
        assert!(html.contains("BTC"));
        assert!(html.contains("ETH"));
        assert!(html.contains("long"));
        assert!(html.contains("short"));
        assert!(html.contains("+25.00%"));
        assert!(html.contains("-25.00%"));
        assert!(html.contains("1.0000"));
        assert!(html.contains("2.0000"));
        assert!(html.contains("30000.0000"));
        assert!(html.contains("(100.0000)"));
        assert!(html.contains("25000.0000"));
        assert!(html.contains("6000.0000"));
    }

    #[test]
    fn open_orders_view_sorts_newest_first() {
        use crate::hyperliquid::live_state::{AccountLiveState, LiveOpenOrder};
        let state = AccountLiveState {
            account_address: "0xtest".to_string(),
            environment: "live".to_string(),
            status: LiveConnectionStatus::Connected,
            open_orders: vec![
                LiveOpenOrder {
                    coin: "ETH".to_string(),
                    side: Some("buy".to_string()),
                    limit_px: Some(rust_decimal::Decimal::new(1900, 0)),
                    sz: Some(rust_decimal::Decimal::new(1, 0)),
                    orig_sz: Some(rust_decimal::Decimal::new(1, 0)),
                    oid: Some("1".to_string()),
                    timestamp: Some(1_700_000_000_000),
                    cloid: None,
                    order_type: Some("limit".to_string()),
                    tif: Some("Gtc".to_string()),
                    reduce_only: Some(false),
                    is_trigger: Some(false),
                    trigger_px: None,
                    trigger_condition: None,
                    is_position_tpsl: Some(false),
                },
                LiveOpenOrder {
                    coin: "BTC".to_string(),
                    side: Some("sell".to_string()),
                    limit_px: Some(rust_decimal::Decimal::new(31000, 0)),
                    sz: Some(rust_decimal::Decimal::new(2, 0)),
                    orig_sz: Some(rust_decimal::Decimal::new(2, 0)),
                    oid: Some("2".to_string()),
                    timestamp: Some(1_700_000_500_000),
                    cloid: None,
                    order_type: Some("limit".to_string()),
                    tif: Some("Gtc".to_string()),
                    reduce_only: Some(false),
                    is_trigger: Some(false),
                    trigger_px: None,
                    trigger_condition: None,
                    is_position_tpsl: Some(false),
                },
            ],
            ..Default::default()
        };
        let view = OpenOrdersView::from_live_state(state);
        assert_eq!(view.orders.len(), 2);
        assert_eq!(view.orders[0].coin, "BTC");
        assert_eq!(view.orders[1].coin, "ETH");
    }

    #[test]
    fn open_orders_partial_renders_flags() {
        use crate::hyperliquid::live_state::{AccountLiveState, LiveOpenOrder};
        let state = AccountLiveState {
            account_address: "0xtest".to_string(),
            environment: "live".to_string(),
            status: LiveConnectionStatus::Connected,
            open_orders: vec![LiveOpenOrder {
                coin: "BTC".to_string(),
                side: Some("sell".to_string()),
                limit_px: Some(rust_decimal::Decimal::new(31000, 0)),
                sz: Some(rust_decimal::Decimal::new(2, 0)),
                orig_sz: Some(rust_decimal::Decimal::new(2, 0)),
                oid: Some("42".to_string()),
                timestamp: Some(chrono::Utc::now().timestamp_millis() as u64),
                cloid: None,
                order_type: Some("take_profit_market".to_string()),
                tif: Some("Gtc".to_string()),
                reduce_only: Some(true),
                is_trigger: Some(true),
                trigger_px: Some(rust_decimal::Decimal::new(32000, 0)),
                trigger_condition: Some("above".to_string()),
                is_position_tpsl: Some(true),
            }],
            ..Default::default()
        };
        let view = OpenOrdersView::from_live_state(state);
        let html = OpenOrdersPartialTemplate::render_view(view).unwrap();
        assert!(html.contains("BTC"));
        assert!(html.contains("sell"));
        assert!(html.contains("take_profit_market"));
        assert!(html.contains("trigger"));
        assert!(html.contains("TP/SL"));
        assert!(html.contains("reduce-only"));
    }

    #[test]
    fn open_orders_partial_renders_loading_when_no_state() {
        use crate::hyperliquid::live_state::AccountLiveState;
        let state = AccountLiveState {
            account_address: "0xtest".to_string(),
            environment: "live".to_string(),
            status: LiveConnectionStatus::Starting,
            ..Default::default()
        };
        let view = OpenOrdersView::from_live_state(state);
        let html = OpenOrdersPartialTemplate::render_view(view).unwrap();
        assert!(html.contains("Loading"));
        assert!(!html.contains("No open orders"));
    }

    #[test]
    fn open_orders_partial_renders_empty_state() {
        use crate::hyperliquid::live_state::AccountLiveState;
        let state = AccountLiveState {
            account_address: "0xtest".to_string(),
            environment: "live".to_string(),
            status: LiveConnectionStatus::Connected,
            ..Default::default()
        };
        let view = OpenOrdersView::from_live_state(state);
        let html = OpenOrdersPartialTemplate::render_view(view).unwrap();
        assert!(html.contains("No open orders"));
    }

    #[test]
    fn format_signed_percent_works() {
        assert_eq!(
            format_signed_percent(rust_decimal::Decimal::new(243, 2), 2),
            "+2.43%"
        );
        assert_eq!(
            format_signed_percent(rust_decimal::Decimal::new(-110, 2), 2),
            "-1.10%"
        );
        assert_eq!(
            format_signed_percent(rust_decimal::Decimal::new(0, 0), 2),
            "+0.00%"
        );
    }
}
