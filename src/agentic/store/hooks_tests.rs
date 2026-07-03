use chrono::Utc;
use sqlx::query_as;

use crate::{
    agentic::model::{JOB_KIND_MARKET_ANALYSIS, RUN_STATUS_QUEUED},
    agents::store::insert_agent,
    test_db,
};

use super::test_support::sample_agent;
use super::{
    QueuedHookRun, delete_agent_hook, get_agent_hook, get_run, insert_agent_hook,
    insert_queued_hook_run, list_agent_hooks, set_hook_enabled, set_hook_timeout,
};

#[tokio::test]
async fn insert_agent_hook_generates_market_analysis_key_and_rejects_duplicates() {
    let pool = test_db::pool().await;
    let key = format!("hook-dup-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0));
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

    let hook = get_agent_hook(&pool, &key, hook_id)
        .await
        .expect("get hook")
        .expect("hook present");
    assert_eq!(hook.job_key, "market-analysis");

    let duplicate = insert_agent_hook(
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
    .await;
    assert!(duplicate.is_err());
}

#[tokio::test]
async fn deleting_hook_cascades_run_rows() {
    let pool = test_db::pool().await;
    let key = format!(
        "delete-hook-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
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
    let run_id = match insert_queued_hook_run(&pool, &key, hook_id)
        .await
        .expect("insert queued hook run")
    {
        QueuedHookRun::Dispatch { run_id, .. } => run_id,
        other => panic!("expected Dispatch, got {other:?}"),
    };

    let deleted = delete_agent_hook(&pool, &key, hook_id)
        .await
        .expect("delete hook");
    assert!(deleted);
    assert!(get_run(&pool, run_id).await.expect("get run").is_none());
}

#[tokio::test]
async fn set_hook_timeout_updates_value() {
    let pool = test_db::pool().await;
    let key = format!(
        "hook-timeout-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
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

    let updated = set_hook_timeout(&pool, &key, hook_id, 777)
        .await
        .expect("update timeout");
    assert!(updated);

    let hook = get_agent_hook(&pool, &key, hook_id)
        .await
        .expect("fetch hook")
        .expect("hook present");
    assert_eq!(hook.timeout_seconds, 777);
}

#[tokio::test]
async fn set_hook_timeout_rejects_non_positive() {
    let pool = test_db::pool().await;
    let key = format!(
        "hook-timeout-bad-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
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

    let err = set_hook_timeout(&pool, &key, hook_id, -5)
        .await
        .expect_err("negative should fail");
    assert!(err.to_string().contains("positive"));
}