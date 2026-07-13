use askama::Template;

use crate::agentic::model::{
    AgentMaintenanceTaskRow, MAINTENANCE_STATUS_ABORTED, MAINTENANCE_STATUS_FAILED,
    MAINTENANCE_STATUS_QUEUED, MAINTENANCE_STATUS_RUNNING, MAINTENANCE_STATUS_SUCCEEDED,
};

#[derive(Debug, Clone)]
pub struct OpenCodeWorkspaceSettingsView {
    pub workspace_host_path: String,
    pub workspace_container_path: String,
    pub profile_source: String,
    pub env_exists: bool,
    pub template_drift: OpenCodeWorkspaceTemplateDriftView,
    pub maintenance_html: String,
    pub maintenance: OpenCodeWorkspaceMaintenanceView,
}

#[derive(Debug, Clone)]
pub struct OpenCodeWorkspaceMaintenanceView {
    pub poll_url: String,
    pub should_poll: bool,
    pub status: Option<OpenCodeWorkspaceMaintenanceStatusView>,
}

impl OpenCodeWorkspaceMaintenanceView {
    pub fn is_visible(&self) -> bool {
        self.status
            .as_ref()
            .map(|status| status.is_visible())
            .unwrap_or(false)
    }

    pub fn idle(agent_key: &str) -> Self {
        Self {
            poll_url: format!("/agents/{agent_key}/settings/workspace-maintenance-status"),
            should_poll: false,
            status: None,
        }
    }

    pub fn from_task(agent_key: &str, task: AgentMaintenanceTaskRow) -> Self {
        let status = OpenCodeWorkspaceMaintenanceStatusView::from_task(task);
        Self {
            poll_url: format!("/agents/{agent_key}/settings/workspace-maintenance-status"),
            should_poll: status.should_poll,
            status: Some(status),
        }
    }
}

#[derive(Debug, Clone)]
pub struct OpenCodeWorkspaceMaintenanceStatusView {
    pub task_id: i64,
    pub hard_reset: bool,
    pub reset_memories: bool,
    pub status_label: String,
    pub status_class: String,
    pub detail_text: String,
    pub is_visible: bool,
    pub should_poll: bool,
    pub show_spinner: bool,
    pub error_text: Option<String>,
}

impl OpenCodeWorkspaceMaintenanceStatusView {
    pub fn is_visible(&self) -> bool {
        self.is_visible
    }

    pub fn from_task(task: AgentMaintenanceTaskRow) -> Self {
        let hard_reset = task.parameter_bool("hard_reset");
        let reset_memories = task.parameter_bool("reset_memories");
        let (status_label, status_class, detail_text, is_visible, should_poll, show_spinner) =
            match task.status.as_str() {
                MAINTENANCE_STATUS_QUEUED => (
                    "Queued".to_string(),
                    "border-amber-900/60 bg-amber-950/30 text-amber-300".to_string(),
                    "Waiting for active jobs and sessions to finish".to_string(),
                    true,
                    true,
                    true,
                ),
                MAINTENANCE_STATUS_RUNNING if hard_reset => (
                    "Running".to_string(),
                    "border-sky-900/60 bg-sky-950/30 text-sky-300".to_string(),
                    "Hard-resetting workspace".to_string(),
                    true,
                    true,
                    true,
                ),
                MAINTENANCE_STATUS_RUNNING => (
                    "Running".to_string(),
                    "border-sky-900/60 bg-sky-950/30 text-sky-300".to_string(),
                    "Re-generating workspace".to_string(),
                    true,
                    true,
                    true,
                ),
                MAINTENANCE_STATUS_SUCCEEDED => (
                    "Succeeded".to_string(),
                    "border-emerald-900/60 bg-emerald-950/30 text-emerald-300".to_string(),
                    "Workspace maintenance completed".to_string(),
                    false,
                    false,
                    false,
                ),
                MAINTENANCE_STATUS_FAILED => (
                    "Failed".to_string(),
                    "border-red-900/60 bg-red-950/30 text-red-300".to_string(),
                    "Workspace maintenance failed".to_string(),
                    true,
                    false,
                    false,
                ),
                MAINTENANCE_STATUS_ABORTED => (
                    "Aborted".to_string(),
                    "border-amber-900/60 bg-amber-950/30 text-amber-300".to_string(),
                    "Workspace maintenance was aborted".to_string(),
                    true,
                    false,
                    false,
                ),
                other => (
                    other.to_string(),
                    "border-zinc-700 bg-zinc-900/60 text-zinc-300".to_string(),
                    "Workspace maintenance status is unknown".to_string(),
                    true,
                    false,
                    false,
                ),
            };

        Self {
            task_id: task.id,
            hard_reset,
            reset_memories,
            status_label,
            status_class,
            detail_text,
            is_visible,
            should_poll,
            show_spinner,
            error_text: task.error_summary,
        }
    }
}

