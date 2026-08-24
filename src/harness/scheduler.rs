use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex, Weak},
    time::Duration,
};

use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard, Semaphore, watch};
use tracing::{debug, error, info, warn};

use crate::{
    agents::{
        store::{get_agent, list_agent_instrument_ids, update_agent_runtime_config},
        strategy_prompts::{
            PROMPT_KIND_ANALYSIS, PROMPT_KIND_ANALYSIS_CODING, default_prompt_for_kind,
            get_agent_strategy_prompt, prompt_kind_for_sub_agent_kind,
        },
    },
    db::DbPool,
    harness::{
        backend::{
            DispatchOutcome, DispatchRequest, HarnessBackend, dispatch_with_timeout,
            dispatch_with_timeout_for_coding,
        },
        in_flight::{InFlightTracker, SHUTDOWN_IN_FLIGHT_GRACE},
        model::{
            HarnessDispatchSubAgentRow, MAINTENANCE_STATUS_QUEUED, SUB_AGENT_KIND_ANALYSIS,
            SUB_AGENT_KIND_ANALYSIS_CODING, SUB_AGENT_KIND_DAILY_REVIEW,
            SUB_AGENT_KIND_MARKET_ANALYSIS, SUB_AGENT_KIND_TRADING,
        },
        store,
        timeframe::{boundary_for_due_at, parse_timeframe_seconds},
        workspace_lease::WorkspaceLeaseManager,
    },
    hyperliquid::live_state::{LiveAccountStore, live_agent_snapshot_for_dispatch},
    memory::{delete_memories_for_agent, get_latest_agent_memory_by_type},
    opencode::{
        client::OpenCodeClient,
        coding_workspace::{changed_paths, manifest_hash},
        workspace::OpenCodeWorkspaceRuntimeConfig,
        workspace_control_client::{WorkspaceAgentInput, WorkspaceController},
    },
};

const SCHEDULER_POLL_INTERVAL: Duration = Duration::from_secs(10);
const OPENCODE_STARTUP_RETRY_INTERVAL: Duration = Duration::from_secs(5);
const ORPHAN_RECOVERY_INTERVAL: Duration = Duration::from_secs(60);
const DUE_SCHEDULE_LIMIT: i64 = 20;
const QUEUED_RUN_RESUME_LIMIT: i64 = 20;

/// Periodic background loop that claims due OpenCode jobs and
/// dispatches them through an [`HarnessBackend`].
///
/// The scheduler is generic over the backend so tests can swap in a
/// fake implementation. In production this is `OpenCodeBackend`.
///
/// CandleSubAgents for the same agent run in two independent lanes:
/// analysis-lane work (`analysis` plus `market_analysis` events) and
/// trading-lane work (`trading`). Different agents may also run
/// concurrently.
pub struct HarnessScheduler {
    pool: DbPool,
    shutdown_rx: watch::Receiver<bool>,
    force_shutdown_rx: watch::Receiver<bool>,
    backend: Arc<dyn HarnessBackend>,
    live_accounts: Arc<LiveAccountStore>,
    workspace_controller: Arc<dyn WorkspaceController>,
    agent_api_base_url: String,
    container_workspaces_root: String,
    opencode_client: Arc<OpenCodeClient>,
    last_orphan_recovery_at: Option<chrono::DateTime<Utc>>,
    in_flight: InFlightTracker,
    workspace_leases: WorkspaceLeaseManager,
    coding_semaphore: Arc<Semaphore>,
    lane_locks: CandleSubAgentrLaneLockManager,
}

pub struct HarnessSchedulerRuntime {
    pub workspace_controller: Arc<dyn WorkspaceController>,
    pub agent_api_base_url: String,
    pub container_workspaces_root: String,
    pub opencode_client: Arc<OpenCodeClient>,
    pub in_flight: InFlightTracker,
}

/// Serializes repeated scheduler ticks for one agent and lane. The run store
/// serializes individual claims, but a later tick must not claim a second due
/// candle_job while the earlier tick is still executing the first one.
#[derive(Clone, Default)]
struct CandleSubAgentrLaneLockManager {
    locks: LaneLockMap,
}

type LaneLockMap = Arc<Mutex<HashMap<(String, CandleSubAgentrLane), Weak<AsyncMutex<()>>>>>;

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
enum CandleSubAgentrLane {
    Analysis,
    Trading,
}

type AnalysisCandleSubAgentSortKey = (i64, chrono::DateTime<Utc>, i64);
type TradingCandleSubAgentSortKey = (chrono::DateTime<Utc>, i64, i64);
type IndexedAnalysisCandleSubAgent = (AnalysisCandleSubAgentSortKey, HarnessDispatchSubAgentRow);
type IndexedTradingCandleSubAgent = (TradingCandleSubAgentSortKey, HarnessDispatchSubAgentRow);

pub struct DispatchRequestInputs {
    pub run_id: i64,
    pub scheduled_for: chrono::DateTime<Utc>,
    pub agent: crate::agents::model::AgentDetailRow,
    pub selected_instruments: Vec<String>,
    pub strategy_prompt: String,
    pub accumulated_learnings: Option<String>,
    pub system_prompt: String,
}

impl CandleSubAgentrLaneLockManager {
    fn lock_for(&self, agent_key: &str, lane: CandleSubAgentrLane) -> Arc<AsyncMutex<()>> {
        let mut locks = self.locks.lock().expect("scheduler lane lock map poisoned");
        let key = (agent_key.to_string(), lane);
        if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
            return lock;
        }

        let lock = Arc::new(AsyncMutex::new(()));
        locks.insert(key, Arc::downgrade(&lock));
        lock
    }

    fn try_acquire(
        &self,
        agent_key: &str,
        lane: CandleSubAgentrLane,
    ) -> Option<OwnedMutexGuard<()>> {
        self.lock_for(agent_key, lane).try_lock_owned().ok()
    }
}

impl HarnessScheduler {
    #[cfg(test)]
    pub fn new(
        pool: DbPool,
        shutdown_rx: watch::Receiver<bool>,
        force_shutdown_rx: watch::Receiver<bool>,
        backend: Arc<dyn HarnessBackend>,
        live_accounts: Arc<LiveAccountStore>,
        runtime: HarnessSchedulerRuntime,
    ) -> Self {
        Self::new_with_workspace_leases(
            pool,
            shutdown_rx,
            force_shutdown_rx,
            backend,
            live_accounts,
            runtime,
            WorkspaceLeaseManager::new(),
        )
    }

    pub fn new_with_workspace_leases(
        pool: DbPool,
        shutdown_rx: watch::Receiver<bool>,
        force_shutdown_rx: watch::Receiver<bool>,
        backend: Arc<dyn HarnessBackend>,
        live_accounts: Arc<LiveAccountStore>,
        runtime: HarnessSchedulerRuntime,
        workspace_leases: WorkspaceLeaseManager,
    ) -> Self {
        Self {
            pool,
            shutdown_rx,
            force_shutdown_rx,
            backend,
            live_accounts,
            workspace_controller: runtime.workspace_controller,
            agent_api_base_url: runtime.agent_api_base_url,
            container_workspaces_root: runtime.container_workspaces_root,
            opencode_client: runtime.opencode_client,
            last_orphan_recovery_at: None,
            in_flight: runtime.in_flight,
            workspace_leases,
            coding_semaphore: Arc::new(Semaphore::new(1)),
            lane_locks: CandleSubAgentrLaneLockManager::default(),
        }
    }

    /// Run the scheduler until a shutdown signal is observed, then
    /// drain any in-flight dispatches before returning. Manual
    /// `Run now` requests from the web UI register with the same
    /// shared [`InFlightTracker`], so `main` calls
    /// `wait_idle_with_timeout` once more after this returns as a
    /// belt-and-suspenders check.
    ///
    /// The drain wait is raced against the `force_shutdown_rx` watch
    /// so a second shutdown signal (force) returns immediately
    /// instead of waiting for the 30-minute grace to elapse.
    pub async fn run(mut self) -> Result<()> {
        info!("harness scheduler starting");
        let mut promotion_recovery_pending = !*self.shutdown_rx.borrow();
        loop {
            if *self.shutdown_rx.borrow() {
                break;
            }

            if promotion_recovery_pending {
                match self.workspace_controller.recover_promotions().await {
                    Ok(journals) => {
                        let mut recovered = 0;
                        for journal in journals {
                            match journal.phase {
                                workspace_store::coding_workspace::PromotionJournalPhase::Completed => {
                                    let _ = store::mark_maintenance_task_succeeded(
                                        &self.pool,
                                        journal.task_id,
                                    )
                                    .await;
                                    recovered += 1;
                                }
                                workspace_store::coding_workspace::PromotionJournalPhase::RolledBack => {
                                    let _ = store::mark_maintenance_task_failed(
                                        &self.pool,
                                        journal.task_id,
                                        "promotion was rolled back during startup recovery",
                                    )
                                    .await;
                                    recovered += 1;
                                }
                                _ => {}
                            }
                        }
                        if recovered > 0 {
                            info!(recovered, "reconciled promotion journals at startup");
                        }
                        promotion_recovery_pending = false;
                    }
                    Err(error) => warn!(
                        error = ?error,
                        retry_in = ?OPENCODE_STARTUP_RETRY_INTERVAL,
                        "promotion journal recovery failed; will retry"
                    ),
                }
            }

            if let Err(error) = self.tick().await {
                warn!(error = ?error, "harness scheduler tick failed");
            }

            tokio::select! {
                _ = tokio::time::sleep(if promotion_recovery_pending {
                    OPENCODE_STARTUP_RETRY_INTERVAL
                } else {
                    SCHEDULER_POLL_INTERVAL
                }) => {}
                _ = self.shutdown_rx.changed() => break,
            }
        }

        let in_flight = self.in_flight.in_flight();
        if in_flight > 0 {
            info!(
                in_flight,
                grace_seconds = SHUTDOWN_IN_FLIGHT_GRACE.as_secs(),
                "waiting for in-flight harness dispatches to complete"
            );
            let mut force_rx = self.force_shutdown_rx.clone();
            let drained = tokio::select! {
                drained = self.in_flight.wait_idle_with_timeout(SHUTDOWN_IN_FLIGHT_GRACE) => drained,
                _ = async {
                    loop {
                        if *force_rx.borrow() { break; }
                        if force_rx.changed().await.is_err() { return; }
                    }
                } => false,
            };
            if !drained {
                warn!(
                    remaining = self.in_flight.in_flight(),
                    "in-flight harness dispatches did not drain; \
                     leaving them orphaned for the next start to recover"
                );
            } else {
                info!("all in-flight harness dispatches completed");
            }
        }

        info!("harness scheduler stopped");
        Ok(())
    }

    /// One scheduling pass: load due jobs, claim each, and run them in
    /// per-agent analysis/trading lanes.
    ///
    /// This is exposed (not just called from [`Self::run`]) so tests can
    /// drive a single tick deterministically.
    pub async fn tick(&mut self) -> Result<()> {
        if *self.shutdown_rx.borrow() {
            return Ok(());
        }

        self.maybe_recover_orphans().await?;

        process_workspace_maintenance_tasks(
            &self.pool,
            &self.workspace_controller,
            &self.agent_api_base_url,
            &self.opencode_client,
            &self.workspace_leases,
        )
        .await?;

        process_provider_config_reload_tasks(
            &self.pool,
            &self.opencode_client,
            self.opencode_client.base_url(),
        )
        .await?;

        spawn_coding_workers(
            &self.pool,
            &self.backend,
            &self.workspace_controller,
            &self.agent_api_base_url,
            &self.in_flight,
            &self.coding_semaphore,
            &self.workspace_leases,
            self.opencode_client.base_url(),
        )
        .await?;

        self.resume_queued_runs().await?;

        let now = Utc::now();
        let due = store::list_due_candle_sub_agents(
            &self.pool,
            now,
            DUE_SCHEDULE_LIMIT,
            self.opencode_client.base_url(),
        )
        .await?;
        debug!(count = due.len(), "due opencode jobs loaded");

        if due.is_empty() {
            return Ok(());
        }

        let mut by_agent: BTreeMap<String, Vec<HarnessDispatchSubAgentRow>> = BTreeMap::new();
        for candle_job in due {
            by_agent
                .entry(candle_job.agent_key.clone())
                .or_default()
                .push(candle_job);
        }

        for (agent_key, jobs) in by_agent {
            if *self.shutdown_rx.borrow() {
                break;
            }

            let mut analysis_jobs = Vec::new();
            let mut daily_review_jobs = Vec::new();
            let mut trading_jobs = Vec::new();
            for candle_job in jobs {
                match candle_job.sub_agent_kind.as_str() {
                    SUB_AGENT_KIND_ANALYSIS => analysis_jobs.push(candle_job),
                    SUB_AGENT_KIND_DAILY_REVIEW => daily_review_jobs.push(candle_job),
                    SUB_AGENT_KIND_TRADING => trading_jobs.push(candle_job),
                    _ => {}
                }
            }

            if (!analysis_jobs.is_empty() || !daily_review_jobs.is_empty())
                && let Some(lane_guard) = self
                    .lane_locks
                    .try_acquire(&agent_key, CandleSubAgentrLane::Analysis)
            {
                let pool = self.pool.clone();
                let backend = self.backend.clone();
                let live_accounts = self.live_accounts.clone();
                let agent_key = agent_key.clone();
                let in_flight = self.in_flight.clone();
                let workspace_leases = self.workspace_leases.clone();
                tokio::spawn(async move {
                    let _guard = in_flight.track();
                    let _lane_guard = lane_guard;
                    process_analysis_lane_for_agent(
                        &pool,
                        &backend,
                        &live_accounts,
                        &agent_key,
                        sort_analysis_jobs_for_dispatch(analysis_jobs),
                        sort_analysis_jobs_for_dispatch(daily_review_jobs),
                        &workspace_leases,
                    )
                    .await;
                });
            }

            if !trading_jobs.is_empty() {
                let Some(lane_guard) = self
                    .lane_locks
                    .try_acquire(&agent_key, CandleSubAgentrLane::Trading)
                else {
                    continue;
                };
                let pool = self.pool.clone();
                let backend = self.backend.clone();
                let live_accounts = self.live_accounts.clone();
                let agent_key = agent_key.clone();
                let in_flight = self.in_flight.clone();
                let workspace_leases = self.workspace_leases.clone();
                tokio::spawn(async move {
                    let _guard = in_flight.track();
                    let _lane_guard = lane_guard;
                    process_trading_lane_for_agent(
                        &pool,
                        &backend,
                        &live_accounts,
                        &agent_key,
                        sort_trading_jobs_for_dispatch(trading_jobs),
                        &workspace_leases,
                    )
                    .await;
                });
            }
        }

        Ok(())
    }

