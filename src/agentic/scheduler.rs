use std::{collections::BTreeMap, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use chrono::Utc;
use tokio::{sync::watch, task::JoinHandle};
use tracing::{debug, error, info, warn};

use crate::{
    agentic::{
        backend::{AgenticBackend, DispatchRequest, dispatch_with_timeout},
        model::{DueOpenCodeScheduleRow, JOB_KIND_ANALYSIS, JOB_KIND_TRADING},
        store,
        timeframe::parse_timeframe_seconds,
    },
    agents::store::{get_agent, list_agent_instrument_ids},
    db::DbPool,
    hyperliquid::live_state::{LiveAccountStore, live_agent_snapshot_for_dispatch},
    settings,
};

const SCHEDULER_POLL_INTERVAL: Duration = Duration::from_secs(10);
const DUE_SCHEDULE_LIMIT: i64 = 20;

const JOB_KIND_PRIORITY_ANALYSIS: u8 = 0;
const JOB_KIND_PRIORITY_TRADING: u8 = 1;
const JOB_KIND_PRIORITY_UNKNOWN: u8 = 2;

/// Periodic background loop that claims due OpenCode schedules and
/// dispatches them through an [`AgenticBackend`].
///
/// The scheduler is generic over the backend so tests can swap in a
/// fake implementation. In production this is `OpenCodeBackend`.
///
/// Schedules for the same agent run sequentially in a single worker
/// task, ordered by shortest timeframe first, then `analysis` before
/// `trading`, then numeric `schedule_id`. Different agents may run
/// concurrently.
pub struct AgenticScheduler {
    pool: DbPool,
    shutdown_rx: watch::Receiver<bool>,
    backend: Arc<dyn AgenticBackend>,
    live_accounts: Arc<LiveAccountStore>,
}

impl AgenticScheduler {
    pub fn new(
        pool: DbPool,
        shutdown_rx: watch::Receiver<bool>,
        backend: Arc<dyn AgenticBackend>,
        live_accounts: Arc<LiveAccountStore>,
    ) -> Self {
        Self {
            pool,
            shutdown_rx,
            backend,
            live_accounts,
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

    /// One scheduling pass: load due schedules, claim each, and run
    /// them sequentially per agent.
    ///
    /// This is exposed (not just called from [`Self::run`]) so tests can
    /// drive a single tick deterministically.
    pub async fn tick(&mut self) -> Result<()> {
        let now = Utc::now();
        let due = store::list_due_opencode_schedules(&self.pool, now, DUE_SCHEDULE_LIMIT).await?;
        debug!(count = due.len(), "due opencode schedules loaded");

        if due.is_empty() {
            return Ok(());
        }

        let sorted = sort_due_for_dispatch(due);
        let mut by_agent: BTreeMap<String, Vec<DueOpenCodeScheduleRow>> = BTreeMap::new();
        for schedule in sorted {
            by_agent
                .entry(schedule.agent_key.clone())
                .or_default()
                .push(schedule);
        }

        for (agent_key, schedules) in by_agent {
            let pool = self.pool.clone();
            let backend = self.backend.clone();
            let live_accounts = self.live_accounts.clone();
            tokio::spawn(async move {
                for schedule in schedules {
                    process_schedule_for_agent(&pool, &backend, &live_accounts, &agent_key, schedule).await;
                }
            });
        }

        Ok(())
    }
}

async fn process_schedule_for_agent(
    pool: &DbPool,
    backend: &Arc<dyn AgenticBackend>,
    live_accounts: &Arc<LiveAccountStore>,
    agent_key: &str,
    schedule: DueOpenCodeScheduleRow,
) {
    let schedule_id = schedule.schedule_id;
    let job_key = schedule.job_key.clone();
    let scheduled_for = schedule.next_run_at;

    let claim = match store::claim_due_schedule(pool, schedule_id, Utc::now()).await {
        Ok(claim) => claim,
        Err(error) => {
            warn!(
                schedule_id,
                agent_key,
                job_key = %job_key,
                error = ?error,
                "failed to claim due schedule"
            );
            return;
        }
    };

    match claim {
        store::ClaimedScheduleRun::NotDue => {
            debug!(
                schedule_id,
                agent_key, "schedule no longer due at claim time"
            );
        }
        store::ClaimedScheduleRun::Skipped { run_id } => {
            info!(
                schedule_id,
                run_id,
                agent_key,
                job_key = %job_key,
                "agentic run skipped because previous run still active"
            );
        }
        store::ClaimedScheduleRun::Dispatch { run_id } => {
            match build_dispatch_request(pool, live_accounts, &schedule, run_id, scheduled_for).await {
                Ok(Some(request)) => {
                    dispatch_run(pool.clone(), backend.clone(), request).await;
                }
                Ok(None) => {
                    warn!(
                        run_id,
                        agent_key = %agent_key,
                        job_key = %job_key,
                        "no currencies selected for agent; job skipped"
                    );
                    let _ = store::mark_run_failed(
                        pool,
                        run_id,
                        "no currencies selected for agent; job skipped",
                        None,
                    )
                    .await;
                }
                Err(error) => {
                    error!(
                        run_id,
                        agent_key = %agent_key,
                        job_key = %job_key,
                        error = ?error,
                        "failed to build dispatch request"
                    );
                    let _ = store::mark_run_failed(
                        pool,
                        run_id,
                        "dispatch request errored",
                        None,
                    )
                    .await;
                }
            }
        }
    }
}

pub fn dispatch_request_from_schedule(
    schedule: &DueOpenCodeScheduleRow,
    run_id: i64,
    scheduled_for: chrono::DateTime<Utc>,
    agent: &crate::agents::model::AgentDetailRow,
    selected_instruments: Vec<String>,
    system_prompt: String,
    account_snapshot: Option<crate::hyperliquid::live_state::LiveAgentSnapshot>,
) -> DispatchRequest {
    DispatchRequest {
        run_id,
        schedule_id: schedule.schedule_id,
        agent_key: schedule.agent_key.clone(),
        display_name: schedule.display_name.clone(),
        job_key: schedule.job_key.clone(),
        job_kind: schedule.job_kind.clone(),
        timeframe: schedule.timeframe.clone(),
        operator_prompt: schedule.operator_prompt.clone(),
        analysis_prompt: agent.analysis_prompt.clone(),
        trading_prompt: agent.trading_prompt.clone(),
        system_prompt,
        environment: agent.environment.clone(),
        selected_instruments,
        account_snapshot,
        model_provider_id: schedule.model_provider_id.clone(),
        model_id: schedule.model_id.clone(),
        timeout_seconds: schedule.timeout_seconds,
        runtime_base_url: schedule.runtime_base_url.clone(),
        runtime_config: schedule.runtime_config.clone(),
        scheduled_for,
    }
}

async fn build_dispatch_request(
    pool: &DbPool,
    live_accounts: &Arc<LiveAccountStore>,
    schedule: &DueOpenCodeScheduleRow,
    run_id: i64,
    scheduled_for: chrono::DateTime<Utc>,
) -> Result<Option<DispatchRequest>> {
    let agent = get_agent(pool, &schedule.agent_key)
        .await?
        .context("agent not found while building dispatch request")?;

    let selected_instruments = list_agent_instrument_ids(pool, &schedule.agent_key).await?;

    if selected_instruments.is_empty() {
        return Ok(None);
    }

    let system_setting = settings::store::get_setting(pool, "opencode_system_prompt").await?;
    let system_prompt = system_setting.map(|s| s.value).unwrap_or_default();

    let account_snapshot = if schedule.job_kind == JOB_KIND_TRADING {
        Some(live_agent_snapshot_for_dispatch(
            &agent.wallet_address,
            &agent.environment,
            live_accounts,
        ))
    } else {
        None
    };

    Ok(Some(dispatch_request_from_schedule(
        schedule,
        run_id,
        scheduled_for,
        &agent,
        selected_instruments,
        system_prompt,
        account_snapshot,
    )))
}

/// Run a single dispatch through the backend, awaiting its completion.
/// Sequential schedulers should call this so that the next schedule
/// for the same agent is not processed until the current run finishes.
pub async fn dispatch_run(
    pool: DbPool,
    backend: Arc<dyn AgenticBackend>,
    request: DispatchRequest,
) {
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
}

/// Spawn a detached dispatch task. Used by manual `Run now` flows
/// where the caller is not waiting for completion.
pub fn spawn_dispatch_task(
    pool: DbPool,
    backend: Arc<dyn AgenticBackend>,
    request: DispatchRequest,
) {
    tokio::spawn(dispatch_run(pool, backend, request));
}

fn job_kind_priority(job_kind: &str) -> u8 {
    match job_kind {
        JOB_KIND_ANALYSIS => JOB_KIND_PRIORITY_ANALYSIS,
        JOB_KIND_TRADING => JOB_KIND_PRIORITY_TRADING,
        _ => JOB_KIND_PRIORITY_UNKNOWN,
    }
}

fn schedule_sort_key(schedule: &DueOpenCodeScheduleRow) -> ScheduleSortKey {
    let duration_seconds = match parse_timeframe_seconds(&schedule.timeframe) {
        Ok(seconds) => seconds,
        Err(error) => {
            warn!(
                schedule_id = schedule.schedule_id,
                timeframe = %schedule.timeframe,
                error = ?error,
                "ignoring schedule with invalid timeframe"
            );
            i64::MAX
        }
    };
    ScheduleSortKey {
        next_run_at: schedule.next_run_at,
        duration_seconds,
        job_kind_priority: job_kind_priority(&schedule.job_kind),
        schedule_id: schedule.schedule_id,
    }
}

#[derive(Debug, Clone, Copy)]
struct ScheduleSortKey {
    next_run_at: chrono::DateTime<Utc>,
    duration_seconds: i64,
    job_kind_priority: u8,
    schedule_id: i64,
}

impl Ord for ScheduleSortKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Tuple ordering: earliest next_run_at first, then shortest
        // timeframe, then analysis-before-trading, then lowest id.
        (
            self.next_run_at,
            self.duration_seconds,
            self.job_kind_priority,
            self.schedule_id,
        )
            .cmp(&(
                other.next_run_at,
                other.duration_seconds,
                other.job_kind_priority,
                other.schedule_id,
            ))
    }
}

impl PartialOrd for ScheduleSortKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Eq for ScheduleSortKey {}

impl PartialEq for ScheduleSortKey {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == std::cmp::Ordering::Equal
    }
}

