use chrono::{DateTime, Utc};
use sqlx::query_as;

use crate::{
    harness::model::{
        RUN_STATUS_ABORTED, RUN_STATUS_FAILED, RUN_STATUS_QUEUED, RUN_STATUS_RUNNING,
        RUN_STATUS_SUCCEEDED,
    },
    harness::timeframe::{
        DEFAULT_TRIGGER_DELAY_SECONDS, boundary_for_due_at, latest_due_at_or_before,
    },
    test_db,
};

use super::common::ERROR_SUMMARY_MAX_CHARS;
use super::test_support::seed_agent_and_job;
use super::{
    QueuedSubAgentRun, count_agent_runs, get_run, insert_queued_manual_run, insert_test_run,
    insert_workspace_regenerate_task, mark_run_aborted, mark_run_failed, mark_run_running,
    mark_run_succeeded, set_run_error_summary,
};

#[tokio::test]
async fn insert_queued_manual_run_returns_blocked_by_maintenance() {
    let pool = test_db::pool().await;
    let key = format!(
        "manual-maint-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let sub_agent_id = seed_agent_and_job(&pool, &key, 0).await;
    insert_workspace_regenerate_task(&pool, &key, false, false)
        .await
        .expect("insert maintenance task");

    let outcome = insert_queued_manual_run(&pool, &key, sub_agent_id)
        .await
        .expect("manual run");
    assert!(matches!(outcome, QueuedSubAgentRun::BlockedByMaintenance));
    assert_eq!(count_agent_runs(&pool, &key).await.expect("count runs"), 0);
}

#[tokio::test]
async fn insert_queued_manual_run_inserts_manual_dispatch_run_and_does_not_advance_job() {
    let pool = test_db::pool().await;
    let key = format!("manual-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0));
    let sub_agent_id = seed_agent_and_job(&pool, &key, 0).await;

    let (before_next_run_at,): (DateTime<Utc>,) =
        query_as("SELECT next_run_at FROM harness_sub_agents WHERE id = $1")
            .bind(sub_agent_id)
            .fetch_one(&pool)
            .await
            .expect("fetch before next_run_at");

    let outcome = insert_queued_manual_run(&pool, &key, sub_agent_id)
        .await
        .expect("manual run");
    let (run_id, scheduled_for) = match outcome {
        QueuedSubAgentRun::Dispatch {
            run_id,
            scheduled_for,
            ..
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
        Some(crate::harness::store::sub_agents::DEFAULT_ANALYSIS_TIMEFRAME)
    );
    let expected_scheduled_for = latest_due_at_or_before(
        after,
        crate::harness::store::sub_agents::DEFAULT_ANALYSIS_TIMEFRAME,
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
        query_as("SELECT next_run_at FROM harness_sub_agents WHERE id = $1")
            .bind(sub_agent_id)
            .fetch_one(&pool)
            .await
            .expect("fetch after next_run_at");
    assert_eq!(
        before_next_run_at, after_next_run_at,
        "manual run must not mutate next_run_at"
    );
}

#[tokio::test]
async fn insert_queued_manual_run_waits_when_previous_run_is_active() {
    let pool = test_db::pool().await;
    let key = format!(
        "manual-skip-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let sub_agent_id = seed_agent_and_job(&pool, &key, 0).await;
    insert_test_run(&pool, sub_agent_id, RUN_STATUS_RUNNING)
        .await
        .expect("seed active run");

    let outcome = insert_queued_manual_run(&pool, &key, sub_agent_id)
        .await
        .expect("manual run");
    match outcome {
        QueuedSubAgentRun::Dispatch {
            run_id,
            wait_for_lane,
            ..
        } => {
            assert!(wait_for_lane);
            let run = get_run(&pool, run_id)
                .await
                .expect("fetch run")
                .expect("run present");
            assert_eq!(run.status, RUN_STATUS_QUEUED);
        }
        other => panic!("expected queued dispatch, got {other:?}"),
    }
}

#[tokio::test]
async fn mark_run_running_sets_status_and_started_at() {
    let pool = test_db::pool().await;
    let key = format!(
        "run-running-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let sub_agent_id = seed_agent_and_job(&pool, &key, 0).await;
    let run_id = insert_test_run(&pool, sub_agent_id, RUN_STATUS_QUEUED)
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
    let sub_agent_id = seed_agent_and_job(&pool, &key, 0).await;
    let run_id = insert_test_run(&pool, sub_agent_id, RUN_STATUS_RUNNING)
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
async fn set_run_error_summary_preserves_running_status_and_can_clear() {
    let pool = test_db::pool().await;
    let key = format!(
        "run-retry-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let sub_agent_id = seed_agent_and_job(&pool, &key, 0).await;
    let run_id = insert_test_run(&pool, sub_agent_id, RUN_STATUS_RUNNING)
        .await
        .expect("seed run");

    assert!(
        set_run_error_summary(&pool, run_id, Some("usage exhausted"))
            .await
            .expect("set retry summary")
    );
    let run = get_run(&pool, run_id)
        .await
        .expect("fetch run")
        .expect("run present");
    assert_eq!(run.status, RUN_STATUS_RUNNING);
    assert_eq!(run.error_summary.as_deref(), Some("usage exhausted"));

    assert!(
        set_run_error_summary(&pool, run_id, None)
            .await
            .expect("clear retry summary")
    );
    let run = get_run(&pool, run_id)
        .await
        .expect("fetch cleared run")
        .expect("run present");
    assert_eq!(run.status, RUN_STATUS_RUNNING);
    assert!(run.error_summary.is_none());
}

#[tokio::test]
async fn mark_run_failed_truncates_error_summary() {
    let pool = test_db::pool().await;
    let key = format!("run-fail-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0));
    let sub_agent_id = seed_agent_and_job(&pool, &key, 0).await;
    let run_id = insert_test_run(&pool, sub_agent_id, RUN_STATUS_RUNNING)
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
    let sub_agent_id = seed_agent_and_job(&pool, &key, 0).await;
    let run_id = insert_test_run(&pool, sub_agent_id, RUN_STATUS_QUEUED)
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
    let sub_agent_id = seed_agent_and_job(&pool, &key, 0).await;
    let run_id = insert_test_run(&pool, sub_agent_id, RUN_STATUS_RUNNING)
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

#[tokio::test]
async fn terminal_run_cannot_be_overwritten_after_cancellation() {
    let pool = test_db::pool().await;
    let key = format!(
        "run-cancel-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let sub_agent_id = seed_agent_and_job(&pool, &key, 0).await;
    let run_id = insert_test_run(&pool, sub_agent_id, RUN_STATUS_RUNNING)
        .await
        .expect("seed run");

    assert!(
        mark_run_aborted(&pool, run_id, "cancelled by operator", Some("ses_cancel"))
            .await
            .expect("mark aborted")
    );
    assert!(
        !mark_run_running(&pool, run_id, Some("ses_late"))
            .await
            .expect("late running update")
    );
    assert!(
        !mark_run_failed(&pool, run_id, "late failure", None)
            .await
            .expect("late failed update")
    );
    assert!(
        !mark_run_succeeded(&pool, run_id, None)
            .await
            .expect("late succeeded update")
    );

    let run = get_run(&pool, run_id)
        .await
        .expect("fetch run")
        .expect("run present");
    assert_eq!(run.status, RUN_STATUS_ABORTED);
    assert_eq!(run.error_summary.as_deref(), Some("cancelled by operator"));
    assert_eq!(run.backend_run_ref.as_deref(), Some("ses_cancel"));
}