    /// Resume persisted runs whose previous in-memory dispatcher was interrupted.
    /// Each lane starts at most one run per tick; later queued runs wait for the
    /// earlier row to finish before the next scheduler pass resumes them.
    async fn resume_queued_runs(&self) -> Result<()> {
        for run in store::list_queued_runs_for_dispatch(&self.pool, QUEUED_RUN_RESUME_LIMIT).await?
        {
            let lane = match run.sub_agent_kind.as_str() {
                SUB_AGENT_KIND_TRADING => CandleSubAgentrLane::Trading,
                SUB_AGENT_KIND_ANALYSIS
                | SUB_AGENT_KIND_MARKET_ANALYSIS
                | SUB_AGENT_KIND_DAILY_REVIEW => CandleSubAgentrLane::Analysis,
                _ => continue,
            };
            let Some(lane_guard) = self.lane_locks.try_acquire(&run.agent_key, lane) else {
                continue;
            };
            let pool = self.pool.clone();
            let backend = self.backend.clone();
            let live_accounts = self.live_accounts.clone();
            let workspace_leases = self.workspace_leases.clone();
            let opencode_base_url = self.opencode_client.base_url().to_string();
            let in_flight = self.in_flight.clone();
            tokio::spawn(async move {
                let _guard = in_flight.track();
                let _lane_guard = lane_guard;
                resume_queued_run(
                    &pool,
                    &backend,
                    &live_accounts,
                    &workspace_leases,
                    &opencode_base_url,
                    run,
                )
                .await;
            });
        }
        Ok(())
    }

    /// Periodically sweep every agent's `harness_sub_agent_runs` for orphans and
    /// mark them with a terminal status. The per-lane recovery in
    /// [`store::recovery::recover_inactive_runs_in_lane_tx`] only fires
    /// when a new run is claimed for the same lane, so a `running` run
    /// whose dispatch worker has died would otherwise block its lane
    /// until something else claimed the candle_job. This periodic sweep
    /// is throttled to `ORPHAN_RECOVERY_INTERVAL` to bound the work
    /// done per tick.
    async fn maybe_recover_orphans(&mut self) -> Result<()> {
        let now = Utc::now();
        if let Some(last) = self.last_orphan_recovery_at {
            let elapsed = now
                .signed_duration_since(last)
                .to_std()
                .unwrap_or(ORPHAN_RECOVERY_INTERVAL);
            if elapsed < ORPHAN_RECOVERY_INTERVAL {
                return Ok(());
            }
        }

        let recovered = store::recover_inactive_runs_all(&self.pool, now).await?;
        if recovered > 0 {
            info!(
                recovered,
                "recovered inactive harness runs during periodic sweep"
            );
        }
        let stale_before = now - chrono::Duration::minutes(2);
        for task in
            store::list_stale_running_maintenance_tasks(&self.pool, stale_before, 20).await?
        {
            if task.task_kind != crate::harness::model::MAINTENANCE_TASK_KIND_ANALYSIS_CODING {
                continue;
            }
            let candidate_directory = format!(
                "{}/coding/{}/{}/workspace",
                self.container_workspaces_root.trim_end_matches('/'),
                task.agent_key,
                task.id
            );
            if let Some(run_id) = task.run_id
                && let Some(run) = store::get_run(&self.pool, run_id).await?
                && coding_run_exceeded_timeout(run.started_at, run.timeout_seconds, now)
            {
                let mut summary =
                    format!("coding run exceeded timeout of {}s", run.timeout_seconds);
                if let Some(session_id) = run.backend_run_ref
                    && let Some(sub_agent_id) = task.sub_agent_id
                    && let Some(event) = store::get_dispatch_sub_agent(
                        &self.pool,
                        &task.agent_key,
                        sub_agent_id,
                        self.opencode_client.base_url(),
                    )
                    .await?
                {
                    match self
                        .backend
                        .abort_session(&event.opencode_base_url, &session_id)
                        .await
                    {
                        Ok(true) => summary.push_str("; OpenCode session aborted"),
                        Ok(false) => summary.push_str("; OpenCode declined session abort"),
                        Err(error) => {
                            warn!(task_id = task.id, session_id, error = ?error, "failed to abort overdue coding session");
                            summary.push_str("; failed to abort OpenCode session");
                        }
                    }
                }
                warn!(task_id = task.id, run_id, summary = %summary, "recovering overdue coding task");
                store::mark_maintenance_task_failed(&self.pool, task.id, &summary).await?;
                store::mark_run_failed(&self.pool, run_id, &summary, None).await?;
                continue;
            }
            if task.phase == crate::harness::model::MAINTENANCE_PHASE_GENERATING
                && let Some(run_id) = task.run_id
                && let Some(run) = store::get_run(&self.pool, run_id).await?
                && let Some(session_id) = run.backend_run_ref
                && let Some(sub_agent_id) = task.sub_agent_id
                && let Some(event) = store::get_dispatch_sub_agent(
                    &self.pool,
                    &task.agent_key,
                    sub_agent_id,
                    self.opencode_client.base_url(),
                )
                .await?
                && let Some(status) = self
                    .backend
                    .get_session_status_in_directory(
                        &event.opencode_base_url,
                        &session_id,
                        Some(&candidate_directory),
                    )
                    .await?
                && status.is_active()
            {
                debug!(task_id = task.id, session_id = %session_id, "coding session remains active during stale-task sweep");
                continue;
            }
            warn!(task_id = task.id, phase = %task.phase, "recovering stale coding task");
            if task.is_in_promotion_window() {
                let recovered = self.workspace_controller.recover_promotions().await?;
                if let Some(journal) = recovered
                    .into_iter()
                    .find(|item| item.agent_key == task.agent_key && item.task_id == task.id)
                {
                    match journal.phase {
                        workspace_store::coding_workspace::PromotionJournalPhase::Completed => {
                            store::mark_maintenance_task_succeeded(&self.pool, task.id).await?;
                            if let Some(run_id) = task.run_id {
                                store::mark_run_succeeded(&self.pool, run_id, None).await?;
                            }
                            continue;
                        }
                        workspace_store::coding_workspace::PromotionJournalPhase::RolledBack => {
                            // Fall through to terminal failure after restoring the
                            // previous live tree.
                        }
                        _ => {}
                    }
                }
            }
            let summary = format!("stale coding task recovered during {} phase", task.phase);
            store::mark_maintenance_task_failed(&self.pool, task.id, &summary).await?;
            if let Some(run_id) = task.run_id {
                store::mark_run_failed(&self.pool, run_id, &summary, None).await?;
            }
        }
        let requeued =
            store::requeue_stale_provider_config_reload_tasks(&self.pool, stale_before).await?;
        if requeued > 0 {
            warn!(requeued, "requeued stale provider config reload tasks");
        }
        self.last_orphan_recovery_at = Some(now);
        Ok(())
    }
}

async fn resume_queued_run(
    pool: &DbPool,
    backend: &Arc<dyn HarnessBackend>,
    live_accounts: &Arc<LiveAccountStore>,
    workspace_leases: &WorkspaceLeaseManager,
    opencode_base_url: &str,
    queued_run: store::QueuedRunForDispatch,
) {
    let run_id = queued_run.run_id;
    let has_prior = match store::has_prior_active_run_in_lane(
        pool,
        &queued_run.agent_key,
        &queued_run.sub_agent_kind,
        run_id,
    )
    .await
    {
        Ok(has_prior) => has_prior,
        Err(error) => {
            warn!(run_id, error = ?error, "failed to check queued run lane before resume");
            return;
        }
    };
    if has_prior {
        return;
    }

    let dispatch_job = match store::get_dispatch_sub_agent(
        pool,
        &queued_run.agent_key,
        queued_run.sub_agent_id,
        opencode_base_url,
    )
    .await
    {
        Ok(Some(job)) => job,
        Ok(None) => {
            let _ = store::mark_run_failed(
                pool,
                run_id,
                "queued run dispatch context is unavailable",
                None,
            )
            .await;
            return;
        }
        Err(error) => {
            warn!(run_id, error = ?error, "failed to load queued run dispatch context");
            return;
        }
    };
    let request = match build_dispatch_request(
        pool,
        live_accounts,
        &dispatch_job,
        run_id,
        queued_run.scheduled_for,
    )
    .await
    {
        Ok(Some(request)) => request,
        Ok(None) => {
            let _ = store::mark_run_failed(
                pool,
                run_id,
                "no currencies selected for agent; job skipped",
                None,
            )
            .await;
            return;
        }
        Err(error) => {
            warn!(run_id, error = ?error, "failed to build queued run dispatch request");
            let _ = store::mark_run_failed(pool, run_id, "dispatch request errored", None).await;
            return;
        }
    };
    let result =
        dispatch_run_with_workspace_lease(pool.clone(), backend.clone(), request, workspace_leases)
            .await;
    if queued_run.sub_agent_kind == SUB_AGENT_KIND_ANALYSIS
        && result.succeeded
        && let Err(error) = dispatch_analysis_batch_completed_event(
            pool,
            backend,
            live_accounts,
            &queued_run.agent_key,
            workspace_leases,
            opencode_base_url,
        )
        .await
    {
        warn!(run_id, error = ?error, "failed to dispatch queued analysis follow-up");
    }
    if queued_run.sub_agent_kind == SUB_AGENT_KIND_DAILY_REVIEW
        && result.succeeded
        && let Err(error) =
            dispatch_daily_review_coding_event(pool, &queued_run.agent_key, run_id).await
    {
        warn!(run_id, error = ?error, "failed to dispatch queued daily-review follow-up");
    }
}

async fn process_workspace_maintenance_tasks(
    pool: &DbPool,
    workspace_controller: &Arc<dyn WorkspaceController>,
    agent_api_base_url: &str,
    opencode_client: &Arc<OpenCodeClient>,
    workspace_leases: &WorkspaceLeaseManager,
) -> Result<()> {
    let Some(task) = store::get_next_queued_workspace_regenerate_task(pool).await? else {
        return Ok(());
    };

    let Some(agent) = get_agent(pool, &task.agent_key).await? else {
        let _ = store::mark_maintenance_task_failed(
            pool,
            task.id,
            "agent disappeared before maintenance",
        )
        .await;
        return Ok(());
    };

    if store::agent_has_active_runs(pool, &task.agent_key).await? {
        debug!(
            task_id = task.id,
            agent_key = %task.agent_key,
            status = MAINTENANCE_STATUS_QUEUED,
            "workspace maintenance remains queued while agent runs are active"
        );
        return Ok(());
    }

    let Some(workspace_runtime) = OpenCodeWorkspaceRuntimeConfig::from_value(&agent.runtime_config)
    else {
        let _ = store::mark_maintenance_task_failed(
            pool,
            task.id,
            "agent is missing OpenCode workspace metadata",
        )
        .await;
        return Ok(());
    };

    let active_sessions = crate::opencode::store::count_active_opencode_sessions_for_directory(
        pool,
        &workspace_runtime.workspace_container_path,
    )
    .await?;
    if active_sessions > 0 {
        debug!(
            task_id = task.id,
            agent_key = %task.agent_key,
            active_sessions,
            "workspace maintenance remains queued while an OpenCode session is active"
        );
        return Ok(());
    }

    if !store::mark_maintenance_task_running(pool, task.id).await? {
        debug!(
            task_id = task.id,
            agent_key = %task.agent_key,
            "workspace maintenance task was claimed concurrently before execution"
        );
        return Ok(());
    }

    let hard_reset = task.parameter_bool("hard_reset");
    let reset_memories = task.parameter_bool("reset_memories");
    let maintenance_result = async {
        let _lease = workspace_leases.acquire_live_write(&agent.agent_key).await;
        let workspace_agent = WorkspaceAgentInput {
            agent_key: agent.agent_key.clone(),
            display_name: agent.display_name.clone(),
            agent_api_key: agent.api_key.clone(),
            api_base_url: agent_api_base_url.to_string(),
        };

        if hard_reset {
            let _ = workspace_controller
                .delete_workspace(
                    &agent.agent_key,
                    &format!("maintenance:{}:hard-reset", task.id),
                )
                .await?;
        }

        let generated = workspace_controller
            .create_workspace(
                workspace_agent,
                true,
                &format!("maintenance:{}:regenerate", task.id),
            )
            .await?;
        let runtime_config = OpenCodeWorkspaceRuntimeConfig {
            workspace_container_path: generated.workspace_container_path.clone(),
            profile_source: generated.profile_source,
        }
        .into_value();
        if !update_agent_runtime_config(pool, &agent.agent_key, runtime_config).await? {
            anyhow::bail!("agent disappeared before workspace metadata update");
        }
        opencode_client
            .dispose_workspace_instance(
                opencode_client.base_url(),
                &generated.workspace_container_path,
            )
            .await?;
        if reset_memories {
            delete_memories_for_agent(pool, &agent.agent_key).await?;
        }

        Ok::<(), anyhow::Error>(())
    }
    .await;

    match maintenance_result {
        Ok(()) => {
            store::mark_maintenance_task_succeeded(pool, task.id).await?;
            info!(
                task_id = task.id,
                agent_key = %task.agent_key,
                hard_reset,
                reset_memories,
                "workspace maintenance completed"
            );
        }
        Err(error) => {
            error!(
                task_id = task.id,
                agent_key = %task.agent_key,
                hard_reset,
                reset_memories,
                error = ?error,
                "workspace maintenance failed"
            );
            let summary = maintenance_error_summary(&error);
            store::mark_maintenance_task_failed(pool, task.id, &summary).await?;
        }
    }

    Ok(())
}

