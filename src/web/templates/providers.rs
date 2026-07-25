use askama::Template;

use super::navbar::Navbar;

#[derive(Debug, Clone)]
pub struct ProviderConnectionView {
    pub id: String,
    pub name: String,
    pub connected: bool,
    pub models: Vec<String>,
    pub methods: Vec<ProviderAuthMethodView>,
}

#[derive(Debug, Clone)]
pub struct ProviderAuthMethodView {
    pub index: usize,
    pub auth_type: String,
    pub label: String,
    pub prompts: Vec<ProviderAuthPromptView>,
    pub disabled_reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ProviderAuthPromptView {
    pub key: String,
    pub message: String,
    pub placeholder: Option<String>,
    pub select: bool,
    pub options: Vec<ProviderAuthOptionView>,
    pub when: Option<ProviderAuthWhenView>,
}

#[derive(Debug, Clone)]
pub struct ProviderAuthOptionView {
    pub label: String,
    pub value: String,
    pub hint: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ProviderAuthWhenView {
    pub key: String,
    pub op: String,
    pub value: String,
}

#[derive(Debug, Clone)]
pub struct ProviderReloadStatusView {
    pub pending: bool,
    pub running: bool,
    pub failed: bool,
    pub error_summary: Option<String>,
}

impl ProviderReloadStatusView {
    pub fn from_task(
        task: Option<crate::agentic::model::GlobalMaintenanceTaskRow>,
    ) -> Option<Self> {
        let task = task?;
        let pending = task.status == crate::agentic::model::MAINTENANCE_STATUS_QUEUED;
        let running = task.status == crate::agentic::model::MAINTENANCE_STATUS_RUNNING;
        let failed = task.status == crate::agentic::model::MAINTENANCE_STATUS_FAILED;
        if !pending && !running && !failed {
            return None;
        }
        Some(Self {
            pending,
            running,
            failed,
            error_summary: task.error_summary,
        })
    }
}

#[derive(Template)]
#[template(path = "providers.html")]
pub struct ProvidersPageTemplate {
    pub current_path: String,
    pub providers: Vec<ProviderConnectionView>,
    pub connected_count: usize,
    pub notice: Option<String>,
    pub error: Option<String>,
    pub reload_status: Option<ProviderReloadStatusView>,
    pub navbar: Navbar,
}

#[derive(Template)]
#[template(path = "provider_connect.html")]
pub struct ProviderConnectionFormTemplate {
    pub current_path: String,
    pub provider_id: String,
    pub provider_name: String,
    pub method: ProviderAuthMethodView,
    pub error: Option<String>,
    pub navbar: Navbar,
}

#[derive(Template)]
#[template(path = "provider_connect_modal_step.html")]
pub struct ProviderConnectionModalStepTemplate {
    pub provider_id: String,
    pub provider_name: String,
    pub method: ProviderAuthMethodView,
    pub error: Option<String>,
}

#[derive(Template)]
#[template(path = "provider_pending.html")]
pub struct ProviderOAuthPendingTemplate {
    pub current_path: String,
    pub provider_id: String,
    pub provider_name: String,
    pub method: usize,
    pub completion_mode: String,
    pub authorization_url: String,
    pub instructions: String,
    pub error: Option<String>,
    pub navbar: Navbar,
}

#[derive(Template)]
#[template(path = "provider_pending_modal_step.html")]
pub struct ProviderOAuthPendingModalStepTemplate {
    pub provider_id: String,
    pub provider_name: String,
    pub method: usize,
    pub completion_mode: String,
    pub authorization_url: String,
    pub instructions: String,
    pub error: Option<String>,
}

#[derive(Template)]
#[template(path = "provider_reload_status.html")]
pub struct ProviderReloadStatusTemplate {
    pub reload_status: Option<ProviderReloadStatusView>,
}
