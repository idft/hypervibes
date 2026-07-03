use askama::Template;
use serde::Deserialize;

use crate::{
    agents::{
        model::{AgentDetailRow, AgentListRow, AgentRuntimeRow, CreateAgentForm},
        prompts::{DEFAULT_ANALYSIS_STRATEGY_PROMPT, DEFAULT_TRADING_STRATEGY_PROMPT},
        store::AgentInstrumentOptionRow,
    },
    hyperliquid::sync_state::SyncStateRow,
    memory::MemoryRecord,
    model_catalog::options::ModelPickerOption,
};

use super::balance::AccountBalanceView;
use super::jobs::AgenticJobScheduleView;
use super::hooks::AgenticJobHookView;
use super::memories::{
    AgentMemoryDetailPartialTemplate, AgentMemoryTimelinePartialTemplate, MemoryTimelineItem,
    MemoryView, TransactionView, build_memory_timeline,
};
use super::opencode::OpenCodeWorkspaceSettingsView;
use super::runs::AgenticRunView;
use super::shared::{format_optional_timestamp_utc, format_timestamp_utc};

#[derive(Debug, Clone)]
pub struct SyncStateView {
    pub stream_name: String,
    pub status_text: String,
    pub status_class: &'static str,
    pub last_event_time_text: String,
    pub last_event_key_text: String,
    pub last_synced_at_text: String,
}

