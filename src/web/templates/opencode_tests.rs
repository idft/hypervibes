use crate::agentic::model::{AgentMaintenanceTaskRow, MAINTENANCE_STATUS_SUCCEEDED};

use super::*;

#[test]
fn workspace_maintenance_partial_hides_succeeded_state() {
    let rendered = OpenCodeWorkspaceMaintenanceStatusTemplate::render_view(
        OpenCodeWorkspaceMaintenanceView::from_task(
            "test-agent",
            AgentMaintenanceTaskRow {
                id: 42,
                agent_key: "test-agent".to_string(),
                hard_reset: false,
                status: MAINTENANCE_STATUS_SUCCEEDED.to_string(),
                error_summary: None,
            },
        ),
    )
    .unwrap();

    assert!(rendered.trim().is_empty());
}
