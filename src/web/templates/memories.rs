use ammonia::Builder as HtmlSanitizer;
use askama::Template;

use super::agents::{AgentShowTab, AgentShowTabLink, build_agent_show_tabs};
use pulldown_cmark::{Options as MarkdownOptions, Parser as MarkdownParser, html};

use crate::{
    agents::model::AgentDetailRow,
    hyperliquid::queries::AccountTransactionRow,
    memory::{MemoryRecord, MemoryTimelineRecord},
};

use super::navbar::Navbar;
use super::shared::{
    LocalTimestampView, MoneyCell, format_money_cell, format_money_text, format_timestamp_iso,
    format_timestamp_utc, local_timestamp_view,
};

#[derive(Debug, Clone)]
pub struct TransactionView {
    pub row: AccountTransactionRow,
    pub event_time: LocalTimestampView,
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
            event_time: local_timestamp_view(row.event_time),
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
    pub delete_action: String,
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
            delete_action: format!("/agents/{}/memories/{}/delete", row.agent_key, row.id),
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

pub(super) fn render_memory_markdown_html(content: &str) -> String {
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

pub(super) fn build_memory_timeline(
    agent_key: &str,
    rows: &[MemoryTimelineRecord],
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
    rows: &[MemoryTimelineRecord],
) -> Vec<MemoryTimelineItem> {
    build_memory_timeline(agent_key, rows, None)
}

#[derive(Template)]
#[template(path = "agents/memories/timeline.html")]
pub struct AgentMemoryTimelinePartialTemplate {
    pub memory_timeline: Vec<MemoryTimelineItem>,
    pub memory_count: usize,
    pub selected_memory_date_text: Option<String>,
    pub next_page_url: Option<String>,
}

impl AgentMemoryTimelinePartialTemplate {
    pub fn render_view(
        memory_timeline: Vec<MemoryTimelineItem>,
        memory_count: usize,
        selected_memory_date_text: Option<String>,
        next_page_url: Option<String>,
    ) -> Result<String, askama::Error> {
        Self {
            memory_timeline,
            memory_count,
            selected_memory_date_text,
            next_page_url,
        }
        .render()
    }
}

#[derive(Template)]
#[template(path = "agents/memories/timeline-items.html")]
pub struct AgentMemoryTimelineItemsPartialTemplate {
    pub memory_timeline: Vec<MemoryTimelineItem>,
    pub next_page_url: Option<String>,
}

impl AgentMemoryTimelineItemsPartialTemplate {
    pub fn render_view(
        memory_timeline: Vec<MemoryTimelineItem>,
        next_page_url: Option<String>,
    ) -> Result<String, askama::Error> {
        Self {
            memory_timeline,
            next_page_url,
        }
        .render()
    }
}

#[derive(Template)]
#[template(path = "agents/memories/detail.html")]
pub struct AgentMemoryDetailPartialTemplate {
    pub memory: MemoryView,
    pub back_url: Option<String>,
}

impl AgentMemoryDetailPartialTemplate {
    pub fn render_view(memory: MemoryView) -> Result<String, askama::Error> {
        Self {
            memory,
            back_url: None,
        }
        .render()
    }

    pub fn render_page_view(memory: MemoryView, back_url: String) -> Result<String, askama::Error> {
        Self {
            memory,
            back_url: Some(back_url),
        }
        .render()
    }
}

#[derive(Template)]
#[template(path = "agents/memories/detail-page.html")]
pub struct AgentMemoryDetailPageTemplate {
    pub agent: AgentDetailRow,
    pub tabs: Vec<AgentShowTabLink>,
    pub agent_tabs_use_htmx: bool,
    pub memory: MemoryView,
    pub memory_detail_html: String,
    pub current_path: String,
    pub navbar: Navbar,
}

impl AgentMemoryDetailPageTemplate {
    pub fn render_view(
        agent: AgentDetailRow,
        memory: MemoryView,
        memory_detail_html: String,
        notification_count: i64,
        navbar: Navbar,
    ) -> Result<String, askama::Error> {
        let current_path = format!("/agents/{}/memories/{}", agent.agent_key, memory.memory_id);
        let navbar = navbar.with_selected_agent(
            agent.agent_key.clone(),
            agent.display_name.clone(),
            agent.enabled,
        );
        Self {
            tabs: build_agent_show_tabs(&agent, AgentShowTab::Memories, notification_count),
            agent_tabs_use_htmx: false,
            agent,
            memory,
            memory_detail_html,
            current_path,
            navbar,
        }
        .render()
    }
}
