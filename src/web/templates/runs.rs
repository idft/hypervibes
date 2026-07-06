use askama::Template;
use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::agents::model::AgentDetailRow;

use super::agents::{AgentShowTab, AgentShowTabLink, build_agent_show_tabs};
use super::shared::{
    LocalTimestampView, add_thousands_separators, format_decimal_with_commas, format_duration,
    local_timestamp_view, optional_local_timestamp_view,
};

/// View-model for a single row in a Runs table.
#[derive(Debug, Clone)]
pub struct AgenticRunView {
    pub status_label: String,
    pub status_class: String,
    pub job_key: String,
    pub timeframe_text: String,
    pub scheduled_for: LocalTimestampView,
    pub started_at: Option<LocalTimestampView>,
    pub finished_at: Option<LocalTimestampView>,
    pub duration_text: String,
    pub backend_run_ref: String,
    pub detail_url: String,
    pub error_summary: String,
}

#[derive(Debug, Clone)]
pub struct AgenticRunDetailView {
    pub id: i64,
    pub status: String,
    pub status_label: String,
    pub status_class: String,
    pub job_key: String,
    pub timeframe_text: String,
    pub scheduled_for: LocalTimestampView,
    pub started_at: Option<LocalTimestampView>,
    pub finished_at: Option<LocalTimestampView>,
    pub duration_text: String,
    pub timeout_text: String,
    pub backend_run_ref: String,
    pub error_summary: String,
    pub job_url: Option<String>,
    pub job_label: &'static str,
}

#[derive(Debug, Clone)]
pub struct OpenCodeSessionView {
    pub model_text: String,
    pub input_tokens_text: String,
    pub output_tokens_text: String,
    pub cache_read_tokens_text: String,
    pub cache_write_tokens_text: String,
    pub reasoning_tokens_text: String,
    pub context_tokens_text: String,
    pub peak_context_tokens_text: String,
    pub estimated_cost_text: String,
    pub compaction_count_text: String,
    pub share_url: String,
    pub transcript: Vec<TranscriptItem>,
    pub session_errors: Vec<OpenCodeSessionErrorView>,
}

#[derive(Debug, Clone)]
pub enum TranscriptItem {
    Message(OpenCodeMessageView),
    Tool(OpenCodeToolExecutionView),
}

#[derive(Debug, Clone)]
pub struct OpenCodeMessageView {
    pub created_at: LocalTimestampView,
    pub role_label: String,
    pub role_class: String,
    pub model_text: String,
    pub text: String,
    pub summary: String,
    pub system_prompt: String,
}

#[derive(Debug, Clone)]
pub struct OpenCodeToolExecutionView {
    pub started_at: Option<LocalTimestampView>,
    pub completed_at: Option<LocalTimestampView>,
    pub tool_name: String,
    pub success_label: String,
    pub success_class: String,
    pub duration_text: String,
    pub args_json: String,
    pub result_json: String,
    pub error_text: String,
}

#[derive(Debug, Clone)]
pub struct OpenCodeSessionErrorView {
    pub created_at: LocalTimestampView,
    pub error_type: String,
    pub error_message: String,
    pub error_data_json: String,
}

impl AgenticRunView {
    pub fn from_row(row: &crate::agentic::model::AgenticRunRow) -> Self {
        let (status_label, status_class) = status_badge(row.status.as_str());
        let duration_text = run_duration_text(row.started_at, row.finished_at);

        Self {
            status_label,
            status_class,
            job_key: row.job_key.clone(),
            timeframe_text: row.timeframe.clone().unwrap_or_else(|| "—".to_string()),
            scheduled_for: local_timestamp_view(row.scheduled_for),
            started_at: optional_local_timestamp_view(row.started_at),
            finished_at: optional_local_timestamp_view(row.finished_at),
            duration_text,
            backend_run_ref: row.backend_run_ref.clone().unwrap_or_default(),
            detail_url: format!("/agents/{}/runs/{}", row.agent_key, row.id),
            error_summary: row.error_summary.clone().unwrap_or_default(),
        }
    }
}