impl SyncStateView {
    pub fn from_row(row: SyncStateRow) -> Self {
        let (status_text, status_class) = match row.status {
            crate::hyperliquid::sync_state::SyncStatus::Healthy => (
                "healthy".to_string(),
                "border-emerald-900/60 bg-emerald-950/30 text-emerald-300",
            ),
            crate::hyperliquid::sync_state::SyncStatus::Running => (
                "running".to_string(),
                "border-sky-900/60 bg-sky-950/30 text-sky-300",
            ),
            crate::hyperliquid::sync_state::SyncStatus::Pending => (
                "pending".to_string(),
                "border-amber-900/60 bg-amber-950/30 text-amber-300",
            ),
            crate::hyperliquid::sync_state::SyncStatus::Failed => (
                "failed".to_string(),
                "border-red-900/60 bg-red-950/30 text-red-300",
            ),
        };

        Self {
            stream_name: row.stream_name,
            status_text,
            status_class,
            last_event_time_text: format_optional_timestamp_utc(row.last_event_time),
            last_event_key_text: row.last_event_key.unwrap_or_else(|| "-".to_string()),
            last_synced_at_text: format_optional_timestamp_utc(row.last_synced_at),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentShowTab {
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
#[template(path = "agents.html")]
pub struct AgentsPageTemplate {
    pub agents: Vec<AgentListEntry>,
    pub current_path: String,
}

#[derive(Template)]
#[template(path = "agents_new.html")]
pub struct AgentsNewPageTemplate {
    pub form: CreateAgentForm,
    pub runtimes: Vec<AgentRuntimeRow>,
    pub errors: Vec<String>,
    pub current_path: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct CreateAgentScheduleFormValues {
    pub job_kind: String,
    pub timeframe: String,
    pub timeout_seconds: String,
    pub model_selection: String,
    pub operator_prompt: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct CreateAgentHookFormValues {
    pub timeout_seconds: String,
    pub model_selection: String,
    pub operator_prompt: String,
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub struct ModelPickerView {
    pub input_id: String,
    pub input_name: String,
    pub selected_value: String,
    pub selected_label: String,
    pub options: Vec<ModelPickerOption>,
    pub warning: Option<String>,
}

#[derive(Template)]
#[template(path = "agent_schedule_new.html")]
pub struct AgentScheduleNewPageTemplate {
    pub agent: AgentDetailRow,
    pub form: CreateAgentScheduleFormValues,
    pub model_picker: ModelPickerView,
    pub errors: Vec<String>,
    pub current_path: String,
}

#[derive(Template)]
#[template(path = "agent_hook_new.html")]
pub struct AgentHookNewPageTemplate {
    pub agent: AgentDetailRow,
    pub form: CreateAgentHookFormValues,
    pub model_picker: ModelPickerView,
    pub errors: Vec<String>,
    pub current_path: String,
}

#[derive(Template)]
#[template(path = "agents_show.html")]
#[allow(dead_code)]
pub struct AgentsShowPageTemplate {
    pub agent: AgentDetailRow,
    pub tabs: Vec<AgentShowTabLink>,
    pub show_positions_tab: bool,
    pub show_transactions_tab: bool,
    pub show_memories_tab: bool,
    pub show_prompts_tab: bool,
    pub show_settings_tab: bool,
    pub show_jobs_tab: bool,
    pub transactions: Vec<TransactionView>,
    pub memory_timeline: Vec<MemoryTimelineItem>,
    pub memory_filter_date_value: String,
    pub memory_filter_error_text: Option<String>,
    pub selected_memory_date_text: Option<String>,
    pub selected_memory_html: String,
    pub memory_timeline_html: String,
    pub has_memory_date_filter: bool,
    pub memory_count: usize,
    pub sync_state: Vec<SyncStateView>,
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
    pub api_key_last_used_text: String,
    pub default_analysis_strategy_prompt: &'static str,
    pub default_trading_strategy_prompt: &'static str,
    pub created_at_text: String,
    pub updated_at_text: String,
    pub current_path: String,
    pub jobs: Vec<AgenticJobScheduleView>,
    pub hooks: Vec<AgenticJobHookView>,
    pub recent_runs: Vec<AgenticRunView>,
    pub jobs_loaded: bool,
    pub hooks_loaded: bool,
    pub recent_runs_loaded: bool,
    pub recent_runs_page: usize,
    pub recent_runs_total_pages: usize,
    pub recent_runs_total_count: usize,
    pub recent_runs_range_start: usize,
    pub recent_runs_range_end: usize,
    pub recent_runs_previous_page_url: Option<String>,
    pub recent_runs_next_page_url: Option<String>,
    pub jobs_warning: Option<String>,
}

impl AgentsShowPageTemplate {
    pub fn new(agent: AgentDetailRow, active_tab: AgentShowTab) -> Self {
        let agent_key = agent.agent_key.clone();
        let uses_opencode_runtime =
            agent.backend_kind == crate::agents::model::BACKEND_KIND_OPENCODE;
        let mut tab_entries: Vec<(&'static str, AgentShowTab)> = vec![
            ("Positions", AgentShowTab::Positions),
            ("Transactions", AgentShowTab::Transactions),
            ("Memories", AgentShowTab::Memories),
            ("Prompts", AgentShowTab::Prompts),
        ];
        if uses_opencode_runtime {
            tab_entries.push(("Jobs", AgentShowTab::Jobs));
        }
        tab_entries.push(("Settings", AgentShowTab::Settings));
        let tabs = tab_entries
            .into_iter()
            .map(|(label, tab)| AgentShowTabLink {
                label,
                href: tab.path(&agent_key),
                active: tab == active_tab,
            })
            .collect();

        Self {
            api_key_last_used_text: format_optional_timestamp_utc(agent.api_key_last_used_at),
            default_analysis_strategy_prompt: DEFAULT_ANALYSIS_STRATEGY_PROMPT,
            default_trading_strategy_prompt: DEFAULT_TRADING_STRATEGY_PROMPT,
            created_at_text: format_timestamp_utc(agent.created_at),
            updated_at_text: format_timestamp_utc(agent.updated_at),
            current_path: active_tab.path(&agent_key),
            tabs,
            show_positions_tab: active_tab == AgentShowTab::Positions,
            show_transactions_tab: active_tab == AgentShowTab::Transactions,
            show_memories_tab: active_tab == AgentShowTab::Memories,
            show_prompts_tab: active_tab == AgentShowTab::Prompts,
            show_settings_tab: active_tab == AgentShowTab::Settings,
            show_jobs_tab: uses_opencode_runtime && active_tab == AgentShowTab::Jobs,
            agent,
            transactions: Vec::new(),
            memory_timeline: Vec::new(),
            memory_filter_date_value: String::new(),
            memory_filter_error_text: None,
            selected_memory_date_text: None,
            selected_memory_html: String::new(),
            memory_timeline_html: String::new(),
            has_memory_date_filter: false,
            memory_count: 0,
            sync_state: Vec::new(),
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
            hooks: Vec::new(),
            recent_runs: Vec::new(),
            jobs_loaded: false,
            hooks_loaded: false,
            recent_runs_loaded: false,
            recent_runs_page: 1,
            recent_runs_total_pages: 0,
            recent_runs_total_count: 0,
            recent_runs_range_start: 0,
            recent_runs_range_end: 0,
            recent_runs_previous_page_url: None,
            recent_runs_next_page_url: None,
            jobs_warning: None,
        }
    }

    pub fn set_memories(
        &mut self,
        rows: Vec<MemoryRecord>,
        filter_date_value: String,
        selected_date_text: Option<String>,
        filter_error_text: Option<String>,
    ) {
        let selected_memory = rows.first().cloned().map(MemoryView::from_record);
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
        )
        .unwrap_or_default();
    }
}