/// Process a queued `provider_config_reload` maintenance task. The task
/// disposes all OpenCode instances so newly stored (or removed) provider
/// credentials are reflected in the `/provider` response. It waits until
/// no OpenCode sessions are active (`busy`/`retry`) to avoid interrupting
/// in-flight agent work, then calls `POST /global/dispose`.
async fn process_provider_config_reload_tasks(
    pool: &DbPool,
    opencode_client: &Arc<OpenCodeClient>,
    opencode_base_url: &str,
) -> Result<()> {
    let Some(task) = store::get_next_queued_provider_config_reload_task(pool).await? else {
        return Ok(());
    };

    let active = crate::opencode::store::count_active_opencode_sessions(pool).await?;
    if active > 0 {
        debug!(
            task_id = task.id,
            active_sessions = active,
            "provider config reload remains queued while OpenCode sessions are active"
        );
        return Ok(());
    }

    if !store::mark_maintenance_task_running(pool, task.id).await? {
        debug!(
            task_id = task.id,
            "provider config reload task was claimed concurrently before execution"
        );
        return Ok(());
    }

    match opencode_client.dispose_instances(opencode_base_url).await {
        Ok(()) => {
            opencode_client.invalidate_provider_cache().await;
            store::mark_maintenance_task_succeeded(pool, task.id).await?;
            info!(task_id = task.id, "provider config reload completed");
        }
        Err(error) => {
            error!(task_id = task.id, error = ?error, "provider config reload failed");
            let summary = maintenance_error_summary(&error);
            store::mark_maintenance_task_failed(pool, task.id, &summary).await?;
        }
    }

    Ok(())
}

const CODING_QUEUE_LIMIT: i64 = 4;
const CODING_DISPATCH_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);

#[allow(clippy::too_many_arguments)]
async fn spawn_coding_workers(
    pool: &DbPool,
    backend: &Arc<dyn HarnessBackend>,
    workspace_controller: &Arc<dyn WorkspaceController>,
    agent_api_base_url: &str,
    in_flight: &InFlightTracker,
    semaphore: &Arc<Semaphore>,
    workspace_leases: &WorkspaceLeaseManager,
    opencode_base_url: &str,
) -> Result<()> {
    let tasks = store::list_queued_maintenance_candidates(pool, CODING_QUEUE_LIMIT).await?;
    for task in tasks {
        if task.task_kind != crate::harness::model::MAINTENANCE_TASK_KIND_ANALYSIS_CODING {
            continue;
        }
        let pool = pool.clone();
        let backend = backend.clone();
        let workspace_controller = workspace_controller.clone();
        let agent_api_base_url = agent_api_base_url.to_string();
        let in_flight = in_flight.clone();
        let semaphore = semaphore.clone();
        let workspace_leases = workspace_leases.clone();
        let opencode_base_url = opencode_base_url.to_string();
        tokio::spawn(async move {
            let Ok(_permit) = semaphore.acquire_owned().await else {
                return;
            };
            let _guard = in_flight.track();
            if let Err(error) = run_coding_task(
                &pool,
                &backend,
                &workspace_controller,
                &agent_api_base_url,
                &workspace_leases,
                &opencode_base_url,
                task,
            )
            .await
            {
                warn!(error = ?error, "coding task worker failed");
            }
        });
    }
    Ok(())
}

async fn run_coding_task(
    pool: &DbPool,
    backend: &Arc<dyn HarnessBackend>,
    workspace_controller: &Arc<dyn WorkspaceController>,
    agent_api_base_url: &str,
    workspace_leases: &WorkspaceLeaseManager,
    opencode_base_url: &str,
    task: crate::harness::model::AgentMaintenanceTaskRow,
) -> Result<()> {
    let Some(run_id) = task.run_id else {
        store::mark_maintenance_task_failed(pool, task.id, "coding task has no run").await?;
        return Ok(());
    };
    if !store::mark_maintenance_task_running(pool, task.id).await? {
        return Ok(());
    }
    let Some(agent) = get_agent(pool, &task.agent_key).await? else {
        fail_coding_task(pool, task.id, run_id, "agent disappeared before coding").await?;
        return Ok(());
    };
    let Some(sub_agent_id) = task.sub_agent_id else {
        fail_coding_task(pool, task.id, run_id, "coding task has no job").await?;
        return Ok(());
    };
    let Some(event) =
        store::get_dispatch_sub_agent(pool, &task.agent_key, sub_agent_id, opencode_base_url)
            .await?
    else {
        fail_coding_task(pool, task.id, run_id, "coding event disappeared").await?;
        return Ok(());
    };

    let existing_candidate = workspace_controller
        .inspect_candidate(&task.agent_key, task.id)
        .await
        .ok();
    let canonical_exists = existing_candidate
        .as_ref()
        .is_some_and(|candidate| candidate.manifest.contains_key("analyze.py"));
    let requested_mode = task
        .parameters
        .get("mode")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("auto");
    let effective_mode = match requested_mode {
        "auto" if canonical_exists => "manual_improvement",
        "auto" => "bootstrap",
        "bootstrap" => "bootstrap",
        "manual_improvement" if canonical_exists => "manual_improvement",
        "manual_improvement" => {
            fail_coding_task(
                pool,
                task.id,
                run_id,
                "manual improvement requires scripts/user/analyze.py",
            )
            .await?;
            return Ok(());
        }
        _ => {
            fail_coding_task(pool, task.id, run_id, "invalid coding mode").await?;
            return Ok(());
        }
    };

    let candidate = match workspace_controller
        .create_candidate(
            WorkspaceAgentInput {
                agent_key: task.agent_key.clone(),
                display_name: agent.display_name.clone(),
                agent_api_key: agent.api_key.clone(),
                api_base_url: agent_api_base_url.to_string(),
            },
            task.id,
        )
        .await
    {
        Ok(candidate) => candidate,
        Err(error) => {
            fail_coding_task(pool, task.id, run_id, &error.to_string()).await?;
            return Ok(());
        }
    };
    store::compare_and_set_maintenance_phase(
        pool,
        task.id,
        crate::harness::model::MAINTENANCE_PHASE_PREPARING,
        crate::harness::model::MAINTENANCE_PHASE_GENERATING,
    )
    .await?;

    let Some(mut request) = build_event_dispatch_request(pool, &event, run_id, Utc::now()).await?
    else {
        fail_coding_task(pool, task.id, run_id, "coding request could not be built").await?;
        return Ok(());
    };
    let analysis_strategy_prompt =
        get_agent_strategy_prompt(pool, &task.agent_key, PROMPT_KIND_ANALYSIS)
            .await?
            .map(|row| row.prompt);
    request.runtime_config = serde_json::json!({
        "workspace_container_path": candidate.workspace_container_path,
        "profile_source": "agent-runtime/workspace-template",
        "coding_task_id": task.id,
        "coding_mode": effective_mode,
        "analysis_strategy_prompt": analysis_strategy_prompt,
    });
    let outcome = dispatch_coding_model(pool, backend.clone(), request, task.id).await?;
    if !outcome.succeeded {
        fail_coding_task(
            pool,
            task.id,
            run_id,
            outcome
                .failure_summary
                .as_deref()
                .unwrap_or("coding model dispatch failed"),
        )
        .await?;
        return Ok(());
    }

    let inspection = workspace_controller
        .inspect_candidate(&task.agent_key, task.id)
        .await?;
    let candidate_manifest = inspection.manifest;
    let actual_changes = changed_paths(&candidate.base_manifest, &candidate_manifest);
    let report = match inspection.report.as_ref() {
        Some(report) => match validate_coding_report(report, &actual_changes) {
            Ok(outcome) => outcome,
            Err(error) => {
                fail_coding_task(pool, task.id, run_id, &error.to_string()).await?;
                return Ok(());
            }
        },
        None => {
            fail_coding_task(
                pool,
                task.id,
                run_id,
                "coding model did not submit a report",
            )
            .await?;
            return Ok(());
        }
    };
    if report.outcome == "no_change" && !actual_changes.is_empty()
        || report.outcome == "changed" && actual_changes.is_empty()
    {
        fail_coding_task(
            pool,
            task.id,
            run_id,
            "coding report outcome does not match candidate diff",
        )
        .await?;
        return Ok(());
    }
    if report.outcome == "no_change" {
        store::mark_maintenance_task_succeeded(pool, task.id).await?;
        if let Err(error) = write_coding_result_memory(
            pool,
            &task,
            CodingResultMemory {
                outcome: "no_change",
                summary: "Candidate produced no reusable code changes",
                changed_paths: &actual_changes,
                report: &report,
                base_manifest_hash: &manifest_hash(&candidate.base_manifest),
                promoted_manifest_hash: &manifest_hash(&candidate_manifest),
            },
        )
        .await
        {
            fail_coding_task(
                pool,
                task.id,
                run_id,
                &format!("result memory failed: {error:#}"),
            )
            .await?;
            return Ok(());
        }
        store::mark_run_succeeded(pool, run_id, None).await?;
        let _ = workspace_controller
            .delete_candidate(&task.agent_key, task.id)
            .await;
    } else {
        store::compare_and_set_maintenance_phase(
            pool,
            task.id,
            crate::harness::model::MAINTENANCE_PHASE_GENERATING,
            crate::harness::model::MAINTENANCE_PHASE_VALIDATING,
        )
        .await?;
        let candidate_manifest_hash = manifest_hash(&candidate_manifest);
        let validation_result = inspection
            .validation
            .as_ref()
            .map(|validation| {
                require_coding_validation(validation, task.id, &candidate_manifest_hash)
            })
            .unwrap_or_else(|| {
                Err(anyhow!(
                    "coding model did not run fixed candidate validation"
                ))
            });
        if let Err(error) = validation_result {
            fail_coding_task(pool, task.id, run_id, &error.to_string()).await?;
            return Ok(());
        }
        store::compare_and_set_maintenance_phase(
            pool,
            task.id,
            crate::harness::model::MAINTENANCE_PHASE_VALIDATING,
            crate::harness::model::MAINTENANCE_PHASE_WAITING_FOR_PROMOTION,
        )
        .await?;
        if store::agent_has_active_live_runs(pool, &task.agent_key).await? {
            fail_coding_task(pool, task.id, run_id, "live runs remain active").await?;
            return Ok(());
        }
        let _lease = workspace_leases.acquire_live_write(&task.agent_key).await;
        if store::agent_has_active_live_runs(pool, &task.agent_key).await? {
            fail_coding_task(pool, task.id, run_id, "live runs started before promotion").await?;
            return Ok(());
        }
        store::compare_and_set_maintenance_phase(
            pool,
            task.id,
            crate::harness::model::MAINTENANCE_PHASE_WAITING_FOR_PROMOTION,
            crate::harness::model::MAINTENANCE_PHASE_PROMOTING,
        )
        .await?;
        let promotion = match workspace_controller
            .promote_candidate(
                &task.agent_key,
                task.id,
                candidate.base_manifest.clone(),
                candidate_manifest_hash.clone(),
            )
            .await
        {
            Ok(result) => result,
            Err(error) => {
                fail_coding_task(pool, task.id, run_id, &error.to_string()).await?;
                return Ok(());
            }
        };
        store::compare_and_set_maintenance_phase(
            pool,
            task.id,
            crate::harness::model::MAINTENANCE_PHASE_PROMOTING,
            crate::harness::model::MAINTENANCE_PHASE_SMOKE_TESTING,
        )
        .await?;
        let promoted_manifest_hash = promotion.manifest_hash;
        store::mark_maintenance_task_succeeded(pool, task.id).await?;
        if let Err(error) = write_coding_result_memory(
            pool,
            &task,
            CodingResultMemory {
                outcome: "changed",
                summary: "Candidate validated and was promoted",
                changed_paths: &actual_changes,
                report: &report,
                base_manifest_hash: &manifest_hash(&candidate.base_manifest),
                promoted_manifest_hash: &promoted_manifest_hash,
            },
        )
        .await
        {
            fail_coding_task(
                pool,
                task.id,
                run_id,
                &format!("result memory failed: {error:#}"),
            )
            .await?;
            return Ok(());
        }
        store::mark_run_succeeded(pool, run_id, None).await?;
        let _ = workspace_controller
            .delete_candidate(&task.agent_key, task.id)
            .await;
    }
    Ok(())
}

async fn fail_coding_task(pool: &DbPool, task_id: i64, run_id: i64, summary: &str) -> Result<()> {
    store::mark_maintenance_task_failed(pool, task_id, summary).await?;
    store::mark_run_failed(pool, run_id, summary, None).await?;
    Ok(())
}

async fn dispatch_coding_model(
    pool: &DbPool,
    backend: Arc<dyn HarnessBackend>,
    request: DispatchRequest,
    task_id: i64,
) -> Result<CodingDispatchResult> {
    store::mark_run_running(pool, request.run_id, None).await?;
    let dispatch = dispatch_with_timeout_for_coding(pool, Arc::clone(&backend), request);
    tokio::pin!(dispatch);
    let mut heartbeat = tokio::time::interval(CODING_DISPATCH_HEARTBEAT_INTERVAL);
    let outcome = loop {
        tokio::select! {
            result = &mut dispatch => break result?,
            _ = heartbeat.tick() => {
                let _ = store::heartbeat_maintenance_task(pool, task_id).await;
            }
        }
    };
    match outcome {
        DispatchOutcome::Succeeded { backend_run_ref } => {
            debug!(task_id, backend_run_ref, "coding model dispatch finished");
            Ok(CodingDispatchResult {
                succeeded: true,
                failure_summary: None,
            })
        }
        DispatchOutcome::Cancelled => Ok(CodingDispatchResult {
            succeeded: false,
            failure_summary: Some("coding run was cancelled".to_string()),
        }),
        DispatchOutcome::Failed { summary } => {
            debug!(task_id, summary, "coding model dispatch failed");
            Ok(CodingDispatchResult {
                succeeded: false,
                failure_summary: Some(summary),
            })
        }
    }
}

struct CodingDispatchResult {
    succeeded: bool,
    failure_summary: Option<String>,
}

fn coding_run_exceeded_timeout(
    started_at: Option<chrono::DateTime<Utc>>,
    timeout_seconds: i32,
    now: chrono::DateTime<Utc>,
) -> bool {
    started_at.is_some_and(|started_at| {
        started_at + chrono::Duration::seconds(i64::from(timeout_seconds.max(0))) <= now
    })
}

struct CodingReport {
    outcome: String,
    summary: String,
    rationale: String,
    validation_notes: String,
    evidence_memory_ids: Vec<uuid::Uuid>,
}