#[derive(Debug, Clone)]
pub struct OpenCodeWorkspaceTemplateDriftView {
    pub status_text: &'static str,
    pub status_class: &'static str,
    pub changed_files: Vec<OpenCodeWorkspaceTemplateFileChangeView>,
    pub is_missing: bool,
}

impl OpenCodeWorkspaceTemplateDriftView {
    pub fn from_diff(diff: crate::opencode::workspace::WorkspaceTemplateDrift) -> Self {
        if !diff.workspace_exists {
            return Self {
                status_text: "Workspace missing",
                status_class: "border-amber-900/60 bg-amber-950/30 text-amber-300",
                changed_files: Vec::new(),
                is_missing: true,
            };
        }

        if diff.is_in_sync() {
            return Self {
                status_text: "In sync",
                status_class: "border-emerald-900/60 bg-emerald-950/30 text-emerald-300",
                changed_files: Vec::new(),
                is_missing: false,
            };
        }

        Self {
            status_text: "Template drift",
            status_class: "border-amber-900/60 bg-amber-950/30 text-amber-300",
            changed_files: diff
                .changed_files
                .into_iter()
                .map(OpenCodeWorkspaceTemplateFileChangeView::from_change)
                .collect(),
            is_missing: false,
        }
    }

    pub fn unavailable() -> Self {
        Self {
            status_text: "Diff unavailable",
            status_class: "border-zinc-800 bg-zinc-950/70 text-zinc-300",
            changed_files: Vec::new(),
            is_missing: false,
        }
    }

    pub fn has_changes(&self) -> bool {
        !self.changed_files.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct OpenCodeWorkspaceTemplateFileChangeView {
    pub status_code: &'static str,
    pub path: String,
    pub added_lines: usize,
    pub removed_lines: usize,
}

impl OpenCodeWorkspaceTemplateFileChangeView {
    fn from_change(change: crate::opencode::workspace::WorkspaceTemplateFileChange) -> Self {
        Self {
            status_code: match change.status {
                crate::opencode::workspace::WorkspaceTemplateFileStatus::Modified => "M",
                crate::opencode::workspace::WorkspaceTemplateFileStatus::Deleted => "D",
            },
            path: change.path,
            added_lines: change.added_lines,
            removed_lines: change.removed_lines,
        }
    }
}

#[derive(Template)]
#[template(path = "agent_workspace_maintenance_status.html")]
pub struct OpenCodeWorkspaceMaintenanceStatusTemplate {
    pub maintenance: OpenCodeWorkspaceMaintenanceView,
}

impl OpenCodeWorkspaceMaintenanceStatusTemplate {
    pub fn render_view(
        maintenance: OpenCodeWorkspaceMaintenanceView,
    ) -> Result<String, askama::Error> {
        Self { maintenance }.render()
    }
}

#[derive(Template)]
#[template(path = "agent_workspace_section.html")]
pub struct OpenCodeWorkspaceSectionTemplate {
    pub opencode_workspace: Option<OpenCodeWorkspaceSettingsView>,
    pub settings_workspace_warning: Option<String>,
}

impl OpenCodeWorkspaceSectionTemplate {
    pub fn render_view(
        opencode_workspace: Option<OpenCodeWorkspaceSettingsView>,
        settings_workspace_warning: Option<String>,
    ) -> Result<String, askama::Error> {
        Self {
            opencode_workspace,
            settings_workspace_warning,
        }
        .render()
    }
}