fn sort_due_for_dispatch(due: Vec<DueOpenCodeScheduleRow>) -> Vec<DueOpenCodeScheduleRow> {
    let mut indexed: Vec<(ScheduleSortKey, DueOpenCodeScheduleRow)> = due
        .into_iter()
        .map(|schedule| (schedule_sort_key(&schedule), schedule))
        .collect();
    indexed.sort_by_key(|(key, _)| *key);
    indexed.into_iter().map(|(_, row)| row).collect()
}

/// Convenience: spawn the scheduler on the current Tokio runtime and
/// return the join handle. The handle aborts when the runtime drops.
#[allow(dead_code)]
pub fn spawn(
    pool: DbPool,
    shutdown_rx: watch::Receiver<bool>,
    backend: Arc<dyn AgenticBackend>,
    live_accounts: Arc<LiveAccountStore>,
) -> JoinHandle<Result<()>> {
    tokio::spawn(async move {
        AgenticScheduler::new(pool, shutdown_rx, backend, live_accounts)
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
    use serde_json::json;

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
            store::{insert_agent, replace_agent_instruments},
        },
        test_db,
    };

    struct FakeBackend {
        calls: Arc<Mutex<Vec<DispatchRequest>>>,
        delay: Duration,
    }

    #[async_trait]
    impl AgenticBackend for FakeBackend {
        async fn dispatch(&self, request: DispatchRequest) -> Result<DispatchResult> {
            if !self.delay.is_zero() {
                tokio::time::sleep(self.delay).await;
            }
            self.calls.lock().unwrap().push(request);
            Ok(DispatchResult {
                backend_run_ref: "ses_fake".to_string(),
            })
        }
    }

    impl FakeBackend {
        fn success(calls: Arc<Mutex<Vec<DispatchRequest>>>) -> Self {
            Self {
                calls,
                delay: Duration::ZERO,
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
            analysis_prompt: "Analyze trends.".to_string(),
            trading_prompt: "Trade breakouts.".to_string(),
            wallet_address: wallet,
            environment: "live".to_string(),
            api_key: format!("vta_{key}"),
            api_key_last_used_at: None,
            backend_kind: BACKEND_KIND_OPENCODE.to_string(),
            runtime_id: "opencode-local".to_string(),
            runtime_config: json!({
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

        // Default schedules need selected instruments and strategy prompts or
        // the enriched dispatch path will skip or produce empty prompts.
        sqlx::query(
            "INSERT INTO hyperliquid.instruments (
                instrument_id, name, market_type, base_asset, quote_asset,
                settlement_asset, asset_index, price_decimals, size_decimals,
                lot_size, max_leverage, is_hip3, active, created_at, updated_at
            ) VALUES (
                'BTC', 'BTC', 'perp', 'BTC', 'USD', 'USDC', 1, 2, 3, 0.001, 50, false, true, NOW(), NOW()
            )
            ON CONFLICT (instrument_id) DO UPDATE SET active = EXCLUDED.active",
        )
        .execute(pool)
        .await
        .expect("seed BTC instrument");
        replace_agent_instruments(pool, key, &["BTC".to_string()])
            .await
            .expect("seed agent instruments");
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

    /// Pin a schedule's `next_run_at` to the latest due boundary for
    /// `timeframe` at or before `now`, so a claim at `now` will fire.
    async fn pin_schedule_due(pool: &DbPool, schedule_id: i64, timeframe: &str) {
        let now = Utc::now();
        let due = crate::agentic::timeframe::latest_due_at_or_before(now, timeframe, 1)
            .expect("compute latest due")
            .expect("should have a previous due boundary");
        sqlx::query(
            "UPDATE agentic_job_schedules
                SET enabled = true, next_run_at = $2
              WHERE id = $1",
        )
        .bind(schedule_id)
        .bind(due)
        .execute(pool)
        .await
        .expect("force due");
    }

    #[tokio::test]
    async fn tick_dispatches_due_schedule_and_marks_run_succeeded() {
        let pool = test_db::pool().await;
        let key = format!("sched-ok-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0));
        seed_test_agent(&pool, &key).await;

        let (schedule_id,): (i64,) = sqlx::query_as(
            "SELECT id FROM agentic_job_schedules
              WHERE agent_key = $1 AND job_key = 'analysis-15m'",
        )
        .bind(&key)
        .fetch_one(&pool)
        .await
        .expect("fetch schedule id");
        pin_schedule_due(&pool, schedule_id, "15m").await;

        let calls: Arc<Mutex<Vec<DispatchRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let backend: Arc<dyn AgenticBackend> = Arc::new(FakeBackend::success(calls.clone()));

        let live_accounts = Arc::new(crate::hyperliquid::live_state::LiveAccountStore::new());
        let (_tx, rx) = watch::channel(false);
        let mut scheduler = AgenticScheduler::new(pool.clone(), rx, backend, live_accounts);
        scheduler.tick().await.expect("tick");

        run_until(|| async { calls.lock().map(|guard| !guard.is_empty()).unwrap_or(false) }).await;

        let guard = calls.lock().unwrap();
        assert_eq!(guard.len(), 1);
        let request = &guard[0];
        assert_eq!(request.agent_key, key);
        assert_eq!(request.job_key, "analysis-15m");
        assert_eq!(request.timeframe, "15m");
        drop(guard);

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
    async fn tick_dispatches_same_agent_schedules_sequentially() {
        let pool = test_db::pool().await;
        let key = format!(
            "sched-seq-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        seed_test_agent(&pool, &key).await;

        // Add a 1h analysis schedule on top of the default 15m.
        let one_h_id = store::insert_agent_schedule(
            &pool, &key, "analysis", true, "1h", 1, None, None, 600, "",
        )
        .await
        .expect("insert 1h analysis");

        // Force both analysis schedules to be due at the same boundary.
        let (fifteen_m_id,): (i64,) = sqlx::query_as(
            "SELECT id FROM agentic_job_schedules
              WHERE agent_key = $1 AND job_key = 'analysis-15m'",
        )
        .bind(&key)
        .fetch_one(&pool)
        .await
        .expect("fetch 15m schedule id");
        pin_schedule_due(&pool, fifteen_m_id, "15m").await;
        pin_schedule_due(&pool, one_h_id, "1h").await;

        // Normalize both schedules to the same next_run_at so ordering is
        // determined by duration, not by where the current wall-clock falls
        // between candle boundaries. Using the later due time keeps both
        // schedules fresh (not stale) for their respective timeframes.
        sqlx::query(
            "UPDATE agentic_job_schedules
                SET next_run_at = (
                    SELECT MAX(next_run_at) FROM agentic_job_schedules
                    WHERE id = $1 OR id = $2
                )
              WHERE id = $1 OR id = $2",
        )
        .bind(fifteen_m_id)
        .bind(one_h_id)
        .execute(&pool)
        .await
        .expect("normalize schedule due times");

        let calls: Arc<Mutex<Vec<DispatchRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let backend: Arc<dyn AgenticBackend> = Arc::new(FakeBackend {
            calls: calls.clone(),
            delay: Duration::from_millis(50),
        });

        let live_accounts = Arc::new(crate::hyperliquid::live_state::LiveAccountStore::new());
        let (_tx, rx) = watch::channel(false);
        let mut scheduler = AgenticScheduler::new(pool.clone(), rx, backend, live_accounts);
        scheduler.tick().await.expect("tick");

        run_until(|| async { calls.lock().map(|guard| guard.len() >= 2).unwrap_or(false) }).await;

        let guard = calls.lock().unwrap();
        assert_eq!(guard.len(), 2);
        let order: Vec<&str> = guard.iter().map(|r| r.job_key.as_str()).collect();
        // 15m is shorter than 1h, so it should dispatch first.
        assert_eq!(order, vec!["analysis-15m", "analysis-1h"]);
    }

    #[tokio::test]
    async fn tick_dispatches_equal_timeframe_analysis_before_trading() {
        let pool = test_db::pool().await;
        let key = format!(
            "sched-kinds-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        seed_test_agent(&pool, &key).await;

        let (analysis_id,): (i64,) = sqlx::query_as(
            "SELECT id FROM agentic_job_schedules
              WHERE agent_key = $1 AND job_key = 'analysis-15m'",
        )
        .bind(&key)
        .fetch_one(&pool)
        .await
        .expect("fetch analysis id");
        let (trading_id,): (i64,) = sqlx::query_as(
            "SELECT id FROM agentic_job_schedules
              WHERE agent_key = $1 AND job_key = 'trading-1m'",
        )
        .bind(&key)
        .fetch_one(&pool)
        .await
        .expect("fetch trading id");
        pin_schedule_due(&pool, analysis_id, "15m").await;
        pin_schedule_due(&pool, trading_id, "1m").await;

        let calls: Arc<Mutex<Vec<DispatchRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let backend: Arc<dyn AgenticBackend> = Arc::new(FakeBackend {
            calls: calls.clone(),
            delay: Duration::from_millis(30),
        });

        let live_accounts = Arc::new(crate::hyperliquid::live_state::LiveAccountStore::new());
        let (_tx, rx) = watch::channel(false);
        let mut scheduler = AgenticScheduler::new(pool.clone(), rx, backend, live_accounts);
        scheduler.tick().await.expect("tick");

        run_until(|| async { calls.lock().map(|guard| guard.len() >= 2).unwrap_or(false) }).await;

        let guard = calls.lock().unwrap();
        let order: Vec<&str> = guard.iter().map(|r| r.job_key.as_str()).collect();
        assert!(
            order[0] == "analysis-15m",
            "expected analysis to dispatch first, got {order:?}"
        );
    }

    #[tokio::test]
    async fn tick_dispatches_different_agents_concurrently() {
        let pool = test_db::pool().await;
        let key_a = format!("agent-a-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0));
        let key_b = format!("agent-b-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0));

        let far_future = Utc::now() + chrono::Duration::days(365);
        sqlx::query("UPDATE agentic_job_schedules SET next_run_at = $1")
            .bind(far_future)
            .execute(&pool)
            .await
            .expect("push existing schedules");
        sqlx::query("UPDATE agentic_job_schedules SET enabled = false")
            .execute(&pool)
            .await
            .expect("disable existing schedules");

        seed_test_agent(&pool, &key_a).await;
        seed_test_agent(&pool, &key_b).await;

        for key in [&key_a, &key_b] {
            let (schedule_id,): (i64,) = sqlx::query_as(
                "SELECT id FROM agentic_job_schedules
                  WHERE agent_key = $1 AND job_key = 'analysis-15m'",
            )
            .bind(key)
            .fetch_one(&pool)
            .await
            .expect("fetch schedule id");
            pin_schedule_due(&pool, schedule_id, "15m").await;
        }

        let calls: Arc<Mutex<Vec<DispatchRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let backend: Arc<dyn AgenticBackend> = Arc::new(FakeBackend {
            calls: calls.clone(),
            delay: Duration::from_millis(150),
        });

        let live_accounts = Arc::new(crate::hyperliquid::live_state::LiveAccountStore::new());
        let (_tx, rx) = watch::channel(false);
        let mut scheduler = AgenticScheduler::new(pool.clone(), rx, backend, live_accounts);
        let started = std::time::Instant::now();
        scheduler.tick().await.expect("tick");

        run_until(|| async { calls.lock().map(|guard| guard.len() >= 2).unwrap_or(false) }).await;
        let elapsed = started.elapsed();

        let guard = calls.lock().unwrap();
        assert_eq!(guard.len(), 2);
        let mut agents: Vec<&str> = guard.iter().map(|r| r.agent_key.as_str()).collect();
        agents.sort();
        assert_eq!(agents, vec![key_a.as_str(), key_b.as_str()]);
        // Concurrency: total time should be roughly one dispatch, not two.
        assert!(
            elapsed < Duration::from_millis(280),
            "expected concurrent dispatch, took {elapsed:?}"
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
        pin_schedule_due(&pool, schedule_id, "15m").await;

        insert_test_run(&pool, schedule_id, "running")
            .await
            .expect("seed active run");

        let calls: Arc<Mutex<Vec<DispatchRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let backend: Arc<dyn AgenticBackend> = Arc::new(FakeBackend::success(calls.clone()));
        let live_accounts = Arc::new(crate::hyperliquid::live_state::LiveAccountStore::new());
        let (_tx, rx) = watch::channel(false);
        let mut scheduler = AgenticScheduler::new(pool.clone(), rx, backend, live_accounts);
        scheduler.tick().await.expect("tick");

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(calls.lock().unwrap().is_empty());

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
    async fn claim_due_schedule_does_not_double_dispatch() {
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
        pin_schedule_due(&pool, schedule_id, "15m").await;

        let now = Utc::now();
        let first = claim_due_schedule(&pool, schedule_id, now)
            .await
            .expect("claim 1");
        assert!(matches!(first, ClaimedScheduleRun::Dispatch { .. }));
        let second = claim_due_schedule(&pool, schedule_id, now)
            .await
            .expect("claim 2");
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
