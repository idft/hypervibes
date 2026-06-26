use std::time::Duration;

use ammonia::Builder as HtmlSanitizer;
use askama::Template;
use chrono::{DateTime, Utc};
use pulldown_cmark::{Options as MarkdownOptions, Parser as MarkdownParser, html};
use rust_decimal::Decimal;

use crate::{
    agents::{
        model::{
            AgentDetailRow, AgentListRow, AgentRuntimeRow, CreateAgentForm, CreateAgentRuntimeForm,
        },
        prompts::{DEFAULT_ANALYSIS_STRATEGY_PROMPT, DEFAULT_TRADING_STRATEGY_PROMPT},
        store::AgentInstrumentOptionRow,
    },
    hyperliquid::{
        live_state::{AccountLiveState, LiveConnectionStatus, LiveOpenOrder, LivePosition},
        queries::{AccountTransactionRow, BalancePoint},
        sync_state::SyncStateRow,
    },
    memory::MemoryRecord,
};

#[derive(Debug, Clone)]
pub struct MoneyCell {
    pub value: String,
    pub color_class: &'static str,
}

fn format_decimal_with_commas(value: Decimal, decimals: usize) -> String {
    let formatted = format!("{:.precision$}", value, precision = decimals);
    add_thousands_separators(&formatted)
}

fn add_thousands_separators(value: &str) -> String {
    let (sign, unsigned) = if let Some(stripped) = value.strip_prefix('-') {
        ("-", stripped)
    } else if let Some(stripped) = value.strip_prefix('+') {
        ("+", stripped)
    } else {
        ("", value)
    };

    let (integer, fractional) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    let mut grouped_reversed = String::with_capacity(integer.len() + integer.len() / 3);
    for (index, ch) in integer.chars().rev().enumerate() {
        if index > 0 && index % 3 == 0 {
            grouped_reversed.push(',');
        }
        grouped_reversed.push(ch);
    }
    let grouped_integer: String = grouped_reversed.chars().rev().collect();

    if fractional.is_empty() {
        format!("{sign}{grouped_integer}")
    } else {
        format!("{sign}{grouped_integer}.{fractional}")
    }
}

fn format_timestamp_utc(value: DateTime<Utc>) -> String {
    value.format("%Y-%m-%d %H:%M UTC").to_string()
}

fn format_timestamp_iso(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true)
}

fn format_optional_timestamp_utc(value: Option<DateTime<Utc>>) -> String {
    value
        .map(format_timestamp_utc)
        .unwrap_or_else(|| "-".to_string())
}

const ANALYSIS_CONTEXT_STALE_AFTER_MINUTES: i64 = 30;
const TRADING_CONTEXT_STALE_AFTER_MINUTES: i64 = 3;

fn is_stale_checkin(
    value: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    stale_after_minutes: i64,
) -> bool {
    value.is_some_and(|timestamp| {
        now.signed_duration_since(timestamp) > chrono::Duration::minutes(stale_after_minutes)
    })
}

fn build_cron_setup_prompt(agent_key: &str) -> String {
    format!(
        "Create the Vibetrading cron jobs for this Hermes profile.\n\nHermes profile: {agent_key}\n\nCreate an analysis cron job:\n- name: vibetrading-analysis\n- schedule: every 15m\n- skill: analysis-loop\n- enabled toolsets: terminal\n- prompt: Run the Vibetrading analysis loop for this profile. Use Vibetrading job context before reasoning.\n\nCreate a trading cron job:\n- name: vibetrading-trading\n- schedule: every 1m\n- skill: trading-loop\n- enabled toolsets: terminal\n- prompt: Run the Vibetrading trading loop for this profile. Use Vibetrading job context before reasoning.\n\nDo not create duplicate jobs if jobs with these names already exist.\n\nExample profile-scoped commands:\nhermes -p {agent_key} cron create ..."
    )
}

pub fn format_money_text(amount: Option<Decimal>) -> String {
    format_money_text_with_decimals(amount, 4)
}

pub fn format_money_text_with_decimals(amount: Option<Decimal>, decimals: usize) -> String {
    match amount {
        None => "-".to_string(),
        Some(value) => {
            let formatted = format_decimal_with_commas(value.abs(), decimals);
            if value.is_sign_negative() {
                format!("({formatted})")
            } else {
                formatted
            }
        }
    }
}

pub fn format_money_cell(amount: Option<Decimal>) -> MoneyCell {
    format_money_cell_with_decimals(amount, 4)
}

