use std::{collections::BTreeMap, sync::Arc, time::Duration};

use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use tokio::sync::watch;
#[cfg(test)]
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

use crate::{
    agentic::{
        backend::{AgenticBackend, DispatchRequest, dispatch_with_timeout},
        in_flight::{InFlightTracker, SHUTDOWN_IN_FLIGHT_GRACE},
        model::{
            DueOpenCodeHookRow, DueOpenCodeScheduleRow, HOOK_EVENT_ANALYSIS_BATCH_COMPLETED,
            JOB_KIND_ANALYSIS, JOB_KIND_DAILY_REVIEW, JOB_KIND_TRADING,
            MAINTENANCE_STATUS_QUEUED,
        },
        store,
        timeframe::{boundary_for_due_at, parse_timeframe_seconds},
    },
    agents::{
        model::BACKEND_KIND_OPENCODE,
        strategy_prompts::{get_agent_strategy_prompt, prompt_kind_for_job_kind},
        store::{get_agent, list_agent_instrument_ids, update_agent_runtime_config},
    },
    db::DbPool,
    hyperliquid::live_state::{LiveAccountStore, live_agent_snapshot_for_dispatch},
    memory::get_latest_agent_memory_by_type,
    opencode::{
        client::OpenCodeClient,
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
const OPENCODE_SESSION_STATUS_IDLE: &str = "idle";

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
}

impl AgenticScheduler {
    pub fn new(
        pool: DbPool,
        shutdown_rx: watch::Receiver<bool>,
        force_shutdown_rx: watch::Receiver<bool>,
        backend: Arc<dyn AgenticBackend>,
        live_accounts: Arc<LiveAccountStore>,
        opencode_workspace_config: OpenCodeWorkspaceConfig,
        opencode_client: Arc<OpenCodeClient>,
        in_flight: InFlightTracker,
    ) -> Self {
        Self {
            pool,
            shutdown_rx,
            force_shutdown_rx,
            backend,
            live_accounts,
            opencode_workspace_config,
            opencode_client,
            last_orphan_recovery_at: None,
            in_flight,
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
        )
        .await?;

        let now = Utc::now();
        let due = store::list_due_opencode_schedules(&self.pool, now, DUE_SCHEDULE_LIMIT).await?;
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

            if !analysis_schedules.is_empty() || !daily_review_schedules.is_empty() {
                let pool = self.pool.clone();
                let backend = self.backend.clone();
                let live_accounts = self.live_accounts.clone();
                let agent_key = agent_key.clone();
                let in_flight = self.in_flight.clone();
                tokio::spawn(async move {
                    let _guard = in_flight.track();
                    process_analysis_lane_for_agent(
                        &pool,
                        &backend,
                        &live_accounts,
                        &agent_key,
                        sort_analysis_schedules_for_dispatch(analysis_schedules),
                        sort_analysis_schedules_for_dispatch(daily_review_schedules),
                    )
                    .await;
                });
            }

            if !trading_schedules.is_empty() {
                let pool = self.pool.clone();
                let backend = self.backend.clone();
                let live_accounts = self.live_accounts.clone();
                let agent_key = agent_key.clone();
                let in_flight = self.in_flight.clone();
                tokio::spawn(async move {
                    let _guard = in_flight.track();
                    process_trading_lane_for_agent(
                        &pool,
                        &backend,
                        &live_accounts,
                        &agent_key,
                        sort_trading_schedules_for_dispatch(trading_schedules),
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
        self.last_orphan_recovery_at = Some(now);
        Ok(())
    }
}

async fn process_workspace_maintenance_tasks(
    pool: &DbPool,
    workspace_config: &OpenCodeWorkspaceConfig,
    opencode_client: &Arc<OpenCodeClient>,
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

    if agent.backend_kind != BACKEND_KIND_OPENCODE {
        let _ = store::mark_maintenance_task_failed(
            pool,
            task.id,
            "workspace maintenance is only supported for OpenCode agents",
        )
        .await;
        return Ok(());
    }

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

    let Some(runtime_base_url) = agent
        .runtime_base_url
        .as_deref()
        .filter(|url| !url.is_empty())
    else {
        let _ =
            store::mark_maintenance_task_failed(pool, task.id, "agent runtime base URL is missing")
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
        if session.status.as_deref() == Some(OPENCODE_SESSION_STATUS_IDLE) {
            continue;
        }

        match opencode_client
            .session_is_active(runtime_base_url, &session.id)
            .await
        {
            Ok(true) => {
                debug!(
                    task_id = task.id,
                    agent_key = %task.agent_key,
                    session_id = %session.id,
                    "workspace maintenance remains queued while an OpenCode session is active"
                );
                return Ok(());
            }
            Ok(false) => {}
            Err(error) => {
                warn!(
                    task_id = task.id,
                    agent_key = %task.agent_key,
                    session_id = %session.id,
                    error = ?error,
                    "failed to probe OpenCode session activity; leaving maintenance queued"
                );
                return Ok(());
            }
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

    let maintenance_result = async {
        let workspace_agent = OpenCodeWorkspaceAgent {
            agent_key: agent.agent_key.clone(),
            display_name: agent.display_name.clone(),
            api_key: agent.api_key.clone(),
        };

        if task.hard_reset {
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

        Ok::<(), anyhow::Error>(())
    }
    .await;

    match maintenance_result {
        Ok(()) => {
            store::mark_maintenance_task_succeeded(pool, task.id).await?;
            info!(
                task_id = task.id,
                agent_key = %task.agent_key,
                hard_reset = task.hard_reset,
                "workspace maintenance completed"
            );
        }
        Err(error) => {
            error!(
                task_id = task.id,
                agent_key = %task.agent_key,
                hard_reset = task.hard_reset,
                error = ?error,
                "workspace maintenance failed"
            );
            let summary = maintenance_error_summary(&error);
            store::mark_maintenance_task_failed(pool, task.id, &summary).await?;
        }
    }

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
) {
    let mut any_succeeded = false;
    for schedule in schedules {
        if process_schedule_for_agent(pool, backend, live_accounts, agent_key, schedule).await {
            any_succeeded = true;
        }
    }

    if any_succeeded
        && let Err(error) =
            dispatch_analysis_batch_completed_hook(pool, backend, live_accounts, agent_key).await
    {
        warn!(agent_key, error = ?error, "failed to dispatch analysis batch completed hook");
    }

    for schedule in daily_review_schedules {
        let _ = process_schedule_for_agent(pool, backend, live_accounts, agent_key, schedule).await;
    }
}

async fn process_trading_lane_for_agent(
    pool: &DbPool,
    backend: &Arc<dyn AgenticBackend>,
    live_accounts: &Arc<LiveAccountStore>,
    agent_key: &str,
    schedules: Vec<DueOpenCodeScheduleRow>,
) {
    for schedule in schedules {
        let _ = process_schedule_for_agent(pool, backend, live_accounts, agent_key, schedule).await;
    }
}

async fn process_schedule_for_agent(
    pool: &DbPool,
    backend: &Arc<dyn AgenticBackend>,
    live_accounts: &Arc<LiveAccountStore>,
    agent_key: &str,
    schedule: DueOpenCodeScheduleRow,
) -> bool {
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
            return false;
        }
    };

    match claim {
        store::ClaimedScheduleRun::NotDue => {
            debug!(
                schedule_id,
                agent_key, "schedule no longer due at claim time"
            );
            false
        }
        store::ClaimedScheduleRun::BlockedByMaintenance => {
            info!(
                schedule_id,
                agent_key,
                job_key = %job_key,
                "scheduled dispatch held because workspace maintenance is queued or running"
            );
            false
        }
        store::ClaimedScheduleRun::Skipped { run_id } => {
            info!(
                schedule_id,
                run_id,
                agent_key,
                job_key = %job_key,
                "agentic run skipped because previous run still active"
            );
            false
        }
        store::ClaimedScheduleRun::Dispatch { run_id } => {
            match build_dispatch_request(pool, live_accounts, &schedule, run_id, scheduled_for)
                .await
            {
                Ok(Some(request)) => {
                    dispatch_run(pool.clone(), backend.clone(), request)
                        .await
                        .succeeded
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
                    false
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
                    false
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
    strategy_prompt: String,
    accumulated_learnings: Option<String>,
    system_prompt: String,
    account_snapshot: Option<crate::hyperliquid::live_state::LiveAgentSnapshot>,
) -> DispatchRequest {
    DispatchRequest {
        run_id,
        schedule_id: Some(schedule.schedule_id),
        hook_id: None,
        agent_key: schedule.agent_key.clone(),
        display_name: schedule.display_name.clone(),
        job_key: schedule.job_key.clone(),
        job_kind: schedule.job_kind.clone(),
        timeframe: Some(schedule.timeframe.clone()),
        operator_prompt: schedule.operator_prompt.clone(),
        strategy_prompt,
        accumulated_learnings,
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
        review_window_start: None,
        review_window_end: None,
    }
}

pub fn dispatch_request_from_hook(
    hook: &DueOpenCodeHookRow,
    run_id: i64,
    scheduled_for: chrono::DateTime<Utc>,
    agent: &crate::agents::model::AgentDetailRow,
    selected_instruments: Vec<String>,
    strategy_prompt: String,
    accumulated_learnings: Option<String>,
    system_prompt: String,
) -> DispatchRequest {
    DispatchRequest {
        run_id,
        schedule_id: None,
        hook_id: Some(hook.hook_id),
        agent_key: hook.agent_key.clone(),
        display_name: hook.display_name.clone(),
        job_key: hook.job_key.clone(),
        job_kind: hook.job_kind.clone(),
        timeframe: None,
        operator_prompt: hook.operator_prompt.clone(),
        strategy_prompt,
        accumulated_learnings,
        system_prompt,
        environment: agent.environment.clone(),
        selected_instruments,
        account_snapshot: None,
        model_provider_id: hook.model_provider_id.clone(),
        model_id: hook.model_id.clone(),
        timeout_seconds: hook.timeout_seconds,
        runtime_base_url: hook.runtime_base_url.clone(),
        runtime_config: hook.runtime_config.clone(),
        scheduled_for,
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

    let selected_instruments = list_agent_instrument_ids(pool, &schedule.agent_key).await?;

    if selected_instruments.is_empty() {
        return Ok(None);
    }

    let system_setting = settings::store::get_setting(pool, "opencode_system_prompt").await?;
    let system_prompt = system_setting
        .map(|s| s.value)
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| crate::agents::prompts::DEFAULT_SYSTEM_PROMPT.to_string());
    let strategy_prompt = load_strategy_prompt(pool, &schedule.agent_key, &schedule.job_kind).await?;
    let accumulated_learnings = load_accumulated_learnings(pool, &schedule.agent_key).await?;

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
        strategy_prompt,
        accumulated_learnings,
        system_prompt,
        account_snapshot,
    )))
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

    let selected_instruments = list_agent_instrument_ids(pool, &hook.agent_key).await?;
    if selected_instruments.is_empty() {
        return Ok(None);
    }

    let system_setting = settings::store::get_setting(pool, "opencode_system_prompt").await?;
    let system_prompt = system_setting
        .map(|s| s.value)
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| crate::agents::prompts::DEFAULT_SYSTEM_PROMPT.to_string());
    let strategy_prompt = load_strategy_prompt(pool, &hook.agent_key, &hook.job_kind).await?;
    let accumulated_learnings = load_accumulated_learnings(pool, &hook.agent_key).await?;

    Ok(Some(dispatch_request_from_hook(
        hook,
        run_id,
        scheduled_for,
        &agent,
        selected_instruments,
        strategy_prompt,
        accumulated_learnings,
        system_prompt,
    )))
}

async fn load_strategy_prompt(pool: &DbPool, agent_key: &str, job_kind: &str) -> Result<String> {
    let prompt_kind = prompt_kind_for_job_kind(job_kind)
        .ok_or_else(|| anyhow!("unknown prompt kind for job kind {job_kind}"))?;
    Ok(get_agent_strategy_prompt(pool, agent_key, prompt_kind)
        .await?
        .map(|row| row.prompt)
        .unwrap_or_default())
}

async fn load_accumulated_learnings(pool: &DbPool, agent_key: &str) -> Result<Option<String>> {
    Ok(get_latest_agent_memory_by_type(pool, agent_key, "agent_learnings")
        .await?
        .map(|memory| {
            format!(
                "Summary: {}\nCreated at: {}\nContent: {}",
                memory.summary,
                memory.created_at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                memory.content
            )
        }))
}

pub async fn dispatch_analysis_batch_completed_hook(
    pool: &DbPool,
    backend: &Arc<dyn AgenticBackend>,
    _live_accounts: &Arc<LiveAccountStore>,
    agent_key: &str,
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
                store::get_opencode_hook_for_dispatch(pool, agent_key, hook.id).await?
            else {
                let _ =
                    store::mark_run_failed(pool, run_id, "hook disappeared before dispatch", None)
                        .await;
                return Ok(());
            };

            match build_hook_dispatch_request(pool, &hook_dispatch, run_id, scheduled_for).await {
                Ok(Some(request)) => {
                    let _ = dispatch_run(pool.clone(), backend.clone(), request).await;
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
        Ok(_) => {
            debug!(
                run_id,
                agent_key = %agent_key,
                job_key = %job_key,
                "agentic dispatch finished"
            );
            DispatchRunResult { succeeded: true }
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

fn timeframe_duration_for_sort(schedule: &DueOpenCodeScheduleRow) -> i64 {
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
    duration_seconds
}

fn sort_analysis_schedules_for_dispatch(
    due: Vec<DueOpenCodeScheduleRow>,
) -> Vec<DueOpenCodeScheduleRow> {
    let mut indexed: Vec<((i64, chrono::DateTime<Utc>, i64), DueOpenCodeScheduleRow)> = due
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
    let mut indexed: Vec<((chrono::DateTime<Utc>, i64, i64), DueOpenCodeScheduleRow)> = due
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

/// Convenience: spawn the scheduler on the current Tokio runtime and
/// return the join handle. The handle aborts when the runtime drops.
#[cfg(test)]
pub fn spawn(
    pool: DbPool,
    shutdown_rx: watch::Receiver<bool>,
    force_shutdown_rx: watch::Receiver<bool>,
    backend: Arc<dyn AgenticBackend>,
    live_accounts: Arc<LiveAccountStore>,
    opencode_workspace_config: OpenCodeWorkspaceConfig,
    opencode_client: Arc<OpenCodeClient>,
    in_flight: InFlightTracker,
) -> JoinHandle<Result<()>> {
    tokio::spawn(async move {
        AgenticScheduler::new(
            pool,
            shutdown_rx,
            force_shutdown_rx,
            backend,
            live_accounts,
            opencode_workspace_config,
            opencode_client,
            in_flight,
        )
        .run()
        .await
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    use std::{
        collections::BTreeSet,
        path::PathBuf,
        sync::{Arc, Mutex},
    };

    use async_trait::async_trait;
    use axum::{
        Router,
        extract::{Path as AxumPath, State as AxumState},
        http::StatusCode,
        routing::get,
    };
    use chrono::Utc;
    use serde_json::json;
    use tokio::net::TcpListener;

    use crate::{
        agentic::{
            backend::{AgenticBackend, DispatchResult},
            in_flight::InFlightTracker,
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

    async fn spawn_session_status_server(active_session_ids: &[String]) -> String {
        async fn session_status(
            AxumState(active_session_ids): AxumState<Arc<BTreeSet<String>>>,
            AxumPath(session_id): AxumPath<String>,
        ) -> StatusCode {
            if active_session_ids.contains(&session_id) {
                StatusCode::OK
            } else {
                StatusCode::NOT_FOUND
            }
        }

        let active_session_ids =
            Arc::new(active_session_ids.iter().cloned().collect::<BTreeSet<_>>());
        let app = Router::new()
            .route("/session/{session_id}/status", get(session_status))
            .with_state(active_session_ids);
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test session status server");
        let address = listener.local_addr().expect("read listener address");
        tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve test session status server");
        });

        format!("http://{address}")
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
            wallet_address: wallet,
            environment: "live".to_string(),
            api_key: format!("vta_{key}"),
            api_key_last_used_at: None,
            backend_kind: BACKEND_KIND_OPENCODE.to_string(),
            runtime_id: "opencode-local".to_string(),
            runtime_config: json!({
                "workspace_host_path": format!("workspaces/agents/{key}"),
                "workspace_container_path": format!("/workspaces/agents/{key}"),
                "profile_source": "agent-runtime/workspace-template"
            }),
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
            sample_workspace_config(),
            sample_opencode_client(),
            InFlightTracker::new(),
        );
        scheduler.tick().await.expect("tick");

        run_until(|| async { calls.lock().map(|guard| !guard.is_empty()).unwrap_or(false) }).await;

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
            sample_workspace_config(),
            sample_opencode_client(),
            InFlightTracker::new(),
        );
        scheduler.tick().await.expect("tick");

        run_until(|| async { calls.lock().map(|guard| guard.len() >= 2).unwrap_or(false) }).await;

        let guard = calls.lock().unwrap();
        assert_eq!(guard.len(), 2);
        let order: Vec<&str> = guard.iter().map(|r| r.job_key.as_str()).collect();
        // 15m is shorter than 1h, so it should dispatch first.
        assert_eq!(order, vec!["analysis-15m", "analysis-1h"]);
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
            sample_workspace_config(),
            sample_opencode_client(),
            InFlightTracker::new(),
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
            sample_workspace_config(),
            sample_opencode_client(),
            InFlightTracker::new(),
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
            sample_workspace_config(),
            sample_opencode_client(),
            InFlightTracker::new(),
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
            sample_workspace_config(),
            sample_opencode_client(),
            InFlightTracker::new(),
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
        store::insert_workspace_regenerate_task(&pool, &key, false)
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
            sample_workspace_config(),
            sample_opencode_client(),
            InFlightTracker::new(),
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
        store::insert_workspace_regenerate_task(&pool, &key, false)
            .await
            .expect("insert maintenance task");

        let session_id = format!(
            "ses-maint-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        let directory = format!("/workspaces/agents/{key}");
        sqlx::query(
            "INSERT INTO opencode.sessions (id, directory, updated_at)
             VALUES ($1, $2, now())",
        )
        .bind(&session_id)
        .bind(&directory)
        .execute(&pool)
        .await
        .expect("insert active session row");

        let base_url = spawn_session_status_server(std::slice::from_ref(&session_id)).await;
        sqlx::query("UPDATE agent_runtimes SET base_url = $2 WHERE id = $1")
            .bind("opencode-local")
            .bind(&base_url)
            .execute(&pool)
            .await
            .expect("update runtime base url");

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
            sample_workspace_config(),
            sample_opencode_client(),
            InFlightTracker::new(),
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
        store::insert_workspace_regenerate_task(&pool, &key, true)
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
            sample_workspace_config(),
            sample_opencode_client(),
            InFlightTracker::new(),
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
    }

    #[tokio::test]
    async fn tick_ignores_idle_workspace_sessions_when_running_maintenance() {
        let pool = test_db::pool().await;
        let key = format!(
            "maint-idle-session-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        seed_test_agent(&pool, &key).await;
        store::insert_workspace_regenerate_task(&pool, &key, false)
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

        let base_url = spawn_session_status_server(std::slice::from_ref(&session_id)).await;
        sqlx::query("UPDATE agent_runtimes SET base_url = $2 WHERE id = $1")
            .bind("opencode-local")
            .bind(&base_url)
            .execute(&pool)
            .await
            .expect("update runtime base url");

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
            sample_workspace_config(),
            sample_opencode_client(),
            InFlightTracker::new(),
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
            sample_workspace_config(),
            sample_opencode_client(),
            InFlightTracker::new(),
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
            sample_workspace_config(),
            sample_opencode_client(),
            in_flight_for_run,
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
        let joined = tokio::time::timeout(Duration::from_secs(2), run_handle)
            .await
            .expect("run should return within 2s of shutdown signal")
            .expect("join")
            .expect("run result");

        assert_eq!(in_flight.in_flight(), 0, "tracker should be empty");
        assert!(calls.lock().unwrap().len() >= 1, "dispatch should have run");
        let _ = joined;
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
            sample_workspace_config(),
            sample_opencode_client(),
            in_flight_for_run,
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
            sample_workspace_config(),
            sample_opencode_client(),
            in_flight,
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
}
