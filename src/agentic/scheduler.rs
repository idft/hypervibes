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
    agentic::{
        backend::{
            AgenticBackend, DispatchOutcome, DispatchRequest, dispatch_with_timeout,
            dispatch_with_timeout_for_coding,
        },
        in_flight::{InFlightTracker, SHUTDOWN_IN_FLIGHT_GRACE},
        model::{
            DueOpenCodeHookRow, DueOpenCodeScheduleRow, HOOK_EVENT_ANALYSIS_BATCH_COMPLETED,
            JOB_KIND_ANALYSIS, JOB_KIND_DAILY_REVIEW, JOB_KIND_MARKET_ANALYSIS, JOB_KIND_TRADING,
            MAINTENANCE_STATUS_QUEUED,
        },
        store,
        timeframe::{boundary_for_due_at, parse_timeframe_seconds},
        workspace_lease::WorkspaceLeaseManager,
    },
    agents::{
        store::{get_agent, list_agent_instrument_ids, update_agent_runtime_config},
        strategy_prompts::{
            PROMPT_KIND_ANALYSIS, PROMPT_KIND_ANALYSIS_CODING, default_prompt_for_kind,
            get_agent_strategy_prompt, prompt_kind_for_job_kind,
        },
    },
    db::DbPool,
    hyperliquid::live_state::{LiveAccountStore, live_agent_snapshot_for_dispatch},
    memory::{delete_memories_for_agent, get_latest_agent_memory_by_type},
    opencode::{
        client::OpenCodeClient,
        coding_workspace::{
            PromotionJournalPhase, candidate_container_root, changed_paths,
            list_promotion_journals, live_user_root, manifest_hash, manifest_tree,
            prepare_coding_candidate, promote_user_tree, recover_promotion_journal,
            retain_successful_version, rollback_user_tree, set_promotion_journal_retained_version,
            update_promotion_journal_phase,
        },
        workspace::{
            OpenCodeWorkspaceAgent, OpenCodeWorkspaceConfig, OpenCodeWorkspaceRuntimeConfig,
            WorkspaceGenerationMode, delete_agent_workspace, generate_agent_workspace,
            runtime_config_for_generated_workspace,
        },
    },
    settings,
};

const SCHEDULER_POLL_INTERVAL: Duration = Duration::from_secs(10);
const ORPHAN_RECOVERY_INTERVAL: Duration = Duration::from_secs(60);
const DUE_SCHEDULE_LIMIT: i64 = 20;
const WORKSPACE_MAINTENANCE_SESSION_PROBE_LIMIT: i64 = 20;
const ACTIVE_OPENCODE_SESSION_STATUSES: [&str; 2] = ["busy", "retry"];

/// Periodic background loop that claims due OpenCode schedules and
/// dispatches them through an [`AgenticBackend`].
///
/// The scheduler is generic over the backend so tests can swap in a
/// fake implementation. In production this is `OpenCodeBackend`.
///
/// Schedules for the same agent run in two independent lanes:
/// analysis-lane work (`analysis` plus `market_analysis` hooks) and
/// trading-lane work (`trading`). Different agents may also run
/// concurrently.
pub struct AgenticScheduler {
    pool: DbPool,
    shutdown_rx: watch::Receiver<bool>,
    force_shutdown_rx: watch::Receiver<bool>,
    backend: Arc<dyn AgenticBackend>,
    live_accounts: Arc<LiveAccountStore>,
    opencode_workspace_config: OpenCodeWorkspaceConfig,
    opencode_client: Arc<OpenCodeClient>,
    last_orphan_recovery_at: Option<chrono::DateTime<Utc>>,
    in_flight: InFlightTracker,
    workspace_leases: WorkspaceLeaseManager,
    coding_semaphore: Arc<Semaphore>,
    lane_locks: SchedulerLaneLockManager,
}

pub struct AgenticSchedulerRuntime {
    pub opencode_workspace_config: OpenCodeWorkspaceConfig,
    pub opencode_client: Arc<OpenCodeClient>,
    pub in_flight: InFlightTracker,
}

/// Serializes repeated scheduler ticks for one agent and lane. The run store
/// serializes individual claims, but a later tick must not claim a second due
/// schedule while the earlier tick is still executing the first one.
#[derive(Clone, Default)]
struct SchedulerLaneLockManager {
    locks: LaneLockMap,
}

type LaneLockMap = Arc<Mutex<HashMap<(String, SchedulerLane), Weak<AsyncMutex<()>>>>>;

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
enum SchedulerLane {
    Analysis,
    Trading,
}

type AnalysisScheduleSortKey = (i64, chrono::DateTime<Utc>, i64);
type TradingScheduleSortKey = (chrono::DateTime<Utc>, i64, i64);
type IndexedAnalysisSchedule = (AnalysisScheduleSortKey, DueOpenCodeScheduleRow);
type IndexedTradingSchedule = (TradingScheduleSortKey, DueOpenCodeScheduleRow);

pub struct DispatchRequestInputs {
    pub run_id: i64,
    pub scheduled_for: chrono::DateTime<Utc>,
    pub agent: crate::agents::model::AgentDetailRow,
    pub selected_instruments: Vec<String>,
    pub strategy_prompt: String,
    pub accumulated_learnings: Option<String>,
    pub system_prompt: String,
}

impl SchedulerLaneLockManager {
    fn lock_for(&self, agent_key: &str, lane: SchedulerLane) -> Arc<AsyncMutex<()>> {
        let mut locks = self.locks.lock().expect("scheduler lane lock map poisoned");
        let key = (agent_key.to_string(), lane);
        if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
            return lock;
        }

        let lock = Arc::new(AsyncMutex::new(()));
        locks.insert(key, Arc::downgrade(&lock));
        lock
    }

    fn try_acquire(&self, agent_key: &str, lane: SchedulerLane) -> Option<OwnedMutexGuard<()>> {
        self.lock_for(agent_key, lane).try_lock_owned().ok()
    }
}