pub fn format_money_cell_with_decimals(amount: Option<Decimal>, decimals: usize) -> MoneyCell {
    match amount {
        None => dash_cell(),
        Some(value) if value.is_zero() => dash_cell(),
        Some(value) => {
            let formatted = format_decimal_with_commas(value.abs(), decimals);
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

/// Like [`format_money_cell`] but prefixes positive values with `+`.
/// Used for sparkline change amounts where the sign is part of the display.
pub fn format_signed_money_cell(amount: Option<Decimal>) -> MoneyCell {
    format_signed_money_cell_with_decimals(amount, 4)
}

pub fn format_signed_money_cell_with_decimals(
    amount: Option<Decimal>,
    decimals: usize,
) -> MoneyCell {
    match amount {
        None => dash_cell(),
        Some(value) if value.is_zero() => dash_cell(),
        Some(value) => {
            let formatted = format_decimal_with_commas(value.abs(), decimals);
            if value.is_sign_negative() {
                MoneyCell {
                    value: format!("({formatted})"),
                    color_class: "text-red-400",
                }
            } else {
                MoneyCell {
                    value: format!("+{formatted}"),
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

fn format_neutral_money_cell(amount: Option<Decimal>) -> MoneyCell {
    format_neutral_money_cell_with_decimals(amount, 4)
}

fn format_neutral_money_cell_with_decimals(amount: Option<Decimal>, decimals: usize) -> MoneyCell {
    match amount {
        None => dash_cell(),
        Some(value) if value.is_zero() => dash_cell(),
        Some(value) => {
            let formatted = format_decimal_with_commas(value.abs(), decimals);
            MoneyCell {
                value: if value.is_sign_negative() {
                    format!("({formatted})")
                } else {
                    formatted
                },
                color_class: "text-zinc-300",
            }
        }
    }
}

/// A number rendered as individual digit spans so the frontend can run a
/// roll animation when the value changes.
///
/// * `raw` is the numeric string used to decide animation direction.
/// * `value` is the full human-readable text shown to the user.
/// * `chars` are the characters that should actually roll; `prefix` and
///   `suffix` are rendered as static spans so symbols such as parentheses
///   around a negative PnL never animate or take on the flash color.
#[derive(Debug, Clone)]
pub struct AnimatedNumber {
    pub value: String,
    pub raw: String,
    pub chars: Vec<char>,
    pub color_class: &'static str,
    pub prefix: String,
    pub suffix: String,
}

impl AnimatedNumber {
    pub fn from_decimal(value: Decimal, color_class: &'static str) -> Self {
        let formatted = format_decimal_with_commas(value, 4);
        Self {
            raw: value.to_string(),
            value: formatted.clone(),
            chars: formatted.chars().collect(),
            color_class,
            prefix: String::new(),
            suffix: String::new(),
        }
    }

    pub fn for_pnl(raw_value: Decimal) -> Self {
        if raw_value.is_zero() {
            Self {
                value: "-".to_string(),
                raw: raw_value.to_string(),
                chars: vec!['-'],
                color_class: "text-zinc-500",
                prefix: String::new(),
                suffix: String::new(),
            }
        } else {
            let abs = raw_value.abs();
            let formatted = format_decimal_with_commas(abs, 4);
            let (value, color_class, prefix, suffix) = if raw_value.is_sign_negative() {
                (
                    format!("({formatted})"),
                    "text-red-400",
                    "(".to_string(),
                    ")".to_string(),
                )
            } else {
                (
                    formatted.clone(),
                    "text-emerald-400",
                    String::new(),
                    String::new(),
                )
            };
            Self {
                raw: raw_value.to_string(),
                value,
                chars: formatted.chars().collect(),
                color_class,
                prefix,
                suffix,
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct TransactionView {
    pub row: AccountTransactionRow,
    pub fee_usdc: String,
    pub realized_pnl_usdc: MoneyCell,
    pub usdc_delta: MoneyCell,
    pub running_balance: MoneyCell,
}

impl TransactionView {
    pub fn from_row(row: AccountTransactionRow) -> Self {
        let fee_usdc = format_money_text(row.fee_usdc);
        let realized_pnl_usdc = format_money_cell(row.realized_pnl_usdc);
        let usdc_delta = format_money_cell(row.usdc_delta);
        let running_balance = format_money_cell(row.running_balance);
        Self {
            row,
            fee_usdc,
            realized_pnl_usdc,
            usdc_delta,
            running_balance,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MemoryView {
    pub memory_id: String,
    pub created_at_iso: String,
    pub created_at_fallback_text: String,
    pub symbol: String,
    pub timeframe: String,
    pub memory_type: String,
    pub summary: String,
    pub content_html: String,
    pub metadata_key_count: usize,
    pub metadata_text: Option<String>,
}

impl MemoryView {
    pub fn from_record(row: MemoryRecord) -> Self {
        let metadata_key_count = row.metadata.as_object().map_or(0, |obj| obj.len());
        let metadata_text = if row.metadata.as_object().is_some_and(|obj| !obj.is_empty()) {
            serde_json::to_string_pretty(&row.metadata).ok()
        } else {
            None
        };

        Self {
            memory_id: row.id.to_string(),
            created_at_iso: format_timestamp_iso(row.created_at),
            created_at_fallback_text: format_timestamp_utc(row.created_at),
            symbol: row.symbol,
            timeframe: row.timeframe.unwrap_or_else(|| "general".to_string()),
            memory_type: row.memory_type,
            summary: row.summary,
            content_html: render_memory_markdown_html(&row.content),
            metadata_key_count,
            metadata_text,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MemoryTimelineItem {
    pub memory_id: String,
    pub detail_url: String,
    pub created_at_iso: String,
    pub created_at_fallback_text: String,
    pub symbol: String,
    pub timeframe: String,
    pub memory_type: String,
    pub summary: String,
    pub selected: bool,
}

fn render_memory_markdown_html(content: &str) -> String {
    let mut options = MarkdownOptions::empty();
    options.insert(MarkdownOptions::ENABLE_TABLES);
    options.insert(MarkdownOptions::ENABLE_STRIKETHROUGH);
    options.insert(MarkdownOptions::ENABLE_TASKLISTS);
    options.insert(MarkdownOptions::ENABLE_FOOTNOTES);

    let parser = MarkdownParser::new_ext(content, options);
    let mut rendered = String::new();
    html::push_html(&mut rendered, parser);

    HtmlSanitizer::default().clean(&rendered).to_string()
}

fn build_memory_timeline(
    agent_key: &str,
    rows: &[MemoryRecord],
    selected_memory_id: Option<&str>,
) -> Vec<MemoryTimelineItem> {
    rows.iter()
        .map(|row| {
            let memory_id = row.id.to_string();
            MemoryTimelineItem {
                detail_url: format!("/agents/{agent_key}/memories/{memory_id}"),
                memory_id: memory_id.clone(),
                created_at_iso: format_timestamp_iso(row.created_at),
                created_at_fallback_text: format_timestamp_utc(row.created_at),
                symbol: row.symbol.clone(),
                timeframe: row
                    .timeframe
                    .clone()
                    .unwrap_or_else(|| "general".to_string()),
                memory_type: row.memory_type.clone(),
                summary: row.summary.clone(),
                selected: selected_memory_id == Some(memory_id.as_str()),
            }
        })
        .collect()
}

pub fn build_memory_timeline_for_sse(
    agent_key: &str,
    rows: &[MemoryRecord],
) -> Vec<MemoryTimelineItem> {
    build_memory_timeline(agent_key, rows, None)
}

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
}

impl AgentShowTab {
    fn path(self, agent_key: &str) -> String {
        match self {
            Self::Positions => format!("/agents/{agent_key}"),
            Self::Transactions => format!("/agents/{agent_key}/transactions"),
            Self::Memories => format!("/agents/{agent_key}/memories"),
            Self::Prompts => format!("/agents/{agent_key}/prompts"),
            Self::Settings => format!("/agents/{agent_key}/settings"),
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

#[derive(Template)]
#[template(path = "backends.html")]
pub struct BackendsPageTemplate {
    pub runtimes: Vec<AgentRuntimeRow>,
    pub current_path: String,
}

#[derive(Template)]
#[template(path = "backends_new.html")]
pub struct BackendsNewPageTemplate {
    pub form: CreateAgentRuntimeForm,
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
    pub account_balance_html: String,
    pub open_positions_html: String,
    pub open_orders_html: String,
    pub latest_trade_execution_summary_html: String,
    pub latest_analysis_summary_html: String,
    pub sparklines_html: String,
    pub api_key_last_used_text: String,
    pub analysis_context_last_used_text: String,
    pub trading_context_last_used_text: String,
    pub analysis_context_never_checked_in: bool,
    pub trading_context_never_checked_in: bool,
    pub analysis_context_stale: bool,
    pub trading_context_stale: bool,
    pub show_cron_setup_alert: bool,
    pub show_cron_stale_warning: bool,
    pub analysis_context_stale_after_minutes: i64,
    pub trading_context_stale_after_minutes: i64,
    pub cron_setup_prompt: String,
    pub default_analysis_strategy_prompt: &'static str,
    pub default_trading_strategy_prompt: &'static str,
    pub created_at_text: String,
    pub updated_at_text: String,
    pub current_path: String,
}

impl AgentsShowPageTemplate {
    pub fn new(agent: AgentDetailRow, active_tab: AgentShowTab) -> Self {
        let agent_key = agent.agent_key.clone();
        let now = Utc::now();
        let uses_hermes_runtime = agent.backend_kind == crate::agents::model::BACKEND_KIND_HERMES;
        let analysis_context_never_checked_in = agent.analysis_context_last_used_at.is_none();
        let trading_context_never_checked_in = agent.trading_context_last_used_at.is_none();
        let analysis_context_stale = is_stale_checkin(
            agent.analysis_context_last_used_at,
            now,
            ANALYSIS_CONTEXT_STALE_AFTER_MINUTES,
        );
        let trading_context_stale = is_stale_checkin(
            agent.trading_context_last_used_at,
            now,
            TRADING_CONTEXT_STALE_AFTER_MINUTES,
        );
        let tabs = [
            ("Positions", AgentShowTab::Positions),
            ("Transactions", AgentShowTab::Transactions),
            ("Memories", AgentShowTab::Memories),
            ("Prompts", AgentShowTab::Prompts),
            ("Settings", AgentShowTab::Settings),
        ]
        .into_iter()
        .map(|(label, tab)| AgentShowTabLink {
            label,
            href: tab.path(&agent_key),
            active: tab == active_tab,
        })
        .collect();

        Self {
            api_key_last_used_text: format_optional_timestamp_utc(agent.api_key_last_used_at),
            analysis_context_last_used_text: format_optional_timestamp_utc(
                agent.analysis_context_last_used_at,
            ),
            trading_context_last_used_text: format_optional_timestamp_utc(
                agent.trading_context_last_used_at,
            ),
            analysis_context_never_checked_in,
            trading_context_never_checked_in,
            analysis_context_stale,
            trading_context_stale,
            show_cron_setup_alert: uses_hermes_runtime
                && (analysis_context_never_checked_in || trading_context_never_checked_in),
            show_cron_stale_warning: uses_hermes_runtime
                && (analysis_context_stale || trading_context_stale),
            analysis_context_stale_after_minutes: ANALYSIS_CONTEXT_STALE_AFTER_MINUTES,
            trading_context_stale_after_minutes: TRADING_CONTEXT_STALE_AFTER_MINUTES,
            cron_setup_prompt: build_cron_setup_prompt(&agent_key),
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
            account_balance_html: String::new(),
            open_positions_html: String::new(),
            open_orders_html: String::new(),
            latest_trade_execution_summary_html: String::new(),
            latest_analysis_summary_html: String::new(),
            sparklines_html: String::new(),
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

#[derive(Debug, Clone)]
pub struct OpenCodeWorkspaceSettingsView {
    pub workspace_host_path: String,
    pub workspace_container_path: String,
    pub profile_source: String,
    pub env_exists: bool,
}

#[derive(Template)]
#[template(path = "agent_memory_timeline.html")]
pub struct AgentMemoryTimelinePartialTemplate {
    pub memory_timeline: Vec<MemoryTimelineItem>,
    pub memory_count: usize,
    pub selected_memory_date_text: Option<String>,
}

impl AgentMemoryTimelinePartialTemplate {
    pub fn render_view(
        memory_timeline: Vec<MemoryTimelineItem>,
        memory_count: usize,
        selected_memory_date_text: Option<String>,
    ) -> Result<String, askama::Error> {
        Self {
            memory_timeline,
            memory_count,
            selected_memory_date_text,
        }
        .render()
    }
}

#[derive(Template)]
#[template(path = "agent_memory_detail.html")]
pub struct AgentMemoryDetailPartialTemplate {
    pub memory: MemoryView,
}

impl AgentMemoryDetailPartialTemplate {
    pub fn render_view(memory: MemoryView) -> Result<String, askama::Error> {
        Self { memory }.render()
    }
}

#[derive(Template)]
#[template(path = "agent_memory_detail_page.html")]
pub struct AgentMemoryDetailPageTemplate {
    pub agent: AgentDetailRow,
    pub memory: MemoryView,
    pub memory_detail_html: String,
    pub current_path: String,
}

impl AgentMemoryDetailPageTemplate {
    pub fn render_view(
        agent: AgentDetailRow,
        memory: MemoryView,
        memory_detail_html: String,
    ) -> Result<String, askama::Error> {
        let current_path = format!("/agents/{}/memories/{}", agent.agent_key, memory.memory_id);
        Self {
            agent,
            memory,
            memory_detail_html,
            current_path,
        }
        .render()
    }
}

#[derive(Template)]
#[template(path = "server_error.html")]
pub struct ServerErrorPageTemplate {
    pub message: String,
    pub current_path: String,
}

#[derive(Template)]
#[template(path = "hermes.html")]
pub struct HermesPageTemplate {
    pub reachable: bool,
    pub version: Option<String>,
    pub active_profile: Option<String>,
    pub profiles: Vec<String>,
    pub error: Option<String>,
    pub dashboard_url: Option<String>,
    pub current_path: String,
}

/// View-model for the live account balance card shown on the agent detail
/// page and as a column on the agents index page. The same struct drives
/// both the initial render (when the page is first loaded) and the
/// live-updating SSE swaps (when the orchestrator pushes a new value).
///
/// The value shown mirrors the "Balance" shown in the Hyperliquid UI:
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
    pub total_u_pnl: AnimatedNumber,
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
        let total_u_pnl = state
            .open_positions
            .iter()
            .filter_map(|position| position.unrealized_pnl)
            .fold(Decimal::ZERO, |acc, value| acc + value);

        Self {
            account_address: state.account_address,
            environment: state.environment,
            total_balance,
            total_u_pnl: AnimatedNumber::for_pnl(total_u_pnl),
            status: state.status,
            updated_at: state.updated_at,
        }
    }

    /// The animated total balance value.
    ///
    /// Returns `None` when the value is not yet known; the template uses
    /// this to render a `Loading…` placeholder.
    pub fn total(&self) -> Option<AnimatedNumber> {
        self.total_balance
            .map(|v| AnimatedNumber::from_decimal(v, "text-zinc-100"))
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

/// View-model for a single server-rendered sparkline. The actual SVG is
/// computed by [`SparklineView::from_series`] and stored as a
/// pre-formatted `<polyline points="...">` attribute string so the
/// template can drop it straight into the markup.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SparklineView {
    pub label: &'static str,
    pub polyline: String,
    pub last_value: String,
    pub change: MoneyCell,
    pub is_empty: bool,
    pub width: u32,
    pub height: u32,
}

impl SparklineView {
    /// Build a sparkline from a balance series. Pure function: no DB,
    /// no I/O, easy to unit test.
    ///
    /// * `points.len() < 2` → empty sparkline (`is_empty = true`); if
    ///   exactly one point is supplied its value is shown as
    ///   `last_value` and the change cell is a dash.
    /// * Otherwise the x-axis maps evenly across `0..width` and the
    ///   y-axis is normalized against `min..max` of the balances with a
    ///   small vertical padding, inverted for SVG (y grows downward).
    ///   The change cell is `last - first` formatted via
    ///   [`format_money_cell`].
    pub fn from_series(
        label: &'static str,
        points: &[BalancePoint],
        width: u32,
        height: u32,
    ) -> Self {
        if points.is_empty() {
            return Self {
                label,
                polyline: String::new(),
                last_value: "-".to_string(),
                change: dash_cell(),
                is_empty: true,
                width,
                height,
            };
        }

        if points.len() == 1 {
            return Self {
                label,
                polyline: String::new(),
                last_value: format_money_text(Some(points[0].balance)),
                change: dash_cell(),
                is_empty: true,
                width,
                height,
            };
        }

        let first = points.first().expect("non-empty").balance;
        let last = points.last().expect("non-empty").balance;
        let change = format_signed_money_cell(Some(last - first));
        let last_value = format_money_text(Some(last));

        let mut min = first;
        let mut max = first;
        for p in points {
            if p.balance < min {
                min = p.balance;
            }
            if p.balance > max {
                max = p.balance;
            }
        }
        // Always leave a small vertical gutter so flat lines don't sit
        // exactly on the edge of the viewBox.
        let span = (max - min).abs();
        let pad = if span.is_zero() {
            Decimal::ONE
        } else {
            span * Decimal::new(1, 1)
        };
        let y_min = min - pad;
        let y_max = max + pad;
        let y_range = (y_max - y_min).abs();

        let count = points.len();
        // Map index 0..count-1 evenly across 0..width. Guard against
        // a single-point series, which we already short-circuited above.
        let denom = (count - 1) as i64;
        let x_for = |i: usize| -> f64 {
            if denom == 0 {
                width as f64 / 2.0
            } else {
                (i as f64 / denom as f64) * (width as f64)
            }
        };
        let y_for = |balance: Decimal| -> f64 {
            let normalized = if y_range.is_zero() {
                0.5
            } else {
                ((balance - y_min) / y_range)
                    .to_string()
                    .parse::<f64>()
                    .unwrap_or(0.5)
            };
            // Invert (SVG y grows downward) and leave a 1px gutter.
            let clamped = normalized.clamp(0.0, 1.0);
            (1.0 - clamped) * (height as f64 - 1.0) + 0.5
        };

        let mut buf = String::new();
        for (i, p) in points.iter().enumerate() {
            if i > 0 {
                buf.push(' ');
            }
            let x = x_for(i);
            let y = y_for(p.balance);
            buf.push_str(&format!("{x:.2},{y:.2}"));
        }

        Self {
            label,
            polyline: buf,
            last_value,
            change,
            is_empty: false,
            width,
            height,
        }
    }
}

#[derive(Template)]
#[template(path = "balance_sparklines.html")]
pub struct BalanceSparklinesPartialTemplate {
    pub sparklines: Vec<SparklineView>,
}

impl BalanceSparklinesPartialTemplate {
    pub fn render_view(sparklines: Vec<SparklineView>) -> Result<String, askama::Error> {
        Self { sparklines }.render()
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
    pub total_u_pnl: AnimatedNumber,
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
            total_u_pnl: AnimatedNumber::for_pnl(total_u_pnl),
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
        entry_px: format_neutral_money_cell_with_decimals(pos.entry_px, 0),
        mark_px_or_value: format_money_text_with_decimals(pos.position_value, 0),
        unrealized_pnl: money_cell_for_pnl(pos.unrealized_pnl.unwrap_or_default()),
        liquidation_px: format_neutral_money_cell_with_decimals(pos.liquidation_px, 0),
        margin_used: format_neutral_money_cell_with_decimals(pos.margin_used, 0),
        return_on_equity: roe,
        roe_color_class,
    }
}

fn money_cell_for_pnl(value: Decimal) -> MoneyCell {
    if value.is_zero() {
        dash_cell()
    } else {
        let abs = value.abs();
        let formatted = format_decimal_with_commas(abs, 4);
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
    format_decimal_with_commas(value, 4)
}

fn format_signed_percent(value: Decimal, decimals: usize) -> String {
    let abs = value.abs();
    let formatted = match decimals {
        0 => format_decimal_with_commas(abs, 0),
        1 => format_decimal_with_commas(abs, 1),
        2 => format_decimal_with_commas(abs, 2),
        3 => format_decimal_with_commas(abs, 3),
        _ => format_decimal_with_commas(abs, 4),
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

        let mut indexed: Vec<(Option<rust_decimal::Decimal>, u64, &LiveOpenOrder)> = state
            .open_orders
            .iter()
            .map(|o| (o.limit_px, o.timestamp.unwrap_or(0), o))
            .collect();
        // Sort highest price first; missing prices sort to the end. When prices
        // match, newer orders come first, then coin for deterministic output.
        indexed.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| b.1.cmp(&a.1))
                .then_with(|| a.2.coin.cmp(&b.2.coin))
        });

        let orders: Vec<OpenOrderView> = indexed
            .into_iter()
            .map(|(_, _, order)| order_view(order))
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
        price: format_neutral_money_cell_with_decimals(order.limit_px, 0),
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

#[derive(Template)]
#[template(path = "latest_trade_execution_summary.html")]
pub struct LatestTradeExecutionSummaryPartialTemplate {
    pub summary: Option<String>,
    /// `created_at` of the latest `trade_execution` memory formatted as
    /// an ISO 8601 / RFC 3339 string with a `Z` suffix, suitable for the
    /// `datetime` attribute of a `<time>` element consumed by
    /// `timeago.js`. Empty when no memory exists yet.
    pub created_at_iso: String,
    /// `created_at` of the latest `trade_execution` memory formatted as
    /// `YYYY-MM-DD HH:MM UTC`. Used as the timeago fallback so the
    /// timestamp is meaningful even before client-side JS hydrates.
    /// Empty when no memory exists yet.
    pub created_at_fallback_text: String,
}

impl LatestTradeExecutionSummaryPartialTemplate {
    pub fn render_view(
        summary: Option<String>,
        created_at: Option<DateTime<Utc>>,
    ) -> Result<String, askama::Error> {
        Self {
            summary,
            created_at_iso: created_at.map(format_timestamp_iso).unwrap_or_default(),
            created_at_fallback_text: created_at.map(format_timestamp_utc).unwrap_or_default(),
        }
        .render()
    }
}

#[derive(Template)]
#[template(path = "latest_analysis_summary.html")]
pub struct LatestAnalysisSummaryPartialTemplate {
    pub summary: Option<String>,
    pub detail_url: Option<String>,
    /// `created_at` of the latest `analysis` memory formatted as an
    /// ISO 8601 / RFC 3339 string with a `Z` suffix, suitable for the
    /// `datetime` attribute of a `<time>` element consumed by
    /// `timeago.js`. Empty when no memory exists yet.
    pub created_at_iso: String,
    /// `created_at` of the latest `analysis` memory formatted as
    /// `YYYY-MM-DD HH:MM UTC`. Used as the timeago fallback so the
    /// timestamp is meaningful even before client-side JS hydrates.
    /// Empty when no memory exists yet.
    pub created_at_fallback_text: String,
    /// `expires_at` of the latest `analysis` memory (resolved from
    /// `stale_after` / `valid_for_seconds` / per-timeframe defaults)
    /// formatted as an ISO 8601 / RFC 3339 string, suitable for the
    /// `title` attribute of a `<time>` element. Empty when the row has
    /// no explicit or implicit expiration.
    pub expires_at_iso: String,
    /// `true` when `expires_at` is at or before the server's `now`. The
    /// agent page uses this to highlight the timestamp and surface a
    /// warning icon, since the trading loop will treat the analysis as
    /// stale and the operator should investigate the gap.
    pub is_expired: bool,
}

impl LatestAnalysisSummaryPartialTemplate {
    pub fn render_view(
        summary: Option<String>,
        detail_url: Option<String>,
        created_at: Option<DateTime<Utc>>,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<String, askama::Error> {
        let is_expired = expires_at.is_some_and(|value| value <= Utc::now());
        Self {
            summary,
            detail_url,
            created_at_iso: created_at.map(format_timestamp_iso).unwrap_or_default(),
            created_at_fallback_text: created_at.map(format_timestamp_utc).unwrap_or_default(),
            expires_at_iso: expires_at.map(format_timestamp_iso).unwrap_or_default(),
            is_expired,
        }
        .render()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    fn sample_agent_list_row() -> AgentListRow {
        AgentListRow {
            display_name: "Test Agent".to_string(),
            agent_key: "test-agent".to_string(),
            enabled: true,
            wallet_address: "0x1234567890abcdef".to_string(),
            environment: "live".to_string(),
            api_key: "vt_test_key".to_string(),
            api_key_last_used_at: None,
            backend_kind: "hermes".to_string(),
            runtime_id: "hermes-local".to_string(),
            runtime_name: "Hermes local".to_string(),
            runtime_base_url: Some("http://localhost:19119".to_string()),
        }
    }

    fn sample_agent_detail_row() -> AgentDetailRow {
        let now = Utc::now();
        AgentDetailRow {
            display_name: "Test Agent".to_string(),
            agent_key: "test-agent".to_string(),
            enabled: true,
            analysis_prompt: "Beep boop analysis.".to_string(),
            trading_prompt: "Beep boop trading.".to_string(),
            wallet_address: "0x1234567890abcdef".to_string(),
            environment: "live".to_string(),
            api_key: "vt_test_key".to_string(),
            api_key_last_used_at: None,
            backend_kind: "hermes".to_string(),
            runtime_id: "hermes-local".to_string(),
            runtime_name: "Hermes local".to_string(),
            runtime_base_url: Some("http://localhost:19119".to_string()),
            runtime_config: serde_json::json!({}),
            analysis_context_last_used_at: None,
            trading_context_last_used_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn sample_account_balance_view() -> AccountBalanceView {
        AccountBalanceView {
            account_address: "0x1234567890abcdef".to_string(),
            environment: "live".to_string(),
            total_balance: Some(rust_decimal::Decimal::new(232_6800, 4)),
            total_u_pnl: AnimatedNumber::for_pnl(rust_decimal::Decimal::new(12_3400, 4)),
            status: crate::hyperliquid::live_state::LiveConnectionStatus::Connected,
            updated_at: Some(Utc::now()),
        }
    }

    fn sample_runtime_row() -> AgentRuntimeRow {
        let now = Utc::now();
        AgentRuntimeRow {
            id: "opencode-local".to_string(),
            created_at: now,
            updated_at: now,
            name: "OpenCode local".to_string(),
            backend_kind: "opencode".to_string(),
            enabled: true,
            base_url: Some("http://localhost:14096".to_string()),
            runtime_config: serde_json::json!({}),
        }
    }

    fn sample_memory_record(
        symbol: &str,
        timeframe: Option<&str>,
        memory_type: &str,
        summary: &str,
        content: &str,
    ) -> MemoryRecord {
        MemoryRecord {
            id: Uuid::from_u128(content.len() as u128 + summary.len() as u128),
            created_at: Utc::now(),
            agent_key: "test-agent".to_string(),
            symbol: symbol.to_string(),
            timeframe: timeframe.map(str::to_string),
            memory_type: memory_type.to_string(),
            summary: summary.to_string(),
            content: content.to_string(),
            metadata: serde_json::json!({
                "confidence": "high",
                "source": "test",
            }),
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
                total_u_pnl: AnimatedNumber::for_pnl(rust_decimal::Decimal::ZERO),
                status: crate::hyperliquid::live_state::LiveConnectionStatus::Connected,
                updated_at: Some(Utc::now()),
            },
            api_key_last_used_iso: None,
        };
        let template = AgentsPageTemplate {
            agents: vec![entry],
            current_path: "/agents".to_string(),
        };
        let rendered = template.render().unwrap();
        assert!(rendered.contains("<!DOCTYPE html>"));
        assert!(rendered.contains("Vibetrading Agents"));
        assert!(!rendered.contains("Registered agents"));
        assert!(!rendered.contains("Agents persisted in the registry database."));
        assert!(rendered.contains("Account balance"));
        assert!(rendered.contains("232.6800"));
        assert!(!rendered.contains("USDC"));
        assert!(rendered.contains("Hermes local"));
        assert!(rendered.contains("hermes"));
    }

    #[test]
    fn agents_page_renders_loading_placeholder_when_no_balance() {
        let entry = AgentListEntry {
            row: sample_agent_list_row(),
            account_balance: AccountBalanceView {
                account_address: "0x1234567890abcdef".to_string(),
                environment: "live".to_string(),
                total_balance: None,
                total_u_pnl: AnimatedNumber::for_pnl(rust_decimal::Decimal::ZERO),
                status: crate::hyperliquid::live_state::LiveConnectionStatus::Starting,
                updated_at: None,
            },
            api_key_last_used_iso: None,
        };
        let template = AgentsPageTemplate {
            agents: vec![entry],
            current_path: "/agents".to_string(),
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
        let sparklines = vec![
            SparklineView::from_series("24h", &[], 240, 48),
            SparklineView::from_series("30d", &[], 240, 48),
        ];
        let sparklines_html = BalanceSparklinesPartialTemplate::render_view(sparklines).unwrap();
        let mut template =
            AgentsShowPageTemplate::new(sample_agent_detail_row(), AgentShowTab::Positions);
        template.account_balance_html = account_balance_html;
        template.open_positions_html = open_positions_html;
        template.open_orders_html = open_orders_html;
        template.latest_trade_execution_summary_html =
            LatestTradeExecutionSummaryPartialTemplate::render_view(
                Some("Scaled out into strength".to_string()),
                Some(Utc::now()),
            )
            .unwrap();
        template.sparklines_html = sparklines_html;
        let rendered = template.render().unwrap();
        assert!(rendered.contains("<!DOCTYPE html>"));
        assert!(rendered.contains("Test Agent · Vibetrading"));
        assert!(rendered.contains("delete-modal"));
        assert!(rendered.contains("Delete agent"));
        assert!(rendered.contains("Agent sections"));
        assert!(rendered.contains("data-agent-tabs"));
        assert!(rendered.contains("hx-target=\"#agent-show-tab-content\""));
        assert!(rendered.contains("aria-current=\"page\""));
        assert!(rendered.contains("Transactions"));
        assert!(rendered.contains("Memories"));
        assert!(rendered.contains("Prompts"));
        assert!(rendered.contains("Settings"));
        assert!(rendered.contains("Balance"));
        assert!(rendered.contains("Unrealized"));
        assert!(rendered.contains("Scaled out into strength"));
    }

    #[test]
    fn memories_tab_renders_timeline_date_filter_and_markdown_content() {
        let mut template =
            AgentsShowPageTemplate::new(sample_agent_detail_row(), AgentShowTab::Memories);
        template.set_memories(
            vec![
                sample_memory_record(
                    "BTC",
                    Some("15m"),
                    "analysis",
                    "Momentum remains constructive",
                    "### Readout\n\n- Wait for a pullback before adding risk.\n- Use patient entries and avoid chasing.",
                ),
                sample_memory_record(
                    "ETH",
                    None,
                    "trade_management",
                    "Tighten invalidation",
                    "Trail the stop closer if funding flips and spot momentum weakens.",
                ),
            ],
            "2026-06-20".to_string(),
            Some("Saturday, June 20, 2026".to_string()),
            None,
        );

        let rendered = template.render().unwrap();
        assert!(rendered.contains("Timeline"));
        assert!(rendered.contains("Showing Saturday, June 20, 2026"));
        assert!(rendered.contains("name=\"date\""));
        assert!(rendered.contains("Momentum remains constructive"));
        assert!(rendered.contains("<h3>Readout</h3>"));
        assert!(rendered.contains("<li>Wait for a pullback before adding risk.</li>"));
        assert!(rendered.contains("metadata keys"));
    }

    #[test]
    fn settings_tab_renders_sync_status_table() {
        let mut template =
            AgentsShowPageTemplate::new(sample_agent_detail_row(), AgentShowTab::Settings);
        template.sync_state = vec![SyncStateView::from_row(SyncStateRow::new(
            "0x1234567890abcdef".to_string(),
            "live".to_string(),
            crate::hyperliquid::sync_state::SyncStream::Fills,
        ))];

        let rendered = template.render().unwrap();
        assert!(rendered.contains("Settings"));
        assert!(rendered.contains("Sync status"));
        assert!(rendered.contains("fills"));
    }

    #[test]
    fn prompts_tab_renders_strategy_copy_and_reset_defaults_ui() {
        let template =
            AgentsShowPageTemplate::new(sample_agent_detail_row(), AgentShowTab::Prompts);

        let rendered = template.render().unwrap();
        assert!(rendered.contains("Strategy Prompts"));
        assert!(rendered.contains("Analysis Strategy Prompt"));
        assert!(rendered.contains("Trading Strategy Prompt"));
        assert!(rendered.contains("data-agent-prompt-form=\"analysis\""));
        assert!(rendered.contains("data-agent-prompt-form=\"trading\""));
        assert!(rendered.contains("data-agent-prompt-reset=\"analysis\""));
        assert!(rendered.contains("data-agent-prompt-reset=\"trading\""));
        assert!(rendered.contains("data-agent-prompt-save=\"analysis\""));
        assert!(rendered.contains("data-agent-prompt-save=\"trading\""));
        assert!(rendered.contains("default-analysis-strategy-prompt-value"));
        assert!(rendered.contains("Default analysis validity"));
        assert!(rendered.contains("Time-in-force"));
    }

    #[test]
    fn account_balance_partial_renders_loading_state_when_value_missing() {
        let view = AccountBalanceView {
            account_address: "0xabc".to_string(),
            environment: "live".to_string(),
            total_balance: None,
            total_u_pnl: AnimatedNumber::for_pnl(rust_decimal::Decimal::ZERO),
            status: crate::hyperliquid::live_state::LiveConnectionStatus::Starting,
            updated_at: None,
        };
        let html = AccountBalancePartialTemplate::render_view(view).unwrap();
        assert!(html.contains("Balance"));
        assert!(html.contains("Loading"));
        assert!(!html.contains("USDC"));
    }

    #[test]
    fn account_balance_partial_renders_value_with_status() {
        let view = sample_account_balance_view();
        let html = AccountBalancePartialTemplate::render_view(view).unwrap();
        assert!(html.contains("Balance"));
        assert!(html.contains("Unrealized"));
        assert!(html.contains("232.6800"));
        assert!(html.contains("USDC"));
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
        assert_eq!(view.total_u_pnl.value, "-");
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
        assert_eq!(view.total_u_pnl.value, "-");
    }

    #[test]
    fn animated_number_formats_with_thousands_separators() {
        let number =
            AnimatedNumber::from_decimal(rust_decimal::Decimal::new(1234567, 0), "text-zinc-100");
        assert_eq!(number.value, "1,234,567.0000");
        assert_eq!(number.chars.iter().collect::<String>(), "1,234,567.0000");
    }

    #[test]
    fn money_formatters_support_custom_decimal_places() {
        assert_eq!(
            format_money_text_with_decimals(Some(rust_decimal::Decimal::new(1234567, 0)), 0),
            "1,234,567"
        );
        assert_eq!(
            format_neutral_money_cell_with_decimals(Some(rust_decimal::Decimal::new(31000, 0)), 0)
                .value,
            "31,000"
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
        assert_eq!(view.total_u_pnl.value, "-");
    }

    #[test]
    fn balance_view_sums_total_upnl_from_open_positions() {
        use crate::hyperliquid::live_state::{AccountLiveState, LivePosition};
        let state = AccountLiveState {
            account_address: "0xtest".to_string(),
            environment: "live".to_string(),
            status: LiveConnectionStatus::Connected,
            open_positions: vec![
                LivePosition {
                    unrealized_pnl: Some(rust_decimal::Decimal::new(125, 0)),
                    ..Default::default()
                },
                LivePosition {
                    unrealized_pnl: Some(rust_decimal::Decimal::new(-25, 0)),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let view = AccountBalanceView::from_live_state(state);
        assert_eq!(view.total_u_pnl.value, "100.0000");
        assert_eq!(view.total_u_pnl.color_class, "text-emerald-400");
    }

    #[test]
    fn agents_new_page_renders_base_layout_and_form() {
        let template = AgentsNewPageTemplate {
            form: CreateAgentForm::default(),
            runtimes: vec![sample_runtime_row()],
            errors: vec![],
            current_path: "/agents/new".to_string(),
        };
        let rendered = template.render().unwrap();
        assert!(rendered.contains("<!DOCTYPE html>"));
        assert!(rendered.contains("Create agent · Vibetrading"));
        assert!(rendered.contains("display_name"));
        assert!(rendered.contains("name=\"backend_kind\""));
        assert!(rendered.contains("name=\"runtime_id\""));
        assert!(rendered.contains("OpenCode local"));
    }

    #[test]
    fn backends_page_renders_runtime_row() {
        let template = BackendsPageTemplate {
            runtimes: vec![sample_runtime_row()],
            current_path: "/backends".to_string(),
        };

        let rendered = template.render().unwrap();
        assert!(rendered.contains("Backends"));
        assert!(rendered.contains("opencode-local"));
        assert!(rendered.contains("OpenCode local"));
        assert!(rendered.contains("http://localhost:14096"));
    }

    #[test]
    fn backends_new_page_renders_create_form() {
        let template = BackendsNewPageTemplate {
            form: CreateAgentRuntimeForm {
                backend_kind: "hermes".to_string(),
                enabled: Some("on".to_string()),
                ..Default::default()
            },
            errors: Vec::new(),
            current_path: "/backends/new".to_string(),
        };

        let rendered = template.render().unwrap();
        assert!(rendered.contains("Create backend"));
        assert!(rendered.contains("name=\"id\""));
        assert!(rendered.contains("name=\"backend_kind\""));
        assert!(rendered.contains("name=\"base_url\""));
    }

    #[test]
    fn opencode_agents_do_not_render_hermes_cron_alerts() {
        let mut agent = sample_agent_detail_row();
        agent.backend_kind = "opencode".to_string();
        agent.runtime_name = "OpenCode local".to_string();
        agent.runtime_id = "opencode-local".to_string();
        agent.runtime_base_url = Some("http://localhost:14096".to_string());

        let template = AgentsShowPageTemplate::new(agent, AgentShowTab::Positions);
        let rendered = template.render().unwrap();

        assert!(!rendered.contains("Hermes cron setup required"));
        assert!(!rendered.contains("Hermes cron check-in is stale"));
    }

    #[test]
    fn server_error_page_renders_base_layout() {
        let template = ServerErrorPageTemplate {
            message: "Internal server error: boom".to_string(),
            current_path: String::new(),
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
        assert_eq!(view.summary.total_notional, "36,000.0000");
        // Margin: 600 + 200 = 800
        assert_eq!(view.summary.total_margin_used, "800.0000");
        assert_eq!(view.positions[0].entry_px.value, "2,000");
        assert_eq!(view.positions[0].mark_px_or_value, "6,000");
        assert_eq!(view.positions[0].entry_px.color_class, "text-zinc-300");
        assert_eq!(
            view.positions[0].liquidation_px.color_class,
            "text-zinc-300"
        );
        assert_eq!(view.positions[0].liquidation_px.value, "2,500");
        assert_eq!(view.positions[0].margin_used.color_class, "text-zinc-300");
        assert_eq!(view.positions[0].margin_used.value, "200");
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
        assert!(html.contains("Notional"));
        assert!(!html.contains(">Coin<"));
        assert!(!html.contains(">Side<"));
        assert!(!html.contains("Mark / NTL"));
        assert!(html.contains("BTC"));
        assert!(html.contains("ETH"));
        assert!(html.contains("long"));
        assert!(html.contains("short"));
        assert!(html.contains("+25.00%"));
        assert!(html.contains("-25.00%"));
        assert!(html.contains("1.0000"));
        assert!(html.contains("2.0000"));
        assert!(html.contains("30,000"));
        assert!(html.contains("4,000"));
        assert!(html.contains("(100.0000)"));
        assert!(html.contains("25,000"));
        assert!(html.contains("6,000"));
    }

    #[test]
    fn open_orders_view_sorts_by_price_descending() {
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
        assert_eq!(view.orders[0].price.color_class, "text-zinc-300");
        assert_eq!(view.orders[0].price.value, "31,000");
        let html = OpenOrdersPartialTemplate::render_view(view).unwrap();
        assert!(html.contains("BTC"));
        assert!(html.contains("sell"));
        assert!(html.contains("take_profit_market"));
        assert!(html.contains("31,000"));
        assert!(html.contains("32,000.0000"));
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

    fn bp(at: DateTime<Utc>, balance: Decimal) -> BalancePoint {
        BalancePoint {
            bucket: at,
            balance,
        }
    }

    #[test]
    fn sparkline_from_series_empty_when_no_points() {
        let view = SparklineView::from_series("24h", &[], 240, 48);
        assert!(view.is_empty);
        assert_eq!(view.label, "24h");
        assert_eq!(view.last_value, "-");
        assert!(view.polyline.is_empty());
        assert_eq!(view.change.value, "-");
        assert_eq!(view.change.color_class, "text-zinc-500");
    }

    #[test]
    fn sparkline_from_series_empty_when_single_point() {
        let now = Utc::now();
        let view = SparklineView::from_series("24h", &[bp(now, Decimal::new(100, 0))], 240, 48);
        assert!(view.is_empty);
        assert_eq!(view.last_value, "100.0000");
        assert!(view.polyline.is_empty());
        // Change is undefined for a single point: dash cell.
        assert_eq!(view.change.value, "-");
    }

    #[test]
    fn sparkline_from_series_builds_polyline_and_change() {
        let now = Utc::now();
        let points = vec![
            bp(now - chrono::Duration::hours(3), Decimal::new(100, 0)),
            bp(now - chrono::Duration::hours(2), Decimal::new(150, 0)),
            bp(now - chrono::Duration::hours(1), Decimal::new(120, 0)),
        ];
        let view = SparklineView::from_series("24h", &points, 240, 48);
        assert!(!view.is_empty);
        assert_eq!(view.last_value, "120.0000");
        // last - first = 120 - 100 = 20 → emerald, + prefix.
        assert_eq!(view.change.value, "+20.0000");
        assert_eq!(view.change.color_class, "text-emerald-400");

        // Polyline: one "x,y" pair per point, space-separated.
        let coords: Vec<&str> = view.polyline.split_whitespace().collect();
        assert_eq!(coords.len(), 3);
        for c in &coords {
            assert!(c.contains(','));
        }
    }

    #[test]
    fn sparkline_from_series_change_sign_and_color() {
        let now = Utc::now();
        // First larger than last: change should render in red, parens.
        let points = vec![
            bp(now - chrono::Duration::hours(2), Decimal::new(200, 0)),
            bp(now - chrono::Duration::hours(1), Decimal::new(50, 0)),
        ];
        let view = SparklineView::from_series("30d", &points, 240, 48);
        assert_eq!(view.change.value, "(150.0000)");
        assert_eq!(view.change.color_class, "text-red-400");
        assert_eq!(view.last_value, "50.0000");
    }
}
