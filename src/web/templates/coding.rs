use askama::Template;

use crate::{
    agents::model::AgentDetailRow,
    harness::model::{
        AgentMaintenanceTaskRow, MAINTENANCE_STATUS_ABORTED, MAINTENANCE_STATUS_FAILED,
        MAINTENANCE_STATUS_QUEUED, MAINTENANCE_STATUS_RUNNING, MAINTENANCE_STATUS_SUCCEEDED,
    },
    web::templates::{AgentShowTabLink, Navbar},
};

#[derive(Debug, Clone)]
pub struct CodingTreeEntryView {
    pub path: String,
    pub name: String,
    pub href: String,
    pub depth: usize,
    pub is_directory: bool,
    pub selected: bool,
    pub initially_hidden: bool,
}

/// Status of the durable Coding package shown on the Coding page.
#[derive(Debug, Clone)]
pub enum CodingPackageStatusView {
    /// No package has been promoted yet.
    Missing,
    /// A valid manifest-backed package.
    Valid {
        version: String,
        manifest_hash: String,
    },
    /// The package exists but could not be inspected safely.
    Invalid { message: String },
}

#[derive(Template)]
#[template(path = "agents/coding/page.html")]
pub struct AgentCodingPageTemplate {
    pub agent: AgentDetailRow,
    pub tabs: Vec<AgentShowTabLink>,
    pub agent_tabs_use_htmx: bool,
    pub navbar: Navbar,
    pub current_path: String,
    pub entries: Vec<CodingTreeEntryView>,
    pub package_exists: bool,
    pub package_status: CodingPackageStatusView,
    pub listing_truncated: bool,
    pub max_entries: usize,
    pub max_depth: usize,
    pub controller_unavailable: bool,
    pub selected_path: String,
    pub preview_text: Option<String>,
    pub preview_missing: bool,
    pub preview_binary: bool,
    pub preview_too_large: bool,
    /// Rendered latest analysis-coding task status partial.
    pub task_status_html: String,
}

#[derive(Debug, Clone)]
pub struct CodingTaskStatusView {
    pub task_id: i64,
    pub status_label: String,
    pub status_class: String,
    pub detail_text: String,
    pub should_poll: bool,
    pub show_spinner: bool,
    pub error_text: Option<String>,
    pub phase: String,
    pub report_summary: Option<String>,
    pub changed_paths: Vec<String>,
}

impl CodingTaskStatusView {
    /// Render the latest analysis-coding task for the Coding page. Only
    /// `analysis_coding` tasks are accepted; a missing task renders nothing.
    pub fn from_task(
        task: Option<AgentMaintenanceTaskRow>,
        report_summary: Option<String>,
        changed_paths: Vec<String>,
    ) -> Option<Self> {
        let task = task?;
        let phase = task.phase.clone();
        let (status_label, status_class, detail_text, should_poll, show_spinner) =
            match task.status.as_str() {
                MAINTENANCE_STATUS_QUEUED | MAINTENANCE_STATUS_RUNNING => {
                    let running = task.status == MAINTENANCE_STATUS_RUNNING;
                    (
                        format!("Coding {}", task.phase.replace('_', " ")),
                        "border-sky-900/60 bg-sky-950/30 text-sky-300".to_string(),
                        "The analysis-coding candidate workspace is being processed".to_string(),
                        true,
                        running,
                    )
                }
                MAINTENANCE_STATUS_SUCCEEDED => (
                    "Coding succeeded".to_string(),
                    "border-emerald-900/60 bg-emerald-950/30 text-emerald-300".to_string(),
                    "Candidate validation and promotion completed".to_string(),
                    false,
                    false,
                ),
                MAINTENANCE_STATUS_FAILED => (
                    "Coding failed".to_string(),
                    "border-red-900/60 bg-red-950/30 text-red-300".to_string(),
                    format!("Coding task failed during {}", task.phase),
                    false,
                    false,
                ),
                MAINTENANCE_STATUS_ABORTED => (
                    "Coding aborted".to_string(),
                    "border-amber-900/60 bg-amber-950/30 text-amber-300".to_string(),
                    "Coding task was aborted".to_string(),
                    false,
                    false,
                ),
                other => (
                    other.to_string(),
                    "border-zinc-700 bg-zinc-900/60 text-zinc-300".to_string(),
                    "Coding task status is unknown".to_string(),
                    false,
                    false,
                ),
            };
        Some(Self {
            task_id: task.id,
            status_label,
            status_class,
            detail_text,
            should_poll,
            show_spinner,
            error_text: task.error_summary,
            phase,
            report_summary,
            changed_paths,
        })
    }
}

#[derive(Template)]
#[template(path = "agents/components/coding-task-status.html")]
pub struct CodingTaskStatusPartialTemplate {
    pub task: Option<CodingTaskStatusView>,
    pub poll_url: String,
}

impl CodingTaskStatusPartialTemplate {
    pub fn render_view(
        task: Option<CodingTaskStatusView>,
        poll_url: String,
    ) -> Result<String, askama::Error> {
        Self { task, poll_url }.render()
    }
}
