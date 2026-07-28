use chrono::Utc;

use crate::agents::store::insert_agent;
use crate::test_db;

use super::test_support::sample_agent;
use super::{insert_default_harness_jobs, list_agent_jobs, set_all_agent_jobs_enabled};
use crate::harness::model::JOB_KIND_ANALYSIS_CODING;

#[tokio::test]
async fn set_all_agent_jobs_enabled_toggles_jobs_together() {
    let pool = test_db::pool().await;
    let key = format!(
        "toggle-all-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    insert_agent(&pool, &sample_agent(&key))
        .await
        .expect("insert agent");
    insert_default_harness_jobs(&pool, &key)
        .await
        .expect("insert defaults");

    sqlx::query(
        "UPDATE harness_jobs
            SET model_provider_id = 'anthropic', model_id = 'claude-sonnet-test'
          WHERE agent_key = $1",
    )
    .bind(&key)
    .execute(&pool)
    .await
    .expect("pin job models");

    set_all_agent_jobs_enabled(&pool, &key, true)
        .await
        .expect("enable all");

    let jobs = list_agent_jobs(&pool, &key).await.expect("list jobs");
    for job in &jobs {
        if job.job_kind == JOB_KIND_ANALYSIS_CODING {
            assert!(
                !job.enabled,
                "coding job {} must stay disabled after bulk enable",
                job.id
            );
        } else {
            assert!(
                job.enabled,
                "job {} ({}) should be enabled",
                job.id, job.job_kind
            );
        }
    }

    set_all_agent_jobs_enabled(&pool, &key, false)
        .await
        .expect("disable all");

    let jobs = list_agent_jobs(&pool, &key).await.expect("list jobs again");
    assert!(jobs.iter().all(|row| !row.enabled));
}
