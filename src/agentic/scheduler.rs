use std::{sync::Arc, time::Duration};

use anyhow::Result;
use chrono::Utc;
use tokio::{sync::watch, task::JoinHandle};
use tracing::{debug, error, info, warn};

use crate::{
    agentic::{
        backend::{AgenticBackend, DispatchRequest, dispatch_with_timeout},
        model::DueOpenCodeScheduleRow,
        store,
    },
    db::DbPool,
};

const SCHEDULER_POLL_INTERVAL: Duration = Duration::from_secs(10);
const DUE_SCHEDULE_LIMIT: i64 = 20;

/// Periodic background loop that claims due OpenCode schedules and
/// dispatches them through an [`AgenticBackend`].
///
/// The scheduler is generic over the backend so tests can swap in a
/// fake implementation. In production this is `OpenCodeBackend`.
pub struct AgenticScheduler {
    pool: DbPool,
    shutdown_rx: watch::Receiver<bool>,
    backend: Arc<dyn AgenticBackend>,
}

impl AgenticScheduler {
    pub fn new(
        pool: DbPool,
        shutdown_rx: watch::Receiver<bool>,
        backend: Arc<dyn AgenticBackend>,
    ) -> Self {
        Self {
            pool,
            shutdown_rx,
            backend,
        }
    }

    /// Run the scheduler until a shutdown signal is observed.
    pub async fn run(mut self) -> Result<()> {
        info!("agentic scheduler starting");
        loop {
            if *self.shutdown_rx.borrow() {
                break;
            }

            if let Err(error) = self.tick().await {
                warn!(error = ?error, "agentic scheduler tick failed");
            }

            tokio::select! {
                _ = tokio::time::sleep(SCHEDULER_POLL_INTERVAL) => {}
                _ = self.shutdown_rx.changed() => break,
            }
        }

        info!("agentic scheduler stopped");
        Ok(())
    }

    /// One scheduling pass: load due schedules, claim each, and spawn
    /// dispatch tasks for newly-queued runs.
    ///
    /// This is exposed (not just called from [`Self::run`]) so tests can
    /// drive a single tick deterministically.
    pub async fn tick(&mut self) -> Result<()> {
        let now = Utc::now();
        let due = store::list_due_opencode_schedules(&self.pool, now, DUE_SCHEDULE_LIMIT).await?;
        debug!(count = due.len(), "due opencode schedules loaded");

        for schedule in due {
            let pool = self.pool.clone();
            let backend = self.backend.clone();
            let schedule_id = schedule.schedule_id;
            let agent_key = schedule.agent_key.clone();
            let job_key = schedule.job_key.clone();
            let job_kind = schedule.job_kind.clone();

            let claim = match store::claim_due_schedule(&self.pool, schedule_id, now).await {
                Ok(claim) => claim,
                Err(error) => {
                    warn!(
                        schedule_id,
                        agent_key = %agent_key,
                        job_key = %job_key,
                        error = ?error,
                        "failed to claim due schedule"
                    );
                    continue;
                }
            };

            match claim {
                store::ClaimedScheduleRun::NotDue => {
                    debug!(schedule_id, "schedule no longer due at claim time");
                }
                store::ClaimedScheduleRun::Skipped { run_id } => {
                    info!(
                        schedule_id,
                        run_id,
                        agent_key = %agent_key,
                        job_key = %job_key,
                        "agentic run skipped because previous run still active"
                    );
                }
                store::ClaimedScheduleRun::Dispatch { run_id } => {
                    let request =
                        dispatch_request_from_schedule(&schedule, run_id, schedule.next_run_at);
                    spawn_dispatch_task(pool, backend, request);
                }
            }

            // `schedule` and `job_kind` are still used in log lines above.
            let _ = job_kind;
        }

        Ok(())
    }
}

