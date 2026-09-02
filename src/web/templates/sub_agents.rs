use askama::Template;

use crate::agents::model::AgentDetailRow;

use super::agents::{AgentShowTab, AgentShowTabLink, ModelPickerView, build_agent_show_tabs};
use super::navbar::Navbar;
use super::runs::HarnessSubAgentRunView;
use super::shared::{LocalTimestampView, TimeoutEditorView, format_duration, local_timestamp_view};

/// View-model for a single row on the Sub-agents table.
#[derive(Debug, Clone)]
pub struct HarnessSubAgentView {
    pub sub_agent_key: String,
    pub sub_agent_kind: String,
    pub enabled: bool,
    pub enabled_label: &'static str,
    pub enabled_class: &'static str,
    pub is_candle_job: bool,
    pub trigger_text: String,
    pub timeout_text: String,
    pub next_run_at: Option<LocalTimestampView>,
    pub model_text: String,
    pub has_model: bool,
    pub model_logo_url: Option<String>,
    pub detail_url: String,
    pub run_now_action: String,
    pub toggle_action: String,
    pub hidden_enabled_value: &'static str,
}

#[derive(Debug, Clone)]
pub struct HarnessSubAgentDetailView {
    pub id: i64,
    pub sub_agent_key: String,
    pub enabled: bool,
    pub enabled_label: &'static str,
    pub enabled_class: &'static str,
    pub has_model: bool,
    pub is_candle_job: bool,
    pub trigger_text: String,
    pub candle_trigger_editor: CandleTriggerEditorView,
    pub timeout_editor: TimeoutEditorView,
    pub next_run_at: Option<LocalTimestampView>,
    pub operator_prompt: String,
    pub operator_prompt_update_action: String,
    pub prompt_preview_text: String,
    pub prompt_preview_error: Option<String>,
    pub model_error: Option<String>,
    pub highlight_model_selector: bool,
    pub model_selection: String,
    pub model_update_action: String,
    pub run_now_action: String,
    pub toggle_action: String,
    pub delete_action: String,
    pub hidden_enabled_value: &'static str,
    pub notification_send_enabled: bool,
    pub notification_capability_update_action: String,
}

#[derive(Debug, Clone)]
pub struct CandleTriggerEditorView {
    pub edit_text: String,
    pub action: String,
    pub error: Option<String>,
}

impl HarnessSubAgentView {
    pub fn from_row(row: &crate::harness::model::HarnessSubAgentRow) -> Self {
        let model_text = match (row.model_provider_id.as_deref(), row.model_id.as_deref()) {
            (Some(provider), Some(model)) => format!("{provider}/{model}"),
            _ => "—".to_string(),
        };
        let model_logo_url = row
            .model_provider_id
            .as_ref()
            .map(|provider| format!("/model-catalog/logos/{provider}"));
        let has_model = row.model_provider_id.is_some() && row.model_id.is_some();

        let (enabled_label, enabled_class) = if row.enabled {
            (
                "Enabled",
                "border-emerald-900/60 bg-emerald-950/30 text-emerald-300",
            )
        } else {
            ("Disabled", "border-zinc-700 bg-zinc-900/60 text-zinc-400")
        };

        let (is_candle_job, trigger_text) = match row.timeframe.as_deref() {
            Some(timeframe) => (true, format!("At {timeframe} candle close")),
            None if row.sub_agent_kind == "market_analysis" => {
                (false, "After analysis batch completes".to_string())
            }
            None if row.sub_agent_kind == "analysis_coding" => (false, "On demand".to_string()),
            None => (false, "Unscheduled".to_string()),
        };

        Self {
            sub_agent_key: row.sub_agent_key.clone(),
            sub_agent_kind: row.sub_agent_kind.clone(),
            enabled: row.enabled,
            enabled_label,
            enabled_class,
            is_candle_job,
            trigger_text,
            timeout_text: format_duration(row.timeout_seconds),
            next_run_at: row.next_run_at.map(local_timestamp_view),
            model_text,
            has_model,
            model_logo_url,
            detail_url: format!("/agents/{}/sub-agents/{}", row.agent_key, row.id),
            run_now_action: format!("/agents/{}/sub-agents/{}/run", row.agent_key, row.id),
            toggle_action: format!("/agents/{}/sub-agents/{}/toggle", row.agent_key, row.id),
            hidden_enabled_value: if row.enabled { "off" } else { "on" },
        }
    }
}

