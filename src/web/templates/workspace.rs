use askama::Template;

use crate::{
    agents::model::AgentDetailRow,
    web::templates::{AgentShowTabLink, Navbar},
};

#[derive(Debug, Clone)]
pub struct WorkspaceTreeEntryView {
    pub path: String,
    pub name: String,
    pub href: String,
    pub depth: usize,
    pub is_directory: bool,
    pub selected: bool,
    pub initially_hidden: bool,
}

#[derive(Template)]
#[template(path = "agents/workspace/page.html")]
pub struct AgentWorkspacePageTemplate {
    pub agent: AgentDetailRow,
    pub tabs: Vec<AgentShowTabLink>,
    pub agent_tabs_use_htmx: bool,
    pub navbar: Navbar,
    pub current_path: String,
    pub entries: Vec<WorkspaceTreeEntryView>,
    pub workspace_exists: bool,
    pub listing_truncated: bool,
    pub max_entries: usize,
    pub max_depth: usize,
    pub controller_unavailable: bool,
    pub selected_path: String,
    pub preview_text: Option<String>,
    pub preview_missing: bool,
    pub preview_binary: bool,
    pub preview_too_large: bool,
}
