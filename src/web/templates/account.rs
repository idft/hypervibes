use askama::Template;

use super::navbar::Navbar;

#[derive(Template)]
#[template(path = "account/page.html")]
pub struct AccountPageTemplate {
    pub wallet_address: String,
    pub fee_bps: i16,
    pub min_fee_bps: i16,
    pub max_fee_bps: i16,
    pub builder_recipient: &'static str,
    pub current_path: String,
    pub accounts: Vec<AccountRowView>,
    pub total_balance: Option<AccountTableBalanceView>,
    pub account_lookup_error: Option<String>,
    pub account_mode_error: Option<String>,
    pub transfers_enabled: bool,
    pub api_wallet_address: Option<String>,
    pub api_wallet_state: String,
    pub api_wallet_expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub api_wallet_expiry_class: &'static str,
    pub api_wallet_show_expired: bool,
    pub builder_fee_approved: bool,
    pub navbar: Navbar,
}

#[derive(Debug, Clone)]
pub struct AccountRowView {
    pub name: String,
    pub address: String,
    pub agent_key: Option<String>,
    pub agent_display_name: Option<String>,
    pub balance: String,
    pub balance_view: AccountTableBalanceView,
    pub transfer_value: String,
}

#[derive(Debug, Clone)]
pub struct AccountTableBalanceView {
    pub formatted: String,
    pub whole: String,
    pub decimals: Option<String>,
    pub is_zero: bool,
}

#[derive(Debug, Clone)]
pub struct TradingAccountChoicesView {
    pub main_address: String,
    pub main_balance: Option<String>,
    pub main_assigned_to: Option<String>,
    pub subaccounts: Vec<SubaccountChoiceView>,
    pub subaccount_capacity: Option<String>,
    pub lookup_error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SubaccountChoiceView {
    pub name: Option<String>,
    pub address: String,
    pub balance: Option<String>,
    pub assigned_to: Option<SubaccountAssignmentView>,
}

/// The agent currently using a trading account, rendered as a link to that
/// agent's page.
#[derive(Debug, Clone)]
pub struct SubaccountAssignmentView {
    pub agent_key: String,
    pub display_name: String,
}