impl AgenticRunDetailView {
    pub fn from_row(row: &crate::agentic::model::AgenticRunRow) -> Self {
        let (status_label, status_class) = status_badge(row.status.as_str());

        Self {
            id: row.id,
            status: row.status.clone(),
            status_label,
            status_class,
            job_key: row.job_key.clone(),
            timeframe_text: row.timeframe.clone().unwrap_or_else(|| "—".to_string()),
            scheduled_for: local_timestamp_view(row.scheduled_for),
            started_at: optional_local_timestamp_view(row.started_at),
            finished_at: optional_local_timestamp_view(row.finished_at),
            duration_text: run_duration_text(row.started_at, row.finished_at),
            timeout_text: format_duration(row.timeout_seconds),
            backend_run_ref: row.backend_run_ref.clone().unwrap_or_default(),
            error_summary: row.error_summary.clone().unwrap_or_default(),
            job_url: row
                .schedule_id
                .map(|schedule_id| format!("/agents/{}/jobs/{}", row.agent_key, schedule_id))
                .or_else(|| {
                    row.hook_id
                        .map(|hook_id| format!("/agents/{}/hooks/{}", row.agent_key, hook_id))
                }),
            job_label: if row.hook_id.is_some() { "hook" } else { "job" },
        }
    }
}

impl OpenCodeSessionView {
    pub fn from_detail(detail: &crate::opencode::store::OpenCodeSessionDetail) -> Self {
        let session = &detail.session;
        let model_text = if session.model_provider.is_empty() || session.model_id.is_empty() {
            "—".to_string()
        } else {
            format!("{}/{}", session.model_provider, session.model_id)
        };

        let mut transcript: Vec<(DateTime<Utc>, String, TranscriptItem)> =
            Vec::with_capacity(detail.messages.len() + detail.tool_executions.len());
        for message in &detail.messages {
            let view = OpenCodeMessageView::from_row(message);
            if view.is_empty() {
                continue;
            }
            transcript.push((
                message.created_at,
                message.id.clone(),
                TranscriptItem::Message(view),
            ));
        }
        for tool in &detail.tool_executions {
            transcript.push((
                tool.started_at.unwrap_or(tool.created_at),
                tool.id.to_string(),
                TranscriptItem::Tool(OpenCodeToolExecutionView::from_row(tool)),
            ));
        }
        transcript.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        let transcript: Vec<TranscriptItem> =
            transcript.into_iter().map(|(_, _, item)| item).collect();

        Self {
            model_text,
            input_tokens_text: format_i32(session.input_tokens),
            output_tokens_text: format_i32(session.output_tokens),
            cache_read_tokens_text: format_i32(session.cache_read_tokens),
            cache_write_tokens_text: format_i32(session.cache_write_tokens),
            reasoning_tokens_text: format_i32(session.reasoning_tokens),
            context_tokens_text: format_i32(session.context_tokens),
            peak_context_tokens_text: format_i32(session.peak_context_tokens),
            estimated_cost_text: format_decimal_with_commas(session.estimated_cost, 6),
            compaction_count_text: format_i32(session.compaction_count),
            share_url: session.share_url.clone().unwrap_or_default(),
            transcript,
            session_errors: detail
                .session_errors
                .iter()
                .map(OpenCodeSessionErrorView::from_row)
                .collect(),
        }
    }
}

impl OpenCodeMessageView {
    fn is_empty(&self) -> bool {
        self.text.is_empty() && self.summary.is_empty() && self.system_prompt.is_empty()
    }

    fn from_row(row: &crate::opencode::store::OpenCodeMessageRow) -> Self {
        let (role_label, role_class) = message_role_badge(row.role.as_str());
        let model_text = match (row.model_provider.as_deref(), row.model_id.as_deref()) {
            (Some(provider), Some(model)) if !provider.is_empty() && !model.is_empty() => {
                format!("{provider}/{model}")
            }
            _ => "—".to_string(),
        };

        Self {
            created_at: local_timestamp_view(row.created_at),
            role_label,
            role_class,
            model_text,
            text: row.text.clone().unwrap_or_default(),
            summary: row.summary.clone().unwrap_or_default(),
            system_prompt: row.system_prompt.clone().unwrap_or_default(),
        }
    }
}

