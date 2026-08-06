use askama::Template;
use serde::Deserialize;

use crate::{
    agents::{
        model::{AgentDetailRow, AgentListRow, CreateAgentForm},
        store::AgentInstrumentOptionRow,
        strategy_prompts::{
            PROMPT_KIND_ANALYSIS, PROMPT_KIND_ANALYSIS_CODING, PROMPT_KIND_DAILY_REVIEW,
            PROMPT_KIND_MARKET_ANALYSIS, PROMPT_KIND_TRADING,
        },
    },
    memory::MemoryRecord,
    model_catalog::options::ModelPickerOption,
};

use super::jobs::HarnessJobView;
use super::memories::{
    AgentMemoryDetailPartialTemplate, AgentMemoryTimelinePartialTemplate, MemoryTimelineItem,
    MemoryView, TransactionView, build_memory_timeline,
};
use super::navbar::Navbar;
use super::opencode::OpenCodeWorkspaceSettingsView;
use super::runs::HarnessRunView;
use super::{account::TradingAccountChoicesView, balance::AccountBalanceView};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentShowTab {
    Chat,
    Positions,
    Transactions,
    Memories,
    Prompts,
    Settings,
    Jobs,
}

impl AgentShowTab {
    fn path(self, agent_key: &str) -> String {
        match self {
            Self::Chat => format!("/agents/{agent_key}/chat"),
            Self::Positions => format!("/agents/{agent_key}"),
            Self::Transactions => format!("/agents/{agent_key}/transactions"),
            Self::Memories => format!("/agents/{agent_key}/memories"),
            Self::Prompts => format!("/agents/{agent_key}/prompts"),
            Self::Settings => format!("/agents/{agent_key}/settings"),
            Self::Jobs => format!("/agents/{agent_key}/jobs"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AgentShowTabLink {
    pub label: &'static str,
    pub href: String,
    pub active: bool,
}

pub fn build_agent_show_tabs(
    agent: &AgentDetailRow,
    active_tab: AgentShowTab,
) -> Vec<AgentShowTabLink> {
    let agent_key = agent.agent_key.as_str();
    [
        ("Positions", AgentShowTab::Positions),
        ("Chat", AgentShowTab::Chat),
        ("Transactions", AgentShowTab::Transactions),
        ("Memories", AgentShowTab::Memories),
        ("Prompts", AgentShowTab::Prompts),
        ("Jobs", AgentShowTab::Jobs),
        ("Settings", AgentShowTab::Settings),
    ]
    .into_iter()
    .map(|(label, tab)| AgentShowTabLink {
        label,
        href: tab.path(agent_key),
        active: tab == active_tab,
    })
    .collect()
}

/// Row entry shown on the agents index page. Combines the durable
/// [`AgentListRow`] with the in-memory live account-balance view so the
/// page can render the current Hyperliquid total balance in a single
/// table cell.
#[derive(Debug, Clone)]
pub struct AgentListEntry {
    pub row: AgentListRow,
    pub account_balance: AccountBalanceView,
    /// API key `last_used_at` formatted as an ISO 8601 / RFC 3339 string
    /// with a `Z` suffix, suitable for the `datetime` attribute of a
    /// `<time>` element consumed by timeago.js. `None` mirrors
    /// `row.api_key_last_used_at`.
    pub api_key_last_used_iso: Option<String>,
}

#[derive(Template)]
#[template(path = "agents/index.html")]
pub struct AgentsPageTemplate {
    pub agents: Vec<AgentListEntry>,
    pub current_path: String,
    pub navbar: Navbar,
}

#[derive(Template)]
#[template(path = "agents/new.html")]
pub struct AgentsNewPageTemplate {
    pub form: CreateAgentForm,
    pub choices: TradingAccountChoicesView,
    pub selected_account: String,
    pub errors: Vec<String>,
    pub current_path: String,
    pub navbar: Navbar,
}

#[derive(Template)]
#[template(path = "agents/trading-account-choices.html")]
pub struct AgentTradingAccountChoicesTemplate {
    pub choices: TradingAccountChoicesView,
    pub selected_account: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct CreateHarnessJobFormValues {
    pub trigger_type: String,
    pub job_kind: String,
    pub timeframe: String,
    pub timeout_seconds: String,
    pub model_selection: String,
    pub model_variant: String,
    pub operator_prompt: String,
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub struct ModelPickerProviderGroup {
    pub provider_id: String,
    pub provider_name: String,
    pub provider_logo_url: Option<String>,
    pub options: Vec<ModelPickerOption>,
}

#[derive(Debug, Clone)]
pub struct ModelPickerView {
    pub input_id: String,
    pub input_name: String,
    pub variant_input_id: String,
    pub variant_input_name: String,
    pub selected_value: String,
    pub selected_variant: String,
    pub selected_label: String,
    pub empty_label: String,
    pub provider_groups: Vec<ModelPickerProviderGroup>,
    pub options: Vec<ModelPickerOption>,
    pub warning: Option<String>,
    pub show_label: bool,
    pub auto_submit: bool,
    pub use_modal: bool,
    pub lazy_options_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PromptEditorView {
    pub prompt_kind: String,
    pub label: &'static str,
    pub textarea_id: &'static str,
    pub placeholder: &'static str,
    pub prompt: String,
    pub default_prompt: &'static str,
}

impl PromptEditorView {
    pub fn new(prompt_kind: &str, prompt: String, default_prompt: &'static str) -> Self {
        let (label, textarea_id, placeholder) = match prompt_kind {
            PROMPT_KIND_ANALYSIS => (
                "Analysis",
                "analysis_prompt",
                "Assets, timeframes, analysis methods, confidence thresholds, validity, and trade blockers.",
            ),
            PROMPT_KIND_MARKET_ANALYSIS => (
                "Market Analysis",
                "market_analysis_prompt",
                "How timeframe analyses should be synthesized into one execution-facing market view.",
            ),
            PROMPT_KIND_TRADING => (
                "Trading",
                "trading_prompt",
                "Sizing, laddering, time-in-force preference, max orders, stale-order policy, and scaling rules.",
            ),
            PROMPT_KIND_DAILY_REVIEW => (
                "Daily Review",
                "daily_review_prompt",
                "What the daily review should inspect, how it should record learnings, and what patterns to emphasize.",
            ),
            PROMPT_KIND_ANALYSIS_CODING => (
                "Analysis Coding",
                "analysis_coding_prompt",
                "How the coding job should improve reusable analysis code, what constraints it must obey, and how to report changes.",
            ),
            _ => ("Strategy", "strategy_prompt", ""),
        };
        Self {
            prompt_kind: prompt_kind.to_string(),
            label,
            textarea_id,
            placeholder,
            prompt,
            default_prompt,
        }
    }
}

#[derive(Template)]
#[template(path = "agents/jobs/new.html")]
pub struct AgentJobNewPageTemplate {
    pub agent: AgentDetailRow,
    pub tabs: Vec<AgentShowTabLink>,
    pub agent_tabs_use_htmx: bool,
    pub form: CreateHarnessJobFormValues,
    pub model_picker: ModelPickerView,
    pub errors: Vec<String>,
    pub current_path: String,
    pub navbar: Navbar,
}

#[derive(Debug, Clone)]
pub struct AgentRecentRunsView {
    pub recent_runs: Vec<HarnessRunView>,
    pub recent_runs_loaded: bool,
    pub recent_runs_page: usize,
    pub recent_runs_total_pages: usize,
    pub recent_runs_total_count: usize,
    pub recent_runs_range_start: usize,
    pub recent_runs_range_end: usize,
    pub recent_runs_previous_page_url: Option<String>,
    pub recent_runs_next_page_url: Option<String>,
    pub stream_url: String,
}

impl AgentRecentRunsView {
    pub fn new(agent_key: &str, page: usize) -> Self {
        Self {
            recent_runs: Vec::new(),
            recent_runs_loaded: false,
            recent_runs_page: page,
            recent_runs_total_pages: 0,
            recent_runs_total_count: 0,
            recent_runs_range_start: 0,
            recent_runs_range_end: 0,
            recent_runs_previous_page_url: None,
            recent_runs_next_page_url: None,
            stream_url: format!("/agents/{agent_key}/jobs/recent-runs/stream?page={page}"),
        }
    }
}

#[derive(Template)]
#[template(path = "agents/components/recent-runs.html")]
pub struct AgentRecentRunsPartialTemplate {
    pub recent_runs_section: AgentRecentRunsView,
}

impl AgentRecentRunsPartialTemplate {
    pub fn render_view(recent_runs_section: AgentRecentRunsView) -> Result<String, askama::Error> {
        Self {
            recent_runs_section,
        }
        .render()
    }
}

#[derive(Template)]
#[template(path = "agents/show.html")]
pub struct AgentsShowPageTemplate {
    pub agent: AgentDetailRow,
    pub tabs: Vec<AgentShowTabLink>,
    pub agent_tabs_use_htmx: bool,
    pub show_positions_tab: bool,
    pub show_transactions_tab: bool,
    pub show_memories_tab: bool,
    pub show_prompts_tab: bool,
    pub show_settings_tab: bool,
    pub show_jobs_tab: bool,
    pub operation_notice: Option<String>,
    pub transactions: Vec<TransactionView>,
    pub transactions_page: usize,
    pub transactions_total_pages: usize,
    pub transactions_total_count: usize,
    pub transactions_range_start: usize,
    pub transactions_range_end: usize,
    pub transactions_previous_page_url: Option<String>,
    pub transactions_next_page_url: Option<String>,
    pub memory_timeline: Vec<MemoryTimelineItem>,
    pub memory_filter_date_value: String,
    pub memory_filter_error_text: Option<String>,
    pub selected_memory_date_text: Option<String>,
    pub selected_memory_html: String,
    pub memory_timeline_html: String,
    pub has_memory_date_filter: bool,
    pub memory_count: usize,
    pub instrument_options: Vec<AgentInstrumentOptionRow>,
    pub instrument_options_loaded: bool,
    pub has_selected_instruments: bool,
    pub opencode_workspace: Option<OpenCodeWorkspaceSettingsView>,
    pub settings_workspace_warning: Option<String>,
    pub account_balance_html: String,
    pub open_positions_html: String,
    pub open_orders_html: String,
    pub latest_trade_execution_summary_html: String,
    pub latest_analysis_summary_html: String,
    pub sparklines_html: String,
    pub api_key_masked: String,
    pub prompt_editors: Vec<PromptEditorView>,
    pub current_path: String,
    pub is_main_account: bool,
    pub subaccount_name: Option<String>,
    pub jobs: Vec<HarnessJobView>,
    pub jobs_loaded: bool,
    pub can_enable_all_jobs: bool,
    pub can_disable_all_jobs: bool,
    pub recent_runs_section: AgentRecentRunsView,
    pub jobs_warning: Option<String>,
    pub navbar: Navbar,
}

impl AgentsShowPageTemplate {
    pub fn new(agent: AgentDetailRow, active_tab: AgentShowTab) -> Self {
        let agent_key = agent.agent_key.clone();
        let tabs = build_agent_show_tabs(&agent, active_tab);
        Self {
            api_key_masked: mask_api_key(&agent.api_key),
            prompt_editors: Vec::new(),
            current_path: active_tab.path(&agent_key),
            is_main_account: false,
            subaccount_name: None,
            tabs,
            agent_tabs_use_htmx: true,
            show_positions_tab: active_tab == AgentShowTab::Positions,
            show_transactions_tab: active_tab == AgentShowTab::Transactions,
            show_memories_tab: active_tab == AgentShowTab::Memories,
            show_prompts_tab: active_tab == AgentShowTab::Prompts,
            show_settings_tab: active_tab == AgentShowTab::Settings,
            show_jobs_tab: active_tab == AgentShowTab::Jobs,
            operation_notice: None,
            agent,
            transactions: Vec::new(),
            transactions_page: 1,
            transactions_total_pages: 0,
            transactions_total_count: 0,
            transactions_range_start: 0,
            transactions_range_end: 0,
            transactions_previous_page_url: None,
            transactions_next_page_url: None,
            memory_timeline: Vec::new(),
            memory_filter_date_value: String::new(),
            memory_filter_error_text: None,
            selected_memory_date_text: None,
            selected_memory_html: String::new(),
            memory_timeline_html: String::new(),
            has_memory_date_filter: false,
            memory_count: 0,
            instrument_options: Vec::new(),
            instrument_options_loaded: false,
            has_selected_instruments: false,
            opencode_workspace: None,
            settings_workspace_warning: None,
            account_balance_html: String::new(),
            open_positions_html: String::new(),
            open_orders_html: String::new(),
            latest_trade_execution_summary_html: String::new(),
            latest_analysis_summary_html: String::new(),
            sparklines_html: String::new(),
            jobs: Vec::new(),
            jobs_loaded: false,
            can_enable_all_jobs: false,
            can_disable_all_jobs: false,
            recent_runs_section: AgentRecentRunsView::new(&agent_key, 1),
            jobs_warning: None,
            navbar: Navbar::default(),
        }
    }

    pub fn set_memories(
        &mut self,
        rows: Vec<crate::memory::MemoryTimelineRecord>,
        selected_memory: Option<MemoryRecord>,
        filter_date_value: String,
        selected_date_text: Option<String>,
        filter_error_text: Option<String>,
        next_page_url: Option<String>,
    ) {
        let selected_memory = selected_memory.map(MemoryView::from_record);
        let selected_memory_id = selected_memory
            .as_ref()
            .map(|memory| memory.memory_id.as_str());
        self.memory_count = rows.len();
        self.memory_timeline =
            build_memory_timeline(&self.agent.agent_key, &rows, selected_memory_id);
        self.memory_filter_date_value = filter_date_value;
        self.selected_memory_date_text = selected_date_text;
        self.memory_filter_error_text = filter_error_text;
        self.selected_memory_html = selected_memory
            .map(AgentMemoryDetailPartialTemplate::render_view)
            .transpose()
            .unwrap_or_default()
            .unwrap_or_default();
        self.has_memory_date_filter = !self.memory_filter_date_value.is_empty();
        self.memory_timeline_html = AgentMemoryTimelinePartialTemplate::render_view(
            self.memory_timeline.clone(),
            self.memory_count,
            self.selected_memory_date_text.clone(),
            next_page_url,
        )
        .unwrap_or_default();
    }

    pub fn set_prompt_editors(&mut self, prompt_editors: Vec<PromptEditorView>) {
        self.prompt_editors = prompt_editors;
    }
}

fn mask_api_key(api_key: &str) -> String {
    let characters: Vec<char> = api_key.chars().collect();
    if characters.len() <= 12 {
        return "****".to_string();
    }

    let prefix: String = characters[..8].iter().collect();
    let suffix: String = characters[characters.len() - 4..].iter().collect();
    format!("{prefix}...{suffix}")
}