pub fn dispatch_request_from_schedule(
    schedule: &DueOpenCodeScheduleRow,
    run_id: i64,
    scheduled_for: chrono::DateTime<Utc>,
) -> DispatchRequest {
    DispatchRequest {
        run_id,
        schedule_id: schedule.schedule_id,
        agent_key: schedule.agent_key.clone(),
        display_name: schedule.display_name.clone(),
        job_key: schedule.job_key.clone(),
        job_kind: schedule.job_kind.clone(),
        operator_prompt: schedule.operator_prompt.clone(),
        model_provider_id: schedule.model_provider_id.clone(),
        model_id: schedule.model_id.clone(),
        timeout_seconds: schedule.timeout_seconds,
        runtime_base_url: schedule.runtime_base_url.clone(),
        runtime_config: schedule.runtime_config.clone(),
        scheduled_for,
    }
}

pub fn spawn_dispatch_task(
    pool: DbPool,
    backend: Arc<dyn AgenticBackend>,
    request: DispatchRequest,
) {
    tokio::spawn(async move {
        let run_id = request.run_id;
        let agent_key = request.agent_key.clone();
        let job_key = request.job_key.clone();

        if let Err(error) = store::mark_run_running(&pool, run_id, None).await {
            warn!(
                run_id,
                agent_key = %agent_key,
                job_key = %job_key,
                error = ?error,
                "failed to mark agentic run as running"
            );
            return;
        }

        match dispatch_with_timeout(&pool, backend, request).await {
            Ok(_) => {
                debug!(
                    run_id,
                    agent_key = %agent_key,
                    job_key = %job_key,
                    "agentic dispatch finished"
                );
            }
            Err(error) => {
                error!(
                    run_id,
                    agent_key = %agent_key,
                    job_key = %job_key,
                    error = ?error,
                    "agentic dispatch errored"
                );
                let _ = store::mark_run_failed(&pool, run_id, "dispatch task errored", None).await;
            }
        }
    });
}

