use askama::Template;

use crate::agents::model::{AgentRuntimeRow, CreateAgentRuntimeForm};

use super::shared::LocalTimestampView;

#[derive(Debug, Clone)]
pub struct AgentRuntimeView {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub base_url: Option<String>,
    pub created_at: LocalTimestampView,
}

impl AgentRuntimeView {
    pub fn from_row(row: AgentRuntimeRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            enabled: row.enabled,
            base_url: row.base_url,
            created_at: super::shared::local_timestamp_view(row.created_at),
        }
    }
}

#[derive(Template)]
#[template(path = "backends.html")]
pub struct BackendsPageTemplate {
    pub runtimes: Vec<AgentRuntimeView>,
    pub current_path: String,
}

#[derive(Template)]
#[template(path = "backends_new.html")]
pub struct BackendsNewPageTemplate {
    pub form: CreateAgentRuntimeForm,
    pub errors: Vec<String>,
    pub current_path: String,
}
