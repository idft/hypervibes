use chrono::{DateTime, Utc};
use sqlx::{query, query_as};

use crate::{
    agentic::{
        model::{
            JOB_KIND_ANALYSIS, JOB_KIND_MARKET_ANALYSIS, RUN_STATUS_FAILED, RUN_STATUS_QUEUED,
            RUN_STATUS_RUNNING, RUN_STATUS_SKIPPED, RUN_STATUS_SUCCEEDED,
        },
        timeframe::{DEFAULT_TRIGGER_DELAY_SECONDS, latest_due_at_or_before, next_due_after},
    },
    agents::store::insert_agent,
    test_db,
};

use super::recovery::{ORPHANED_QUEUED_RUN_SUMMARY, ORPHANED_RUNNING_RUN_SUMMARY};
use super::schedules::{
    DEFAULT_ANALYSIS_TIMEFRAME, DEFAULT_ANALYSIS_TIMEOUT_SECONDS, DEFAULT_TRADING_TIMEFRAME,
    DEFAULT_TRADING_TIMEOUT_SECONDS, default_analysis_job_key, default_trading_job_key,
};
use super::test_support::{
    insert_test_opencode_command, insert_test_opencode_session, sample_agent,
    seed_agent_and_schedule,
};
use super::{
    ClaimedScheduleRun, claim_due_schedule, count_agent_runs, delete_agent_schedule,
    get_agent_schedule, get_run, insert_agent_schedule, insert_default_opencode_schedules,
    insert_test_run, insert_workspace_regenerate_task, list_agent_hooks, list_agent_schedules,
    list_due_opencode_schedules, set_schedule_timeframe, set_schedule_timeout,
};

