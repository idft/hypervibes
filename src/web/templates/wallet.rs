use askama::Template;

#[derive(Template)]
#[template(path = "wallet.html")]
pub struct WalletPageTemplate {
    pub wallet_address: String,
    pub fee_tenths_of_bp: i16,
    pub builder_fee_approved: bool,
    pub builder_recipient: &'static str,
    pub current_path: String,
    pub agents: Vec<WalletAgentView>,
    pub api_wallet_address: Option<String>,
    pub api_wallet_state: String,
}

#[derive(Debug, Clone)]
pub struct WalletAgentView {
    pub agent_key: String,
    pub display_name: String,
    pub lifecycle: String,
    pub trading_account_address: String,
    pub balance: String,
    pub expiry: String,
}

#[derive(Debug, Clone)]
pub struct TradingAccountChoicesView {
    pub main_address: String,
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
