use askama::Template;

use crate::agents::model::AgentDetailRow;

use super::agents::{AgentShowTab, AgentShowTabLink, ModelPickerView, build_agent_show_tabs};
use super::navbar::Navbar;
use super::runs::AgenticRunView;
use super::shared::{LocalTimestampView, TimeoutEditorView, format_duration, local_timestamp_view};

#[derive(Debug, Clone)]
pub struct AgenticJobHookView {
    pub job_key: String,
    pub job_kind: String,
    pub hook_event: String,
    pub enabled: bool,
    pub enabled_label: &'static str,
    pub enabled_class: &'static str,
    pub timeout_text: String,
    pub model_text: String,
    pub has_model: bool,
    pub model_logo_url: Option<String>,
    pub detail_url: String,
    pub run_now_action: String,
    pub toggle_action: String,
    pub hidden_enabled_value: &'static str,
}

#[derive(Debug, Clone)]
pub struct AgenticHookDetailView {
    pub id: i64,
    pub job_key: String,
    pub job_kind: String,
    pub hook_event: String,
    pub enabled: bool,
    pub enabled_label: &'static str,
    pub enabled_class: &'static str,
    pub has_model: bool,
    pub timeout_editor: TimeoutEditorView,
    pub created_at: LocalTimestampView,
    pub updated_at: LocalTimestampView,
    pub operator_prompt_text: String,
    pub prompt_preview_text: String,
    pub prompt_preview_error: Option<String>,
    pub model_selection: String,
    pub model_update_action: String,
    pub run_now_action: String,
    pub toggle_action: String,
    pub delete_action: String,
    pub hidden_enabled_value: &'static str,
}

impl AgenticJobHookView {
    pub fn from_row(row: &crate::agentic::model::AgenticJobHookRow) -> Self {
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

        Self {
            job_key: row.job_key.clone(),
            job_kind: row.job_kind.clone(),
            hook_event: row.hook_event.clone(),
            enabled: row.enabled,
            enabled_label,
            enabled_class,
            timeout_text: format_duration(row.timeout_seconds),
            model_text,
            has_model,
            model_logo_url,
            detail_url: format!("/agents/{}/hooks/{}", row.agent_key, row.id),
            run_now_action: format!("/agents/{}/hooks/{}/run", row.agent_key, row.id),
            toggle_action: format!("/agents/{}/hooks/{}/toggle", row.agent_key, row.id),
            hidden_enabled_value: if row.enabled { "off" } else { "on" },
        }
    }
}

impl AgenticHookDetailView {
    pub fn from_row(row: &crate::agentic::model::AgenticJobHookRow) -> Self {
        let summary = AgenticJobHookView::from_row(row);

        Self {
            id: row.id,
            job_key: row.job_key.clone(),
            job_kind: row.job_kind.clone(),
            hook_event: row.hook_event.clone(),
            enabled: row.enabled,
            enabled_label: summary.enabled_label,
            enabled_class: summary.enabled_class,
            has_model: summary.has_model,
            timeout_editor: TimeoutEditorView {
                display_text: summary.timeout_text,
                edit_text: format_duration(row.timeout_seconds),
                action: format!("/agents/{}/hooks/{}/timeout", row.agent_key, row.id),
                error: None,
            },
            created_at: local_timestamp_view(row.created_at),
            updated_at: local_timestamp_view(row.updated_at),
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
            model_update_action: format!("/agents/{}/hooks/{}/model", row.agent_key, row.id),
            run_now_action: summary.run_now_action,
            toggle_action: summary.toggle_action,
            delete_action: format!("/agents/{}/hooks/{}/delete", row.agent_key, row.id),
            hidden_enabled_value: summary.hidden_enabled_value,
        }
    }
}

#[derive(Template)]
#[template(path = "agents/hooks/detail.html")]
pub struct AgentHookDetailPageTemplate {
    pub agent: AgentDetailRow,
    pub tabs: Vec<AgentShowTabLink>,
    pub agent_tabs_use_htmx: bool,
    pub hook: AgenticHookDetailView,
    pub model_picker: ModelPickerView,
    pub hook_runs: Vec<AgenticRunView>,
    pub hook_runs_loaded: bool,
    pub run_now_warning: Option<String>,
    pub current_path: String,
    pub navbar: Navbar,
}

impl AgentHookDetailPageTemplate {
    pub fn render_view(
        agent: AgentDetailRow,
        hook: AgenticHookDetailView,
        model_picker: ModelPickerView,
        hook_runs: Vec<AgenticRunView>,
        hook_runs_loaded: bool,
        run_now_warning: Option<String>,
        navbar: Navbar,
    ) -> Result<String, askama::Error> {
        let current_path = format!("/agents/{}/hooks/{}", agent.agent_key, hook.id);
        let navbar = navbar.with_selected_agent(agent.display_name.clone(), agent.enabled);
        Self {
            tabs: build_agent_show_tabs(&agent, AgentShowTab::Jobs),
            agent_tabs_use_htmx: false,
            agent,
            hook,
            model_picker,
            hook_runs,
            hook_runs_loaded,
            run_now_warning,
            current_path,
            navbar,
        }
        .render()
    }
}
