use askama::Template;
use uuid::Uuid;

use crate::{
    agent_conversations::model::AgentConversationListRow, agents::model::AgentDetailRow,
    opencode::client::OpenCodePermissionRequest,
};

use super::{
    agents::{AgentShowTab, AgentShowTabLink, ModelPickerView, build_agent_show_tabs},
    navbar::Navbar,
    runs::{OpenCodeSessionView, TranscriptItem},
};

#[derive(Debug, Clone)]
pub struct AgentConversationListItemView {
    pub title: String,
    pub model_text: String,
    pub status_text: String,
    pub selected: bool,
    pub href: String,
}
impl AgentConversationListItemView {
    pub fn from_row(row: &AgentConversationListRow, selected: Option<Uuid>) -> Self {
        Self {
            title: row.title.clone(),
            model_text: match row.model_variant.as_deref() {
                Some(variant) => format!("{}/{} - {variant}", row.model_provider_id, row.model_id),
                None => format!("{}/{}", row.model_provider_id, row.model_id),
            },
            status_text: row
                .opencode_status
                .clone()
                .unwrap_or_else(|| "syncing".to_string()),
            selected: selected == Some(row.id),
            href: format!("/agents/{}/chat/{}", row.agent_key, row.id),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AgentConversationSettingsView {
    pub model_picker: ModelPickerView,
    pub orders_policy: String,
    pub memory_writes_policy: String,
    pub disabled: bool,
}
#[derive(Debug, Clone)]
pub struct AgentConversationPermissionRequestView {
    pub id: String,
    pub permission: String,
    pub patterns: String,
}
impl From<&OpenCodePermissionRequest> for AgentConversationPermissionRequestView {
    fn from(value: &OpenCodePermissionRequest) -> Self {
        Self {
            id: value.id.clone(),
            permission: value.permission.clone(),
            patterns: value.patterns.join(", "),
        }
    }
}

#[derive(Template)]
#[template(path = "agents/chat/page.html")]
pub struct AgentConversationPageTemplate {
    pub agent: AgentDetailRow,
    pub tabs: Vec<AgentShowTabLink>,
    pub agent_tabs_use_htmx: bool,
    pub current_path: String,
    pub conversation_id: Uuid,
    pub sidebar_html: String,
    pub summary_html: String,
    pub transcript_html: String,
    pub composer_html: String,
    pub permissions_html: String,
    pub navbar: Navbar,
}
#[derive(Template)]
#[template(path = "agents/chat/empty.html")]
pub struct AgentConversationEmptyPageTemplate {
    pub agent: AgentDetailRow,
    pub tabs: Vec<AgentShowTabLink>,
    pub agent_tabs_use_htmx: bool,
    pub current_path: String,
    pub model_picker: ModelPickerView,
    pub errors: Vec<String>,
    pub navbar: Navbar,
}
#[derive(Template)]
#[template(path = "agents/chat/sidebar.html")]
pub struct AgentConversationSidebarPartialTemplate {
    pub agent_key: String,
    pub new_conversation_model_selection: String,
    pub new_conversation_model_variant: String,
    pub conversations: Vec<AgentConversationListItemView>,
}
#[derive(Template)]
#[template(path = "agents/chat/summary.html")]
pub struct AgentConversationSummaryPartialTemplate {
    pub agent_key: String,
    pub conversation_id: Uuid,
    pub title: String,
    pub model_text: String,
    pub busy: bool,
    pub session: Option<OpenCodeSessionView>,
    pub settings: AgentConversationSettingsView,
}
#[derive(Template)]
#[template(path = "agents/chat/transcript.html")]
pub struct AgentConversationTranscriptPartialTemplate {
    pub session: Option<OpenCodeSessionView>,
    pub busy: bool,
}
#[derive(Template)]
#[template(path = "agents/chat/composer.html")]
pub struct AgentConversationComposerPartialTemplate {
    pub agent_key: String,
    pub conversation_id: Uuid,
    pub message_id: String,
    pub busy: bool,
    pub message: String,
    pub error: Option<String>,
}
#[derive(Template)]
#[template(path = "agents/chat/permissions.html")]
pub struct AgentConversationPermissionsPartialTemplate {
    pub agent_key: String,
    pub conversation_id: Uuid,
    pub requests: Vec<AgentConversationPermissionRequestView>,
}

impl AgentConversationPageTemplate {
    pub fn render_view(input: AgentConversationPageInput) -> Result<String, askama::Error> {
        let AgentConversationPageInput {
            agent,
            conversation_id,
            sidebar_html,
            summary_html,
            transcript_html,
            composer_html,
            permissions_html,
            navbar,
        } = input;
        let current_path = format!("/agents/{}/chat/{conversation_id}", agent.agent_key);
        Self {
            tabs: build_agent_show_tabs(&agent, AgentShowTab::Chat),
            agent_tabs_use_htmx: false,
            current_path,
            agent,
            conversation_id,
            sidebar_html,
            summary_html,
            transcript_html,
            composer_html,
            permissions_html,
            navbar,
        }
        .render()
    }
}

pub struct AgentConversationPageInput {
    pub agent: AgentDetailRow,
    pub conversation_id: Uuid,
    pub sidebar_html: String,
    pub summary_html: String,
    pub transcript_html: String,
    pub composer_html: String,
    pub permissions_html: String,
    pub navbar: Navbar,
}
impl AgentConversationEmptyPageTemplate {
    pub fn render_view(
        agent: AgentDetailRow,
        mut model_picker: ModelPickerView,
        errors: Vec<String>,
        navbar: Navbar,
    ) -> Result<String, askama::Error> {
        let current_path = format!("/agents/{}/chat", agent.agent_key);
        // The empty Chat view is viewport-bounded, so use the picker modal
        // rather than an inline option list that could extend off-screen.
        model_picker.use_modal = true;
        Self {
            tabs: build_agent_show_tabs(&agent, AgentShowTab::Chat),
            agent_tabs_use_htmx: false,
            current_path,
            agent,
            model_picker,
            errors,
            navbar,
        }
        .render()
    }
}
pub fn conversation_items(
    conversations: &[AgentConversationListRow],
    selected: Option<Uuid>,
) -> Vec<AgentConversationListItemView> {
    conversations
        .iter()
        .map(|row| AgentConversationListItemView::from_row(row, selected))
        .collect()
}
