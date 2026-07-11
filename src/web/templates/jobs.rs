use askama::Template;

use crate::agents::model::AgentDetailRow;

use super::agents::{AgentShowTab, AgentShowTabLink, ModelPickerView, build_agent_show_tabs};
use super::runs::AgenticRunView;
use super::shared::{LocalTimestampView, TimeoutEditorView, format_duration, local_timestamp_view};

/// View-model for a single row on the Jobs table.
#[derive(Debug, Clone)]
pub struct AgenticJobScheduleView {
    pub job_key: String,
    pub job_kind: String,
    pub enabled: bool,
    pub enabled_label: &'static str,
    pub enabled_class: &'static str,
    pub timeframe_text: String,
    pub timeout_text: String,
    pub next_run_at: LocalTimestampView,
    pub model_text: String,
    pub model_logo_url: Option<String>,
    pub detail_url: String,
    pub run_now_action: String,
    pub toggle_action: String,
    pub delete_action: String,
    pub hidden_enabled_value: &'static str,
}

#[derive(Debug, Clone)]
pub struct AgenticJobDetailView {
    pub id: i64,
    pub job_key: String,
    pub enabled: bool,
    pub enabled_label: &'static str,
    pub enabled_class: &'static str,
    pub timeframe_text: String,
    pub timeout_editor: TimeoutEditorView,
    pub next_run_at: LocalTimestampView,
    pub operator_prompt_text: String,
    pub prompt_preview_text: String,
    pub prompt_preview_error: Option<String>,
    pub model_selection: String,
    pub model_update_action: String,
    pub run_now_action: String,
    pub toggle_action: String,
    pub hidden_enabled_value: &'static str,
}

impl AgenticJobScheduleView {
    pub fn from_row(row: &crate::agentic::model::AgenticJobScheduleRow) -> Self {
        let model_text = match (row.model_provider_id.as_deref(), row.model_id.as_deref()) {
            (Some(provider), Some(model)) => format!("{provider}/{model}"),
            _ => "—".to_string(),
        };
        let model_logo_url = row
            .model_provider_id
            .as_ref()
            .map(|provider| format!("/model-catalog/logos/{provider}"));

        let (enabled_label, enabled_class) = if row.enabled {
            (
                "Enabled",
                "border-emerald-900/60 bg-emerald-950/30 text-emerald-300",
            )
        } else {
            ("Disabled", "border-zinc-700 bg-zinc-900/60 text-zinc-400")
        };

        let timeframe_text = row.timeframe.clone();

        Self {
            job_key: row.job_key.clone(),
            job_kind: row.job_kind.clone(),
            enabled: row.enabled,
            enabled_label,
            enabled_class,
            timeframe_text,
            timeout_text: format_duration(row.timeout_seconds),
            next_run_at: local_timestamp_view(row.next_run_at),
            model_text,
            model_logo_url,
            detail_url: format!("/agents/{}/jobs/{}", row.agent_key, row.id),
            run_now_action: format!("/agents/{}/jobs/{}/run", row.agent_key, row.id),
            toggle_action: format!("/agents/{}/jobs/{}/toggle", row.agent_key, row.id),
            delete_action: format!("/agents/{}/jobs/{}/delete", row.agent_key, row.id),
            hidden_enabled_value: if row.enabled { "off" } else { "on" },
        }
    }
}

impl AgenticJobDetailView {
    pub fn from_row(row: &crate::agentic::model::AgenticJobScheduleRow) -> Self {
        let summary = AgenticJobScheduleView::from_row(row);

        Self {
            id: row.id,
            job_key: row.job_key.clone(),
            enabled: row.enabled,
            enabled_label: summary.enabled_label,
            enabled_class: summary.enabled_class,
            timeframe_text: summary.timeframe_text,
            timeout_editor: TimeoutEditorView {
                display_text: summary.timeout_text,
                edit_text: format_duration(row.timeout_seconds),
                action: format!("/agents/{}/jobs/{}/timeout", row.agent_key, row.id),
                error: None,
            },
            next_run_at: summary.next_run_at,
            operator_prompt_text: if row.operator_prompt.trim().is_empty() {
                "—".to_string()
            } else {
                row.operator_prompt.clone()
            },
            prompt_preview_text: String::new(),
            prompt_preview_error: None,
            model_selection: match (row.model_provider_id.as_deref(), row.model_id.as_deref()) {
                (Some(provider), Some(model)) => format!("{provider}/{model}"),
                _ => String::new(),
            },
            model_update_action: format!("/agents/{}/jobs/{}/model", row.agent_key, row.id),
            run_now_action: summary.run_now_action,
            toggle_action: summary.toggle_action,
            hidden_enabled_value: summary.hidden_enabled_value,
        }
    }
}

#[derive(Template)]
#[template(path = "agent_job_detail_page.html")]
pub struct AgentJobDetailPageTemplate {
    pub agent: AgentDetailRow,
    pub tabs: Vec<AgentShowTabLink>,
    pub agent_tabs_use_htmx: bool,
    pub job: AgenticJobDetailView,
    pub model_picker: ModelPickerView,
    pub job_runs: Vec<AgenticRunView>,
    pub job_runs_loaded: bool,
    pub current_path: String,
}

impl AgentJobDetailPageTemplate {
    pub fn render_view(
        agent: AgentDetailRow,
        job: AgenticJobDetailView,
        model_picker: ModelPickerView,
        job_runs: Vec<AgenticRunView>,
        job_runs_loaded: bool,
    ) -> Result<String, askama::Error> {
        let current_path = format!("/agents/{}/jobs/{}", agent.agent_key, job.id);
        Self {
            tabs: build_agent_show_tabs(&agent, AgentShowTab::Jobs),
            agent_tabs_use_htmx: false,
            agent,
            job,
            model_picker,
            job_runs,
            job_runs_loaded,
            current_path,
        }
        .render()
    }
}

#[derive(Template)]
#[template(path = "model_picker_partial.html")]
pub struct ModelPickerPartialTemplate {
    pub model_picker: ModelPickerView,
}

impl ModelPickerPartialTemplate {
    pub fn render_view(model_picker: ModelPickerView) -> Result<String, askama::Error> {
        Self { model_picker }.render()
    }
}
