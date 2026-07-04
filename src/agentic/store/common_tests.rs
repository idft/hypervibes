use chrono::Utc;

use crate::agents::store::insert_agent;
use crate::test_db;

use super::test_support::sample_agent;
use super::{
    insert_default_opencode_schedules, list_agent_hooks, list_agent_schedules,
    set_all_agent_jobs_enabled,
};

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

    set_all_agent_jobs_enabled(&pool, &key, true)
        .await
        .expect("enable all");

    let schedules = list_agent_schedules(&pool, &key)
        .await
        .expect("list schedules");
    assert!(schedules.iter().all(|row| row.enabled));
    let hooks = list_agent_hooks(&pool, &key).await.expect("list hooks");
    assert!(hooks.iter().all(|row| row.enabled));

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
