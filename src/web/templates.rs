use askama::Template;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::{
    agents::model::{AgentDetailRow, AgentListRow, CreateAgentForm},
    hyperliquid::{
        live_state::LiveConnectionStatus,
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
        let template = AgentsShowPageTemplate {
            agent: sample_agent_detail_row(),
            transactions: vec![],
            sync_state: vec![],
            account_balance_html,
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
        assert_eq!(view.total_balance, Some(rust_decimal::Decimal::new(1000, 0)));
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
}
