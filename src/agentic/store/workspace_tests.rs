use chrono::Utc;

use crate::{agentic::model::MAINTENANCE_STATUS_QUEUED, test_db};

use super::test_support::seed_agent_and_schedule;
use super::{
    InsertWorkspaceMaintenanceTaskOutcome, agent_has_blocking_workspace_maintenance,
    get_latest_workspace_regenerate_task, insert_workspace_regenerate_task,
    mark_maintenance_task_running, mark_maintenance_task_succeeded,
};

#[tokio::test]
async fn insert_workspace_regenerate_task_rejects_duplicate_active_task() {
    let pool = test_db::pool().await;
    let key = format!(
        "maintenance-dup-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let _schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;

    let first = insert_workspace_regenerate_task(&pool, &key, false, false)
        .await
        .expect("insert maintenance task");
    let task_id = match first {
        InsertWorkspaceMaintenanceTaskOutcome::Inserted { task_id } => task_id,
        other => panic!("expected Inserted, got {other:?}"),
    };

    let latest = get_latest_workspace_regenerate_task(&pool, &key)
        .await
        .expect("load latest task")
        .expect("task present");
    assert_eq!(latest.id, task_id);
    assert_eq!(latest.status, MAINTENANCE_STATUS_QUEUED);
    assert!(!latest.parameter_bool("hard_reset"));

    let duplicate = insert_workspace_regenerate_task(&pool, &key, true, false)
        .await
        .expect("insert duplicate maintenance task");
    assert_eq!(
        duplicate,
        InsertWorkspaceMaintenanceTaskOutcome::DuplicateActiveTask
    );
}

#[tokio::test]
async fn agent_has_blocking_workspace_maintenance_only_for_queued_or_running_tasks() {
    let pool = test_db::pool().await;
    let key = format!(
        "maintenance-state-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let _schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;

    assert!(
        !agent_has_blocking_workspace_maintenance(&pool, &key)
            .await
            .expect("initial maintenance state")
    );

    let task_id = match insert_workspace_regenerate_task(&pool, &key, false, false)
        .await
        .expect("insert maintenance task")
    {
        InsertWorkspaceMaintenanceTaskOutcome::Inserted { task_id } => task_id,
        other => panic!("expected Inserted, got {other:?}"),
    };
    assert!(
        agent_has_blocking_workspace_maintenance(&pool, &key)
            .await
            .expect("queued maintenance state")
    );

    mark_maintenance_task_running(&pool, task_id)
        .await
        .expect("mark running");
    assert!(
        agent_has_blocking_workspace_maintenance(&pool, &key)
            .await
            .expect("running maintenance state")
    );

    mark_maintenance_task_succeeded(&pool, task_id)
        .await
        .expect("mark succeeded");
    assert!(
        !agent_has_blocking_workspace_maintenance(&pool, &key)
            .await
            .expect("terminal maintenance state")
    );
}