/// Convenience: spawn the scheduler on the current Tokio runtime and
/// return the join handle. The handle aborts when the runtime drops.
#[allow(dead_code)]
pub fn spawn(
    pool: DbPool,
    shutdown_rx: watch::Receiver<bool>,
    backend: Arc<dyn AgenticBackend>,
) -> JoinHandle<Result<()>> {
    tokio::spawn(async move {
        AgenticScheduler::new(pool, shutdown_rx, backend)
            .run()
            .await
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use chrono::Utc;

    use crate::{
        agentic::{
            backend::{AgenticBackend, DispatchResult},
            model::{AgenticRunRow, RUN_STATUS_SKIPPED, RUN_STATUS_SUCCEEDED},
            store::{self, ClaimedScheduleRun, insert_default_opencode_schedules, insert_test_run},
        },
        agents::{
            crypto::{EncryptionKey, encrypt},
            keys::derive_wallet_address,
            model::{AgentRegistryRow, BACKEND_KIND_OPENCODE},
            store::insert_agent,
        },
        test_db,
    };

    struct FakeBackend {
        calls: Arc<Mutex<Vec<DispatchRequest>>>,
        fail: bool,
        fail_when_no_workspace: bool,
    }

    #[async_trait]
    impl AgenticBackend for FakeBackend {
        async fn dispatch(&self, request: DispatchRequest) -> Result<DispatchResult> {
            if self.fail_when_no_workspace
                && request.runtime_config.get("workspace_host_path").is_none()
            {
                return Err(anyhow::anyhow!("OpenCode workspace is not configured"));
            }
            self.calls.lock().unwrap().push(request);
            if self.fail {
                Err(anyhow::anyhow!("fake backend failure"))
            } else {
                Ok(DispatchResult {
                    backend_run_ref: "ses_fake".to_string(),
                })
            }
        }
    }

    impl FakeBackend {
        fn success(calls: Arc<Mutex<Vec<DispatchRequest>>>) -> Self {
            Self {
                calls,
                fail: false,
                fail_when_no_workspace: false,
            }
        }

        fn failing(calls: Arc<Mutex<Vec<DispatchRequest>>>) -> Self {
            Self {
                calls,
                fail: true,
                fail_when_no_workspace: false,
            }
        }

        fn missing_workspace(calls: Arc<Mutex<Vec<DispatchRequest>>>) -> Self {
            Self {
                calls,
                fail: false,
                fail_when_no_workspace: true,
            }
        }
    }

    fn deterministic_private_key(key: &str) -> String {
        use rand::rngs::StdRng;
        use rand::{Rng, SeedableRng};

        let seed = key
            .bytes()
            .fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
        let mut rng = StdRng::seed_from_u64(seed);
        let bytes: [u8; 32] = rng.r#gen();
        format!("0x{}", hex::encode(bytes))
    }

    fn sample_agent(key: &str) -> AgentRegistryRow {
        let enc = EncryptionKey::new(
            "test",
            [
                0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22,
                23, 24, 25, 26, 27, 28, 29, 30, 31,
            ],
        );
        let private_key = deterministic_private_key(key);
        let ciphertext = encrypt(&enc, &private_key).unwrap();
        let wallet = derive_wallet_address(&private_key).unwrap();
        let now = Utc::now();

        AgentRegistryRow {
            agent_key: key.to_string(),
            created_at: now,
            updated_at: now,
            enabled: true,
            display_name: format!("Test {key}"),
            analysis_prompt: String::new(),
            trading_prompt: String::new(),
            wallet_address: wallet,
            environment: "live".to_string(),
            api_key: format!("vta_{key}"),
            api_key_last_used_at: None,
            backend_kind: BACKEND_KIND_OPENCODE.to_string(),
            runtime_id: "opencode-local".to_string(),
            runtime_config: serde_json::json!({
                "workspace_host_path": format!("workspaces/agents/{key}"),
                "workspace_container_path": format!("/workspaces/agents/{key}"),
                "profile_source": "agent-runtime/opencode"
            }),
            analysis_context_last_used_at: None,
            trading_context_last_used_at: None,
            hyperliquid_private_key_ciphertext: ciphertext,
            hyperliquid_private_key_key_id: "test".to_string(),
        }
    }

    async fn seed_test_agent(pool: &DbPool, key: &str) {
        // Push all existing schedules' `next_run_at` far into the future so
        // leftover state from earlier tests in the same database does not
        // get picked up by the due-schedule query during this test.
        let far_future = Utc::now() + chrono::Duration::days(365);
        sqlx::query("UPDATE agentic_job_schedules SET next_run_at = $1")
            .bind(far_future)
            .execute(pool)
            .await
            .expect("push existing schedules");
        sqlx::query("UPDATE agentic_job_schedules SET enabled = false")
            .execute(pool)
            .await
            .expect("disable existing schedules");

        insert_agent(pool, &sample_agent(key))
            .await
            .expect("insert agent");
        insert_default_opencode_schedules(pool, key)
            .await
            .expect("insert defaults");
    }

    async fn run_until<F, Fut>(predicate: F)
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = bool>,
    {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if predicate().await {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("timed out waiting for predicate");
    }

    async fn list_run_statuses(pool: &DbPool, agent_key: &str) -> Vec<String> {
        store::list_agent_runs(pool, agent_key, 10)
            .await
            .map(|rows| rows.into_iter().map(|row| row.status).collect())
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn tick_dispatches_due_schedule_and_marks_run_succeeded() {
        let pool = test_db::pool().await;
        let key = format!("sched-ok-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0));
        seed_test_agent(&pool, &key).await;

        // Force the analysis schedule to be due.
        sqlx::query(
            "UPDATE agentic_job_schedules
                SET enabled = true, next_run_at = $2
              WHERE agent_key = $1 AND job_key = 'analysis-15m'",
        )
        .bind(&key)
        .bind(Utc::now() - chrono::Duration::seconds(5))
        .execute(&pool)
        .await
        .expect("force due");

        let calls: Arc<Mutex<Vec<DispatchRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let backend: Arc<dyn AgenticBackend> = Arc::new(FakeBackend::success(calls.clone()));

        let (_tx, rx) = watch::channel(false);
        let mut scheduler = AgenticScheduler::new(pool.clone(), rx, backend);
        scheduler.tick().await.expect("tick");

        run_until(|| async { calls.lock().map(|guard| !guard.is_empty()).unwrap_or(false) }).await;

        let guard = calls.lock().unwrap();
        assert_eq!(guard.len(), 1);
        let request = &guard[0];
        assert_eq!(request.agent_key, key);
        assert_eq!(request.job_key, "analysis-15m");

        // Wait for the dispatch task to finalize the run.
        run_until(|| async {
            list_run_statuses(&pool, &key)
                .await
                .iter()
                .any(|status| status == RUN_STATUS_SUCCEEDED)
        })
        .await;

        let runs: Vec<AgenticRunRow> = store::list_agent_runs(&pool, &key, 10)
            .await
            .expect("list runs");
        let run = runs
            .iter()
            .find(|row| row.status == RUN_STATUS_SUCCEEDED)
            .expect("succeeded run present");
        assert_eq!(run.backend_run_ref.as_deref(), Some("ses_fake"));
    }

    #[tokio::test]
    async fn tick_marks_run_failed_when_backend_errors() {
        let pool = test_db::pool().await;
        let key = format!(
            "sched-fail-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        seed_test_agent(&pool, &key).await;

        sqlx::query(
            "UPDATE agentic_job_schedules
                SET enabled = true, next_run_at = $2
              WHERE agent_key = $1 AND job_key = 'analysis-15m'",
        )
        .bind(&key)
        .bind(Utc::now() - chrono::Duration::seconds(5))
        .execute(&pool)
        .await
        .expect("force due");

        let calls: Arc<Mutex<Vec<DispatchRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let backend: Arc<dyn AgenticBackend> = Arc::new(FakeBackend::failing(calls.clone()));

        let (_tx, rx) = watch::channel(false);
        let mut scheduler = AgenticScheduler::new(pool.clone(), rx, backend);
        scheduler.tick().await.expect("tick");

        run_until(|| async {
            list_run_statuses(&pool, &key)
                .await
                .iter()
                .any(|status| status == "failed")
        })
        .await;

        let runs = store::list_agent_runs(&pool, &key, 10)
            .await
            .expect("list runs");
        let failed = runs
            .iter()
            .find(|row| row.status == "failed")
            .expect("failed run present");
        assert!(
            failed
                .error_summary
                .as_deref()
                .map(|s| s.contains("fake backend failure"))
                .unwrap_or(false),
            "expected error_summary to mention fake backend failure, got {:?}",
            failed.error_summary
        );
    }

    #[tokio::test]
    async fn tick_inserts_skipped_run_when_active_run_exists() {
        let pool = test_db::pool().await;
        let key = format!(
            "sched-skip-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        seed_test_agent(&pool, &key).await;

        let (schedule_id,): (i64,) = sqlx::query_as(
            "SELECT id FROM agentic_job_schedules
              WHERE agent_key = $1 AND job_key = 'analysis-15m'",
        )
        .bind(&key)
        .fetch_one(&pool)
        .await
        .expect("fetch schedule id");

        sqlx::query(
            "UPDATE agentic_job_schedules
                SET enabled = true, next_run_at = $2
              WHERE id = $1",
        )
        .bind(schedule_id)
        .bind(Utc::now() - chrono::Duration::seconds(5))
        .execute(&pool)
        .await
        .expect("force due");

        // Insert a running run to force the next claim to skip.
        insert_test_run(&pool, schedule_id, "running")
            .await
            .expect("seed active run");

        let calls: Arc<Mutex<Vec<DispatchRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let backend: Arc<dyn AgenticBackend> = Arc::new(FakeBackend::success(calls.clone()));
        let (_tx, rx) = watch::channel(false);
        let mut scheduler = AgenticScheduler::new(pool.clone(), rx, backend);
        scheduler.tick().await.expect("tick");

        // No new dispatch should occur.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(calls.lock().unwrap().is_empty());

        // A skipped run should be present.
        let runs = store::list_agent_runs(&pool, &key, 10)
            .await
            .expect("list runs");
        let skipped = runs
            .iter()
            .find(|row| row.status == RUN_STATUS_SKIPPED)
            .expect("skipped run present");
        assert_eq!(
            skipped.error_summary.as_deref(),
            Some("previous run still active")
        );
    }

    #[tokio::test]
    async fn tick_marks_run_failed_when_workspace_missing() {
        // The store-level "is_due" query requires `runtime_config` to be
        // parseable, but `OpenCodeWorkspaceRuntimeConfig::from_value` is
        // called inside the backend. We simulate the failure by handing
        // the backend a request whose `runtime_config` is empty.
        let pool = test_db::pool().await;
        let key = format!(
            "sched-workspace-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        seed_test_agent(&pool, &key).await;

        // Clear the runtime_config so the backend will see a missing
        // workspace.
        sqlx::query("UPDATE agents SET runtime_config = '{}'::jsonb WHERE agent_key = $1")
            .bind(&key)
            .execute(&pool)
            .await
            .expect("clear runtime_config");

        sqlx::query(
            "UPDATE agentic_job_schedules
                SET enabled = true, next_run_at = $2
              WHERE agent_key = $1 AND job_key = 'analysis-15m'",
        )
        .bind(&key)
        .bind(Utc::now() - chrono::Duration::seconds(5))
        .execute(&pool)
        .await
        .expect("force due");

        let calls: Arc<Mutex<Vec<DispatchRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let backend: Arc<dyn AgenticBackend> =
            Arc::new(FakeBackend::missing_workspace(calls.clone()));
        let (_tx, rx) = watch::channel(false);
        let mut scheduler = AgenticScheduler::new(pool.clone(), rx, backend);
        scheduler.tick().await.expect("tick");

        // Wait for the run to be marked failed.
        run_until(|| async {
            list_run_statuses(&pool, &key)
                .await
                .iter()
                .any(|status| status == "failed")
        })
        .await;

        let runs = store::list_agent_runs(&pool, &key, 10)
            .await
            .expect("list runs");
        let failed = runs
            .iter()
            .find(|row| row.status == "failed")
            .expect("failed run present");
        assert!(
            failed
                .error_summary
                .as_deref()
                .map(|s| s.contains("workspace"))
                .unwrap_or(false),
            "expected error_summary to mention workspace, got {:?}",
            failed.error_summary
        );
    }

    #[tokio::test]
    async fn claim_due_schedule_does_not_double_dispatch() {
        // Sanity check: when two ticks run back-to-back, only the first
        // should claim a queued run. The second should observe NotDue.
        let pool = test_db::pool().await;
        let key = format!(
            "sched-double-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        seed_test_agent(&pool, &key).await;

        let (schedule_id,): (i64,) = sqlx::query_as(
            "SELECT id FROM agentic_job_schedules
              WHERE agent_key = $1 AND job_key = 'analysis-15m'",
        )
        .bind(&key)
        .fetch_one(&pool)
        .await
        .expect("fetch schedule id");

        sqlx::query(
            "UPDATE agentic_job_schedules
                SET enabled = true, next_run_at = $2
              WHERE id = $1",
        )
        .bind(schedule_id)
        .bind(Utc::now() - chrono::Duration::seconds(5))
        .execute(&pool)
        .await
        .expect("force due");

        let now = Utc::now();
        let first = claim_due_schedule(&pool, schedule_id, now)
            .await
            .expect("claim 1");
        assert!(matches!(first, ClaimedScheduleRun::Dispatch { .. }));
        let second = claim_due_schedule(&pool, schedule_id, now)
            .await
            .expect("claim 2");
        // The second claim should find the next_run_at already advanced.
        assert!(matches!(second, ClaimedScheduleRun::NotDue));
    }

    async fn claim_due_schedule(
        pool: &DbPool,
        schedule_id: i64,
        now: chrono::DateTime<Utc>,
    ) -> Result<ClaimedScheduleRun> {
        store::claim_due_schedule(pool, schedule_id, now).await
    }
}