fn require_coding_validation(
    validation: &serde_json::Value,
    task_id: i64,
    candidate_manifest_hash: &str,
) -> Result<()> {
    if validation
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        != Some(1)
        || validation
            .get("task_id")
            .and_then(serde_json::Value::as_i64)
            != Some(task_id)
    {
        anyhow::bail!("coding validation identity is invalid");
    }
    if validation.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
        anyhow::bail!("coding candidate failed fixed validation");
    }
    if validation
        .get("candidate_manifest_sha256")
        .and_then(serde_json::Value::as_str)
        != Some(candidate_manifest_hash)
    {
        anyhow::bail!("coding candidate changed after fixed validation");
    }
    Ok(())
}

fn validate_coding_report(
    report: &serde_json::Value,
    actual_changes: &[String],
) -> Result<CodingReport> {
    if report
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        != Some(1)
    {
        anyhow::bail!("coding report schema version is invalid");
    }
    let outcome = report
        .get("outcome")
        .and_then(serde_json::Value::as_str)
        .context("coding report has no outcome")?;
    if !matches!(outcome, "changed" | "no_change") {
        anyhow::bail!("coding report outcome is invalid");
    }
    let mut reported_changes: Vec<String> = report
        .get("changed_paths")
        .and_then(serde_json::Value::as_array)
        .context("coding report has no changed_paths")?
        .iter()
        .map(|path| path.as_str().map(ToString::to_string))
        .collect::<Option<Vec<_>>>()
        .context("coding report changed_paths are invalid")?;
    reported_changes.sort();
    if reported_changes != actual_changes {
        anyhow::bail!("coding report paths do not match candidate diff");
    }
    let evidence_memory_ids = report
        .get("evidence_memory_ids")
        .and_then(serde_json::Value::as_array)
        .context("coding report has no evidence_memory_ids")?
        .iter()
        .map(|id| {
            let value = id
                .as_str()
                .context("coding evidence memory id is not a string")?;
            uuid::Uuid::parse_str(value).context("coding evidence memory id is invalid")
        })
        .collect::<Result<Vec<_>>>()?;
    let summary = report
        .get("summary")
        .and_then(serde_json::Value::as_str)
        .context("coding report has no summary")?;
    let rationale = report
        .get("rationale")
        .and_then(serde_json::Value::as_str)
        .context("coding report has no rationale")?;
    let validation_notes = report
        .get("validation_notes")
        .and_then(serde_json::Value::as_str)
        .context("coding report has no validation_notes")?;
    for (name, value) in [
        ("summary", summary),
        ("rationale", rationale),
        ("validation_notes", validation_notes),
    ] {
        if value.chars().count() > 4_000 {
            anyhow::bail!("coding report {name} is too long");
        }
    }
    Ok(CodingReport {
        outcome: outcome.to_string(),
        summary: summary.to_string(),
        rationale: rationale.to_string(),
        validation_notes: validation_notes.to_string(),
        evidence_memory_ids,
    })
}

struct CodingResultMemory<'a> {
    outcome: &'a str,
    summary: &'a str,
    changed_paths: &'a [String],
    report: &'a CodingReport,
    base_manifest_hash: &'a str,
    promoted_manifest_hash: &'a str,
}

async fn write_coding_result_memory(
    pool: &DbPool,
    task: &crate::harness::model::AgentMaintenanceTaskRow,
    result: CodingResultMemory<'_>,
) -> Result<()> {
    let existing: (bool,) = sqlx::query_as(
        "SELECT EXISTS (
             SELECT 1 FROM memory.records
              WHERE agent_key = $1
                AND memory_type = 'analysis_coding'
                AND metadata->>'task_id' = $2
         )",
    )
    .bind(&task.agent_key)
    .bind(task.id.to_string())
    .fetch_one(pool)
    .await
    .context("failed to check coding result memory idempotency")?;
    if existing.0 {
        return Ok(());
    }
    let mut links = task
        .source_memory_id
        .map(|id| {
            vec![crate::memory::CreateMemoryLink {
                target_memory_id: id,
                link_type: "responds_to".to_string(),
                metadata: Some(serde_json::json!({ "task_id": task.id })),
            }]
        })
        .unwrap_or_default();
    links.extend(result.report.evidence_memory_ids.iter().map(|id| {
        crate::memory::CreateMemoryLink {
            target_memory_id: *id,
            link_type: "derived_from".to_string(),
            metadata: Some(serde_json::json!({ "task_id": task.id })),
        }
    }));
    let input = crate::memory::CreateMemory {
        symbol: "__agent__".to_string(),
        timeframe: None,
        memory_type: "analysis_coding".to_string(),
        summary: result.summary.to_string(),
        content: format!(
            "Coding task {} finished with outcome {}.\n\nModel summary: {}\n\nRationale: {}\n\nValidation notes: {}",
            task.id,
            result.outcome,
            result.report.summary,
            result.report.rationale,
            result.report.validation_notes
        ),
        metadata: Some(serde_json::json!({
            "schema_version": 1,
            "task_id": task.id,
            "run_id": task.run_id,
            "source_sub_agent_run_id": task.source_sub_agent_run_id,
            "source_memory_id": task.source_memory_id,
            "source_daily_review_run_id": task.source_sub_agent_run_id,
            "source_daily_review_memory_id": task.source_memory_id,
            "outcome": result.outcome,
            "mode": task.parameters.get("mode"),
            "changed_paths": result.changed_paths,
            "base_manifest_sha256": result.base_manifest_hash,
            "promoted_manifest_sha256": result.promoted_manifest_hash,
            "validation": if result.outcome == "changed" {
                serde_json::json!({
                    "fixed_contract": "passed",
                    "candidate_tests": "optional",
                    "promotion_hash": "passed"
                })
            } else {
                serde_json::json!({ "fixed_contract": "not_run" })
            },
        })),
        links: Some(links),
    };
    crate::memory::insert_memory(pool, &task.agent_key, &input).await?;
    Ok(())
}

fn maintenance_error_summary(error: &anyhow::Error) -> String {
    let summary = error.root_cause().to_string();
    if summary.trim().is_empty() {
        "workspace maintenance failed".to_string()
    } else {
        summary
    }
}

async fn process_analysis_lane_for_agent(
    pool: &DbPool,
    backend: &Arc<dyn HarnessBackend>,
    live_accounts: &Arc<LiveAccountStore>,
    agent_key: &str,
    jobs: Vec<HarnessDispatchSubAgentRow>,
    daily_review_jobs: Vec<HarnessDispatchSubAgentRow>,
    workspace_leases: &WorkspaceLeaseManager,
) {
    let opencode_base_url = jobs
        .first()
        .or_else(|| daily_review_jobs.first())
        .map(|candle_job| candle_job.opencode_base_url.clone())
        .unwrap_or_else(|| "http://localhost:14096".to_string());
    let _lease = workspace_leases.acquire_live_read(agent_key).await;
    let mut any_succeeded = false;
    for candle_job in jobs {
        if process_candle_job_for_agent(
            pool,
            backend,
            live_accounts,
            agent_key,
            candle_job,
            workspace_leases,
        )
        .await
        .is_some()
        {
            any_succeeded = true;
        }
    }

    if any_succeeded
        && let Err(error) = dispatch_analysis_batch_completed_event(
            pool,
            backend,
            live_accounts,
            agent_key,
            workspace_leases,
            &opencode_base_url,
        )
        .await
    {
        warn!(agent_key, error = ?error, "failed to dispatch analysis batch completed event");
    }

    for candle_job in daily_review_jobs {
        if let Some(run_id) = process_candle_job_for_agent(
            pool,
            backend,
            live_accounts,
            agent_key,
            candle_job,
            workspace_leases,
        )
        .await
            && let Err(error) = dispatch_daily_review_coding_event(pool, agent_key, run_id).await
        {
            warn!(agent_key, error = ?error, "failed to process daily-review coding request");
        }
    }
}

async fn process_trading_lane_for_agent(
    pool: &DbPool,
    backend: &Arc<dyn HarnessBackend>,
    live_accounts: &Arc<LiveAccountStore>,
    agent_key: &str,
    jobs: Vec<HarnessDispatchSubAgentRow>,
    workspace_leases: &WorkspaceLeaseManager,
) {
    let _lease = workspace_leases.acquire_live_read(agent_key).await;
    for candle_job in jobs {
        let _ = process_candle_job_for_agent(
            pool,
            backend,
            live_accounts,
            agent_key,
            candle_job,
            workspace_leases,
        )
        .await;
    }
}

async fn process_candle_job_for_agent(
    pool: &DbPool,
    backend: &Arc<dyn HarnessBackend>,
    live_accounts: &Arc<LiveAccountStore>,
    agent_key: &str,
    candle_job: HarnessDispatchSubAgentRow,
    _workspace_leases: &WorkspaceLeaseManager,
) -> Option<i64> {
    let sub_agent_id = candle_job.sub_agent_id;
    let sub_agent_key = candle_job.sub_agent_key.clone();
    let scheduled_for = boundary_for_due_at(
        candle_job
            .next_run_at
            .expect("due candle job has next_run_at"),
        candle_job
            .trigger_delay_seconds
            .expect("due candle job has trigger delay"),
    );

    let claim = match store::claim_due_candle_sub_agent(pool, sub_agent_id, Utc::now()).await {
        Ok(claim) => claim,
        Err(error) => {
            warn!(
                sub_agent_id,
                agent_key,
                sub_agent_key = %sub_agent_key,
                error = ?error,
                "failed to claim due candle_job"
            );
            return None;
        }
    };

    match claim {
        store::ClaimedCandleSubAgentRun::NotDue => {
            debug!(
                sub_agent_id,
                agent_key, "candle_job no longer due at claim time"
            );
            None
        }
        store::ClaimedCandleSubAgentRun::BlockedByMaintenance => {
            info!(
                sub_agent_id,
                agent_key,
                sub_agent_key = %sub_agent_key,
                "scheduled dispatch held because workspace maintenance is queued or running"
            );
            None
        }
        store::ClaimedCandleSubAgentRun::Skipped { run_id } => {
            info!(
                sub_agent_id,
                run_id,
                agent_key,
                sub_agent_key = %sub_agent_key,
                "harness run skipped because previous run still active"
            );
            None
        }
        store::ClaimedCandleSubAgentRun::Dispatch { run_id } => {
            match build_dispatch_request(pool, live_accounts, &candle_job, run_id, scheduled_for)
                .await
            {
                Ok(Some(request)) => dispatch_run(pool.clone(), backend.clone(), request)
                    .await
                    .succeeded
                    .then_some(run_id),
                Ok(None) => {
                    warn!(
                        run_id,
                        agent_key = %agent_key,
                        sub_agent_key = %sub_agent_key,
                        "no currencies selected for agent; job skipped"
                    );
                    let _ = store::mark_run_failed(
                        pool,
                        run_id,
                        "no currencies selected for agent; job skipped",
                        None,
                    )
                    .await;
                    None
                }
                Err(error) => {
                    error!(
                        run_id,
                        agent_key = %agent_key,
                        sub_agent_key = %sub_agent_key,
                        error = ?error,
                        "failed to build dispatch request"
                    );
                    let _ = store::mark_run_failed(pool, run_id, "dispatch request errored", None)
                        .await;
                    None
                }
            }
        }
    }
}

pub fn dispatch_request_from_job(
    candle_job: &HarnessDispatchSubAgentRow,
    inputs: DispatchRequestInputs,
    account_snapshot: Option<crate::hyperliquid::live_state::LiveAgentSnapshot>,
) -> DispatchRequest {
    DispatchRequest {
        run_id: inputs.run_id,
        sub_agent_id: candle_job.sub_agent_id,
        agent_key: candle_job.agent_key.clone(),
        display_name: candle_job.display_name.clone(),
        sub_agent_key: candle_job.sub_agent_key.clone(),
        sub_agent_kind: candle_job.sub_agent_kind.clone(),
        timeframe: candle_job.timeframe.clone(),
        operator_prompt: candle_job.operator_prompt.clone(),
        strategy_prompt: inputs.strategy_prompt,
        accumulated_learnings: inputs.accumulated_learnings,
        system_prompt: inputs.system_prompt,
        environment: inputs.agent.environment.clone(),
        selected_instruments: inputs.selected_instruments,
        account_snapshot,
        model_provider_id: candle_job.model_provider_id.clone(),
        model_id: candle_job.model_id.clone(),
        model_variant: candle_job.model_variant.clone(),
        timeout_seconds: candle_job.timeout_seconds,
        opencode_base_url: candle_job.opencode_base_url.clone(),
        runtime_config: candle_job.runtime_config.clone(),
        scheduled_for: inputs.scheduled_for,
        review_window_start: None,
        review_window_end: None,
    }
}

pub async fn build_dispatch_request(
    pool: &DbPool,
    live_accounts: &Arc<LiveAccountStore>,
    candle_job: &HarnessDispatchSubAgentRow,
    run_id: i64,
    scheduled_for: chrono::DateTime<Utc>,
) -> Result<Option<DispatchRequest>> {
    let agent = get_agent(pool, &candle_job.agent_key)
        .await?
        .context("agent not found while building dispatch request")?;
    if agent.lifecycle != crate::agents::model::AGENT_LIFECYCLE_ACTIVE {
        return Ok(None);
    }

    let selected_instruments = list_agent_instrument_ids(pool, &candle_job.agent_key).await?;

    if selected_instruments.is_empty() && requires_selected_instruments(&candle_job.sub_agent_kind)
    {
        return Ok(None);
    }

    let system_prompt = crate::agents::prompts::SYSTEM_PROMPT.to_string();
    let strategy_prompt =
        load_strategy_prompt(pool, &candle_job.agent_key, &candle_job.sub_agent_kind).await?;
    let accumulated_learnings = load_accumulated_learnings(pool, &candle_job.agent_key).await?;

    let account_snapshot = if candle_job.sub_agent_kind == SUB_AGENT_KIND_TRADING {
        Some(live_agent_snapshot_for_dispatch(
            agent.trading_account_address.as_deref().unwrap_or_default(),
            &agent.environment,
            live_accounts,
        ))
    } else {
        None
    };

    let mut request = dispatch_request_from_job(
        candle_job,
        DispatchRequestInputs {
            run_id,
            scheduled_for,
            agent,
            selected_instruments,
            strategy_prompt,
            accumulated_learnings,
            system_prompt,
        },
        account_snapshot,
    );
    if run_id != 0 {
        apply_run_model_snapshot(pool, &mut request).await?;
    }
    Ok(Some(request))
}

