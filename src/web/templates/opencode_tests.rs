use crate::agentic::model::{
    AgentMaintenanceTaskRow, MAINTENANCE_PHASE_COMPLETED, MAINTENANCE_PHASE_GENERATING,
    MAINTENANCE_STATUS_RUNNING, MAINTENANCE_STATUS_SUCCEEDED,
    MAINTENANCE_TASK_KIND_ANALYSIS_CODING, MAINTENANCE_TASK_KIND_WORKSPACE_REGENERATE,
};

use super::*;

#[test]
fn workspace_maintenance_partial_hides_succeeded_state() {
    let rendered = OpenCodeWorkspaceMaintenanceStatusTemplate::render_view(
        OpenCodeWorkspaceMaintenanceView::from_task(
            "test-agent",
            AgentMaintenanceTaskRow {
                id: 42,
                agent_key: "test-agent".to_string(),
                task_kind: MAINTENANCE_TASK_KIND_WORKSPACE_REGENERATE.to_string(),
                parameters: serde_json::json!({"hard_reset": false}),
                status: MAINTENANCE_STATUS_SUCCEEDED.to_string(),
                phase: MAINTENANCE_PHASE_COMPLETED.to_string(),
                error_summary: None,
                run_id: None,
                source_run_id: None,
                source_memory_id: None,
            },
        ),
    )
    .unwrap();

    assert!(rendered.trim().is_empty());
}

#[test]
fn coding_maintenance_partial_shows_generation_phase() {
    let status = OpenCodeWorkspaceMaintenanceStatusView::from_task_with_report(
        AgentMaintenanceTaskRow {
            id: 43,
            agent_key: "test-agent".to_string(),
            task_kind: MAINTENANCE_TASK_KIND_ANALYSIS_CODING.to_string(),
            parameters: serde_json::json!({"mode": "bootstrap"}),
            status: MAINTENANCE_STATUS_RUNNING.to_string(),
            phase: MAINTENANCE_PHASE_GENERATING.to_string(),
            error_summary: None,
            run_id: Some(9),
            source_run_id: None,
            source_memory_id: None,
        },
        Some("Improved volatility regime analysis".to_string()),
        vec!["analyze.py".to_string()],
    );
    let view = OpenCodeWorkspaceMaintenanceView {
        poll_url: "/status".to_string(),
        should_poll: status.should_poll,
        status: Some(status),
    };
    assert!(view.is_visible());
    assert!(view.should_poll);
    let status = view.status.unwrap();
    assert!(status.status_label.contains("Coding"));
    assert_eq!(
        status.report_summary.as_deref(),
        Some("Improved volatility regime analysis")
    );
    assert_eq!(status.changed_paths, vec!["analyze.py"]);
}