impl AgenticScheduler {
    #[cfg(test)]
    pub fn new(
        pool: DbPool,
        shutdown_rx: watch::Receiver<bool>,
        force_shutdown_rx: watch::Receiver<bool>,
        backend: Arc<dyn AgenticBackend>,
        live_accounts: Arc<LiveAccountStore>,
        runtime: AgenticSchedulerRuntime,
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
        backend: Arc<dyn AgenticBackend>,
        live_accounts: Arc<LiveAccountStore>,
        runtime: AgenticSchedulerRuntime,
        workspace_leases: WorkspaceLeaseManager,
    ) -> Self {
        Self {
            pool,
            shutdown_rx,
            force_shutdown_rx,
            backend,
            live_accounts,
            opencode_workspace_config: runtime.opencode_workspace_config,
            opencode_client: runtime.opencode_client,
            last_orphan_recovery_at: None,
            in_flight: runtime.in_flight,
            workspace_leases,
            coding_semaphore: Arc::new(Semaphore::new(1)),
            lane_locks: SchedulerLaneLockManager::default(),
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
        info!("agentic scheduler starting");
        match list_promotion_journals(&self.opencode_workspace_config) {
            Ok(journals) => {
                let mut recovered = 0;
                for journal in journals {
                    match recover_promotion_journal(&self.opencode_workspace_config, &journal) {
                        Ok(PromotionJournalPhase::Completed) => {
                            let _ =
                                store::mark_maintenance_task_succeeded(&self.pool, journal.task_id)
                                    .await;
                            recovered += 1;
                        }
                        Ok(PromotionJournalPhase::RolledBack) => {
                            let _ = store::mark_maintenance_task_failed(
                                &self.pool,
                                journal.task_id,
                                "promotion was rolled back during startup recovery",
                            )
                            .await;
                            recovered += 1;
                        }
                        Ok(_) => {}
                        Err(error) => {
                            warn!(error = ?error, task_id = journal.task_id, "promotion journal recovery failed")
                        }
                    }
                }
                if recovered > 0 {
                    info!(recovered, "reconciled promotion journals at startup");
                }
            }
            Err(error) => warn!(error = ?error, "promotion journal recovery failed"),
        }
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

        let in_flight = self.in_flight.in_flight();
        if in_flight > 0 {
            info!(
                in_flight,
                grace_seconds = SHUTDOWN_IN_FLIGHT_GRACE.as_secs(),
                "waiting for in-flight agentic dispatches to complete"
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
                    "in-flight agentic dispatches did not drain; \
                     leaving them orphaned for the next start to recover"
                );
            } else {
                info!("all in-flight agentic dispatches completed");
            }
        }

        info!("agentic scheduler stopped");
        Ok(())
    }

    /// One scheduling pass: load due schedules, claim each, and run them in
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
            &self.opencode_workspace_config,
            &self.opencode_client,
            &self.workspace_leases,
        )
        .await?;

        spawn_coding_workers(
            &self.pool,
            &self.backend,
            &self.opencode_workspace_config,
            &self.in_flight,
            &self.coding_semaphore,
            &self.workspace_leases,
            self.opencode_client.base_url(),
        )
        .await?;

        let now = Utc::now();
        let due = store::list_due_opencode_schedules(
            &self.pool,
            now,
            DUE_SCHEDULE_LIMIT,
            self.opencode_client.base_url(),
        )
        .await?;
        debug!(count = due.len(), "due opencode schedules loaded");

        if due.is_empty() {
            return Ok(());
        }

        let mut by_agent: BTreeMap<String, Vec<DueOpenCodeScheduleRow>> = BTreeMap::new();
        for schedule in due {
            by_agent
                .entry(schedule.agent_key.clone())
                .or_default()
                .push(schedule);
        }

        for (agent_key, schedules) in by_agent {
            if *self.shutdown_rx.borrow() {
                break;
            }

            let mut analysis_schedules = Vec::new();
            let mut daily_review_schedules = Vec::new();
            let mut trading_schedules = Vec::new();
            for schedule in schedules {
                match schedule.job_kind.as_str() {
                    JOB_KIND_ANALYSIS => analysis_schedules.push(schedule),
                    JOB_KIND_DAILY_REVIEW => daily_review_schedules.push(schedule),
                    JOB_KIND_TRADING => trading_schedules.push(schedule),
                    _ => {}
                }
            }

            if (!analysis_schedules.is_empty() || !daily_review_schedules.is_empty())
                && let Some(lane_guard) = self
                    .lane_locks
                    .try_acquire(&agent_key, SchedulerLane::Analysis)
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
                        sort_analysis_schedules_for_dispatch(analysis_schedules),
                        sort_analysis_schedules_for_dispatch(daily_review_schedules),
                        &workspace_leases,
                    )
                    .await;
                });
            }

            if !trading_schedules.is_empty() {
                let Some(lane_guard) = self
                    .lane_locks
                    .try_acquire(&agent_key, SchedulerLane::Trading)
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
                        sort_trading_schedules_for_dispatch(trading_schedules),
                        &workspace_leases,
                    )
                    .await;
                });
            }
        }

        Ok(())
    }

    /// Periodically sweep every agent's `agentic_runs` for orphans and
    /// mark them with a terminal status. The per-lane recovery in
    /// [`store::recovery::recover_inactive_runs_in_lane_tx`] only fires
    /// when a new run is claimed for the same lane, so a `running` run
    /// whose dispatch worker has died would otherwise block its lane
    /// until something else claimed the schedule. This periodic sweep
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
                "recovered inactive agentic runs during periodic sweep"
            );
        }
        let stale_before = now - chrono::Duration::minutes(2);
        for task in
            store::list_stale_running_maintenance_tasks(&self.pool, stale_before, 20).await?
        {
            if task.task_kind != crate::agentic::model::MAINTENANCE_TASK_KIND_ANALYSIS_CODING {
                continue;
            }
            if task.phase == crate::agentic::model::MAINTENANCE_PHASE_GENERATING
                && let Some(run_id) = task.run_id
                && let Some(run) = store::get_run(&self.pool, run_id).await?
                && let Some(session_id) = run.backend_run_ref
                && let Some(hook_id) = task.parameter_i64("hook_id")
                && let Some(hook) = store::get_opencode_hook_for_dispatch(
                    &self.pool,
                    &task.agent_key,
                    hook_id,
                    self.opencode_client.base_url(),
                )
                .await?
                && let Some(status) = self
                    .backend
                    .get_session_status(&hook.opencode_base_url, &session_id)
                    .await?
                && status.is_active()
            {
                debug!(task_id = task.id, session_id = %session_id, "coding session remains active during stale-task sweep");
                continue;
            }
            warn!(task_id = task.id, phase = %task.phase, "recovering stale coding task");
            if task.is_in_promotion_window()
                && let Some(journal) = crate::opencode::coding_workspace::read_promotion_journal(
                    &self.opencode_workspace_config,
                    &task.agent_key,
                    task.id,
                )?
            {
                match recover_promotion_journal(&self.opencode_workspace_config, &journal)? {
                    PromotionJournalPhase::Completed => {
                        store::mark_maintenance_task_succeeded(&self.pool, task.id).await?;
                        if let Some(run_id) = task.run_id {
                            store::mark_run_succeeded(&self.pool, run_id, None).await?;
                        }
                        continue;
                    }
                    PromotionJournalPhase::RolledBack => {
                        // Fall through to terminal failure after restoring the
                        // previous live tree.
                    }
                    _ => {}
                }
            }
            let summary = format!("stale coding task recovered during {} phase", task.phase);
            store::mark_maintenance_task_failed(&self.pool, task.id, &summary).await?;
            if let Some(run_id) = task.run_id {
                store::mark_run_failed(&self.pool, run_id, &summary, None).await?;
            }
        }
        self.last_orphan_recovery_at = Some(now);
        Ok(())
    }
}

