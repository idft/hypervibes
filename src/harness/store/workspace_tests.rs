use chrono::Utc;

use crate::{
    harness::model::{MAINTENANCE_STATUS_FAILED, MAINTENANCE_STATUS_QUEUED},
    test_db,
};
use uuid::Uuid;

use super::test_support::seed_agent_and_job;
use super::{
    AnalysisCodingTaskRequest, CodingTriggerMode, InsertAnalysisCodingTaskOutcome,
    InsertWorkspaceMaintenanceTaskOutcome, agent_has_blocking_workspace_maintenance,
    get_latest_provider_config_reload_task, get_latest_workspace_regenerate_task,
    insert_analysis_coding_task_and_run, insert_provider_config_reload_task,
    insert_workspace_regenerate_task, mark_maintenance_task_failed, mark_maintenance_task_running,
    mark_maintenance_task_succeeded, requeue_stale_provider_config_reload_tasks,
};

#[tokio::test]
async fn insert_workspace_regenerate_task_rejects_duplicate_active_task() {
    let pool = test_db::pool().await;
    let key = format!(
        "maintenance-dup-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let _job_id = seed_agent_and_job(&pool, &key, 0).await;

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
    let _job_id = seed_agent_and_job(&pool, &key, 0).await;
    sqlx::query(
        "INSERT INTO harness_jobs
            (agent_key, job_key, job_kind, trigger_type, enabled, timeout_seconds, operator_prompt)
         VALUES ($1, 'analysis-coding', 'analysis_coding', 'daily_review_completed', true, 1800, '')
         ON CONFLICT (agent_key, job_kind, trigger_type) DO NOTHING",
    )
    .bind(&key)
    .execute(&pool)
    .await
    .expect("seed coding event job");
    let job_id: (i64,) = sqlx::query_as(
        "SELECT id FROM harness_jobs WHERE agent_key = $1 AND job_kind = 'analysis_coding'",
    )
    .bind(&key)
    .fetch_one(&pool)
    .await
    .expect("load coding event job");
    assert!(
        insert_analysis_coding_task_and_run(
            &pool,
            AnalysisCodingTaskRequest {
                agent_key: &key,
                job_id: job_id.0,
                trigger_mode: CodingTriggerMode::Automatic,
                source_run_id: None,
                source_memory_id: None,
                operator_prompt: None,
                requested_mode: Some("auto"),
            },
        )
        .await
        .is_err()
    );
    sqlx::query("UPDATE harness_jobs SET enabled = true, model_provider_id = 'test', model_id = 'strong' WHERE id = $1")
        .bind(job_id.0)
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
        AnalysisCodingTaskRequest {
            agent_key: &key,
            job_id: job_id.0,
            trigger_mode: CodingTriggerMode::Automatic,
            source_run_id: None,
            source_memory_id: Some(source_memory),
            operator_prompt: Some("review"),
            requested_mode: Some("auto"),
        },
    )
    .await
    .expect("queue coding task");
    assert!(matches!(
        inserted,
        InsertAnalysisCodingTaskOutcome::Inserted { .. }
    ));
    let blocked = insert_analysis_coding_task_and_run(
        &pool,
        AnalysisCodingTaskRequest {
            agent_key: &key,
            job_id: job_id.0,
            trigger_mode: CodingTriggerMode::Automatic,
            source_run_id: None,
            source_memory_id: None,
            operator_prompt: Some("review"),
            requested_mode: Some("auto"),
        },
    )
    .await
    .expect("blocked coding task outcome");
    assert_eq!(
        blocked,
        InsertAnalysisCodingTaskOutcome::BlockedByMaintenance
    );
    let duplicate = insert_analysis_coding_task_and_run(
        &pool,
        AnalysisCodingTaskRequest {
            agent_key: &key,
            job_id: job_id.0,
            trigger_mode: CodingTriggerMode::Automatic,
            source_run_id: None,
            source_memory_id: Some(source_memory),
            operator_prompt: Some("review"),
            requested_mode: Some("auto"),
        },
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
    let _job_id = seed_agent_and_job(&pool, &key, 0).await;

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

#[tokio::test]
async fn mark_maintenance_task_failed_preserves_error_summary() {
    let pool = test_db::pool().await;
    let key = format!(
        "maintenance-failure-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let _job_id = seed_agent_and_job(&pool, &key, 0).await;
    let task_id = match insert_workspace_regenerate_task(&pool, &key, false, false)
        .await
        .expect("insert maintenance task")
    {
        InsertWorkspaceMaintenanceTaskOutcome::Inserted { task_id } => task_id,
        other => panic!("expected Inserted, got {other:?}"),
    };

    mark_maintenance_task_failed(&pool, task_id, "OpenCode command timed out")
        .await
        .expect("mark failed");

    let (status, error_summary): (String, Option<String>) =
        sqlx::query_as("SELECT status, error_summary FROM harness_maintenance_tasks WHERE id = $1")
            .bind(task_id)
            .fetch_one(&pool)
            .await
            .expect("load failed maintenance task");
    assert_eq!(status, MAINTENANCE_STATUS_FAILED);
    assert_eq!(error_summary.as_deref(), Some("OpenCode command timed out"));
}

#[tokio::test]
async fn stale_running_provider_reload_is_requeued() {
    let pool = test_db::pool().await;
    let task_id = match insert_provider_config_reload_task(&pool)
        .await
        .expect("insert provider reload task")
    {
        super::InsertGlobalMaintenanceTaskOutcome::Inserted { task_id } => task_id,
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
