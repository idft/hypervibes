use chrono::{DateTime, Utc};
use sqlx::{query, query_as};

use crate::{
    agentic::model::{
        JOB_KIND_MARKET_ANALYSIS, RUN_STATUS_ABORTED, RUN_STATUS_FAILED, RUN_STATUS_QUEUED,
        RUN_STATUS_RUNNING, RUN_STATUS_SUCCEEDED,
    },
    agentic::timeframe::{
        DEFAULT_TRIGGER_DELAY_SECONDS, boundary_for_due_at, latest_due_at_or_before,
    },
    agents::store::insert_agent,
    test_db,
};

use super::common::ERROR_SUMMARY_MAX_CHARS;
use super::test_support::{sample_agent, seed_agent_and_schedule};
use super::{
    QueuedHookRun, QueuedScheduleRun, count_agent_runs, get_run, insert_agent_hook,
    insert_queued_hook_run, insert_queued_hook_run_for_automatic_dispatch, insert_queued_run,
    insert_test_run, insert_workspace_regenerate_task, list_agent_hooks, list_agent_schedules,
    mark_run_aborted, mark_run_failed, mark_run_running, mark_run_succeeded,
};

#[tokio::test]
async fn insert_queued_hook_run_inserts_manual_dispatch_run_without_timeframe() {
    let pool = test_db::pool().await;
    let key = format!("hook-run-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0));
    insert_agent(&pool, &sample_agent(&key))
        .await
        .expect("insert agent");
    let hook_id = insert_agent_hook(
        &pool,
        &key,
        JOB_KIND_MARKET_ANALYSIS,
        crate::agentic::model::HOOK_EVENT_ANALYSIS_BATCH_COMPLETED,
        true,
        None,
        None,
        600,
        "",
    )
    .await
    .expect("insert hook");

    let outcome = insert_queued_hook_run(&pool, &key, hook_id)
        .await
        .expect("insert queued hook run");
    let run_id = match outcome {
        QueuedHookRun::Dispatch { run_id, .. } => run_id,
        other => panic!("expected Dispatch, got {other:?}"),
    };

    let run = get_run(&pool, run_id)
        .await
        .expect("get run")
        .expect("run present");
    assert_eq!(run.schedule_id, None);
    assert_eq!(run.hook_id, Some(hook_id));
    assert_eq!(run.job_key, "market-analysis");
    assert_eq!(run.timeframe, None);
}

#[tokio::test]
async fn agentic_runs_exactly_one_source_check_rejects_both_and_neither_sources() {
    let pool = test_db::pool().await;
    let key = format!(
        "run-source-check-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;
    let schedule = list_agent_schedules(&pool, &key)
        .await
        .expect("list schedules")
        .into_iter()
        .find(|row| row.id == schedule_id)
        .expect("schedule present");
    let hook_id = list_agent_hooks(&pool, &key)
        .await
        .expect("list hooks")
        .first()
        .expect("default hook present")
        .id;

    let both_sources = query(
        "INSERT INTO agentic_runs (
            schedule_id,
            hook_id,
            agent_key,
            job_key,
            job_kind,
            timeframe,
            status,
            scheduled_for,
            timeout_seconds
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(schedule_id)
    .bind(hook_id)
    .bind(&key)
    .bind(&schedule.job_key)
    .bind(&schedule.job_kind)
    .bind(&schedule.timeframe)
    .bind(RUN_STATUS_QUEUED)
    .bind(Utc::now())
    .bind(schedule.timeout_seconds)
    .execute(&pool)
    .await;
    assert!(both_sources.is_err(), "expected both-source insert to fail");

    let neither_source = query(
        "INSERT INTO agentic_runs (
            schedule_id,
            hook_id,
            agent_key,
            job_key,
            job_kind,
            timeframe,
            status,
            scheduled_for,
            timeout_seconds
         ) VALUES (NULL, NULL, $1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(&key)
    .bind(&schedule.job_key)
    .bind(&schedule.job_kind)
    .bind(&schedule.timeframe)
    .bind(RUN_STATUS_QUEUED)
    .bind(Utc::now())
    .bind(schedule.timeout_seconds)
    .execute(&pool)
    .await;
    assert!(
        neither_source.is_err(),
        "expected neither-source insert to fail"
    );
}

#[tokio::test]
async fn insert_queued_run_returns_blocked_by_maintenance() {
    let pool = test_db::pool().await;
    let key = format!(
        "manual-maint-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;
    insert_workspace_regenerate_task(&pool, &key, false, false)
        .await
        .expect("insert maintenance task");

    let outcome = insert_queued_run(&pool, &key, schedule_id)
        .await
        .expect("manual run");
    assert!(matches!(outcome, QueuedScheduleRun::BlockedByMaintenance));
    assert_eq!(count_agent_runs(&pool, &key).await.expect("count runs"), 0);
}

#[tokio::test]
async fn insert_queued_hook_run_returns_blocked_by_maintenance() {
    let pool = test_db::pool().await;
    let key = format!(
        "hook-maint-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let _schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;
    let hook_id = list_agent_hooks(&pool, &key)
        .await
        .expect("list hooks")
        .first()
        .expect("default hook present")
        .id;
    insert_workspace_regenerate_task(&pool, &key, false, false)
        .await
        .expect("insert maintenance task");

    let outcome = insert_queued_hook_run(&pool, &key, hook_id)
        .await
        .expect("manual hook run");
    assert!(matches!(outcome, QueuedHookRun::BlockedByMaintenance));
    assert_eq!(count_agent_runs(&pool, &key).await.expect("count runs"), 0);
}

#[tokio::test]
async fn automatic_hook_insert_still_dispatches_during_maintenance() {
    let pool = test_db::pool().await;
    let key = format!(
        "hook-auto-maint-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let _schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;
    let hook_id = list_agent_hooks(&pool, &key)
        .await
        .expect("list hooks")
        .first()
        .expect("default hook present")
        .id;
    insert_workspace_regenerate_task(&pool, &key, false, false)
        .await
        .expect("insert maintenance task");

    let outcome = insert_queued_hook_run_for_automatic_dispatch(&pool, &key, hook_id)
        .await
        .expect("automatic hook insert");
    let run_id = match outcome {
        QueuedHookRun::Dispatch { run_id, .. } => run_id,
        other => panic!("expected Dispatch, got {other:?}"),
    };

    let run = get_run(&pool, run_id)
        .await
        .expect("get run")
        .expect("run present");
    assert_eq!(run.hook_id, Some(hook_id));
    assert_eq!(run.status, RUN_STATUS_QUEUED);
}

#[tokio::test]
async fn insert_queued_run_inserts_manual_dispatch_run_and_does_not_advance_schedule() {
    let pool = test_db::pool().await;
    let key = format!("manual-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0));
    let schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;

    let (before_next_run_at,): (DateTime<Utc>,) =
        query_as("SELECT next_run_at FROM agentic_job_schedules WHERE id = $1")
            .bind(schedule_id)
            .fetch_one(&pool)
            .await
            .expect("fetch before next_run_at");

    let outcome = insert_queued_run(&pool, &key, schedule_id)
        .await
        .expect("manual run");
    let (run_id, scheduled_for) = match outcome {
        QueuedScheduleRun::Dispatch {
            run_id,
            scheduled_for,
        } => (run_id, scheduled_for),
        other => panic!("expected Dispatch, got {other:?}"),
    };
    let after = Utc::now();

    let run = get_run(&pool, run_id)
        .await
        .expect("fetch run")
        .expect("run present");
    assert_eq!(run.status, RUN_STATUS_QUEUED);
    assert_eq!(
        run.timeframe.as_deref(),
        Some(super::schedules::DEFAULT_ANALYSIS_TIMEFRAME)
    );
    let expected_scheduled_for = latest_due_at_or_before(
        after,
        super::schedules::DEFAULT_ANALYSIS_TIMEFRAME,
        DEFAULT_TRIGGER_DELAY_SECONDS,
    )
    .expect("compute latest due")
    .map(|due| boundary_for_due_at(due, DEFAULT_TRIGGER_DELAY_SECONDS))
    .expect("expected a closed 15m boundary");
    assert_eq!(run.scheduled_for, expected_scheduled_for);
    let delta = (run.scheduled_for - scheduled_for)
        .num_microseconds()
        .unwrap_or(i64::MAX);
    assert!(delta.abs() <= 1, "scheduled_for delta too large: {delta}us");

    let (after_next_run_at,): (DateTime<Utc>,) =
        query_as("SELECT next_run_at FROM agentic_job_schedules WHERE id = $1")
            .bind(schedule_id)
            .fetch_one(&pool)
            .await
            .expect("fetch after next_run_at");
    assert_eq!(
        before_next_run_at, after_next_run_at,
        "manual run must not mutate next_run_at"
    );
}

#[tokio::test]
async fn insert_queued_run_inserts_skipped_run_when_previous_run_is_active() {
    let pool = test_db::pool().await;
    let key = format!(
        "manual-skip-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;
    insert_test_run(&pool, schedule_id, RUN_STATUS_RUNNING)
        .await
        .expect("seed active run");

    let outcome = insert_queued_run(&pool, &key, schedule_id)
        .await
        .expect("manual run");
    match outcome {
        QueuedScheduleRun::Skipped => {}
        other => panic!("expected Skipped, got {other:?}"),
    }
}

#[tokio::test]
async fn mark_run_running_sets_status_and_started_at() {
    let pool = test_db::pool().await;
    let key = format!(
        "run-running-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;
    let run_id = insert_test_run(&pool, schedule_id, RUN_STATUS_QUEUED)
        .await
        .expect("seed run");

    let updated = mark_run_running(&pool, run_id, Some("ses_abc"))
        .await
        .expect("mark running");
    assert!(updated);

    let run = get_run(&pool, run_id)
        .await
        .expect("fetch run")
        .expect("run present");
    assert_eq!(run.status, RUN_STATUS_RUNNING);
    assert!(run.started_at.is_some());
    assert_eq!(run.backend_run_ref.as_deref(), Some("ses_abc"));
}

#[tokio::test]
async fn mark_run_succeeded_records_finished_at_and_backend_ref() {
    let pool = test_db::pool().await;
    let key = format!("run-ok-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0));
    let schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;
    let run_id = insert_test_run(&pool, schedule_id, RUN_STATUS_RUNNING)
        .await
        .expect("seed run");

    let updated = mark_run_succeeded(&pool, run_id, Some("ses_xyz"))
        .await
        .expect("mark succeeded");
    assert!(updated);

    let run = get_run(&pool, run_id)
        .await
        .expect("fetch run")
        .expect("run present");
    assert_eq!(run.status, RUN_STATUS_SUCCEEDED);
    assert!(run.finished_at.is_some());
    assert_eq!(run.backend_run_ref.as_deref(), Some("ses_xyz"));
}

#[tokio::test]
async fn mark_run_failed_truncates_error_summary() {
    let pool = test_db::pool().await;
    let key = format!("run-fail-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0));
    let schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;
    let run_id = insert_test_run(&pool, schedule_id, RUN_STATUS_RUNNING)
        .await
        .expect("seed run");

    let huge = "x".repeat(ERROR_SUMMARY_MAX_CHARS + 100);
    let updated = mark_run_failed(&pool, run_id, &huge, Some("ses_zzz"))
        .await
        .expect("mark failed");
    assert!(updated);

    let run = get_run(&pool, run_id)
        .await
        .expect("fetch run")
        .expect("run present");
    assert_eq!(run.status, RUN_STATUS_FAILED);
    assert!(run.finished_at.is_some());
    let summary = run.error_summary.expect("error summary");
    assert!(summary.chars().count() <= ERROR_SUMMARY_MAX_CHARS + 1);
    assert!(summary.ends_with('…'));
}

#[tokio::test]
async fn mark_run_failed_preserves_existing_backend_ref_when_not_provided() {
    let pool = test_db::pool().await;
    let key = format!(
        "run-fail-preserve-ref-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;
    let run_id = insert_test_run(&pool, schedule_id, RUN_STATUS_QUEUED)
        .await
        .expect("seed run");

    mark_run_running(&pool, run_id, Some("ses_keep"))
        .await
        .expect("mark running");
    mark_run_failed(&pool, run_id, "boom", None)
        .await
        .expect("mark failed");

    let run = get_run(&pool, run_id)
        .await
        .expect("fetch run")
        .expect("run present");
    assert_eq!(run.status, RUN_STATUS_FAILED);
    assert_eq!(run.backend_run_ref.as_deref(), Some("ses_keep"));
}

#[tokio::test]
async fn mark_run_aborted_marks_status_finished_at() {
    let pool = test_db::pool().await;
    let key = format!(
        "run-abort-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;
    let run_id = insert_test_run(&pool, schedule_id, RUN_STATUS_RUNNING)
        .await
        .expect("seed run");

    let updated = mark_run_aborted(&pool, run_id, "timeout", None)
        .await
        .expect("mark aborted");
    assert!(updated);

    let run = get_run(&pool, run_id)
        .await
        .expect("fetch run")
        .expect("run present");
    assert_eq!(run.status, RUN_STATUS_ABORTED);
    assert!(run.finished_at.is_some());
}