#[tokio::test]
async fn default_schedules_insert_expected_rows_with_disabled_defaults() {
    let pool = test_db::pool().await;
    let key = format!(
        "default-sched-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    insert_agent(&pool, &sample_agent(&key))
        .await
        .expect("insert agent");

    insert_default_opencode_schedules(&pool, &key)
        .await
        .expect("insert defaults");

    let rows = list_agent_schedules(&pool, &key)
        .await
        .expect("list schedules");
    assert_eq!(rows.len(), 5);

    let analysis = rows
        .iter()
        .find(|row| row.job_key == default_analysis_job_key())
        .expect("analysis schedule present");
    assert!(!analysis.enabled);
    assert_eq!(analysis.job_kind, JOB_KIND_ANALYSIS);
    assert_eq!(analysis.timeframe, DEFAULT_ANALYSIS_TIMEFRAME);
    assert_eq!(analysis.timeout_seconds, DEFAULT_ANALYSIS_TIMEOUT_SECONDS);

    let analysis_1h = rows
        .iter()
        .find(|row| row.job_key == "analysis-1h")
        .expect("1h analysis schedule present");
    assert!(!analysis_1h.enabled);
    assert_eq!(analysis_1h.job_kind, JOB_KIND_ANALYSIS);
    assert_eq!(analysis_1h.timeframe, "1h");

    let analysis_1d = rows
        .iter()
        .find(|row| row.job_key == "analysis-1d")
        .expect("1d analysis schedule present");
    assert!(!analysis_1d.enabled);
    assert_eq!(analysis_1d.job_kind, JOB_KIND_ANALYSIS);
    assert_eq!(analysis_1d.timeframe, "1d");

    let trading = rows
        .iter()
        .find(|row| row.job_key == default_trading_job_key())
        .expect("trading schedule present");
    assert!(!trading.enabled);
    assert_eq!(trading.job_kind, crate::agentic::model::JOB_KIND_TRADING);
    assert_eq!(trading.timeframe, DEFAULT_TRADING_TIMEFRAME);
    assert_eq!(trading.timeout_seconds, DEFAULT_TRADING_TIMEOUT_SECONDS);

    let daily_review = rows
        .iter()
        .find(|row| row.job_key == "daily-review-1d")
        .expect("daily review schedule present");
    assert!(!daily_review.enabled);
    assert_eq!(daily_review.job_kind, crate::agentic::model::JOB_KIND_DAILY_REVIEW);
    assert_eq!(daily_review.timeframe, "1d");

    let hooks = list_agent_hooks(&pool, &key).await.expect("list hooks");
    assert_eq!(hooks.len(), 1);
    let hook = hooks.first().expect("default hook present");
    assert_eq!(hook.job_key, "market-analysis");
    assert_eq!(hook.job_kind, JOB_KIND_MARKET_ANALYSIS);
    assert_eq!(
        hook.hook_event,
        crate::agentic::model::HOOK_EVENT_ANALYSIS_BATCH_COMPLETED
    );
    assert!(!hook.enabled);
}

#[tokio::test]
async fn default_schedules_are_idempotent() {
    let pool = test_db::pool().await;
    let key = format!(
        "idempotent-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    insert_agent(&pool, &sample_agent(&key))
        .await
        .expect("insert agent");

    insert_default_opencode_schedules(&pool, &key)
        .await
        .expect("first insert");
    insert_default_opencode_schedules(&pool, &key)
        .await
        .expect("second insert");

    let count: (i64,) = query_as("SELECT COUNT(*) FROM agentic_job_schedules WHERE agent_key = $1")
        .bind(&key)
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(count.0, 5);

    let hook_count: (i64,) =
        query_as("SELECT COUNT(*) FROM agentic_job_hooks WHERE agent_key = $1")
            .bind(&key)
            .fetch_one(&pool)
            .await
            .expect("hook count");
    assert_eq!(hook_count.0, 1);
}

#[tokio::test]
async fn default_schedules_align_next_run_at_to_future_boundary() {
    let pool = test_db::pool().await;
    let key = format!(
        "default-aligned-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    insert_agent(&pool, &sample_agent(&key))
        .await
        .expect("insert agent");

    let now = Utc::now();
    insert_default_opencode_schedules(&pool, &key)
        .await
        .expect("insert defaults");

    let rows = list_agent_schedules(&pool, &key)
        .await
        .expect("list schedules");
    let analysis = rows
        .iter()
        .find(|row| row.job_key == default_analysis_job_key())
        .expect("analysis schedule present");
    assert!(
        analysis.next_run_at > now,
        "expected future next_run_at, got {:?} <= {:?}",
        analysis.next_run_at,
        now
    );
    // The next due instant for a 15m schedule from any moment is a
    // 15-minute UTC boundary + 1s.
    let expected = next_due_after(
        now,
        DEFAULT_ANALYSIS_TIMEFRAME,
        DEFAULT_TRIGGER_DELAY_SECONDS,
    )
    .expect("next due");
    assert_eq!(analysis.next_run_at, expected);
}

#[tokio::test]
async fn insert_agent_schedule_persists_custom_schedule() {
    let pool = test_db::pool().await;
    let key = format!(
        "custom-sched-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    insert_agent(&pool, &sample_agent(&key))
        .await
        .expect("insert agent");

    let schedule_id = insert_agent_schedule(
        &pool,
        &key,
        JOB_KIND_ANALYSIS,
        true,
        "1h",
        1,
        Some("anthropic"),
        Some("claude-sonnet-4"),
        600,
        "Check higher timeframe structure.",
    )
    .await
    .expect("insert schedule");

    let rows = list_agent_schedules(&pool, &key)
        .await
        .expect("list schedules");
    let row = rows
        .iter()
        .find(|row| row.id == schedule_id)
        .expect("inserted schedule present");
    assert_eq!(row.job_key, "analysis-1h");
    assert_eq!(row.job_kind, JOB_KIND_ANALYSIS);
    assert!(row.enabled);
    assert_eq!(row.timeframe, "1h");
    assert_eq!(row.timeout_seconds, 600);
    assert_eq!(row.model_provider_id.as_deref(), Some("anthropic"));
    assert_eq!(row.model_id.as_deref(), Some("claude-sonnet-4"));
    assert_eq!(row.operator_prompt, "Check higher timeframe structure.");
}

#[tokio::test]
async fn insert_agent_schedule_rejects_invalid_timeframe() {
    let pool = test_db::pool().await;
    let key = format!(
        "custom-bad-tf-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    insert_agent(&pool, &sample_agent(&key))
        .await
        .expect("insert agent");

    let result = insert_agent_schedule(
        &pool,
        &key,
        JOB_KIND_ANALYSIS,
        true,
        "15s",
        1,
        None,
        None,
        600,
        "",
    )
    .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn insert_agent_schedule_rejects_duplicate_job_kind_timeframe() {
    let pool = test_db::pool().await;
    let key = format!(
        "custom-dup-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    insert_agent(&pool, &sample_agent(&key))
        .await
        .expect("insert agent");

    insert_agent_schedule(
        &pool,
        &key,
        JOB_KIND_ANALYSIS,
        true,
        "1h",
        1,
        None,
        None,
        600,
        "",
    )
    .await
    .expect("first insert");

    let result = insert_agent_schedule(
        &pool,
        &key,
        JOB_KIND_ANALYSIS,
        true,
        "1h",
        1,
        None,
        None,
        600,
        "",
    )
    .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn updating_schedule_timeframe_reanchors_the_next_run() {
    let pool = test_db::pool().await;
    let key = format!(
        "update-timeframe-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;

    assert!(set_schedule_timeframe(&pool, &key, schedule_id, "4h")
        .await
        .expect("update timeframe"));

    let schedule = get_agent_schedule(&pool, &key, schedule_id)
        .await
        .expect("get schedule")
        .expect("schedule present");
    assert_eq!(schedule.timeframe, "4h");
    assert_eq!(schedule.job_key, "analysis-4h");
    assert!(schedule.next_run_at > Utc::now());
    assert_eq!(
        (schedule.next_run_at.timestamp() - i64::from(DEFAULT_TRIGGER_DELAY_SECONDS)) % (4 * 60 * 60),
        0
    );
}

#[tokio::test]
async fn deleting_schedule_cascades_run_rows() {
    let pool = test_db::pool().await;
    let key = format!(
        "delete-schedule-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;
    let run_id = insert_test_run(&pool, schedule_id, RUN_STATUS_QUEUED)
        .await
        .expect("insert run");

    let deleted = delete_agent_schedule(&pool, &key, schedule_id)
        .await
        .expect("delete schedule");
    assert!(deleted);
    assert!(get_run(&pool, run_id).await.expect("get run").is_none());
}

#[tokio::test]
async fn list_due_opencode_schedules_excludes_disabled_and_other_backends() {
    let pool = test_db::pool().await;
    let key = format!("due-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0));
    insert_agent(&pool, &sample_agent(&key))
        .await
        .expect("insert agent");
    insert_default_opencode_schedules(&pool, &key)
        .await
        .expect("defaults");

    query("UPDATE agentic_job_schedules SET enabled = false WHERE job_key = $1")
        .bind(default_analysis_job_key())
        .execute(&pool)
        .await
        .expect("disable analysis");
    let future = Utc::now() + chrono::Duration::seconds(3600);
    query("UPDATE agentic_job_schedules SET next_run_at = $1 WHERE job_key = $2")
        .bind(future)
        .bind(default_trading_job_key())
        .execute(&pool)
        .await
        .expect("push trading future");
    query("UPDATE agentic_job_schedules SET enabled = true WHERE job_key = $1")
        .bind(default_trading_job_key())
        .execute(&pool)
        .await
        .expect("enable trading");

    let now = Utc::now();
    let due = list_due_opencode_schedules(&pool, now, 20)
        .await
        .expect("list due");

    assert!(
        due.iter().all(|row| row.agent_key != key),
        "expected no rows for disabled/future agent, got {due:?}"
    );

    query("UPDATE agentic_job_schedules SET enabled = true, next_run_at = $1 WHERE job_key = $2")
        .bind(now - chrono::Duration::seconds(1))
        .bind(default_analysis_job_key())
        .execute(&pool)
        .await
        .expect("re-enable analysis due");
    let due = list_due_opencode_schedules(&pool, now, 20)
        .await
        .expect("list due again");
    let analysis_due = due
        .iter()
        .find(|row| row.agent_key == key && row.job_key == default_analysis_job_key())
        .expect("analysis row should be due");
    assert!(analysis_due.runtime_base_url.contains("14096"));
    assert_eq!(analysis_due.timeframe, DEFAULT_ANALYSIS_TIMEFRAME);
}

#[tokio::test]
async fn list_due_opencode_schedules_filters_by_agent_enabled_flag() {
    let pool = test_db::pool().await;
    let key = format!(
        "agent-disabled-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    insert_agent(&pool, &sample_agent(&key))
        .await
        .expect("insert agent");
    insert_default_opencode_schedules(&pool, &key)
        .await
        .expect("defaults");

    let past = Utc::now() - chrono::Duration::seconds(60);
    query("UPDATE agentic_job_schedules SET enabled = true, next_run_at = $1 WHERE job_key = $2")
        .bind(past)
        .bind(default_analysis_job_key())
        .execute(&pool)
        .await
        .expect("set analysis due");
    query("UPDATE agents SET enabled = false WHERE agent_key = $1")
        .bind(&key)
        .execute(&pool)
        .await
        .expect("disable agent");

    let now = Utc::now();
    let due = list_due_opencode_schedules(&pool, now, 20)
        .await
        .expect("list due");
    assert!(
        due.iter().all(|row| row.agent_key != key),
        "disabled parent agent should be excluded"
    );
}

#[tokio::test]
async fn claim_due_schedule_inserts_queued_run_and_aligns_next_run_at() {
    let pool = test_db::pool().await;
    let key = format!("claim-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0));
    let schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;

    // Set the schedule to a previous 15m due boundary so it is
    // due "now" no matter when the test actually runs.
    let now = Utc::now();
    let due_boundary = latest_due_at_or_before(
        now,
        DEFAULT_ANALYSIS_TIMEFRAME,
        DEFAULT_TRIGGER_DELAY_SECONDS,
    )
    .expect("compute latest due")
    .expect("should have a previous due boundary");
    query("UPDATE agentic_job_schedules SET next_run_at = $1 WHERE id = $2")
        .bind(due_boundary)
        .bind(schedule_id)
        .execute(&pool)
        .await
        .expect("set due");

    let outcome = claim_due_schedule(&pool, schedule_id, now)
        .await
        .expect("claim");
    let run_id = match outcome {
        ClaimedScheduleRun::Dispatch { run_id } => run_id,
        other => panic!("expected Dispatch, got {other:?}"),
    };

    let run = get_run(&pool, run_id)
        .await
        .expect("fetch run")
        .expect("run present");
    assert_eq!(run.status, RUN_STATUS_QUEUED);
    assert_eq!(run.agent_key, key);
    assert_eq!(run.timeframe.as_deref(), Some(DEFAULT_ANALYSIS_TIMEFRAME));
    assert_eq!(
        run.scheduled_for,
        crate::agentic::timeframe::boundary_for_due_at(due_boundary, DEFAULT_TRIGGER_DELAY_SECONDS)
    );

    // The next_run_at should be aligned to the next 15m boundary
    // after `now`, regardless of how late the claim was.
    let (next_run_at,): (DateTime<Utc>,) =
        query_as("SELECT next_run_at FROM agentic_job_schedules WHERE id = $1")
            .bind(schedule_id)
            .fetch_one(&pool)
            .await
            .expect("fetch next_run_at");
    let expected = next_due_after(
        now,
        DEFAULT_ANALYSIS_TIMEFRAME,
        DEFAULT_TRIGGER_DELAY_SECONDS,
    )
    .expect("compute next due");
    assert_eq!(next_run_at, expected);
}

#[tokio::test]
async fn claim_due_schedule_advances_stale_schedule_without_dispatching() {
    let pool = test_db::pool().await;
    let key = format!("stale-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0));
    let schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;

    // Push next_run_at well into the past, simulating a long
    // downtime.
    let long_ago = Utc::now() - chrono::Duration::hours(2);
    query("UPDATE agentic_job_schedules SET next_run_at = $1 WHERE id = $2")
        .bind(long_ago)
        .bind(schedule_id)
        .execute(&pool)
        .await
        .expect("set stale");

    let now = Utc::now();
    let outcome = claim_due_schedule(&pool, schedule_id, now)
        .await
        .expect("claim");
    assert!(matches!(outcome, ClaimedScheduleRun::NotDue));

    let (next_run_at,): (DateTime<Utc>,) =
        query_as("SELECT next_run_at FROM agentic_job_schedules WHERE id = $1")
            .bind(schedule_id)
            .fetch_one(&pool)
            .await
            .expect("fetch next_run_at");
    let expected = next_due_after(
        now,
        DEFAULT_ANALYSIS_TIMEFRAME,
        DEFAULT_TRIGGER_DELAY_SECONDS,
    )
    .expect("compute next due");
    assert_eq!(next_run_at, expected);
}

#[tokio::test]
async fn claim_due_schedule_inserts_skipped_run_when_active_run_exists_for_same_agent() {
    let pool = test_db::pool().await;
    let key = format!("skip-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0));
    let schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;

    let now = Utc::now();
    let due_boundary = latest_due_at_or_before(
        now,
        DEFAULT_ANALYSIS_TIMEFRAME,
        DEFAULT_TRIGGER_DELAY_SECONDS,
    )
    .expect("compute latest due")
    .expect("should have a previous due boundary");
    query("UPDATE agentic_job_schedules SET next_run_at = $1 WHERE id = $2")
        .bind(due_boundary)
        .bind(schedule_id)
        .execute(&pool)
        .await
        .expect("set due");
    insert_test_run(&pool, schedule_id, RUN_STATUS_RUNNING)
        .await
        .expect("seed active run");

    let outcome = claim_due_schedule(&pool, schedule_id, now)
        .await
        .expect("claim");
    let run_id = match outcome {
        ClaimedScheduleRun::Skipped { run_id } => run_id,
        other => panic!("expected Skipped, got {other:?}"),
    };

    let run = get_run(&pool, run_id)
        .await
        .expect("fetch run")
        .expect("run present");
    assert_eq!(run.status, RUN_STATUS_SKIPPED);
    assert_eq!(
        run.error_summary.as_deref(),
        Some("previous run still active")
    );
    assert!(run.finished_at.is_some());
    assert_eq!(run.timeframe.as_deref(), Some(DEFAULT_ANALYSIS_TIMEFRAME));
}

#[tokio::test]
async fn claim_due_schedule_recovers_idle_running_run_and_dispatches_next() {
    let pool = test_db::pool().await;
    let key = format!(
        "recover-idle-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;
    let previous_run_id = insert_test_run(&pool, schedule_id, RUN_STATUS_RUNNING)
        .await
        .expect("seed running run");
    let session_id = format!(
        "ses_recovered_{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    query(
        "UPDATE agentic_runs
            SET backend_run_ref = $2,
                started_at = COALESCE(started_at, now())
          WHERE id = $1",
    )
    .bind(previous_run_id)
    .bind(&session_id)
    .execute(&pool)
    .await
    .expect("attach backend session ref");
    insert_test_opencode_session(&pool, &session_id, "idle").await;
    insert_test_opencode_command(&pool, &session_id).await;

    let now = Utc::now();
    let due_boundary = latest_due_at_or_before(
        now,
        DEFAULT_ANALYSIS_TIMEFRAME,
        DEFAULT_TRIGGER_DELAY_SECONDS,
    )
    .expect("compute latest due")
    .expect("should have a previous due boundary");
    query("UPDATE agentic_job_schedules SET next_run_at = $1 WHERE id = $2")
        .bind(due_boundary)
        .bind(schedule_id)
        .execute(&pool)
        .await
        .expect("set due");

    let outcome = claim_due_schedule(&pool, schedule_id, now)
        .await
        .expect("claim");
    let new_run_id = match outcome {
        ClaimedScheduleRun::Dispatch { run_id } => run_id,
        other => panic!("expected Dispatch, got {other:?}"),
    };

    let previous_run = get_run(&pool, previous_run_id)
        .await
        .expect("fetch previous run")
        .expect("previous run present");
    assert_eq!(previous_run.status, RUN_STATUS_SUCCEEDED);

    let new_run = get_run(&pool, new_run_id)
        .await
        .expect("fetch new run")
        .expect("new run present");
    assert_eq!(new_run.status, RUN_STATUS_QUEUED);
}

#[tokio::test]
async fn claim_due_schedule_keeps_active_opencode_run_blocking() {
    let pool = test_db::pool().await;
    let key = format!(
        "recover-busy-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;
    let previous_run_id = insert_test_run(&pool, schedule_id, RUN_STATUS_RUNNING)
        .await
        .expect("seed running run");
    let session_id = format!(
        "ses_active_{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    query(
        "UPDATE agentic_runs
            SET backend_run_ref = $2,
                started_at = COALESCE(started_at, now())
          WHERE id = $1",
    )
    .bind(previous_run_id)
    .bind(&session_id)
    .execute(&pool)
    .await
    .expect("attach backend session ref");
    insert_test_opencode_session(&pool, &session_id, "active").await;
    insert_test_opencode_command(&pool, &session_id).await;

    let now = Utc::now();
    let due_boundary = latest_due_at_or_before(
        now,
        DEFAULT_ANALYSIS_TIMEFRAME,
        DEFAULT_TRIGGER_DELAY_SECONDS,
    )
    .expect("compute latest due")
    .expect("should have a previous due boundary");
    query("UPDATE agentic_job_schedules SET next_run_at = $1 WHERE id = $2")
        .bind(due_boundary)
        .bind(schedule_id)
        .execute(&pool)
        .await
        .expect("set due");

    let outcome = claim_due_schedule(&pool, schedule_id, now)
        .await
        .expect("claim");
    let skipped_run_id = match outcome {
        ClaimedScheduleRun::Skipped { run_id } => run_id,
        other => panic!("expected Skipped, got {other:?}"),
    };

    let previous_run = get_run(&pool, previous_run_id)
        .await
        .expect("fetch previous run")
        .expect("previous run present");
    assert_eq!(previous_run.status, RUN_STATUS_RUNNING);

    let skipped_run = get_run(&pool, skipped_run_id)
        .await
        .expect("fetch skipped run")
        .expect("skipped run present");
    assert_eq!(skipped_run.status, RUN_STATUS_SKIPPED);
    assert_eq!(
        skipped_run.error_summary.as_deref(),
        Some("previous run still active")
    );
}

#[tokio::test]
async fn claim_due_schedule_fails_timed_out_queued_orphan_and_dispatches_next() {
    let pool = test_db::pool().await;
    let key = format!(
        "recover-queued-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;
    let previous_run_id = insert_test_run(&pool, schedule_id, RUN_STATUS_QUEUED)
        .await
        .expect("seed queued run");
    query(
        "UPDATE agentic_runs
            SET created_at = now() - (timeout_seconds + 60) * interval '1 second',
                updated_at = now() - (timeout_seconds + 60) * interval '1 second'
          WHERE id = $1",
    )
    .bind(previous_run_id)
    .execute(&pool)
    .await
    .expect("age queued run past timeout");

    let now = Utc::now();
    let due_boundary = latest_due_at_or_before(
        now,
        DEFAULT_ANALYSIS_TIMEFRAME,
        DEFAULT_TRIGGER_DELAY_SECONDS,
    )
    .expect("compute latest due")
    .expect("should have a previous due boundary");
    query("UPDATE agentic_job_schedules SET next_run_at = $1 WHERE id = $2")
        .bind(due_boundary)
        .bind(schedule_id)
        .execute(&pool)
        .await
        .expect("set due");

    let outcome = claim_due_schedule(&pool, schedule_id, now)
        .await
        .expect("claim");
    let new_run_id = match outcome {
        ClaimedScheduleRun::Dispatch { run_id } => run_id,
        other => panic!("expected Dispatch, got {other:?}"),
    };

    let previous_run = get_run(&pool, previous_run_id)
        .await
        .expect("fetch previous run")
        .expect("previous run present");
    assert_eq!(previous_run.status, RUN_STATUS_FAILED);
    assert_eq!(
        previous_run.error_summary.as_deref(),
        Some(ORPHANED_QUEUED_RUN_SUMMARY)
    );

    let new_run = get_run(&pool, new_run_id)
        .await
        .expect("fetch new run")
        .expect("new run present");
    assert_eq!(new_run.status, RUN_STATUS_QUEUED);
}

#[tokio::test]
async fn claim_due_schedule_fails_timed_out_running_orphan_without_backend_ref() {
    let pool = test_db::pool().await;
    let key = format!(
        "recover-running-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;
    let previous_run_id = insert_test_run(&pool, schedule_id, RUN_STATUS_RUNNING)
        .await
        .expect("seed running run");
    query(
        "UPDATE agentic_runs
            SET started_at = now() - (timeout_seconds + 60) * interval '1 second',
                created_at = now() - (timeout_seconds + 60) * interval '1 second',
                updated_at = now() - (timeout_seconds + 60) * interval '1 second'
          WHERE id = $1",
    )
    .bind(previous_run_id)
    .execute(&pool)
    .await
    .expect("age running run past timeout");

    let now = Utc::now();
    let due_boundary = latest_due_at_or_before(
        now,
        DEFAULT_ANALYSIS_TIMEFRAME,
        DEFAULT_TRIGGER_DELAY_SECONDS,
    )
    .expect("compute latest due")
    .expect("should have a previous due boundary");
    query("UPDATE agentic_job_schedules SET next_run_at = $1 WHERE id = $2")
        .bind(due_boundary)
        .bind(schedule_id)
        .execute(&pool)
        .await
        .expect("set due");

    let outcome = claim_due_schedule(&pool, schedule_id, now)
        .await
        .expect("claim");
    let new_run_id = match outcome {
        ClaimedScheduleRun::Dispatch { run_id } => run_id,
        other => panic!("expected Dispatch, got {other:?}"),
    };

    let previous_run = get_run(&pool, previous_run_id)
        .await
        .expect("fetch previous run")
        .expect("previous run present");
    assert_eq!(previous_run.status, RUN_STATUS_FAILED);
    assert_eq!(
        previous_run.error_summary.as_deref(),
        Some(ORPHANED_RUNNING_RUN_SUMMARY)
    );

    let new_run = get_run(&pool, new_run_id)
        .await
        .expect("fetch new run")
        .expect("new run present");
    assert_eq!(new_run.status, RUN_STATUS_QUEUED);
}

#[tokio::test]
async fn claim_due_schedule_fails_timed_out_running_orphan_with_stuck_session() {
    let pool = test_db::pool().await;
    let key = format!(
        "recover-running-stuck-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;
    let previous_run_id = insert_test_run(&pool, schedule_id, RUN_STATUS_RUNNING)
        .await
        .expect("seed running run");
    let session_id = format!(
        "ses_stuck_{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    query(
        "UPDATE agentic_runs
            SET backend_run_ref = $2,
                started_at = now() - (timeout_seconds + 60) * interval '1 second',
                created_at = now() - (timeout_seconds + 60) * interval '1 second',
                updated_at = now() - (timeout_seconds + 60) * interval '1 second'
          WHERE id = $1",
    )
    .bind(previous_run_id)
    .bind(&session_id)
    .execute(&pool)
    .await
    .expect("attach stuck session and age past timeout");
    insert_test_opencode_session(&pool, &session_id, "active").await;

    let now = Utc::now();
    let due_boundary = latest_due_at_or_before(
        now,
        DEFAULT_ANALYSIS_TIMEFRAME,
        DEFAULT_TRIGGER_DELAY_SECONDS,
    )
    .expect("compute latest due")
    .expect("should have a previous due boundary");
    query("UPDATE agentic_job_schedules SET next_run_at = $1 WHERE id = $2")
        .bind(due_boundary)
        .bind(schedule_id)
        .execute(&pool)
        .await
        .expect("set due");

    let outcome = claim_due_schedule(&pool, schedule_id, now)
        .await
        .expect("claim");
    let new_run_id = match outcome {
        ClaimedScheduleRun::Dispatch { run_id } => run_id,
        other => panic!("expected Dispatch, got {other:?}"),
    };

    let previous_run = get_run(&pool, previous_run_id)
        .await
        .expect("fetch previous run")
        .expect("previous run present");
    assert_eq!(previous_run.status, RUN_STATUS_FAILED);
    assert_eq!(
        previous_run.error_summary.as_deref(),
        Some(ORPHANED_RUNNING_RUN_SUMMARY)
    );

    let new_run = get_run(&pool, new_run_id)
        .await
        .expect("fetch new run")
        .expect("new run present");
    assert_eq!(new_run.status, RUN_STATUS_QUEUED);
}

#[tokio::test]
async fn recover_inactive_runs_all_fails_stuck_session_orphan() {
    let pool = test_db::pool().await;
    let key = format!(
        "recover-all-stuck-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;
    let run_id = insert_test_run(&pool, schedule_id, RUN_STATUS_RUNNING)
        .await
        .expect("seed running run");
    let session_id = format!(
        "ses_stuck_all_{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    query(
        "UPDATE agentic_runs
            SET backend_run_ref = $2,
                started_at = now() - (timeout_seconds + 60) * interval '1 second',
                created_at = now() - (timeout_seconds + 60) * interval '1 second',
                updated_at = now() - (timeout_seconds + 60) * interval '1 second'
          WHERE id = $1",
    )
    .bind(run_id)
    .bind(&session_id)
    .execute(&pool)
    .await
    .expect("attach stuck session and age past timeout");
    insert_test_opencode_session(&pool, &session_id, "active").await;

    let recovered = crate::agentic::store::recover_inactive_runs_all(&pool, Utc::now())
        .await
        .expect("recover all");
    assert!(
        recovered >= 1,
        "expected at least one recovery, got {recovered}"
    );

    let run = get_run(&pool, run_id)
        .await
        .expect("fetch run")
        .expect("run present");
    assert_eq!(run.status, RUN_STATUS_FAILED);
    assert_eq!(
        run.error_summary.as_deref(),
        Some(ORPHANED_RUNNING_RUN_SUMMARY)
    );
    assert!(run.finished_at.is_some());
}

#[tokio::test]
async fn recover_inactive_runs_all_recovers_idle_session_with_command() {
    let pool = test_db::pool().await;
    let key = format!(
        "recover-all-idle-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;
    let run_id = insert_test_run(&pool, schedule_id, RUN_STATUS_RUNNING)
        .await
        .expect("seed running run");
    let session_id = format!(
        "ses_idle_all_{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    query(
        "UPDATE agentic_runs
            SET backend_run_ref = $2
          WHERE id = $1",
    )
    .bind(run_id)
    .bind(&session_id)
    .execute(&pool)
    .await
    .expect("attach session");
    insert_test_opencode_session(&pool, &session_id, "idle").await;
    insert_test_opencode_command(&pool, &session_id).await;

    let recovered = crate::agentic::store::recover_inactive_runs_all(&pool, Utc::now())
        .await
        .expect("recover all");
    assert!(
        recovered >= 1,
        "expected at least one recovery, got {recovered}"
    );

    let run = get_run(&pool, run_id)
        .await
        .expect("fetch run")
        .expect("run present");
    assert_eq!(run.status, RUN_STATUS_SUCCEEDED);
    assert!(run.finished_at.is_some());
}

#[tokio::test]
async fn recover_inactive_runs_all_skips_fresh_running_run() {
    let pool = test_db::pool().await;
    let key = format!(
        "recover-all-fresh-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;
    let run_id = insert_test_run(&pool, schedule_id, RUN_STATUS_RUNNING)
        .await
        .expect("seed running run");

    // Recover the run normally so other agents' orphans don't leak in.
    let _ = crate::agentic::store::recover_inactive_runs_all(&pool, Utc::now())
        .await
        .expect("recover all");

    let run = get_run(&pool, run_id)
        .await
        .expect("fetch run")
        .expect("run present");
    assert_eq!(run.status, RUN_STATUS_RUNNING);
    assert!(run.finished_at.is_none());
}

#[tokio::test]
async fn claim_due_schedule_returns_not_due_when_schedule_disabled() {
    let pool = test_db::pool().await;
    let key = format!("notdue-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0));
    let schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;

    let now = Utc::now();
    let due = next_due_after(
        now,
        DEFAULT_ANALYSIS_TIMEFRAME,
        DEFAULT_TRIGGER_DELAY_SECONDS,
    )
    .expect("compute next due");
    query("UPDATE agentic_job_schedules SET enabled = false, next_run_at = $1 WHERE id = $2")
        .bind(due)
        .bind(schedule_id)
        .execute(&pool)
        .await
        .expect("disable");

    let outcome = claim_due_schedule(&pool, schedule_id, now)
        .await
        .expect("claim");
    assert!(matches!(outcome, ClaimedScheduleRun::NotDue));
}

#[tokio::test]
async fn claim_due_schedule_returns_blocked_by_maintenance_without_inserting_run() {
    let pool = test_db::pool().await;
    let key = format!(
        "claim-maint-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;

    let now = Utc::now();
    let due_boundary = latest_due_at_or_before(
        now,
        DEFAULT_ANALYSIS_TIMEFRAME,
        DEFAULT_TRIGGER_DELAY_SECONDS,
    )
    .expect("compute latest due")
    .expect("should have a previous due boundary");
    query("UPDATE agentic_job_schedules SET next_run_at = $1 WHERE id = $2")
        .bind(due_boundary)
        .bind(schedule_id)
        .execute(&pool)
        .await
        .expect("set due");

    insert_workspace_regenerate_task(&pool, &key, false, false)
        .await
        .expect("insert maintenance task");

    let outcome = claim_due_schedule(&pool, schedule_id, now)
        .await
        .expect("claim");
    assert!(matches!(outcome, ClaimedScheduleRun::BlockedByMaintenance));
    assert_eq!(count_agent_runs(&pool, &key).await.expect("count runs"), 0);
}

#[tokio::test]
async fn set_schedule_timeout_updates_value() {
    let pool = test_db::pool().await;
    let key = format!(
        "sched-timeout-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;

    let updated = set_schedule_timeout(&pool, &key, schedule_id, 1234)
        .await
        .expect("update timeout");
    assert!(updated);

    let schedule = get_agent_schedule(&pool, &key, schedule_id)
        .await
        .expect("fetch schedule")
        .expect("schedule present");
    assert_eq!(schedule.timeout_seconds, 1234);
}

#[tokio::test]
async fn set_schedule_timeout_rejects_non_positive() {
    let pool = test_db::pool().await;
    let key = format!(
        "sched-timeout-bad-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let schedule_id = seed_agent_and_schedule(&pool, &key, 0).await;

    let err = set_schedule_timeout(&pool, &key, schedule_id, 0)
        .await
        .expect_err("zero should fail");
    assert!(err.to_string().contains("positive"));
}

#[tokio::test]
async fn set_schedule_timeout_missing_returns_false() {
    let pool = test_db::pool().await;
    let err = set_schedule_timeout(&pool, "no-such-agent", 999, 900)
        .await
        .expect("update returns false for missing schedule");
    assert!(!err);
}
