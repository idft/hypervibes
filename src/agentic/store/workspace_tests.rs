use chrono::Utc;

use crate::{agentic::model::MAINTENANCE_STATUS_QUEUED, test_db};
use uuid::Uuid;

use super::test_support::seed_agent_and_schedule;
use super::{
    CodingTriggerMode, InsertAnalysisCodingTaskOutcome,
    InsertWorkspaceMaintenanceTaskOutcome, agent_has_blocking_workspace_maintenance,
    get_latest_workspace_regenerate_task, insert_analysis_coding_task_and_run,
    insert_workspace_regenerate_task, mark_maintenance_task_running,
    mark_maintenance_task_succeeded,
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
async fn coding_queue_requires_model_and_deduplicates_source_memory() {
    let pool = test_db::pool().await;
    let key = format!(
        "coding-queue-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let _schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;
    sqlx::query(
        "INSERT INTO agentic_job_hooks
            (agent_key, job_key, job_kind, hook_event, enabled, timeout_seconds, operator_prompt)
         VALUES ($1, 'analysis-coding', 'analysis_coding', 'daily_review_completed', true, 1800, '')
         ON CONFLICT (agent_key, job_kind, hook_event) DO NOTHING",
    )
    .bind(&key)
    .execute(&pool)
    .await
    .expect("seed coding hook");
    let hook_id: (i64,) = sqlx::query_as(
        "SELECT id FROM agentic_job_hooks WHERE agent_key = $1 AND job_kind = 'analysis_coding'",
    )
    .bind(&key)
    .fetch_one(&pool)
    .await
    .expect("load coding hook");
    assert!(
        insert_analysis_coding_task_and_run(
            &pool,
            &key,
            hook_id.0,
            CodingTriggerMode::Automatic,
            None,
            None,
            None,
            Some("auto")
        )
        .await
        .is_err()
    );
    sqlx::query("UPDATE agentic_job_hooks SET enabled = true, model_provider_id = 'test', model_id = 'strong' WHERE id = $1")
        .bind(hook_id.0)
        .execute(&pool)
        .await
        .expect("set model");
    let source_memory = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO memory.records (id, agent_key, symbol, memory_type, summary, content, metadata)
         VALUES ($1, $2, '__agent__', 'daily_review', 'review', 'review', '{}'::jsonb)",
    )
    .bind(source_memory)
    .bind(&key)
    .execute(&pool)
    .await
    .expect("seed source memory");
    let inserted = insert_analysis_coding_task_and_run(
        &pool,
        &key,
        hook_id.0,
        CodingTriggerMode::Automatic,
        None,
        Some(source_memory),
        Some("review"),
        Some("auto"),
    )
    .await
    .expect("queue coding task");
    assert!(matches!(
        inserted,
        InsertAnalysisCodingTaskOutcome::Inserted { .. }
    ));
    let duplicate = insert_analysis_coding_task_and_run(
        &pool,
        &key,
        hook_id.0,
        CodingTriggerMode::Automatic,
        None,
        Some(source_memory),
        Some("review"),
        Some("auto"),
    )
    .await;
    assert!(matches!(
        duplicate,
        Ok(InsertAnalysisCodingTaskOutcome::AlreadyQueued)
    ));
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