async fn process_workspace_maintenance_tasks(
    pool: &DbPool,
    workspace_config: &OpenCodeWorkspaceConfig,
    _opencode_client: &Arc<OpenCodeClient>,
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

    let sessions = crate::opencode::store::list_sessions_for_directory(
        pool,
        &workspace_runtime.workspace_container_path,
        WORKSPACE_MAINTENANCE_SESSION_PROBE_LIMIT,
    )
    .await?;

    for session in sessions {
        if ACTIVE_OPENCODE_SESSION_STATUSES.contains(&session.status.as_deref().unwrap_or_default())
        {
            debug!(
                task_id = task.id,
                agent_key = %task.agent_key,
                session_id = %session.id,
                status = ?session.status,
                "workspace maintenance remains queued while an OpenCode session is active"
            );
            return Ok(());
        }
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
        let workspace_agent = OpenCodeWorkspaceAgent {
            agent_key: agent.agent_key.clone(),
            display_name: agent.display_name.clone(),
            api_key: agent.api_key.clone(),
        };

        if hard_reset {
            let _ = delete_agent_workspace(workspace_config, &agent.agent_key)?;
        }

        let generated = generate_agent_workspace(
            workspace_config,
            &workspace_agent,
            WorkspaceGenerationMode::Regenerate,
        )?;
        let runtime_config = runtime_config_for_generated_workspace(&generated).into_value();
        if !update_agent_runtime_config(pool, &agent.agent_key, runtime_config).await? {
            anyhow::bail!("agent disappeared before workspace metadata update");
        }
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

const CODING_QUEUE_LIMIT: i64 = 4;

async fn spawn_coding_workers(
    pool: &DbPool,
    backend: &Arc<dyn AgenticBackend>,
    workspace_config: &OpenCodeWorkspaceConfig,
    in_flight: &InFlightTracker,
    semaphore: &Arc<Semaphore>,
    workspace_leases: &WorkspaceLeaseManager,
    opencode_base_url: &str,
) -> Result<()> {
    let tasks = store::list_queued_maintenance_candidates(pool, CODING_QUEUE_LIMIT).await?;
    for task in tasks {
        if task.task_kind != crate::agentic::model::MAINTENANCE_TASK_KIND_ANALYSIS_CODING {
            continue;
        }
        let pool = pool.clone();
        let backend = backend.clone();
        let workspace_config = workspace_config.clone();
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
                &workspace_config,
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
    backend: &Arc<dyn AgenticBackend>,
    workspace_config: &OpenCodeWorkspaceConfig,
    workspace_leases: &WorkspaceLeaseManager,
    opencode_base_url: &str,
    task: crate::agentic::model::AgentMaintenanceTaskRow,
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
    let Some(hook_id) = task.parameter_i64("hook_id") else {
        fail_coding_task(pool, task.id, run_id, "coding task has no hook").await?;
        return Ok(());
    };
    let Some(hook) =
        store::get_opencode_hook_for_dispatch(pool, &task.agent_key, hook_id, opencode_base_url)
            .await?
    else {
        fail_coding_task(pool, task.id, run_id, "coding hook disappeared").await?;
        return Ok(());
    };

    let live_root = live_user_root(workspace_config, &task.agent_key)?;
    let canonical_exists = live_root.join("analyze.py").is_file();
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

    let candidate = match prepare_coding_candidate(
        workspace_config,
        &task.agent_key,
        task.id,
        &agent.display_name,
        &agent.api_key,
    ) {
        Ok(candidate) => candidate,
        Err(error) => {
            fail_coding_task(pool, task.id, run_id, &error.to_string()).await?;
            return Ok(());
        }
    };
    store::compare_and_set_maintenance_phase(
        pool,
        task.id,
        crate::agentic::model::MAINTENANCE_PHASE_PREPARING,
        crate::agentic::model::MAINTENANCE_PHASE_GENERATING,
    )
    .await?;

    let Some(mut request) = build_hook_dispatch_request(pool, &hook, run_id, Utc::now()).await?
    else {
        fail_coding_task(pool, task.id, run_id, "coding request could not be built").await?;
        return Ok(());
    };
    let analysis_strategy_prompt =
        get_agent_strategy_prompt(pool, &task.agent_key, PROMPT_KIND_ANALYSIS)
            .await?
            .map(|row| row.prompt);
    request.runtime_config = serde_json::json!({
        "workspace_host_path": candidate.root.to_string_lossy(),
        "workspace_container_path": candidate_container_root(workspace_config, &task.agent_key, task.id)?,
        "profile_source": "agent-runtime/workspace-template",
        "coding_task_id": task.id,
        "coding_mode": effective_mode,
        "analysis_strategy_prompt": analysis_strategy_prompt,
    });
    let outcome = dispatch_coding_model(pool, backend.clone(), request, task.id).await?;
    if !outcome.succeeded {
        fail_coding_task(pool, task.id, run_id, "coding model dispatch failed").await?;
        return Ok(());
    }

    let candidate_manifest = manifest_tree(&candidate.user_root)?;
    let actual_changes = changed_paths(&candidate.base_manifest, &candidate_manifest);
    let report = match validate_coding_report(&candidate.root, &actual_changes) {
        Ok(outcome) => outcome,
        Err(error) => {
            fail_coding_task(pool, task.id, run_id, &error.to_string()).await?;
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
        let _ = std::fs::remove_dir_all(&candidate.root);
    } else {
        store::compare_and_set_maintenance_phase(
            pool,
            task.id,
            crate::agentic::model::MAINTENANCE_PHASE_GENERATING,
            crate::agentic::model::MAINTENANCE_PHASE_VALIDATING,
        )
        .await?;
        let candidate_manifest_hash = manifest_hash(&candidate_manifest);
        if let Err(error) =
            require_coding_validation(&candidate.root, task.id, &candidate_manifest_hash)
        {
            fail_coding_task(pool, task.id, run_id, &error.to_string()).await?;
            return Ok(());
        }
        store::compare_and_set_maintenance_phase(
            pool,
            task.id,
            crate::agentic::model::MAINTENANCE_PHASE_VALIDATING,
            crate::agentic::model::MAINTENANCE_PHASE_WAITING_FOR_PROMOTION,
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
            crate::agentic::model::MAINTENANCE_PHASE_WAITING_FOR_PROMOTION,
            crate::agentic::model::MAINTENANCE_PHASE_PROMOTING,
        )
        .await?;
        let _backup = match promote_user_tree(
            workspace_config,
            &task.agent_key,
            task.id,
            &candidate.base_manifest,
        ) {
            Ok(backup) => backup,
            Err(error) => {
                fail_coding_task(pool, task.id, run_id, &error.to_string()).await?;
                return Ok(());
            }
        };
        store::compare_and_set_maintenance_phase(
            pool,
            task.id,
            crate::agentic::model::MAINTENANCE_PHASE_PROMOTING,
            crate::agentic::model::MAINTENANCE_PHASE_SMOKE_TESTING,
        )
        .await?;
        let live_workspace = workspace_config
            .host_workspaces_root
            .join("agents")
            .join(&task.agent_key);
        let promoted_manifest_hash = manifest_tree(&live_workspace.join("scripts/user"))
            .map(|manifest| manifest_hash(&manifest));
        let promoted_matches = matches!(
            &promoted_manifest_hash,
            Ok(hash) if hash == &candidate_manifest_hash
        );
        if !promoted_matches {
            let _ = rollback_user_tree(workspace_config, &task.agent_key, task.id);
            let _ = update_promotion_journal_phase(
                workspace_config,
                &task.agent_key,
                task.id,
                PromotionJournalPhase::RolledBack,
            );
            let summary = match promoted_manifest_hash {
                Ok(_) => "promoted analysis tree does not match validated candidate".to_string(),
                Err(error) => format!("failed to hash promoted analysis tree: {error}"),
            };
            fail_coding_task(pool, task.id, run_id, &summary).await?;
            return Ok(());
        }
        update_promotion_journal_phase(
            workspace_config,
            &task.agent_key,
            task.id,
            PromotionJournalPhase::SmokeTestPassed,
        )?;
        let retained = retain_successful_version(workspace_config, &task.agent_key, task.id)?;
        set_promotion_journal_retained_version(
            workspace_config,
            &task.agent_key,
            task.id,
            retained,
        )?;
        update_promotion_journal_phase(
            workspace_config,
            &task.agent_key,
            task.id,
            PromotionJournalPhase::Completed,
        )?;
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
        let _ = std::fs::remove_dir_all(&candidate.root);
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
    backend: Arc<dyn AgenticBackend>,
    request: DispatchRequest,
    task_id: i64,
) -> Result<DispatchRunResult> {
    store::mark_run_running(pool, request.run_id, None).await?;
    let dispatch = dispatch_with_timeout_for_coding(pool, backend, request);
    tokio::pin!(dispatch);
    let mut heartbeat = tokio::time::interval(Duration::from_secs(15));
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
            Ok(DispatchRunResult { succeeded: true })
        }
        DispatchOutcome::Failed { summary } => {
            debug!(task_id, summary, "coding model dispatch failed");
            Ok(DispatchRunResult { succeeded: false })
        }
    }
}

struct CodingReport {
    outcome: String,
    summary: String,
    rationale: String,
    validation_notes: String,
    evidence_memory_ids: Vec<uuid::Uuid>,
}

fn require_coding_validation(
    candidate_root: &std::path::Path,
    task_id: i64,
    candidate_manifest_hash: &str,
) -> Result<()> {
    let path = candidate_root
        .parent()
        .context("candidate task path has no parent")?
        .join("coding-validation.json");
    let validation = std::fs::read_to_string(path)
        .context("coding model did not run fixed candidate validation")?;
    let validation: serde_json::Value =
        serde_json::from_str(&validation).context("coding validation result is not valid JSON")?;
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
    candidate_root: &std::path::Path,
    actual_changes: &[String],
) -> Result<CodingReport> {
    let report_path = candidate_root
        .parent()
        .context("candidate task path has no parent")?
        .join("coding-report.json");
    let report =
        std::fs::read_to_string(&report_path).context("coding model did not submit a report")?;
    let report: serde_json::Value =
        serde_json::from_str(&report).context("coding report is not valid JSON")?;
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
    task: &crate::agentic::model::AgentMaintenanceTaskRow,
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
            "source_run_id": task.source_run_id,
            "source_memory_id": task.source_memory_id,
            "source_daily_review_run_id": task.source_run_id,
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
    backend: &Arc<dyn AgenticBackend>,
    live_accounts: &Arc<LiveAccountStore>,
    agent_key: &str,
    schedules: Vec<DueOpenCodeScheduleRow>,
    daily_review_schedules: Vec<DueOpenCodeScheduleRow>,
    workspace_leases: &WorkspaceLeaseManager,
) {
    let opencode_base_url = schedules
        .first()
        .or_else(|| daily_review_schedules.first())
        .map(|schedule| schedule.opencode_base_url.clone())
        .unwrap_or_else(|| "http://localhost:14096".to_string());
    let _lease = workspace_leases.acquire_live_read(agent_key).await;
    let mut any_succeeded = false;
    for schedule in schedules {
        if process_schedule_for_agent(
            pool,
            backend,
            live_accounts,
            agent_key,
            schedule,
            workspace_leases,
        )
        .await
        .is_some()
        {
            any_succeeded = true;
        }
    }

    if any_succeeded
        && let Err(error) = dispatch_analysis_batch_completed_hook(
            pool,
            backend,
            live_accounts,
            agent_key,
            workspace_leases,
            &opencode_base_url,
        )
        .await
    {
        warn!(agent_key, error = ?error, "failed to dispatch analysis batch completed hook");
    }

    for schedule in daily_review_schedules {
        if let Some(run_id) = process_schedule_for_agent(
            pool,
            backend,
            live_accounts,
            agent_key,
            schedule,
            workspace_leases,
        )
        .await
            && let Err(error) = dispatch_daily_review_coding_hook(pool, agent_key, run_id).await
        {
            warn!(agent_key, error = ?error, "failed to process daily-review coding request");
        }
    }
}

async fn process_trading_lane_for_agent(
    pool: &DbPool,
    backend: &Arc<dyn AgenticBackend>,
    live_accounts: &Arc<LiveAccountStore>,
    agent_key: &str,
    schedules: Vec<DueOpenCodeScheduleRow>,
    workspace_leases: &WorkspaceLeaseManager,
) {
    let _lease = workspace_leases.acquire_live_read(agent_key).await;
    for schedule in schedules {
        let _ = process_schedule_for_agent(
            pool,
            backend,
            live_accounts,
            agent_key,
            schedule,
            workspace_leases,
        )
        .await;
    }
}

async fn process_schedule_for_agent(
    pool: &DbPool,
    backend: &Arc<dyn AgenticBackend>,
    live_accounts: &Arc<LiveAccountStore>,
    agent_key: &str,
    schedule: DueOpenCodeScheduleRow,
    _workspace_leases: &WorkspaceLeaseManager,
) -> Option<i64> {
    let schedule_id = schedule.schedule_id;
    let job_key = schedule.job_key.clone();
    let scheduled_for = boundary_for_due_at(schedule.next_run_at, schedule.trigger_delay_seconds);

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
            return None;
        }
    };

    match claim {
        store::ClaimedScheduleRun::NotDue => {
            debug!(
                schedule_id,
                agent_key, "schedule no longer due at claim time"
            );
            None
        }
        store::ClaimedScheduleRun::BlockedByMaintenance => {
            info!(
                schedule_id,
                agent_key,
                job_key = %job_key,
                "scheduled dispatch held because workspace maintenance is queued or running"
            );
            None
        }
        store::ClaimedScheduleRun::Skipped { run_id } => {
            info!(
                schedule_id,
                run_id,
                agent_key,
                job_key = %job_key,
                "agentic run skipped because previous run still active"
            );
            None
        }
        store::ClaimedScheduleRun::Dispatch { run_id } => {
            match build_dispatch_request(pool, live_accounts, &schedule, run_id, scheduled_for)
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
                    None
                }
                Err(error) => {
                    error!(
                        run_id,
                        agent_key = %agent_key,
                        job_key = %job_key,
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

pub fn dispatch_request_from_schedule(
    schedule: &DueOpenCodeScheduleRow,
    inputs: DispatchRequestInputs,
    account_snapshot: Option<crate::hyperliquid::live_state::LiveAgentSnapshot>,
) -> DispatchRequest {
    DispatchRequest {
        run_id: inputs.run_id,
        schedule_id: Some(schedule.schedule_id),
        hook_id: None,
        agent_key: schedule.agent_key.clone(),
        display_name: schedule.display_name.clone(),
        job_key: schedule.job_key.clone(),
        job_kind: schedule.job_kind.clone(),
        timeframe: Some(schedule.timeframe.clone()),
        operator_prompt: schedule.operator_prompt.clone(),
        strategy_prompt: inputs.strategy_prompt,
        accumulated_learnings: inputs.accumulated_learnings,
        system_prompt: inputs.system_prompt,
        environment: inputs.agent.environment.clone(),
        selected_instruments: inputs.selected_instruments,
        account_snapshot,
        model_provider_id: schedule.model_provider_id.clone(),
        model_id: schedule.model_id.clone(),
        timeout_seconds: schedule.timeout_seconds,
        opencode_base_url: schedule.opencode_base_url.clone(),
        runtime_config: schedule.runtime_config.clone(),
        scheduled_for: inputs.scheduled_for,
        review_window_start: None,
        review_window_end: None,
    }
}

pub fn dispatch_request_from_hook(
    hook: &DueOpenCodeHookRow,
    inputs: DispatchRequestInputs,
) -> DispatchRequest {
    DispatchRequest {
        run_id: inputs.run_id,
        schedule_id: None,
        hook_id: Some(hook.hook_id),
        agent_key: hook.agent_key.clone(),
        display_name: hook.display_name.clone(),
        job_key: hook.job_key.clone(),
        job_kind: hook.job_kind.clone(),
        timeframe: None,
        operator_prompt: hook.operator_prompt.clone(),
        strategy_prompt: inputs.strategy_prompt,
        accumulated_learnings: inputs.accumulated_learnings,
        system_prompt: inputs.system_prompt,
        environment: inputs.agent.environment.clone(),
        selected_instruments: inputs.selected_instruments,
        account_snapshot: None,
        model_provider_id: hook.model_provider_id.clone(),
        model_id: hook.model_id.clone(),
        timeout_seconds: hook.timeout_seconds,
        opencode_base_url: hook.opencode_base_url.clone(),
        runtime_config: hook.runtime_config.clone(),
        scheduled_for: inputs.scheduled_for,
        review_window_start: None,
        review_window_end: None,
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
    if agent.lifecycle != crate::agents::model::AGENT_LIFECYCLE_ACTIVE {
        return Ok(None);
    }

    let selected_instruments = list_agent_instrument_ids(pool, &schedule.agent_key).await?;

    if selected_instruments.is_empty() && requires_selected_instruments(&schedule.job_kind) {
        return Ok(None);
    }

    let system_setting = settings::store::get_user_settings(pool, agent.user_id).await?;
    let system_prompt = system_setting
        .map(|s| s.opencode_system_prompt)
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| crate::agents::prompts::DEFAULT_SYSTEM_PROMPT.to_string());
    let strategy_prompt =
        load_strategy_prompt(pool, &schedule.agent_key, &schedule.job_kind).await?;
    let accumulated_learnings = load_accumulated_learnings(pool, &schedule.agent_key).await?;

    let account_snapshot = if schedule.job_kind == JOB_KIND_TRADING {
        Some(live_agent_snapshot_for_dispatch(
            agent.trading_account_address.as_deref().unwrap_or_default(),
            &agent.environment,
            live_accounts,
        ))
    } else {
        None
    };

    Ok(Some(dispatch_request_from_schedule(
        schedule,
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
    )))
}

pub(crate) async fn dispatch_daily_review_coding_hook(
    pool: &DbPool,
    agent_key: &str,
    source_run_id: i64,
) -> Result<()> {
    let Some(memory) =
        crate::memory::get_daily_review_memory_for_run(pool, agent_key, source_run_id).await?
    else {
        debug!(
            agent_key,
            source_run_id, "daily review wrote no linked memory"
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
    let Some(hook) = store::get_enabled_hook_for_event(
        pool,
        agent_key,
        crate::agentic::model::HOOK_EVENT_DAILY_REVIEW_COMPLETED,
    )
    .await?
    else {
        debug!(agent_key, "coding hook is disabled or missing");
        return Ok(());
    };
    store::insert_analysis_coding_task_and_run(
        pool,
        store::AnalysisCodingTaskRequest {
            agent_key,
            hook_id: hook.id,
            trigger_mode: store::CodingTriggerMode::Automatic,
            source_run_id: Some(source_run_id),
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

pub async fn build_hook_dispatch_request(
    pool: &DbPool,
    hook: &DueOpenCodeHookRow,
    run_id: i64,
    scheduled_for: chrono::DateTime<Utc>,
) -> Result<Option<DispatchRequest>> {
    let agent = get_agent(pool, &hook.agent_key)
        .await?
        .context("agent not found while building hook dispatch request")?;
    if agent.lifecycle != crate::agents::model::AGENT_LIFECYCLE_ACTIVE {
        return Ok(None);
    }

    let selected_instruments = list_agent_instrument_ids(pool, &hook.agent_key).await?;
    if selected_instruments.is_empty() && requires_selected_instruments(&hook.job_kind) {
        return Ok(None);
    }

    let system_setting = settings::store::get_user_settings(pool, agent.user_id).await?;
    let system_prompt = system_setting
        .map(|s| s.opencode_system_prompt)
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| crate::agents::prompts::DEFAULT_SYSTEM_PROMPT.to_string());
    let strategy_prompt = load_strategy_prompt(pool, &hook.agent_key, &hook.job_kind).await?;
    let accumulated_learnings = load_accumulated_learnings(pool, &hook.agent_key).await?;

    Ok(Some(dispatch_request_from_hook(
        hook,
        DispatchRequestInputs {
            run_id,
            scheduled_for,
            agent,
            selected_instruments,
            strategy_prompt,
            accumulated_learnings,
            system_prompt,
        },
    )))
}

fn requires_selected_instruments(job_kind: &str) -> bool {
    matches!(
        job_kind,
        JOB_KIND_ANALYSIS | JOB_KIND_MARKET_ANALYSIS | JOB_KIND_TRADING
    )
}

async fn load_strategy_prompt(pool: &DbPool, agent_key: &str, job_kind: &str) -> Result<String> {
    let prompt_kind = prompt_kind_for_job_kind(job_kind)
        .ok_or_else(|| anyhow!("unknown prompt kind for job kind {job_kind}"))?;
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

pub async fn dispatch_analysis_batch_completed_hook(
    pool: &DbPool,
    backend: &Arc<dyn AgenticBackend>,
    _live_accounts: &Arc<LiveAccountStore>,
    agent_key: &str,
    workspace_leases: &WorkspaceLeaseManager,
    opencode_base_url: &str,
) -> Result<()> {
    let Some(hook) =
        store::get_enabled_hook_for_event(pool, agent_key, HOOK_EVENT_ANALYSIS_BATCH_COMPLETED)
            .await?
    else {
        debug!(
            agent_key,
            "no enabled analysis batch completed hook configured"
        );
        return Ok(());
    };

    match store::insert_queued_hook_run_for_automatic_dispatch(pool, agent_key, hook.id).await? {
        store::QueuedHookRun::Dispatch {
            run_id,
            scheduled_for,
        } => {
            let Some(hook_dispatch) =
                store::get_opencode_hook_for_dispatch(pool, agent_key, hook.id, opencode_base_url)
                    .await?
            else {
                let _ =
                    store::mark_run_failed(pool, run_id, "hook disappeared before dispatch", None)
                        .await;
                return Ok(());
            };

            match build_hook_dispatch_request(pool, &hook_dispatch, run_id, scheduled_for).await {
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
        store::QueuedHookRun::Skipped { run_id } => {
            info!(
                agent_key,
                hook_id = hook.id,
                run_id,
                "hook run skipped because previous analysis-lane run is still active"
            );
        }
        store::QueuedHookRun::Missing => {
            debug!(
                agent_key,
                hook_id = hook.id,
                "hook disappeared before queue insert"
            );
        }
        store::QueuedHookRun::BlockedByMaintenance => {
            warn!(
                agent_key,
                hook_id = hook.id,
                "automatic follow-up hook was unexpectedly blocked by maintenance"
            );
        }
    }

    Ok(())
}

/// Run a single dispatch through the backend, awaiting its completion.
/// Sequential schedulers should call this so that the next schedule
/// for the same agent is not processed until the current run finishes.
pub struct DispatchRunResult {
    pub succeeded: bool,
}

pub async fn dispatch_run(
    pool: DbPool,
    backend: Arc<dyn AgenticBackend>,
    request: DispatchRequest,
) -> DispatchRunResult {
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
        return DispatchRunResult { succeeded: false };
    }

    match dispatch_with_timeout(&pool, backend, request).await {
        Ok(DispatchOutcome::Succeeded { backend_run_ref }) => {
            debug!(
                run_id,
                agent_key = %agent_key,
                job_key = %job_key,
                backend_run_ref,
                "agentic dispatch finished"
            );
            DispatchRunResult { succeeded: true }
        }
        Ok(DispatchOutcome::Failed { summary }) => {
            // `dispatch_with_timeout` already persisted the terminal
            // failure row with a sanitized summary. Treat this as a
            // non-success so follow-up hooks (e.g. market analysis)
            // are not triggered by a failed analysis run.
            debug!(
                run_id,
                agent_key = %agent_key,
                job_key = %job_key,
                summary,
                "agentic dispatch failed; follow-up hooks will not fire"
            );
            DispatchRunResult { succeeded: false }
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
            DispatchRunResult { succeeded: false }
        }
    }
}

/// Dispatch a live-workspace job while holding its agent's shared read lease.
/// The lease spans the complete OpenCode session, so promotion or regeneration
/// cannot expose a tree swap to an active session.
pub async fn dispatch_run_with_workspace_lease(
    pool: DbPool,
    backend: Arc<dyn AgenticBackend>,
    request: DispatchRequest,
    workspace_leases: &WorkspaceLeaseManager,
) -> DispatchRunResult {
    let _lease = workspace_leases.acquire_live_read(&request.agent_key).await;
    dispatch_run(pool, backend, request).await
}

fn timeframe_duration_for_sort(schedule: &DueOpenCodeScheduleRow) -> i64 {
    match parse_timeframe_seconds(&schedule.timeframe) {
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
    }
}

fn sort_analysis_schedules_for_dispatch(
    due: Vec<DueOpenCodeScheduleRow>,
) -> Vec<DueOpenCodeScheduleRow> {
    let mut indexed: Vec<IndexedAnalysisSchedule> = due
        .into_iter()
        .map(|schedule| {
            (
                (
                    timeframe_duration_for_sort(&schedule),
                    schedule.next_run_at,
                    schedule.schedule_id,
                ),
                schedule,
            )
        })
        .collect();
    indexed.sort_by_key(|(key, _)| *key);
    indexed.into_iter().map(|(_, row)| row).collect()
}

fn sort_trading_schedules_for_dispatch(
    due: Vec<DueOpenCodeScheduleRow>,
) -> Vec<DueOpenCodeScheduleRow> {
    let mut indexed: Vec<IndexedTradingSchedule> = due
        .into_iter()
        .map(|schedule| {
            (
                (
                    schedule.next_run_at,
                    timeframe_duration_for_sort(&schedule),
                    schedule.schedule_id,
                ),
                schedule,
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
        path::PathBuf,
        sync::{Arc, Mutex},
        time::{SystemTime, UNIX_EPOCH},
    };

    use async_trait::async_trait;
    use chrono::Utc;
    use serde_json::json;

    use crate::{
        agentic::{
            backend::{AgenticBackend, DispatchResult},
            in_flight::InFlightTracker,
            model::{AgenticRunRow, RUN_STATUS_SKIPPED, RUN_STATUS_SUCCEEDED},
            store::{self, ClaimedScheduleRun, insert_default_opencode_schedules, insert_test_run},
        },
        agents::{
            keys::derive_wallet_address,
            model::AgentRegistryRow,
            store::{insert_agent, replace_agent_instruments},
        },
        test_db,
    };

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
        let candidate_hash = manifest_hash(&manifest_tree(&user).expect("candidate manifest"));
        fs::write(
            root.join("coding-validation.json"),
            serde_json::to_vec(&json!({
                "schema_version": 1,
                "task_id": 42,
                "ok": true,
                "candidate_manifest_sha256": candidate_hash,
            }))
            .expect("serialize validation"),
        )
        .expect("write validation");

        require_coding_validation(&candidate, 42, &candidate_hash).expect("matching validation");
        assert!(require_coding_validation(&candidate, 43, &candidate_hash).is_err());
        assert!(require_coding_validation(&candidate, 42, "stale").is_err());

        fs::remove_dir_all(root).expect("remove validation fixture");
    }

    struct FakeBackend {
        calls: Arc<Mutex<Vec<DispatchRequest>>>,
        delay: Duration,
        active_calls: Arc<AtomicUsize>,
        max_active_calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl AgenticBackend for FakeBackend {
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

    fn sample_workspace_config() -> OpenCodeWorkspaceConfig {
        OpenCodeWorkspaceConfig {
            source_root: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join(crate::opencode::workspace::PROFILE_SOURCE_RELATIVE_PATH),
            host_workspaces_root: PathBuf::from("/tmp/opencode/vibetrading-scheduler-tests"),
            container_workspaces_root: "/workspaces".to_string(),
            api_base_url: "http://host.containers.internal:3003".to_string(),
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

    fn scheduler_runtime(in_flight: InFlightTracker) -> AgenticSchedulerRuntime {
        AgenticSchedulerRuntime {
            opencode_workspace_config: sample_workspace_config(),
            opencode_client: sample_opencode_client(),
            in_flight,
        }
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
    impl AgenticBackend for FailingBackend {
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
        sqlx::query(
            "UPDATE agentic_job_hooks
                SET model_provider_id = 'anthropic', model_id = 'claude-sonnet-test'
              WHERE agent_key = $1",
        )
        .bind(key)
        .execute(pool)
        .await
        .expect("seed hook model");
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
    async fn pin_schedule_due(
        pool: &DbPool,
        schedule_id: i64,
        timeframe: &str,
    ) -> chrono::DateTime<Utc> {
        let now = Utc::now();
        let due = crate::agentic::timeframe::latest_due_at_or_before(
            now,
            timeframe,
            crate::agentic::timeframe::DEFAULT_TRIGGER_DELAY_SECONDS,
        )
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
        due
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
        let due = pin_schedule_due(&pool, schedule_id, "15m").await;

        let calls: Arc<Mutex<Vec<DispatchRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let backend: Arc<dyn AgenticBackend> = Arc::new(FakeBackend::success(calls.clone()));

        let live_accounts = Arc::new(crate::hyperliquid::live_state::LiveAccountStore::new());
        let (_tx, rx) = watch::channel(false);
        let (_force_tx, force_rx) = watch::channel(false);
        let mut scheduler = AgenticScheduler::new(
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
            assert_eq!(request.job_key, "analysis-15m");
            assert_eq!(request.timeframe.as_deref(), Some("15m"));
            assert_eq!(
                request.scheduled_for,
                crate::agentic::timeframe::boundary_for_due_at(
                    due,
                    crate::agentic::timeframe::DEFAULT_TRIGGER_DELAY_SECONDS,
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
    async fn overlapping_ticks_do_not_skip_same_agent_analysis_schedules() {
        let pool = test_db::pool().await;
        let key = format!(
            "sched-seq-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        seed_test_agent(&pool, &key).await;

        // Force both analysis schedules to be due at the same boundary.
        let (fifteen_m_id,): (i64,) = sqlx::query_as(
            "SELECT id FROM agentic_job_schedules
              WHERE agent_key = $1 AND job_key = 'analysis-15m'",
        )
        .bind(&key)
        .fetch_one(&pool)
        .await
        .expect("fetch 15m schedule id");
        let (one_h_id,): (i64,) = sqlx::query_as(
            "SELECT id FROM agentic_job_schedules
              WHERE agent_key = $1 AND job_key = 'analysis-1h'",
        )
        .bind(&key)
        .fetch_one(&pool)
        .await
        .expect("fetch 1h schedule id");
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
        let backend: Arc<dyn AgenticBackend> = Arc::new(FakeBackend::with_delay(
            calls.clone(),
            Duration::from_millis(50),
        ));

        let live_accounts = Arc::new(crate::hyperliquid::live_state::LiveAccountStore::new());
        let (_tx, rx) = watch::channel(false);
        let (_force_tx, force_rx) = watch::channel(false);
        let mut scheduler = AgenticScheduler::new(
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
                .map(|request| request.job_key.clone())
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
        let backend_impl = Arc::new(FakeBackend::with_delay(
            calls.clone(),
            Duration::from_millis(150),
        ));
        let backend: Arc<dyn AgenticBackend> = backend_impl.clone();

        let live_accounts = Arc::new(crate::hyperliquid::live_state::LiveAccountStore::new());
        let (_tx, rx) = watch::channel(false);
        let (_force_tx, force_rx) = watch::channel(false);
        let mut scheduler = AgenticScheduler::new(
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
        let mut jobs: Vec<&str> = guard.iter().map(|r| r.job_key.as_str()).collect();
        jobs.sort();
        assert!(
            jobs == vec!["analysis-15m", "trading-1m"],
            "expected both lanes to dispatch, got {jobs:?}"
        );
        assert!(
            backend_impl.max_active_calls() >= 2,
            "expected same-agent lanes to overlap, max active calls was {}",
            backend_impl.max_active_calls()
        );
    }

    #[tokio::test]
    async fn tick_dispatches_market_analysis_hook_after_successful_analysis_batch() {
        let pool = test_db::pool().await;
        let key = format!(
            "sched-hook-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        seed_test_agent(&pool, &key).await;

        let hook_id = store::list_agent_hooks(&pool, &key)
            .await
            .expect("list hooks")
            .first()
            .expect("default hook present")
            .id;
        store::set_hook_enabled(&pool, &key, hook_id, true)
            .await
            .expect("enable default hook");

        let (schedule_id,): (i64,) = sqlx::query_as(
            "SELECT id FROM agentic_job_schedules
              WHERE agent_key = $1 AND job_key = 'analysis-15m'",
        )
        .bind(&key)
        .fetch_one(&pool)
        .await
        .expect("fetch analysis id");
        pin_schedule_due(&pool, schedule_id, "15m").await;

        let calls: Arc<Mutex<Vec<DispatchRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let backend: Arc<dyn AgenticBackend> = Arc::new(FakeBackend::success(calls.clone()));
        let live_accounts = Arc::new(crate::hyperliquid::live_state::LiveAccountStore::new());
        let (_tx, rx) = watch::channel(false);
        let (_force_tx, force_rx) = watch::channel(false);
        let mut scheduler = AgenticScheduler::new(
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
            .map(|request| request.job_key.as_str())
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
        let backend_impl = Arc::new(FakeBackend::with_delay(
            calls.clone(),
            Duration::from_millis(150),
        ));
        let backend: Arc<dyn AgenticBackend> = backend_impl.clone();

        let live_accounts = Arc::new(crate::hyperliquid::live_state::LiveAccountStore::new());
        let (_tx, rx) = watch::channel(false);
        let (_force_tx, force_rx) = watch::channel(false);
        let mut scheduler = AgenticScheduler::new(
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
        let (_force_tx, force_rx) = watch::channel(false);
        let mut scheduler = AgenticScheduler::new(
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

        let (schedule_id,): (i64,) = sqlx::query_as(
            "SELECT id FROM agentic_job_schedules
              WHERE agent_key = $1 AND job_key = 'analysis-15m'",
        )
        .bind(&key)
        .fetch_one(&pool)
        .await
        .expect("fetch schedule id");
        insert_test_run(&pool, schedule_id, "running")
            .await
            .expect("seed active run");
        store::insert_workspace_regenerate_task(&pool, &key, false, false)
            .await
            .expect("insert maintenance task");

        let backend: Arc<dyn AgenticBackend> =
            Arc::new(FakeBackend::success(Arc::new(Mutex::new(Vec::new()))));
        let live_accounts = Arc::new(crate::hyperliquid::live_state::LiveAccountStore::new());
        let (_tx, rx) = watch::channel(false);
        let (_force_tx, force_rx) = watch::channel(false);
        let mut scheduler = AgenticScheduler::new(
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
            crate::agentic::model::MAINTENANCE_STATUS_QUEUED
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

        let backend: Arc<dyn AgenticBackend> =
            Arc::new(FakeBackend::success(Arc::new(Mutex::new(Vec::new()))));
        let live_accounts = Arc::new(crate::hyperliquid::live_state::LiveAccountStore::new());
        let (_tx, rx) = watch::channel(false);
        let (_force_tx, force_rx) = watch::channel(false);
        let mut scheduler = AgenticScheduler::new(
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
            crate::agentic::model::MAINTENANCE_STATUS_QUEUED
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

        let backend: Arc<dyn AgenticBackend> =
            Arc::new(FakeBackend::success(Arc::new(Mutex::new(Vec::new()))));
        let live_accounts = Arc::new(crate::hyperliquid::live_state::LiveAccountStore::new());
        let (_tx, rx) = watch::channel(false);
        let (_force_tx, force_rx) = watch::channel(false);
        let mut scheduler = AgenticScheduler::new(
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
            crate::agentic::model::MAINTENANCE_STATUS_SUCCEEDED
        );

        let agent = get_agent(&pool, &key)
            .await
            .expect("get agent")
            .expect("agent present");
        let workspace = OpenCodeWorkspaceRuntimeConfig::from_value(&agent.runtime_config)
            .expect("updated workspace runtime config");
        let workspace_path = PathBuf::from(&workspace.workspace_host_path);
        assert!(workspace_path.join("AGENTS.md").exists());
        assert!(workspace_path.join("scripts/user").exists());

        let (memory_count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM memory.records WHERE agent_key = $1")
                .bind(&key)
                .fetch_one(&pool)
                .await
                .expect("count memories");
        assert_eq!(memory_count, 0);
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

        let backend: Arc<dyn AgenticBackend> =
            Arc::new(FakeBackend::success(Arc::new(Mutex::new(Vec::new()))));
        let live_accounts = Arc::new(crate::hyperliquid::live_state::LiveAccountStore::new());
        let (_tx, rx) = watch::channel(false);
        let (_force_tx, force_rx) = watch::channel(false);
        let mut scheduler = AgenticScheduler::new(
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
            crate::agentic::model::MAINTENANCE_STATUS_SUCCEEDED
        );
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

        let backend: Arc<dyn AgenticBackend> =
            Arc::new(FakeBackend::success(Arc::new(Mutex::new(Vec::new()))));
        let live_accounts = Arc::new(crate::hyperliquid::live_state::LiveAccountStore::new());
        let (_tx, rx) = watch::channel(false);
        let (_force_tx, force_rx) = watch::channel(false);
        let mut scheduler = AgenticScheduler::new(
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
            crate::agentic::model::MAINTENANCE_STATUS_SUCCEEDED
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

    #[tokio::test]
    async fn tick_does_not_spawn_dispatches_when_shutdown_is_already_signaled() {
        let pool = test_db::pool().await;
        let key = format!(
            "shutdown-noop-{}",
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

        let calls: Arc<Mutex<Vec<DispatchRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let backend: Arc<dyn AgenticBackend> = Arc::new(FakeBackend::success(calls.clone()));
        let (_shutdown_tx, shutdown_rx) = watch::channel(true);
        let (_force_tx, force_rx) = watch::channel(false);

        let live_accounts = Arc::new(crate::hyperliquid::live_state::LiveAccountStore::new());
        let mut scheduler = AgenticScheduler::new(
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

        let (schedule_id,): (i64,) = sqlx::query_as(
            "SELECT id FROM agentic_job_schedules
              WHERE agent_key = $1 AND job_key = 'analysis-15m'",
        )
        .bind(&key)
        .fetch_one(&pool)
        .await
        .expect("fetch schedule id");
        pin_schedule_due(&pool, schedule_id, "15m").await;

        // The fake backend takes 200ms; the in-flight tracker should
        // keep `run()` alive long enough for the dispatch to finish
        // after the shutdown signal is observed.
        let calls: Arc<Mutex<Vec<DispatchRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let backend_impl = Arc::new(FakeBackend::with_delay(
            calls.clone(),
            Duration::from_millis(200),
        ));
        let backend: Arc<dyn AgenticBackend> = backend_impl.clone();

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let (_force_tx, force_rx) = watch::channel(false);
        let in_flight = InFlightTracker::new();
        let in_flight_for_run = in_flight.clone();
        let live_accounts = Arc::new(crate::hyperliquid::live_state::LiveAccountStore::new());
        let mut scheduler = AgenticScheduler::new(
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

        let (schedule_id,): (i64,) = sqlx::query_as(
            "SELECT id FROM agentic_job_schedules
              WHERE agent_key = $1 AND job_key = 'analysis-15m'",
        )
        .bind(&key)
        .fetch_one(&pool)
        .await
        .expect("fetch schedule id");
        pin_schedule_due(&pool, schedule_id, "15m").await;

        // The fake backend holds the dispatch for 30s so the only way
        // `run` can return quickly is via the force signal.
        let calls: Arc<Mutex<Vec<DispatchRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let backend_impl = Arc::new(FakeBackend::with_delay(
            calls.clone(),
            Duration::from_secs(30),
        ));
        let backend: Arc<dyn AgenticBackend> = backend_impl.clone();

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let (force_tx, force_rx) = watch::channel(false);
        let in_flight = InFlightTracker::new();
        let in_flight_for_run = in_flight.clone();
        let live_accounts = Arc::new(crate::hyperliquid::live_state::LiveAccountStore::new());
        let mut scheduler = AgenticScheduler::new(
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
        let backend: Arc<dyn AgenticBackend> =
            Arc::new(FakeBackend::success(Arc::new(Mutex::new(Vec::new()))));
        let live_accounts = Arc::new(crate::hyperliquid::live_state::LiveAccountStore::new());
        let scheduler = AgenticScheduler::new(
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
    /// `analysis_batch_completed` market-analysis follow-up hook must
    /// not fire when the analysis batch itself failed.
    #[tokio::test]
    async fn tick_failed_dispatch_does_not_trigger_analysis_batch_completed_hook() {
        let pool = test_db::pool().await;
        let key = format!(
            "sched-fail-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        seed_test_agent(&pool, &key).await;

        // Enable the default market-analysis hook so we can verify it
        // is NOT triggered by a failed analysis batch.
        let hook_id = store::list_agent_hooks(&pool, &key)
            .await
            .expect("list hooks")
            .first()
            .expect("default hook present")
            .id;
        store::set_hook_enabled(&pool, &key, hook_id, true)
            .await
            .expect("enable default hook");

        let (schedule_id,): (i64,) = sqlx::query_as(
            "SELECT id FROM agentic_job_schedules
              WHERE agent_key = $1 AND job_key = 'analysis-15m'",
        )
        .bind(&key)
        .fetch_one(&pool)
        .await
        .expect("fetch analysis id");
        pin_schedule_due(&pool, schedule_id, "15m").await;

        let calls: Arc<Mutex<Vec<DispatchRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let failing: Arc<dyn AgenticBackend> = Arc::new(FailingBackend);
        let live_accounts = Arc::new(crate::hyperliquid::live_state::LiveAccountStore::new());
        let (_tx, rx) = watch::channel(false);
        let (_force_tx, force_rx) = watch::channel(false);
        let mut scheduler = AgenticScheduler::new(
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
                .any(|status| status == crate::agentic::model::RUN_STATUS_FAILED)
        })
        .await;

        // No dispatch should have succeeded and no follow-up hook call
        // should have been issued.
        assert!(
            calls.lock().unwrap().is_empty(),
            "failing backend must not produce successful dispatches or hook calls"
        );

        // Only the failed analysis run should exist; no market-analysis run.
        let runs = store::list_agent_runs(&pool, &key, 20)
            .await
            .expect("list runs");
        let job_keys: Vec<String> = runs.iter().map(|r| r.job_key.clone()).collect();
        assert!(
            !job_keys.iter().any(|k| k == "market-analysis"),
            "market-analysis hook must not fire after a failed analysis batch, got {job_keys:?}"
        );
        assert!(
            job_keys.iter().any(|k| k == "analysis-15m"),
            "analysis run should be present, got {job_keys:?}"
        );
    }
}