pub(crate) async fn dispatch_daily_review_coding_event(
    pool: &DbPool,
    agent_key: &str,
    source_sub_agent_run_id: i64,
) -> Result<()> {
    let Some(memory) =
        crate::memory::get_daily_review_memory_for_run(pool, agent_key, source_sub_agent_run_id)
            .await?
    else {
        debug!(
            agent_key,
            source_sub_agent_run_id, "daily review wrote no linked memory"
        );
        return Ok(());
    };
    let requested = memory
        .metadata
        .get("analysis_coding_requested")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if !requested {
        return Ok(());
    }
    let Some(event) =
        store::get_enabled_sub_agent(pool, agent_key, SUB_AGENT_KIND_ANALYSIS_CODING).await?
    else {
        debug!(agent_key, "coding event is disabled or missing");
        return Ok(());
    };
    store::insert_analysis_coding_task_and_run(
        pool,
        store::AnalysisCodingTaskRequest {
            agent_key,
            sub_agent_id: event.id,
            trigger_mode: store::CodingTriggerMode::Automatic,
            request_origin: "daily_review",
            source_sub_agent_run_id: Some(source_sub_agent_run_id),
            source_memory_id: Some(memory.id),
            operator_prompt: memory
                .metadata
                .get("analysis_coding_reason")
                .and_then(serde_json::Value::as_str),
            requested_mode: Some("auto"),
        },
    )
    .await
    .map(|_| ())
}

pub async fn build_event_dispatch_request(
    pool: &DbPool,
    event: &HarnessDispatchSubAgentRow,
    run_id: i64,
    scheduled_for: chrono::DateTime<Utc>,
) -> Result<Option<DispatchRequest>> {
    let agent = get_agent(pool, &event.agent_key)
        .await?
        .context("agent not found while building event dispatch request")?;
    if agent.lifecycle != crate::agents::model::AGENT_LIFECYCLE_ACTIVE {
        return Ok(None);
    }

    let selected_instruments = list_agent_instrument_ids(pool, &event.agent_key).await?;
    if selected_instruments.is_empty() && requires_selected_instruments(&event.sub_agent_kind) {
        return Ok(None);
    }

    let system_prompt = crate::agents::prompts::SYSTEM_PROMPT.to_string();
    let strategy_prompt =
        load_strategy_prompt(pool, &event.agent_key, &event.sub_agent_kind).await?;
    let accumulated_learnings = load_accumulated_learnings(pool, &event.agent_key).await?;

    let mut request = dispatch_request_from_job(
        event,
        DispatchRequestInputs {
            run_id,
            scheduled_for,
            agent,
            selected_instruments,
            strategy_prompt,
            accumulated_learnings,
            system_prompt,
        },
        None,
    );
    apply_run_model_snapshot(pool, &mut request).await?;
    Ok(Some(request))
}

async fn apply_run_model_snapshot(pool: &DbPool, request: &mut DispatchRequest) -> Result<()> {
    let run = store::get_run(pool, request.run_id)
        .await?
        .context("run disappeared before dispatch")?;
    request.model_provider_id = run.model_provider_id;
    request.model_id = run.model_id;
    request.model_variant = run.model_variant;
    Ok(())
}

fn requires_selected_instruments(sub_agent_kind: &str) -> bool {
    matches!(
        sub_agent_kind,
        SUB_AGENT_KIND_ANALYSIS | SUB_AGENT_KIND_MARKET_ANALYSIS | SUB_AGENT_KIND_TRADING
    )
}

async fn load_strategy_prompt(
    pool: &DbPool,
    agent_key: &str,
    sub_agent_kind: &str,
) -> Result<String> {
    let prompt_kind = prompt_kind_for_sub_agent_kind(sub_agent_kind)
        .ok_or_else(|| anyhow!("unknown prompt kind for job kind {sub_agent_kind}"))?;
    let stored = get_agent_strategy_prompt(pool, agent_key, prompt_kind)
        .await?
        .map(|row| row.prompt)
        .unwrap_or_default();
    Ok(effective_strategy_prompt(prompt_kind, stored))
}

fn effective_strategy_prompt(prompt_kind: &str, stored: String) -> String {
    if prompt_kind == PROMPT_KIND_ANALYSIS_CODING && stored.trim().is_empty() {
        return default_prompt_for_kind(prompt_kind).to_string();
    }
    stored
}

async fn load_accumulated_learnings(pool: &DbPool, agent_key: &str) -> Result<Option<String>> {
    Ok(
        get_latest_agent_memory_by_type(pool, agent_key, "agent_learnings")
            .await?
            .map(|memory| {
                format!(
                    "Summary: {}\nCreated at: {}\nContent: {}",
                    memory.summary,
                    memory
                        .created_at
                        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                    memory.content
                )
            }),
    )
}

pub async fn dispatch_analysis_batch_completed_event(
    pool: &DbPool,
    backend: &Arc<dyn HarnessBackend>,
    _live_accounts: &Arc<LiveAccountStore>,
    agent_key: &str,
    workspace_leases: &WorkspaceLeaseManager,
    opencode_base_url: &str,
) -> Result<()> {
    let Some(event) =
        store::get_enabled_sub_agent(pool, agent_key, SUB_AGENT_KIND_MARKET_ANALYSIS).await?
    else {
        debug!(
            agent_key,
            "no enabled analysis batch completed event configured"
        );
        return Ok(());
    };

    match store::insert_queued_event_run_for_automatic_dispatch(pool, agent_key, event.id).await? {
        store::QueuedSubAgentRun::Dispatch {
            run_id,
            scheduled_for,
            ..
        } => {
            let Some(event_dispatch) =
                store::get_dispatch_sub_agent(pool, agent_key, event.id, opencode_base_url).await?
            else {
                let _ =
                    store::mark_run_failed(pool, run_id, "event disappeared before dispatch", None)
                        .await;
                return Ok(());
            };

            match build_event_dispatch_request(pool, &event_dispatch, run_id, scheduled_for).await {
                Ok(Some(request)) => {
                    let _ = dispatch_run_with_workspace_lease(
                        pool.clone(),
                        backend.clone(),
                        request,
                        workspace_leases,
                    )
                    .await;
                }
                Ok(None) => {
                    let _ = store::mark_run_failed(
                        pool,
                        run_id,
                        "no currencies selected for agent; job skipped",
                        None,
                    )
                    .await;
                }
                Err(error) => {
                    let _ = store::mark_run_failed(pool, run_id, "dispatch request errored", None)
                        .await;
                    return Err(error);
                }
            }
        }
        store::QueuedSubAgentRun::Skipped { run_id } => {
            info!(
                agent_key,
                sub_agent_id = event.id,
                run_id,
                "event run skipped because previous analysis-lane run is still active"
            );
        }
        store::QueuedSubAgentRun::Missing => {
            debug!(
                agent_key,
                sub_agent_id = event.id,
                "event disappeared before queue insert"
            );
        }
        store::QueuedSubAgentRun::BlockedByMaintenance => {
            warn!(
                agent_key,
                sub_agent_id = event.id,
                "automatic follow-up event was unexpectedly blocked by maintenance"
            );
        }
    }

    Ok(())
}

/// Run a single dispatch through the backend, awaiting its completion.
/// Sequential schedulers should call this so that the next candle_job
/// for the same agent is not processed until the current run finishes.
pub struct DispatchRunResult {
    pub succeeded: bool,
}

pub async fn dispatch_run(
    pool: DbPool,
    backend: Arc<dyn HarnessBackend>,
    request: DispatchRequest,
) -> DispatchRunResult {
    let run_id = request.run_id;
    let agent_key = request.agent_key.clone();
    let sub_agent_key = request.sub_agent_key.clone();

    if let Err(error) = store::mark_run_running(&pool, run_id, None).await {
        warn!(
            run_id,
            agent_key = %agent_key,
            sub_agent_key = %sub_agent_key,
            error = ?error,
            "failed to mark harness run as running"
        );
        return DispatchRunResult { succeeded: false };
    }

    match dispatch_with_timeout(&pool, backend, request).await {
        Ok(DispatchOutcome::Succeeded { backend_run_ref }) => {
            debug!(
                run_id,
                agent_key = %agent_key,
                sub_agent_key = %sub_agent_key,
                backend_run_ref,
                "harness dispatch finished"
            );
            DispatchRunResult { succeeded: true }
        }
        Ok(DispatchOutcome::Cancelled) => {
            debug!(
                run_id,
                agent_key = %agent_key,
                sub_agent_key = %sub_agent_key,
                "harness dispatch was cancelled; follow-up events will not fire"
            );
            DispatchRunResult { succeeded: false }
        }
        Ok(DispatchOutcome::Failed { summary }) => {
            // `dispatch_with_timeout` already persisted the terminal
            // failure row with a sanitized summary. Treat this as a
            // non-success so follow-up events (e.g. market analysis)
            // are not triggered by a failed analysis run.
            debug!(
                run_id,
                agent_key = %agent_key,
                sub_agent_key = %sub_agent_key,
                summary,
                "harness dispatch failed; follow-up events will not fire"
            );
            DispatchRunResult { succeeded: false }
        }
        Err(error) => {
            error!(
                run_id,
                agent_key = %agent_key,
                sub_agent_key = %sub_agent_key,
                error = ?error,
                "harness dispatch errored"
            );
            let _ = store::mark_run_failed(&pool, run_id, "dispatch task errored", None).await;
            DispatchRunResult { succeeded: false }
        }
    }
}

/// Dispatch a live-workspace job while holding its agent's shared read lease.
/// The lease spans the complete OpenCode session, so promotion or regeneration
/// cannot expose a tree swap to an active session.
pub async fn dispatch_run_with_workspace_lease(
    pool: DbPool,
    backend: Arc<dyn HarnessBackend>,
    request: DispatchRequest,
    workspace_leases: &WorkspaceLeaseManager,
) -> DispatchRunResult {
    let _lease = workspace_leases.acquire_live_read(&request.agent_key).await;
    dispatch_run(pool, backend, request).await
}

fn timeframe_duration_for_sort(candle_job: &HarnessDispatchSubAgentRow) -> i64 {
    match candle_job
        .timeframe
        .as_deref()
        .ok_or_else(|| anyhow!("candle job is missing timeframe"))
        .and_then(parse_timeframe_seconds)
    {
        Ok(seconds) => seconds,
        Err(error) => {
            warn!(
                sub_agent_id = candle_job.sub_agent_id,
                timeframe = ?candle_job.timeframe,
                error = ?error,
                "ignoring candle_job with invalid timeframe"
            );
            i64::MAX
        }
    }
}

fn sort_analysis_jobs_for_dispatch(
    due: Vec<HarnessDispatchSubAgentRow>,
) -> Vec<HarnessDispatchSubAgentRow> {
    let mut indexed: Vec<IndexedAnalysisCandleSubAgent> = due
        .into_iter()
        .map(|candle_job| {
            (
                (
                    timeframe_duration_for_sort(&candle_job),
                    candle_job
                        .next_run_at
                        .expect("due candle job has next_run_at"),
                    candle_job.sub_agent_id,
                ),
                candle_job,
            )
        })
        .collect();
    indexed.sort_by_key(|(key, _)| *key);
    indexed.into_iter().map(|(_, row)| row).collect()
}