impl OpenCodeToolExecutionView {
    fn from_row(row: &crate::opencode::store::OpenCodeToolExecutionRow) -> Self {
        let (success_label, success_class) = match row.success {
            Some(true) => (
                "success".to_string(),
                "border-emerald-900/60 bg-emerald-950/30 text-emerald-300".to_string(),
            ),
            Some(false) => (
                "failed".to_string(),
                "border-red-900/60 bg-red-950/30 text-red-300".to_string(),
            ),
            None => (
                "unknown".to_string(),
                "border-zinc-700 bg-zinc-900/60 text-zinc-300".to_string(),
            ),
        };

        Self {
            started_at: optional_local_timestamp_view(row.started_at),
            completed_at: optional_local_timestamp_view(row.completed_at),
            tool_name: row.tool_name.clone(),
            success_label,
            success_class,
            duration_text: row
                .duration_ms
                .map(|duration_ms| format!("{duration_ms}ms"))
                .unwrap_or_else(|| "—".to_string()),
            args_json: format_json_value(row.args.as_ref()),
            result_json: format_json_value(row.result.as_ref()),
            error_text: row.error.clone().unwrap_or_default(),
        }
    }
}

impl OpenCodeSessionErrorView {
    fn from_row(row: &crate::opencode::store::OpenCodeSessionErrorRow) -> Self {
        Self {
            created_at: local_timestamp_view(row.created_at),
            error_type: row.error_type.clone().unwrap_or_default(),
            error_message: row.error_message.clone().unwrap_or_default(),
            error_data_json: format_json_value(row.error_data.as_ref()),
        }
    }
}

#[derive(Template)]
#[template(path = "agent_run_detail_page.html")]
pub struct AgentRunDetailPageTemplate {
    pub agent: AgentDetailRow,
    pub tabs: Vec<AgentShowTabLink>,
    pub agent_tabs_use_htmx: bool,
    pub run: AgenticRunDetailView,
    pub session: Option<OpenCodeSessionView>,
    pub session_lookup_attempted: bool,
    pub current_path: String,
}

impl AgentRunDetailPageTemplate {
    pub fn render_view(
        agent: AgentDetailRow,
        run: AgenticRunDetailView,
        session: Option<OpenCodeSessionView>,
        session_lookup_attempted: bool,
    ) -> Result<String, askama::Error> {
        let current_path = format!("/agents/{}/runs/{}", agent.agent_key, run.id);
        Self {
            tabs: build_agent_show_tabs(&agent, AgentShowTab::Jobs),
            agent_tabs_use_htmx: false,
            agent,
            run,
            session,
            session_lookup_attempted,
            current_path,
        }
        .render()
    }
}

pub(super) fn status_badge(status: &str) -> (String, String) {
    match status {
        "queued" => (
            "queued".to_string(),
            "border-zinc-700 bg-zinc-900/60 text-zinc-300".to_string(),
        ),
        "running" => (
            "running".to_string(),
            "border-sky-900/60 bg-sky-950/30 text-sky-300".to_string(),
        ),
        "succeeded" => (
            "succeeded".to_string(),
            "border-emerald-900/60 bg-emerald-950/30 text-emerald-300".to_string(),
        ),
        "failed" => (
            "failed".to_string(),
            "border-red-900/60 bg-red-950/30 text-red-300".to_string(),
        ),
        "aborted" => (
            "aborted".to_string(),
            "border-amber-900/60 bg-amber-950/30 text-amber-300".to_string(),
        ),
        "skipped" => (
            "skipped".to_string(),
            "border-violet-900/60 bg-violet-950/30 text-violet-300".to_string(),
        ),
        other => (
            other.to_string(),
            "border-zinc-700 bg-zinc-900/60 text-zinc-300".to_string(),
        ),
    }
}

pub(super) fn run_duration_text(
    started_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
) -> String {
    match (started_at, finished_at) {
        (Some(start), Some(end)) => {
            let secs = (end - start).num_seconds().max(0);
            format!("{secs}s")
        }
        _ => "—".to_string(),
    }
}

pub(super) fn message_role_badge(role: &str) -> (String, String) {
    match role {
        "assistant" => (
            "assistant".to_string(),
            "border-sky-900/60 bg-sky-950/30 text-sky-300".to_string(),
        ),
        "user" => (
            "user".to_string(),
            "border-emerald-900/60 bg-emerald-950/30 text-emerald-300".to_string(),
        ),
        "system" => (
            "system".to_string(),
            "border-violet-900/60 bg-violet-950/30 text-violet-300".to_string(),
        ),
        other => (
            other.to_string(),
            "border-zinc-700 bg-zinc-900/60 text-zinc-300".to_string(),
        ),
    }
}

pub(super) fn format_json_value(value: Option<&Value>) -> String {
    value
        .and_then(|value| serde_json::to_string_pretty(value).ok())
        .unwrap_or_default()
}

pub(super) fn format_i32(value: i32) -> String {
    add_thousands_separators(&value.to_string())
}