impl HarnessSubAgentDetailView {
    pub fn from_row(row: &crate::harness::model::HarnessSubAgentRow) -> Self {
        let summary = HarnessSubAgentView::from_row(row);

        Self {
            id: row.id,
            sub_agent_key: row.sub_agent_key.clone(),
            enabled: row.enabled,
            enabled_label: summary.enabled_label,
            enabled_class: summary.enabled_class,
            has_model: summary.has_model,
            is_candle_job: summary.is_candle_job,
            trigger_text: summary.trigger_text.clone(),
            candle_trigger_editor: CandleTriggerEditorView {
                edit_text: row.timeframe.clone().unwrap_or_default(),
                action: format!("/agents/{}/sub-agents/{}/timeframe", row.agent_key, row.id),
                error: None,
            },
            timeout_editor: TimeoutEditorView {
                display_text: summary.timeout_text,
                edit_text: format_duration(row.timeout_seconds),
                action: format!("/agents/{}/sub-agents/{}/timeout", row.agent_key, row.id),
                error: None,
            },
            next_run_at: summary.next_run_at,
            operator_prompt: row.operator_prompt.clone(),
            operator_prompt_update_action: format!(
                "/agents/{}/sub-agents/{}/operator-prompt",
                row.agent_key, row.id
            ),
            prompt_preview_text: String::new(),
            prompt_preview_error: None,
            model_error: None,
            highlight_model_selector: false,
            model_selection: match (row.model_provider_id.as_deref(), row.model_id.as_deref()) {
                (Some(provider), Some(model)) => format!("{provider}/{model}"),
                _ => String::new(),
            },
            model_update_action: format!("/agents/{}/sub-agents/{}/model", row.agent_key, row.id),
            run_now_action: summary.run_now_action,
            toggle_action: summary.toggle_action,
            delete_action: format!("/agents/{}/sub-agents/{}/delete", row.agent_key, row.id),
            hidden_enabled_value: summary.hidden_enabled_value,
            notification_send_enabled: row
                .enabled_capabilities
                .iter()
                .any(|capability| capability == "hypervibes:notification_send"),
            notification_capability_update_action: format!(
                "/agents/{}/sub-agents/{}/notification-capability",
                row.agent_key, row.id
            ),
        }
    }
}

#[derive(Template)]
#[template(path = "agents/sub_agents/detail.html")]
pub struct AgentJobDetailPageTemplate {
    pub agent: AgentDetailRow,
    pub tabs: Vec<AgentShowTabLink>,
    pub agent_tabs_use_htmx: bool,
    pub job: HarnessSubAgentDetailView,
    pub model_picker: ModelPickerView,
    pub job_runs: Vec<HarnessSubAgentRunView>,
    pub job_runs_loaded: bool,
    pub job_runs_page: usize,
    pub job_runs_total_pages: usize,
    pub job_runs_total_count: usize,
    pub job_runs_range_start: usize,
    pub job_runs_range_end: usize,
    pub job_runs_previous_page_url: Option<String>,
    pub job_runs_next_page_url: Option<String>,
    pub current_path: String,
    pub navbar: Navbar,
}

#[derive(Debug, Clone)]
pub struct HarnessSubAgentRunsPagination {
    pub page: usize,
    pub total_pages: usize,
    pub total_count: usize,
    pub range_start: usize,
    pub range_end: usize,
    pub previous_page_url: Option<String>,
    pub next_page_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AgentJobPageNavigation {
    pub notification_count: i64,
    pub navbar: Navbar,
}

impl AgentJobDetailPageTemplate {
    pub fn render_view(
        agent: AgentDetailRow,
        job: HarnessSubAgentDetailView,
        model_picker: ModelPickerView,
        job_runs: Vec<HarnessSubAgentRunView>,
        job_runs_loaded: bool,
        pagination: HarnessSubAgentRunsPagination,
        navigation: AgentJobPageNavigation,
    ) -> Result<String, askama::Error> {
        let AgentJobPageNavigation {
            notification_count,
            navbar,
        } = navigation;
        let current_path = format!("/agents/{}/sub-agents/{}", agent.agent_key, job.id);
        let navbar = navbar.with_selected_agent(
            agent.agent_key.clone(),
            agent.display_name.clone(),
            agent.enabled,
        );
        Self {
            tabs: build_agent_show_tabs(&agent, AgentShowTab::SubAgents, notification_count),
            agent_tabs_use_htmx: false,
            agent,
            job,
            model_picker,
            job_runs,
            job_runs_loaded,
            job_runs_page: pagination.page,
            job_runs_total_pages: pagination.total_pages,
            job_runs_total_count: pagination.total_count,
            job_runs_range_start: pagination.range_start,
            job_runs_range_end: pagination.range_end,
            job_runs_previous_page_url: pagination.previous_page_url,
            job_runs_next_page_url: pagination.next_page_url,
            current_path,
            navbar,
        }
        .render()
    }
}

#[derive(Template)]
#[template(path = "agents/components/model-picker-partial.html")]
pub struct ModelPickerPartialTemplate {
    pub model_picker: ModelPickerView,
}

impl ModelPickerPartialTemplate {
    pub fn render_view(model_picker: ModelPickerView) -> Result<String, askama::Error> {
        Self { model_picker }.render()
    }
}