fn sort_trading_jobs_for_dispatch(
    due: Vec<HarnessDispatchSubAgentRow>,
) -> Vec<HarnessDispatchSubAgentRow> {
    let mut indexed: Vec<IndexedTradingCandleSubAgent> = due
        .into_iter()
        .map(|candle_job| {
            (
                (
                    candle_job
                        .next_run_at
                        .expect("due candle job has next_run_at"),
                    timeframe_duration_for_sort(&candle_job),
                    candle_job.sub_agent_id,
                ),
                candle_job,
            )
        })
        .collect();
    indexed.sort_by_key(|(key, _)| *key);
    indexed.into_iter().map(|(_, row)| row).collect()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    use std::{
        fs,
        sync::{Arc, Mutex},
        time::{SystemTime, UNIX_EPOCH},
    };

    use async_trait::async_trait;
    use chrono::Utc;
    use serde_json::json;

    use crate::{
        agents::{
            keys::derive_wallet_address,
            model::AgentRegistryRow,
            store::{insert_agent, replace_agent_instruments},
        },
        harness::{
            backend::{DispatchResult, HarnessBackend},
            in_flight::InFlightTracker,
            model::{
                HarnessSubAgentRunRow, RUN_STATUS_QUEUED, RUN_STATUS_SKIPPED, RUN_STATUS_SUCCEEDED,
            },
            store::{
                self, ClaimedCandleSubAgentRun, insert_default_harness_sub_agents, insert_test_run,
            },
        },
        test_db,
    };

    #[test]
    fn coding_run_timeout_uses_the_run_deadline() {
        let now = Utc::now();
        assert!(coding_run_exceeded_timeout(
            Some(now - chrono::Duration::seconds(601)),
            600,
            now,
        ));
        assert!(!coding_run_exceeded_timeout(
            Some(now - chrono::Duration::seconds(599)),
            600,
            now,
        ));
        assert!(!coding_run_exceeded_timeout(None, 600, now));
    }

    #[test]
    fn blank_coding_strategy_uses_safe_default() {
        assert_eq!(
            effective_strategy_prompt(PROMPT_KIND_ANALYSIS_CODING, String::new()),
            default_prompt_for_kind(PROMPT_KIND_ANALYSIS_CODING)
        );
        assert_eq!(
            effective_strategy_prompt(PROMPT_KIND_ANALYSIS, String::new()),
            ""
        );
    }

    #[test]
    fn coding_validation_is_bound_to_task_and_candidate_hash() {
        let root = std::env::temp_dir().join(format!(
            "coding-validation-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        ));
        let candidate = root.join("workspace");
        let user = candidate.join("scripts/user");
        fs::create_dir_all(&user).expect("create candidate user tree");
        fs::write(user.join("analyze.py"), "print('ok')\n").expect("write candidate");
        let candidate_hash = "candidate-hash";
        let validation = json!({
            "schema_version": 1,
            "task_id": 42,
            "ok": true,
            "candidate_manifest_sha256": candidate_hash,
        });

        require_coding_validation(&validation, 42, candidate_hash).expect("matching validation");
        assert!(require_coding_validation(&validation, 43, candidate_hash).is_err());
        assert!(require_coding_validation(&validation, 42, "stale").is_err());

        fs::remove_dir_all(root).expect("remove validation fixture");
    }

    struct FakeBackend {
        calls: Arc<Mutex<Vec<DispatchRequest>>>,
        delay: Duration,
        active_calls: Arc<AtomicUsize>,
        max_active_calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl HarnessBackend for FakeBackend {
        async fn dispatch(&self, request: DispatchRequest) -> Result<DispatchResult> {
            let active = self.active_calls.fetch_add(1, Ordering::SeqCst) + 1;
            let _ =
                self.max_active_calls
                    .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                        (active > current).then_some(active)
                    });
            if !self.delay.is_zero() {
                tokio::time::sleep(self.delay).await;
            }
            self.calls.lock().unwrap().push(request);
            self.active_calls.fetch_sub(1, Ordering::SeqCst);
            Ok(DispatchResult {
                backend_run_ref: "ses_fake".to_string(),
            })
        }
    }

    fn sample_opencode_client() -> Arc<OpenCodeClient> {
        Arc::new(
            OpenCodeClient::new(crate::opencode::client::OpenCodeClientConfig::new(
                "opencode".to_string(),
                None,
            ))
            .expect("build OpenCode client"),
        )
    }

    fn scheduler_runtime(in_flight: InFlightTracker) -> HarnessSchedulerRuntime {
        HarnessSchedulerRuntime {
            workspace_controller: Arc::new(
                crate::opencode::workspace_control_client::LocalWorkspaceController::new(
                    crate::opencode::workspace::OpenCodeWorkspaceConfig {
                        source_root: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                            .join(crate::opencode::workspace::PROFILE_SOURCE_RELATIVE_PATH),
                        host_workspaces_root: std::path::PathBuf::from(
                            "/tmp/opencode/hypervibes-scheduler-tests",
                        ),
                        container_workspaces_root: "/workspaces".to_string(),
                        api_base_url: "http://host.containers.internal:3003".to_string(),
                    },
                ),
            ),
            agent_api_base_url: "http://host.containers.internal:3003".to_string(),
            container_workspaces_root: "/workspaces".to_string(),
            opencode_client: sample_opencode_client(),
            in_flight,
        }
    }

    async fn workspace_maintenance_runtime(
        in_flight: InFlightTracker,
    ) -> (HarnessSchedulerRuntime, tokio::task::JoinHandle<()>) {
        use axum::{Json, Router, routing::post};

        let app = Router::new().route("/instance/dispose", post(|| async { Json(true) }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind workspace instance dispose test server");
        let base_url = format!("http://{}", listener.local_addr().expect("local address"));
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve workspace instance dispose test server");
        });
        let mut runtime = scheduler_runtime(in_flight);
        runtime.opencode_client = Arc::new(
            OpenCodeClient::new(
                crate::opencode::client::OpenCodeClientConfig::new_with_base_url(
                    "opencode".to_string(),
                    None,
                    base_url,
                ),
            )
            .expect("build OpenCode client"),
        );
        (runtime, server)
    }

    impl FakeBackend {
        fn success(calls: Arc<Mutex<Vec<DispatchRequest>>>) -> Self {
            Self {
                calls,
                delay: Duration::ZERO,
                active_calls: Arc::new(AtomicUsize::new(0)),
                max_active_calls: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn with_delay(calls: Arc<Mutex<Vec<DispatchRequest>>>, delay: Duration) -> Self {
            Self {
                calls,
                delay,
                active_calls: Arc::new(AtomicUsize::new(0)),
                max_active_calls: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn max_active_calls(&self) -> usize {
            self.max_active_calls.load(Ordering::SeqCst)
        }
    }

    /// Backend whose `dispatch` always returns an error, simulating a
    /// failed OpenCode invocation such as a 5xx from the model API.
    struct FailingBackend;

    #[async_trait]
    impl HarnessBackend for FailingBackend {
        async fn dispatch(&self, _request: DispatchRequest) -> Result<DispatchResult> {
            Err(anyhow!("simulated dispatch failure"))
        }
    }

    fn deterministic_private_key(key: &str) -> String {
        use rand::rngs::StdRng;
        use rand::{RngExt, SeedableRng};

        let seed = key
            .bytes()
            .fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
        let mut rng = StdRng::seed_from_u64(seed);
        let bytes: [u8; 32] = rng.random();
        format!("0x{}", hex::encode(bytes))
    }

    fn sample_agent(key: &str) -> AgentRegistryRow {
        let private_key = deterministic_private_key(key);
        let wallet = derive_wallet_address(&private_key).unwrap();
        let now = Utc::now();

        AgentRegistryRow {
            agent_key: key.to_string(),
            user_id: crate::test_db::test_user_id(),
            created_at: now,
            updated_at: now,
            enabled: true,
            lifecycle: crate::agents::model::AGENT_LIFECYCLE_ACTIVE.to_string(),
            display_name: format!("Test {key}"),
            trading_account_address: Some(wallet.clone()),
            environment: "live".to_string(),
            api_key: format!("vta_{key}"),
            api_key_last_used_at: None,
            runtime_config: json!({
                "workspace_host_path": format!("workspaces/agents/{key}"),
                "workspace_container_path": format!("/workspaces/agents/{key}"),
                "profile_source": "agent-runtime/workspace-template"
            }),
        }
    }

    async fn seed_test_agent(pool: &DbPool, key: &str) {
        let far_future = Utc::now() + chrono::Duration::days(365);
        sqlx::query("UPDATE harness_sub_agents SET next_run_at = $1 WHERE next_run_at IS NOT NULL")
            .bind(far_future)
            .execute(pool)
            .await
            .expect("push existing jobs");
        sqlx::query("UPDATE harness_sub_agents SET enabled = false")
            .execute(pool)
            .await
            .expect("disable existing jobs");

        insert_agent(pool, &sample_agent(key))
            .await
            .expect("insert agent");
        insert_default_harness_sub_agents(pool, key)
            .await
            .expect("insert defaults");

        // Default jobs need selected instruments and strategy prompts or
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
        sqlx::query(
            "UPDATE harness_sub_agents
                SET model_provider_id = 'anthropic', model_id = 'claude-sonnet-test'
              WHERE agent_key = $1",
        )
        .bind(key)
        .execute(pool)
        .await
        .expect("seed event model");
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

    /// Pin a candle_job's `next_run_at` to the latest due boundary for
    /// `timeframe` at or before `now`, so a claim at `now` will fire.
    async fn pin_job_due(
        pool: &DbPool,
        sub_agent_id: i64,
        timeframe: &str,
    ) -> chrono::DateTime<Utc> {
        let now = Utc::now();
        let due = crate::harness::timeframe::latest_due_at_or_before(
            now,
            timeframe,
            crate::harness::timeframe::DEFAULT_TRIGGER_DELAY_SECONDS,
        )
        .expect("compute latest due")
        .expect("should have a previous due boundary");
        sqlx::query(
            "UPDATE harness_sub_agents
                SET enabled = true, next_run_at = $2
              WHERE id = $1",
        )
        .bind(sub_agent_id)
        .bind(due)
        .execute(pool)
        .await
        .expect("force due");
        due
    }

    #[tokio::test]
    async fn tick_dispatches_due_job_and_marks_run_succeeded() {
        let pool = test_db::pool().await;
        let key = format!("sched-ok-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0));
        seed_test_agent(&pool, &key).await;

        let (sub_agent_id,): (i64,) = sqlx::query_as(
            "SELECT id FROM harness_sub_agents
              WHERE agent_key = $1 AND sub_agent_key = 'analysis-15m'",
        )
        .bind(&key)
        .fetch_one(&pool)
        .await
        .expect("fetch candle_job id");
        let due = pin_job_due(&pool, sub_agent_id, "15m").await;

        let calls: Arc<Mutex<Vec<DispatchRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let backend: Arc<dyn HarnessBackend> = Arc::new(FakeBackend::success(calls.clone()));

        let live_accounts = Arc::new(crate::hyperliquid::live_state::LiveAccountStore::new());
        let (_tx, rx) = watch::channel(false);
        let (_force_tx, force_rx) = watch::channel(false);
        let mut scheduler = HarnessScheduler::new(
            pool.clone(),
            rx,
            force_rx,
            backend,
            live_accounts,
            scheduler_runtime(InFlightTracker::new()),
        );
        scheduler.tick().await.expect("tick");

        run_until(|| async { calls.lock().map(|guard| !guard.is_empty()).unwrap_or(false) }).await;

        {
            let guard = calls.lock().unwrap();
            assert_eq!(guard.len(), 1);
            let request = &guard[0];
            assert_eq!(request.agent_key, key);
            assert_eq!(request.sub_agent_key, "analysis-15m");
            assert_eq!(request.timeframe.as_deref(), Some("15m"));
            assert_eq!(
                request.scheduled_for,
                crate::harness::timeframe::boundary_for_due_at(
                    due,
                    crate::harness::timeframe::DEFAULT_TRIGGER_DELAY_SECONDS,
                )
            );
        }

        run_until(|| async {
            list_run_statuses(&pool, &key)
                .await
                .iter()
                .any(|status| status == RUN_STATUS_SUCCEEDED)
        })
        .await;

        let runs: Vec<HarnessSubAgentRunRow> = store::list_agent_runs(&pool, &key, 10)
            .await
            .expect("list runs");
        let run = runs
            .iter()
            .find(|row| row.status == RUN_STATUS_SUCCEEDED)
            .expect("succeeded run present");
        assert_eq!(run.backend_run_ref.as_deref(), Some("ses_fake"));
    }

    #[tokio::test]
    async fn tick_resumes_queued_run_after_dispatcher_restart() {
        let pool = test_db::pool().await;
        let key = format!(
            "sched-resume-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        seed_test_agent(&pool, &key).await;
        let (sub_agent_id,): (i64,) = sqlx::query_as(
            "SELECT id FROM harness_sub_agents
              WHERE agent_key = $1 AND sub_agent_key = 'analysis-1h'",
        )
        .bind(&key)
        .fetch_one(&pool)
        .await
        .expect("fetch queued job");
        let run_id = insert_test_run(&pool, sub_agent_id, RUN_STATUS_QUEUED)
            .await
            .expect("insert queued run");

        let calls: Arc<Mutex<Vec<DispatchRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let backend: Arc<dyn HarnessBackend> = Arc::new(FakeBackend::success(calls.clone()));
        let live_accounts = Arc::new(crate::hyperliquid::live_state::LiveAccountStore::new());
        let (_tx, rx) = watch::channel(false);
        let (_force_tx, force_rx) = watch::channel(false);
        let mut scheduler = HarnessScheduler::new(
            pool.clone(),
            rx,
            force_rx,
            backend,
            live_accounts,
            scheduler_runtime(InFlightTracker::new()),
        );

        scheduler.tick().await.expect("tick");
        run_until(|| async { calls.lock().is_ok_and(|calls| !calls.is_empty()) }).await;

        {
            let calls = calls.lock().expect("lock dispatch calls");
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].run_id, run_id);
            assert_eq!(calls[0].sub_agent_key, "analysis-1h");
        }
        run_until(|| async {
            store::get_run(&pool, run_id)
                .await
                .ok()
                .flatten()
                .is_some_and(|run| run.status == RUN_STATUS_SUCCEEDED)
        })
        .await;
    }

    #[tokio::test]
    async fn overlapping_ticks_do_not_skip_same_agent_analysis_jobs() {
        let pool = test_db::pool().await;
        let key = format!(
            "sched-seq-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        seed_test_agent(&pool, &key).await;

        // Force both analysis jobs to be due at the same boundary.
        let (fifteen_m_id,): (i64,) = sqlx::query_as(
            "SELECT id FROM harness_sub_agents
              WHERE agent_key = $1 AND sub_agent_key = 'analysis-15m'",
        )
        .bind(&key)
        .fetch_one(&pool)
        .await
        .expect("fetch 15m candle_job id");
        let (one_h_id,): (i64,) = sqlx::query_as(
            "SELECT id FROM harness_sub_agents
              WHERE agent_key = $1 AND sub_agent_key = 'analysis-1h'",
        )
        .bind(&key)
        .fetch_one(&pool)
        .await
        .expect("fetch 1h candle_job id");
        pin_job_due(&pool, fifteen_m_id, "15m").await;
        pin_job_due(&pool, one_h_id, "1h").await;

        // Normalize both jobs to the same next_run_at so ordering is
        // determined by duration, not by where the current wall-clock falls
        // between candle boundaries. Using the later due time keeps both
        // jobs fresh (not stale) for their respective timeframes.
        sqlx::query(
            "UPDATE harness_sub_agents
                SET next_run_at = (
                    SELECT MAX(next_run_at) FROM harness_sub_agents
                    WHERE id = $1 OR id = $2
                )
              WHERE id = $1 OR id = $2",
        )
        .bind(fifteen_m_id)
        .bind(one_h_id)
        .execute(&pool)
        .await
        .expect("normalize candle_job due times");

        let calls: Arc<Mutex<Vec<DispatchRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let backend: Arc<dyn HarnessBackend> = Arc::new(FakeBackend::with_delay(
            calls.clone(),
            Duration::from_millis(50),
        ));

        let live_accounts = Arc::new(crate::hyperliquid::live_state::LiveAccountStore::new());
        let (_tx, rx) = watch::channel(false);
        let (_force_tx, force_rx) = watch::channel(false);
        let mut scheduler = HarnessScheduler::new(
            pool.clone(),
            rx,
            force_rx,
            backend,
            live_accounts,
            scheduler_runtime(InFlightTracker::new()),
        );
        scheduler.tick().await.expect("tick");

        run_until(|| async { calls.lock().map(|guard| !guard.is_empty()).unwrap_or(false) }).await;
        scheduler.tick().await.expect("overlapping tick");

        run_until(|| async { calls.lock().map(|guard| guard.len() >= 2).unwrap_or(false) }).await;

        let order: Vec<String> = {
            let guard = calls.lock().unwrap();
            assert_eq!(guard.len(), 2);
            guard
                .iter()
                .map(|request| request.sub_agent_key.clone())
                .collect()
        };
        // 15m is shorter than 1h, so it should dispatch first.
        assert_eq!(order, vec!["analysis-15m", "analysis-1h"]);

        let runs = store::list_agent_runs(&pool, &key, 10)
            .await
            .expect("list runs");
        assert!(runs.iter().all(|run| run.status != RUN_STATUS_SKIPPED));
    }

    #[tokio::test]
    async fn tick_runs_analysis_and_trading_lanes_concurrently_for_same_agent() {
        let pool = test_db::pool().await;
        let key = format!(
            "sched-lanes-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        seed_test_agent(&pool, &key).await;

        let (analysis_id,): (i64,) = sqlx::query_as(
            "SELECT id FROM harness_sub_agents
              WHERE agent_key = $1 AND sub_agent_key = 'analysis-15m'",
        )
        .bind(&key)
        .fetch_one(&pool)
        .await
        .expect("fetch analysis id");
        let (trading_id,): (i64,) = sqlx::query_as(
            "SELECT id FROM harness_sub_agents
              WHERE agent_key = $1 AND sub_agent_key = 'trading-5m'",
        )
        .bind(&key)
        .fetch_one(&pool)
        .await
        .expect("fetch trading id");
        pin_job_due(&pool, analysis_id, "15m").await;
        pin_job_due(&pool, trading_id, "5m").await;

        let calls: Arc<Mutex<Vec<DispatchRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let backend_impl = Arc::new(FakeBackend::with_delay(
            calls.clone(),
            Duration::from_millis(150),
        ));
        let backend: Arc<dyn HarnessBackend> = backend_impl.clone();

        let live_accounts = Arc::new(crate::hyperliquid::live_state::LiveAccountStore::new());
        let (_tx, rx) = watch::channel(false);
        let (_force_tx, force_rx) = watch::channel(false);
        let mut scheduler = HarnessScheduler::new(
            pool.clone(),
            rx,
            force_rx,
            backend,
            live_accounts,
            scheduler_runtime(InFlightTracker::new()),
        );
        scheduler.tick().await.expect("tick");

        run_until(|| async { calls.lock().map(|guard| guard.len() >= 2).unwrap_or(false) }).await;

        let guard = calls.lock().unwrap();
        let mut jobs: Vec<&str> = guard.iter().map(|r| r.sub_agent_key.as_str()).collect();
        jobs.sort();
        assert!(
            jobs == vec!["analysis-15m", "trading-5m"],
            "expected both lanes to dispatch, got {jobs:?}"
        );
        assert!(
            backend_impl.max_active_calls() >= 2,
            "expected same-agent lanes to overlap, max active calls was {}",
            backend_impl.max_active_calls()
        );
    }

    #[tokio::test]
    async fn tick_dispatches_market_analysis_event_after_successful_analysis_batch() {
        let pool = test_db::pool().await;
        let key = format!(
            "sched-event-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        seed_test_agent(&pool, &key).await;

        let sub_agent_id = store::list_agent_sub_agents(&pool, &key)
            .await
            .expect("list jobs")
            .into_iter()
            .find(|job| job.sub_agent_kind == SUB_AGENT_KIND_MARKET_ANALYSIS)
            .expect("default event present")
            .id;
        store::set_sub_agent_enabled(&pool, &key, sub_agent_id, true)
            .await
            .expect("enable default event");

        let (sub_agent_id,): (i64,) = sqlx::query_as(
            "SELECT id FROM harness_sub_agents
              WHERE agent_key = $1 AND sub_agent_key = 'analysis-15m'",
        )
        .bind(&key)
        .fetch_one(&pool)
        .await
        .expect("fetch analysis id");
        pin_job_due(&pool, sub_agent_id, "15m").await;

        let calls: Arc<Mutex<Vec<DispatchRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let backend: Arc<dyn HarnessBackend> = Arc::new(FakeBackend::success(calls.clone()));
        let live_accounts = Arc::new(crate::hyperliquid::live_state::LiveAccountStore::new());
        let (_tx, rx) = watch::channel(false);
        let (_force_tx, force_rx) = watch::channel(false);
        let mut scheduler = HarnessScheduler::new(
            pool.clone(),
            rx,
            force_rx,
            backend,
            live_accounts,
            scheduler_runtime(InFlightTracker::new()),
        );
        scheduler.tick().await.expect("tick");

        run_until(|| async { calls.lock().map(|guard| guard.len() >= 2).unwrap_or(false) }).await;

        let guard = calls.lock().unwrap();
        let mut jobs: Vec<&str> = guard
            .iter()
            .map(|request| request.sub_agent_key.as_str())
            .collect();
        jobs.sort();
        assert_eq!(jobs, vec!["analysis-15m", "market-analysis"]);
    }

    #[tokio::test]
    async fn tick_dispatches_different_agents_concurrently() {
        let pool = test_db::pool().await;
        let key_a = format!("agent-a-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0));
        let key_b = format!("agent-b-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0));

        let far_future = Utc::now() + chrono::Duration::days(365);
        sqlx::query("UPDATE harness_sub_agents SET next_run_at = $1")
            .bind(far_future)
            .execute(&pool)
            .await
            .expect("push existing jobs");
        sqlx::query("UPDATE harness_sub_agents SET enabled = false")
            .execute(&pool)
            .await
            .expect("disable existing jobs");

        seed_test_agent(&pool, &key_a).await;
        seed_test_agent(&pool, &key_b).await;

        for key in [&key_a, &key_b] {
            let (sub_agent_id,): (i64,) = sqlx::query_as(
                "SELECT id FROM harness_sub_agents
                  WHERE agent_key = $1 AND sub_agent_key = 'analysis-15m'",
            )
            .bind(key)
            .fetch_one(&pool)
            .await
            .expect("fetch candle_job id");
            pin_job_due(&pool, sub_agent_id, "15m").await;
        }

        let calls: Arc<Mutex<Vec<DispatchRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let backend_impl = Arc::new(FakeBackend::with_delay(
            calls.clone(),
            Duration::from_millis(150),
        ));
        let backend: Arc<dyn HarnessBackend> = backend_impl.clone();

        let live_accounts = Arc::new(crate::hyperliquid::live_state::LiveAccountStore::new());
        let (_tx, rx) = watch::channel(false);
        let (_force_tx, force_rx) = watch::channel(false);
        let mut scheduler = HarnessScheduler::new(
            pool.clone(),
            rx,
            force_rx,
            backend,
            live_accounts,
            scheduler_runtime(InFlightTracker::new()),
        );
        scheduler.tick().await.expect("tick");

        run_until(|| async { calls.lock().map(|guard| guard.len() >= 2).unwrap_or(false) }).await;

        let guard = calls.lock().unwrap();
        assert_eq!(guard.len(), 2);
        let mut agents: Vec<&str> = guard.iter().map(|r| r.agent_key.as_str()).collect();
        agents.sort();
        assert_eq!(agents, vec![key_a.as_str(), key_b.as_str()]);
        assert!(
            backend_impl.max_active_calls() >= 2,
            "expected concurrent dispatch, max active calls was {}",
            backend_impl.max_active_calls()
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

        let (sub_agent_id,): (i64,) = sqlx::query_as(
            "SELECT id FROM harness_sub_agents
              WHERE agent_key = $1 AND sub_agent_key = 'analysis-15m'",
        )
        .bind(&key)
        .fetch_one(&pool)
        .await
        .expect("fetch candle_job id");
        pin_job_due(&pool, sub_agent_id, "15m").await;

        insert_test_run(&pool, sub_agent_id, "running")
            .await
            .expect("seed active run");

        let calls: Arc<Mutex<Vec<DispatchRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let backend: Arc<dyn HarnessBackend> = Arc::new(FakeBackend::success(calls.clone()));
        let live_accounts = Arc::new(crate::hyperliquid::live_state::LiveAccountStore::new());
        let (_tx, rx) = watch::channel(false);
        let (_force_tx, force_rx) = watch::channel(false);
        let mut scheduler = HarnessScheduler::new(
            pool.clone(),
            rx,
            force_rx,
            backend,
            live_accounts,
            scheduler_runtime(InFlightTracker::new()),
        );
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
    async fn tick_leaves_workspace_maintenance_queued_while_agent_run_is_active() {
        let pool = test_db::pool().await;
        let key = format!(
            "maint-busy-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        seed_test_agent(&pool, &key).await;

        let (sub_agent_id,): (i64,) = sqlx::query_as(
            "SELECT id FROM harness_sub_agents
              WHERE agent_key = $1 AND sub_agent_key = 'analysis-15m'",
        )
        .bind(&key)
        .fetch_one(&pool)
        .await
        .expect("fetch candle_job id");
        insert_test_run(&pool, sub_agent_id, "running")
            .await
            .expect("seed active run");
        store::insert_workspace_regenerate_task(&pool, &key, false, false)
            .await
            .expect("insert maintenance task");

        let backend: Arc<dyn HarnessBackend> =
            Arc::new(FakeBackend::success(Arc::new(Mutex::new(Vec::new()))));
        let live_accounts = Arc::new(crate::hyperliquid::live_state::LiveAccountStore::new());
        let (_tx, rx) = watch::channel(false);
        let (_force_tx, force_rx) = watch::channel(false);
        let mut scheduler = HarnessScheduler::new(
            pool.clone(),
            rx,
            force_rx,
            backend,
            live_accounts,
            scheduler_runtime(InFlightTracker::new()),
        );
        scheduler.tick().await.expect("tick");

        let task = store::get_latest_workspace_regenerate_task(&pool, &key)
            .await
            .expect("load maintenance task")
            .expect("maintenance task present");
        assert_eq!(
            task.status,
            crate::harness::model::MAINTENANCE_STATUS_QUEUED
        );
    }

    #[tokio::test]
    async fn tick_leaves_workspace_maintenance_queued_while_live_session_is_active() {
        let pool = test_db::pool().await;
        let key = format!(
            "maint-session-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        seed_test_agent(&pool, &key).await;
        store::insert_workspace_regenerate_task(&pool, &key, false, false)
            .await
            .expect("insert maintenance task");

        let session_id = format!(
            "ses-maint-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        let directory = format!("/workspaces/agents/{key}");
        sqlx::query(
            "INSERT INTO opencode.sessions (id, directory, status, updated_at)
             VALUES ($1, $2, 'busy', now())",
        )
        .bind(&session_id)
        .bind(&directory)
        .execute(&pool)
        .await
        .expect("insert active session row");

        let backend: Arc<dyn HarnessBackend> =
            Arc::new(FakeBackend::success(Arc::new(Mutex::new(Vec::new()))));
        let live_accounts = Arc::new(crate::hyperliquid::live_state::LiveAccountStore::new());
        let (_tx, rx) = watch::channel(false);
        let (_force_tx, force_rx) = watch::channel(false);
        let mut scheduler = HarnessScheduler::new(
            pool.clone(),
            rx,
            force_rx,
            backend,
            live_accounts,
            scheduler_runtime(InFlightTracker::new()),
        );
        scheduler.tick().await.expect("tick");

        let task = store::get_latest_workspace_regenerate_task(&pool, &key)
            .await
            .expect("load maintenance task")
            .expect("maintenance task present");
        assert_eq!(
            task.status,
            crate::harness::model::MAINTENANCE_STATUS_QUEUED
        );
    }

    #[tokio::test]
    async fn tick_runs_workspace_maintenance_once_agent_is_idle() {
        let pool = test_db::pool().await;
        let key = format!(
            "maint-idle-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        seed_test_agent(&pool, &key).await;
        sqlx::query(
            "INSERT INTO memory.records (
                id, agent_key, symbol, memory_type, summary, content
             ) VALUES ($1, $2, 'BTC', 'observation', 'memory', 'memory content')",
        )
        .bind(uuid::Uuid::new_v4())
        .bind(&key)
        .execute(&pool)
        .await
        .expect("insert memory");
        store::insert_workspace_regenerate_task(&pool, &key, true, true)
            .await
            .expect("insert maintenance task");

        let backend: Arc<dyn HarnessBackend> =
            Arc::new(FakeBackend::success(Arc::new(Mutex::new(Vec::new()))));
        let live_accounts = Arc::new(crate::hyperliquid::live_state::LiveAccountStore::new());
        let (_tx, rx) = watch::channel(false);
        let (_force_tx, force_rx) = watch::channel(false);
        let (runtime, server) = workspace_maintenance_runtime(InFlightTracker::new()).await;
        let mut scheduler =
            HarnessScheduler::new(pool.clone(), rx, force_rx, backend, live_accounts, runtime);
        scheduler.tick().await.expect("tick");

        let task = store::get_latest_workspace_regenerate_task(&pool, &key)
            .await
            .expect("load maintenance task")
            .expect("maintenance task present");
        assert_eq!(
            task.status,
            crate::harness::model::MAINTENANCE_STATUS_SUCCEEDED
        );

        let agent = get_agent(&pool, &key)
            .await
            .expect("get agent")
            .expect("agent present");
        let workspace = OpenCodeWorkspaceRuntimeConfig::from_value(&agent.runtime_config)
            .expect("updated workspace runtime config");
        assert_eq!(
            workspace.workspace_container_path,
            format!("/workspaces/agents/{key}")
        );

        let (memory_count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM memory.records WHERE agent_key = $1")
                .bind(&key)
                .fetch_one(&pool)
                .await
                .expect("count memories");
        assert_eq!(memory_count, 0);
        server.abort();
    }

    #[tokio::test]
    async fn tick_ignores_idle_workspace_sessions_when_running_maintenance() {
        let pool = test_db::pool().await;
        let key = format!(
            "maint-idle-session-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        seed_test_agent(&pool, &key).await;
        store::insert_workspace_regenerate_task(&pool, &key, false, false)
            .await
            .expect("insert maintenance task");

        let session_id = format!(
            "ses-idle-maint-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        let directory = format!("/workspaces/agents/{key}");
        sqlx::query(
            "INSERT INTO opencode.sessions (id, directory, status, updated_at)
             VALUES ($1, $2, 'idle', now())",
        )
        .bind(&session_id)
        .bind(&directory)
        .execute(&pool)
        .await
        .expect("insert idle session row");

        let backend: Arc<dyn HarnessBackend> =
            Arc::new(FakeBackend::success(Arc::new(Mutex::new(Vec::new()))));
        let live_accounts = Arc::new(crate::hyperliquid::live_state::LiveAccountStore::new());
        let (_tx, rx) = watch::channel(false);
        let (_force_tx, force_rx) = watch::channel(false);
        let (runtime, server) = workspace_maintenance_runtime(InFlightTracker::new()).await;
        let mut scheduler =
            HarnessScheduler::new(pool.clone(), rx, force_rx, backend, live_accounts, runtime);
        scheduler.tick().await.expect("tick");

        let task = store::get_latest_workspace_regenerate_task(&pool, &key)
            .await
            .expect("load maintenance task")
            .expect("maintenance task present");
        assert_eq!(
            task.status,
            crate::harness::model::MAINTENANCE_STATUS_SUCCEEDED
        );
        server.abort();
    }

    #[tokio::test]
    async fn tick_ignores_workspace_sessions_without_an_active_status_when_running_maintenance() {
        let pool = test_db::pool().await;
        let key = format!(
            "maint-unknown-session-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        seed_test_agent(&pool, &key).await;
        store::insert_workspace_regenerate_task(&pool, &key, false, false)
            .await
            .expect("insert maintenance task");

        let directory = format!("/workspaces/agents/{key}");
        sqlx::query(
            "INSERT INTO opencode.sessions (id, directory, updated_at)
             VALUES ($1, $2, now())",
        )
        .bind(format!(
            "ses-unknown-maint-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ))
        .bind(&directory)
        .execute(&pool)
        .await
        .expect("insert session without status");

        let backend: Arc<dyn HarnessBackend> =
            Arc::new(FakeBackend::success(Arc::new(Mutex::new(Vec::new()))));
        let live_accounts = Arc::new(crate::hyperliquid::live_state::LiveAccountStore::new());
        let (_tx, rx) = watch::channel(false);
        let (_force_tx, force_rx) = watch::channel(false);
        let (runtime, server) = workspace_maintenance_runtime(InFlightTracker::new()).await;
        let mut scheduler =
            HarnessScheduler::new(pool.clone(), rx, force_rx, backend, live_accounts, runtime);
        scheduler.tick().await.expect("tick");

        let task = store::get_latest_workspace_regenerate_task(&pool, &key)
            .await
            .expect("load maintenance task")
            .expect("maintenance task present");
        assert_eq!(
            task.status,
            crate::harness::model::MAINTENANCE_STATUS_SUCCEEDED
        );
        server.abort();
    }

    #[tokio::test]
    async fn claim_due_candle_sub_agent_does_not_double_dispatch() {
        let pool = test_db::pool().await;
        let key = format!(
            "sched-double-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        seed_test_agent(&pool, &key).await;

        let (sub_agent_id,): (i64,) = sqlx::query_as(
            "SELECT id FROM harness_sub_agents
              WHERE agent_key = $1 AND sub_agent_key = 'analysis-15m'",
        )
        .bind(&key)
        .fetch_one(&pool)
        .await
        .expect("fetch candle_job id");
        pin_job_due(&pool, sub_agent_id, "15m").await;

        let now = Utc::now();
        let first = claim_due_candle_sub_agent(&pool, sub_agent_id, now)
            .await
            .expect("claim 1");
        assert!(matches!(first, ClaimedCandleSubAgentRun::Dispatch { .. }));
        let second = claim_due_candle_sub_agent(&pool, sub_agent_id, now)
            .await
            .expect("claim 2");
        assert!(matches!(second, ClaimedCandleSubAgentRun::NotDue));
    }

    async fn claim_due_candle_sub_agent(
        pool: &DbPool,
        sub_agent_id: i64,
        now: chrono::DateTime<Utc>,
    ) -> Result<ClaimedCandleSubAgentRun> {
        store::claim_due_candle_sub_agent(pool, sub_agent_id, now).await
    }

    #[tokio::test]
    async fn tick_does_not_spawn_dispatches_when_shutdown_is_already_signaled() {
        let pool = test_db::pool().await;
        let key = format!(
            "shutdown-noop-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        seed_test_agent(&pool, &key).await;

        let (sub_agent_id,): (i64,) = sqlx::query_as(
            "SELECT id FROM harness_sub_agents
              WHERE agent_key = $1 AND sub_agent_key = 'analysis-15m'",
        )
        .bind(&key)
        .fetch_one(&pool)
        .await
        .expect("fetch candle_job id");
        pin_job_due(&pool, sub_agent_id, "15m").await;

        let calls: Arc<Mutex<Vec<DispatchRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let backend: Arc<dyn HarnessBackend> = Arc::new(FakeBackend::success(calls.clone()));
        let (_shutdown_tx, shutdown_rx) = watch::channel(true);
        let (_force_tx, force_rx) = watch::channel(false);

        let live_accounts = Arc::new(crate::hyperliquid::live_state::LiveAccountStore::new());
        let mut scheduler = HarnessScheduler::new(
            pool.clone(),
            shutdown_rx,
            force_rx,
            backend,
            live_accounts,
            scheduler_runtime(InFlightTracker::new()),
        );
        scheduler.tick().await.expect("tick");

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(
            calls.lock().unwrap().is_empty(),
            "tick should be a no-op when shutdown_rx is set"
        );
    }

    #[tokio::test]
    async fn run_drains_in_flight_dispatch_after_shutdown_signal() {
        let pool = test_db::pool().await;
        let key = format!(
            "shutdown-drain-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        seed_test_agent(&pool, &key).await;

        let (sub_agent_id,): (i64,) = sqlx::query_as(
            "SELECT id FROM harness_sub_agents
              WHERE agent_key = $1 AND sub_agent_key = 'analysis-15m'",
        )
        .bind(&key)
        .fetch_one(&pool)
        .await
        .expect("fetch candle_job id");
        pin_job_due(&pool, sub_agent_id, "15m").await;

        // The fake backend takes 200ms; the in-flight tracker should
        // keep `run()` alive long enough for the dispatch to finish
        // after the shutdown signal is observed.
        let calls: Arc<Mutex<Vec<DispatchRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let backend_impl = Arc::new(FakeBackend::with_delay(
            calls.clone(),
            Duration::from_millis(200),
        ));
        let backend: Arc<dyn HarnessBackend> = backend_impl.clone();

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let (_force_tx, force_rx) = watch::channel(false);
        let in_flight = InFlightTracker::new();
        let in_flight_for_run = in_flight.clone();
        let live_accounts = Arc::new(crate::hyperliquid::live_state::LiveAccountStore::new());
        let mut scheduler = HarnessScheduler::new(
            pool.clone(),
            shutdown_rx,
            force_rx,
            backend,
            live_accounts,
            scheduler_runtime(in_flight_for_run),
        );

        // Drive the first tick manually so the spawn happens before we
        // signal shutdown, then run the loop on a background task.
        scheduler.tick().await.expect("first tick");

        let run_handle = tokio::spawn(async move { scheduler.run().await });

        // Give the spawned dispatch a moment to actually start.
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(in_flight.in_flight() >= 1, "dispatch should be in flight");

        // Signal shutdown. `run` should observe it on the next
        // `changed()`, break the loop, then wait for the dispatch.
        shutdown_tx.send(true).expect("send shutdown");

        // If the drain logic works, `run` returns only after the
        // dispatch (200ms total) finishes. Generous bound: 2s.
        tokio::time::timeout(Duration::from_secs(2), run_handle)
            .await
            .expect("run should return within 2s of shutdown signal")
            .expect("join")
            .expect("run result");

        assert_eq!(in_flight.in_flight(), 0, "tracker should be empty");
        assert!(
            !calls.lock().unwrap().is_empty(),
            "dispatch should have run"
        );
    }

    #[tokio::test]
    async fn run_abandons_in_flight_wait_when_force_signal_fires() {
        let pool = test_db::pool().await;
        let key = format!(
            "shutdown-force-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        seed_test_agent(&pool, &key).await;

        let (sub_agent_id,): (i64,) = sqlx::query_as(
            "SELECT id FROM harness_sub_agents
              WHERE agent_key = $1 AND sub_agent_key = 'analysis-15m'",
        )
        .bind(&key)
        .fetch_one(&pool)
        .await
        .expect("fetch candle_job id");
        pin_job_due(&pool, sub_agent_id, "15m").await;

        // The fake backend holds the dispatch for 30s so the only way
        // `run` can return quickly is via the force signal.
        let calls: Arc<Mutex<Vec<DispatchRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let backend_impl = Arc::new(FakeBackend::with_delay(
            calls.clone(),
            Duration::from_secs(30),
        ));
        let backend: Arc<dyn HarnessBackend> = backend_impl.clone();

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let (force_tx, force_rx) = watch::channel(false);
        let in_flight = InFlightTracker::new();
        let in_flight_for_run = in_flight.clone();
        let live_accounts = Arc::new(crate::hyperliquid::live_state::LiveAccountStore::new());
        let mut scheduler = HarnessScheduler::new(
            pool.clone(),
            shutdown_rx,
            force_rx,
            backend,
            live_accounts,
            scheduler_runtime(in_flight_for_run),
        );

        scheduler.tick().await.expect("first tick");
        let run_handle = tokio::spawn(async move { scheduler.run().await });

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(in_flight.in_flight() >= 1, "dispatch should be in flight");

        shutdown_tx.send(true).expect("send shutdown");
        // Let `run` enter its drain wait.
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            !run_handle.is_finished(),
            "run should still be waiting for the in-flight dispatch"
        );

        let force_started = std::time::Instant::now();
        force_tx.send(true).expect("send force");

        // Without the force short-circuit, this would block for the
        // 30-minute SHUTDOWN_IN_FLIGHT_GRACE. With force, the wait
        // unwinds quickly. Bound generously to avoid CI flake.
        tokio::time::timeout(Duration::from_secs(2), run_handle)
            .await
            .expect("run should return promptly after force signal")
            .expect("join")
            .expect("run result");
        assert!(
            force_started.elapsed() < Duration::from_secs(1),
            "force path should bypass the 30-minute grace; took {:?}",
            force_started.elapsed()
        );
        // The in-flight tracker is still occupied; we abandoned the
        // wait, not the dispatch itself.
        assert_eq!(in_flight.in_flight(), 1, "tracker should still hold");
        let _ = calls;
    }

    #[tokio::test]
    async fn run_returns_immediately_when_no_dispatches_are_in_flight() {
        let pool = test_db::pool().await;
        let (_shutdown_tx, shutdown_rx) = watch::channel(true);
        let (_force_tx, force_rx) = watch::channel(false);
        let in_flight = InFlightTracker::new();
        let backend: Arc<dyn HarnessBackend> =
            Arc::new(FakeBackend::success(Arc::new(Mutex::new(Vec::new()))));
        let live_accounts = Arc::new(crate::hyperliquid::live_state::LiveAccountStore::new());
        let scheduler = HarnessScheduler::new(
            pool.clone(),
            shutdown_rx,
            force_rx,
            backend,
            live_accounts,
            scheduler_runtime(in_flight),
        );

        // Pre-set shutdown so the loop exits immediately, then assert
        // `run` returns promptly.
        let started = std::time::Instant::now();
        tokio::time::timeout(Duration::from_secs(1), scheduler.run())
            .await
            .expect("run should return immediately with no in-flight work")
            .expect("run result");
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "run took {}ms; expected <500ms",
            started.elapsed().as_millis()
        );
        let _ = pool;
    }

    /// Regression test for the dispatch-outcome handling: a failed
    /// analysis dispatch must not be reported as success, and the
    /// `analysis_batch_completed` market-analysis follow-up event must
    /// not fire when the analysis batch itself failed.
    #[tokio::test]
    async fn tick_failed_dispatch_does_not_trigger_analysis_batch_completed_event() {
        let pool = test_db::pool().await;
        let key = format!(
            "sched-fail-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        seed_test_agent(&pool, &key).await;

        // Enable the default market-analysis event so we can verify it
        // is NOT triggered by a failed analysis batch.
        let sub_agent_id = store::list_agent_sub_agents(&pool, &key)
            .await
            .expect("list jobs")
            .into_iter()
            .find(|job| job.sub_agent_kind == SUB_AGENT_KIND_MARKET_ANALYSIS)
            .expect("default event present")
            .id;
        store::set_sub_agent_enabled(&pool, &key, sub_agent_id, true)
            .await
            .expect("enable default event");

        let (sub_agent_id,): (i64,) = sqlx::query_as(
            "SELECT id FROM harness_sub_agents
              WHERE agent_key = $1 AND sub_agent_key = 'analysis-15m'",
        )
        .bind(&key)
        .fetch_one(&pool)
        .await
        .expect("fetch analysis id");
        pin_job_due(&pool, sub_agent_id, "15m").await;

        let calls: Arc<Mutex<Vec<DispatchRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let failing: Arc<dyn HarnessBackend> = Arc::new(FailingBackend);
        let live_accounts = Arc::new(crate::hyperliquid::live_state::LiveAccountStore::new());
        let (_tx, rx) = watch::channel(false);
        let (_force_tx, force_rx) = watch::channel(false);
        let mut scheduler = HarnessScheduler::new(
            pool.clone(),
            rx,
            force_rx,
            failing,
            live_accounts,
            scheduler_runtime(InFlightTracker::new()),
        );
        scheduler.tick().await.expect("tick");

        // The failing backend records nothing; let the spawned task finish.
        run_until(|| async {
            list_run_statuses(&pool, &key)
                .await
                .iter()
                .any(|status| status == crate::harness::model::RUN_STATUS_FAILED)
        })
        .await;

        // No dispatch should have succeeded and no follow-up event call
        // should have been issued.
        assert!(
            calls.lock().unwrap().is_empty(),
            "failing backend must not produce successful dispatches or event calls"
        );

        // Only the failed analysis run should exist; no market-analysis run.
        let runs = store::list_agent_runs(&pool, &key, 20)
            .await
            .expect("list runs");
        let sub_agent_keys: Vec<String> = runs.iter().map(|r| r.sub_agent_key.clone()).collect();
        assert!(
            !sub_agent_keys.iter().any(|k| k == "market-analysis"),
            "market-analysis event must not fire after a failed analysis batch, got {sub_agent_keys:?}"
        );
        assert!(
            sub_agent_keys.iter().any(|k| k == "analysis-15m"),
            "analysis run should be present, got {sub_agent_keys:?}"
        );
    }
}
