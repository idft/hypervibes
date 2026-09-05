use chrono::Utc;

use crate::{
    harness::model::{MAINTENANCE_STATUS_FAILED, MAINTENANCE_STATUS_QUEUED},
    test_db,
};

use super::{
    InsertGlobalMaintenanceTaskOutcome, get_latest_provider_config_reload_task,
    get_next_queued_provider_config_reload_task, insert_provider_config_reload_task,
    mark_maintenance_task_failed, mark_maintenance_task_running, mark_maintenance_task_succeeded,
    requeue_stale_provider_config_reload_tasks,
};

#[tokio::test]
async fn mark_maintenance_task_failed_preserves_error_summary() {
    let pool = test_db::pool().await;
    let task_id = match insert_provider_config_reload_task(&pool)
        .await
        .expect("insert provider reload task")
    {
        InsertGlobalMaintenanceTaskOutcome::Inserted { task_id } => task_id,
        other => panic!("expected Inserted, got {other:?}"),
    };

    mark_maintenance_task_failed(&pool, task_id, "OpenCode command timed out")
        .await
        .expect("mark failed");

    let task = get_latest_provider_config_reload_task(&pool)
        .await
        .expect("load failed maintenance task")
        .expect("provider reload task present");
    assert_eq!(task.status, MAINTENANCE_STATUS_FAILED);
    assert_eq!(
        task.error_summary.as_deref(),
        Some("OpenCode command timed out")
    );
}

#[tokio::test]
async fn stale_running_provider_reload_is_requeued() {
    let pool = test_db::pool().await;
    let task_id = match insert_provider_config_reload_task(&pool)
        .await
        .expect("insert provider reload task")
    {
        InsertGlobalMaintenanceTaskOutcome::Inserted { task_id } => task_id,
        other => panic!("expected Inserted, got {other:?}"),
    };
    mark_maintenance_task_running(&pool, task_id)
        .await
        .expect("mark provider reload running");
    sqlx::query(
        "UPDATE harness_maintenance_tasks
            SET heartbeat_at = now() - interval '3 minutes'
          WHERE id = $1",
    )
    .bind(task_id)
    .execute(&pool)
    .await
    .expect("age provider reload task");

    let requeued = requeue_stale_provider_config_reload_tasks(
        &pool,
        Utc::now() - chrono::Duration::minutes(2),
    )
    .await
    .expect("requeue stale provider reload task");
    assert_eq!(requeued, 1);

    let task = get_latest_provider_config_reload_task(&pool)
        .await
        .expect("load provider reload task")
        .expect("provider reload task present");
    assert_eq!(task.id, task_id);
    assert_eq!(task.status, MAINTENANCE_STATUS_QUEUED);
}

#[tokio::test]
async fn queued_provider_reload_task_is_claimed_in_order() {
    let pool = test_db::pool().await;
    let first = match insert_provider_config_reload_task(&pool)
        .await
        .expect("insert reload task")
    {
        InsertGlobalMaintenanceTaskOutcome::Inserted { task_id } => task_id,
        other => panic!("expected Inserted, got {other:?}"),
    };
    mark_maintenance_task_succeeded(&pool, first)
        .await
        .expect("mark reload succeeded");
    let second = match insert_provider_config_reload_task(&pool)
        .await
        .expect("insert second reload task")
    {
        InsertGlobalMaintenanceTaskOutcome::Inserted { task_id } => task_id,
        other => panic!("expected Inserted, got {other:?}"),
    };
    assert!(second > first);

    let claimed = get_next_queued_provider_config_reload_task(&pool)
        .await
        .expect("claim reload task")
        .expect("queued reload task present");
    assert_eq!(claimed.id, second);
}
