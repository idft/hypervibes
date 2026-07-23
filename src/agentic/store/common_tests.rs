use chrono::Utc;

use crate::agents::store::insert_agent;
use crate::test_db;

use super::test_support::sample_agent;
use super::{
    insert_default_opencode_schedules, list_agent_hooks, list_agent_schedules,
    set_all_agent_jobs_enabled,
};
use crate::agentic::model::JOB_KIND_ANALYSIS_CODING;

#[tokio::test]
async fn set_all_agent_jobs_enabled_toggles_schedules_and_hooks_together() {
    let pool = test_db::pool().await;
    let key = format!(
        "toggle-all-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    insert_agent(&pool, &sample_agent(&key))
        .await
        .expect("insert agent");
    insert_default_opencode_schedules(&pool, &key)
        .await
        .expect("insert defaults");

    sqlx::query(
        "UPDATE agentic_job_schedules
            SET model_provider_id = 'anthropic', model_id = 'claude-sonnet-test'
          WHERE agent_key = $1",
    )
    .bind(&key)
    .execute(&pool)
    .await
    .expect("pin schedule models");
    sqlx::query(
        "UPDATE agentic_job_hooks
            SET model_provider_id = 'anthropic', model_id = 'claude-sonnet-test'
          WHERE agent_key = $1",
    )
    .bind(&key)
    .execute(&pool)
    .await
    .expect("pin hook models");

    set_all_agent_jobs_enabled(&pool, &key, true)
        .await
        .expect("enable all");

    let schedules = list_agent_schedules(&pool, &key)
        .await
        .expect("list schedules");
    assert!(schedules.iter().all(|row| row.enabled));
    let hooks = list_agent_hooks(&pool, &key).await.expect("list hooks");
    // Every hook *except* the autonomous analysis-coding hook is
    // enabled by the bulk-enable helper. Coding must be enabled
    // explicitly with a pinned strong model so operators cannot
    // accidentally enable autonomous code modification.
    for hook in &hooks {
        if hook.job_kind == JOB_KIND_ANALYSIS_CODING {
            assert!(
                !hook.enabled,
                "coding hook {} must stay disabled after bulk enable",
                hook.id
            );
        } else {
            assert!(
                hook.enabled,
                "hook {} ({}) should be enabled",
                hook.id, hook.job_kind
            );
        }
    }

    set_all_agent_jobs_enabled(&pool, &key, false)
        .await
        .expect("disable all");

    let schedules = list_agent_schedules(&pool, &key)
        .await
        .expect("list schedules again");
    assert!(schedules.iter().all(|row| !row.enabled));
    let hooks = list_agent_hooks(&pool, &key)
        .await
        .expect("list hooks again");
    assert!(hooks.iter().all(|row| !row.enabled));
}
