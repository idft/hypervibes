use std::{sync::Arc, time::Duration};

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_json::json;
use tokio::{
    sync::{Semaphore, watch},
    task::JoinSet,
};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::{
    db::DbPool,
    harness::timeframe::{
        DEFAULT_TRIGGER_DELAY_SECONDS, boundary_for_due_at, latest_due_at_or_before,
    },
    indicators::{
        coordination::IndicatorQueueNotifier,
        market_data::{
            CandleFetchCache, CandleFetchErrorKind, IndicatorCandleClient,
            IndicatorMarketDataClient,
        },
        model::IndicatorRun,
        runtime::{DEFAULT_INDICATOR_HISTORY_BARS, MAX_INDICATOR_RESULT_BYTES, execute_indicator},
        store,
    },
};

const POLL_INTERVAL: Duration = Duration::from_secs(10);
const LEASE_RENEW_INTERVAL: Duration = Duration::from_secs(20);
const MAX_RUN_ATTEMPTS: i32 = 3;

#[derive(Clone, Copy)]
enum RefillReason {
    Startup,
    Reconciliation,
    Notification,
    WorkerCompletion,
    RetryDeadline,
}

impl RefillReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::Startup => "startup",
            Self::Reconciliation => "reconciliation",
            Self::Notification => "notification",
            Self::WorkerCompletion => "worker_completion",
            Self::RetryDeadline => "retry_deadline",
        }
    }
}

#[derive(Clone)]
struct IndicatorWorker {
    pool: DbPool,
    candles: CandleFetchCache,
    execution_limit: Arc<Semaphore>,
}

enum AttemptFailure {
    Retryable(String),
    Terminal(String),
    OwnershipLost,
}

/// Reconciles canonical indicator boundaries and drains ready work into a
/// bounded set of independently leased workers.
pub struct IndicatorScheduler {
    pool: DbPool,
    shutdown_rx: watch::Receiver<bool>,
    force_shutdown_rx: watch::Receiver<bool>,
    notifier: IndicatorQueueNotifier,
    worker: IndicatorWorker,
    execution_limit: usize,
    last_claimed_agent: Option<String>,
}

impl IndicatorScheduler {
    pub fn new(
        pool: DbPool,
        shutdown_rx: watch::Receiver<bool>,
        force_shutdown_rx: watch::Receiver<bool>,
        notifier: IndicatorQueueNotifier,
        execution_limit: usize,
    ) -> Self {
        Self::with_client(
            pool,
            shutdown_rx,
            force_shutdown_rx,
            notifier,
            execution_limit,
            Arc::new(IndicatorMarketDataClient::new()),
        )
    }

    pub fn with_client(
        pool: DbPool,
        shutdown_rx: watch::Receiver<bool>,
        force_shutdown_rx: watch::Receiver<bool>,
        notifier: IndicatorQueueNotifier,
        execution_limit: usize,
        candles: Arc<dyn IndicatorCandleClient>,
    ) -> Self {
        assert!(
            execution_limit > 0,
            "indicator execution limit must be positive"
        );
        Self {
            pool: pool.clone(),
            shutdown_rx,
            force_shutdown_rx,
            notifier,
            worker: IndicatorWorker {
                pool,
                candles: CandleFetchCache::new(candles),
                execution_limit: Arc::new(Semaphore::new(execution_limit)),
            },
            execution_limit,
            last_claimed_agent: None,
        }
    }

    pub async fn run(mut self) -> Result<()> {
        info!(
            configured_limit = self.execution_limit,
            "indicator scheduler started"
        );
        let mut workers = JoinSet::new();
        if *self.shutdown_rx.borrow() || *self.force_shutdown_rx.borrow() {
            return Ok(());
        }
        if !self.reconcile_interruptibly(Utc::now()).await? {
            return Ok(());
        }
        self.refill(&mut workers, RefillReason::Startup).await?;
        let mut next_attempt_at = store::next_queued_attempt_at(&self.pool).await?;
        let mut interval = tokio::time::interval(POLL_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        interval.tick().await;

        let mut forced = false;
        loop {
            let event = tokio::select! {
                _ = interval.tick() => Some(RefillReason::Reconciliation),
                _ = self.notifier.notified() => Some(RefillReason::Notification),
                _ = sleep_until(next_attempt_at) => Some(RefillReason::RetryDeadline),
                completed = async {
                    if workers.is_empty() {
                        std::future::pending().await
                    } else {
                        workers.join_next().await
                    }
                } => {
                    if let Some(Err(error)) = completed {
                        warn!(error = ?error, "indicator worker task failed");
                    }
                    Some(RefillReason::WorkerCompletion)
                }
                changed = self.shutdown_rx.changed() => {
                    if changed.is_err() || *self.shutdown_rx.borrow() { None } else { continue }
                }
                changed = self.force_shutdown_rx.changed() => {
                    if changed.is_err() || *self.force_shutdown_rx.borrow() {
                        forced = true;
                        None
                    } else {
                        continue
                    }
                }
            };
            let Some(reason) = event else {
                break;
            };
            if *self.shutdown_rx.borrow() || *self.force_shutdown_rx.borrow() {
                forced = *self.force_shutdown_rx.borrow();
                break;
            }
            if matches!(reason, RefillReason::Reconciliation) {
                match self.reconcile_interruptibly(Utc::now()).await {
                    Ok(true) => {}
                    Ok(false) => break,
                    Err(error) => {
                        error!(error = ?error, "indicator scheduler reconciliation failed")
                    }
                }
            }
            if let Err(error) = self.refill(&mut workers, reason).await {
                error!(error = ?error, "indicator scheduler queue refill failed");
            }
            next_attempt_at = match store::next_queued_attempt_at(&self.pool).await {
                Ok(deadline) => deadline,
                Err(error) => {
                    error!(error = ?error, "failed to schedule indicator retry wake-up");
                    None
                }
            };
        }

        forced |= *self.force_shutdown_rx.borrow();
        if forced {
            warn!(
                active_workers = workers.len(),
                "aborting indicator workers after forced shutdown"
            );
            workers.abort_all();
            while workers.join_next().await.is_some() {}
            return Ok(());
        }

        info!(
            active_workers = workers.len(),
            "indicator scheduler draining workers"
        );
        while !workers.is_empty() {
            tokio::select! {
                completed = workers.join_next() => {
                    if let Some(Err(error)) = completed {
                        warn!(error = ?error, "indicator worker failed while draining");
                    }
                }
                changed = self.force_shutdown_rx.changed() => {
                    if changed.is_err() || *self.force_shutdown_rx.borrow() {
                        warn!(active_workers = workers.len(), "aborting indicator workers after forced shutdown");
                        workers.abort_all();
                        while workers.join_next().await.is_some() {}
                        break;
                    }
                }
            }
        }
        Ok(())
    }

    async fn reconcile(&self, now: DateTime<Utc>) -> Result<()> {
        let recovered = store::recover_stale_running_runs(&self.pool, MAX_RUN_ATTEMPTS).await?;
        if recovered > 0 {
            info!(recovered, "recovered expired indicator execution leases");
            crate::indicators::coordination::notify_completion();
        }
        for target in store::list_enabled_targets(&self.pool).await? {
            let Some(due_at) =
                latest_due_at_or_before(now, &target.timeframe, DEFAULT_TRIGGER_DELAY_SECONDS)?
            else {
                continue;
            };
            store::enqueue_run(
                &self.pool,
                &target.agent_key,
                target.definition_id,
                target.version_id,
                &target.instrument_id,
                &target.timeframe,
                boundary_for_due_at(due_at, DEFAULT_TRIGGER_DELAY_SECONDS),
            )
            .await?;
        }
        let health = store::indicator_queue_health(&self.pool).await?;
        debug!(
            ready_count = health.ready_count,
            oldest_ready_age_seconds = health.oldest_ready_age_seconds,
            running_count = health.running_count,
            retry_count = health.retry_count,
            timed_out_dependency_count = health.timed_out_dependency_count,
            "indicator queue health"
        );
        Ok(())
    }

    async fn reconcile_interruptibly(&self, now: DateTime<Utc>) -> Result<bool> {
        let mut shutdown_rx = self.shutdown_rx.clone();
        let mut force_shutdown_rx = self.force_shutdown_rx.clone();
        tokio::select! {
            result = self.reconcile(now) => result.map(|()| true),
            _ = wait_for_shutdown(&mut shutdown_rx, &mut force_shutdown_rx) => Ok(false),
        }
    }

    async fn refill(&mut self, workers: &mut JoinSet<()>, reason: RefillReason) -> Result<()> {
        debug!(
            reason = reason.as_str(),
            active_workers = workers.len(),
            configured_limit = self.execution_limit,
            "refilling indicator queue"
        );
        let mut claimed_agents = Vec::with_capacity(self.execution_limit + 1);
        if let Some(agent_key) = &self.last_claimed_agent {
            claimed_agents.push(agent_key.clone());
        }
        while workers.len() < self.execution_limit {
            if *self.shutdown_rx.borrow() || *self.force_shutdown_rx.borrow() {
                break;
            }
            let mut run = store::claim_next_queued_run(&self.pool, &claimed_agents).await?;
            if run.is_none() && !claimed_agents.is_empty() {
                claimed_agents.clear();
                if *self.shutdown_rx.borrow() || *self.force_shutdown_rx.borrow() {
                    break;
                }
                run = store::claim_next_queued_run(&self.pool, &claimed_agents).await?;
            }
            let Some(run) = run else {
                break;
            };
            let claim_token = run
                .claim_token
                .expect("claimed indicator run must have a claim token");
            if *self.shutdown_rx.borrow() || *self.force_shutdown_rx.borrow() {
                if !store::release_owned_claim(&self.pool, run.id, claim_token).await? {
                    warn!(run_id = %run.id, claim_token = %claim_token, "shutdown-raced indicator claim lost ownership before release");
                }
                break;
            }
            self.last_claimed_agent = Some(run.agent_key.clone());
            if !claimed_agents.contains(&run.agent_key) {
                claimed_agents.push(run.agent_key.clone());
            }
            info!(
                run_id = %run.id,
                agent_key = %run.agent_key,
                attempt = run.attempt_count,
                active_workers = workers.len() + 1,
                configured_limit = self.execution_limit,
                "claimed indicator run"
            );
            let worker = self.worker.clone();
            workers.spawn(async move { worker.execute_run(run).await });
        }
        Ok(())
    }
}

async fn wait_for_shutdown(
    shutdown_rx: &mut watch::Receiver<bool>,
    force_shutdown_rx: &mut watch::Receiver<bool>,
) {
    loop {
        if *shutdown_rx.borrow() || *force_shutdown_rx.borrow() {
            return;
        }
        tokio::select! {
            changed = shutdown_rx.changed() => {
                if changed.is_err() || *shutdown_rx.borrow() {
                    return;
                }
            }
            changed = force_shutdown_rx.changed() => {
                if changed.is_err() || *force_shutdown_rx.borrow() {
                    return;
                }
            }
        }
    }
}

async fn sleep_until(deadline: Option<DateTime<Utc>>) {
    let Some(deadline) = deadline else {
        std::future::pending::<()>().await;
        return;
    };
    let delay = (deadline - Utc::now()).to_std().unwrap_or(Duration::ZERO);
    tokio::time::sleep(delay).await;
}

impl IndicatorWorker {
    async fn execute_run(&self, run: IndicatorRun) {
        let claim_token = run
            .claim_token
            .expect("claimed indicator run must have a claim token");
        info!(run_id = %run.id, claim_token = %claim_token, "starting indicator run");
        let mut attempt = Box::pin(self.execute_attempt(&run));
        let mut renewal = tokio::time::interval(LEASE_RENEW_INTERVAL);
        renewal.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        renewal.tick().await;
        let result = loop {
            tokio::select! {
                result = &mut attempt => break result,
                _ = renewal.tick() => {
                    match store::renew_run_lease(&self.pool, run.id, claim_token).await {
                        Ok(true) => debug!(run_id = %run.id, "renewed indicator run lease"),
                        Ok(false) => break Err(AttemptFailure::OwnershipLost),
                        Err(error) => warn!(run_id = %run.id, error = ?error, "failed to renew indicator run lease"),
                    }
                }
            }
        };

        match result {
            Ok(output) => {
                match store::finish_run_succeeded(&self.pool, run.id, claim_token, output).await {
                    Ok(true) => {
                        info!(run_id = %run.id, attempt = run.attempt_count, "completed indicator run");
                        crate::indicators::coordination::notify_completion();
                    }
                    Ok(false) => {
                        warn!(run_id = %run.id, claim_token = %claim_token, "indicator completion lost ownership")
                    }
                    Err(error) => {
                        warn!(run_id = %run.id, error = ?error, "failed to persist indicator result; lease recovery will retry")
                    }
                }
            }
            Err(AttemptFailure::Retryable(error)) => {
                self.retry_or_fail(&run, claim_token, &error).await;
            }
            Err(AttemptFailure::Terminal(error)) => {
                self.fail(&run, claim_token, &error).await;
            }
            Err(AttemptFailure::OwnershipLost) => {
                warn!(run_id = %run.id, claim_token = %claim_token, "indicator worker lost ownership");
            }
        }
    }

    async fn execute_attempt(
        &self,
        run: &IndicatorRun,
    ) -> Result<store::PersistedIndicatorOutput, AttemptFailure> {
        let version = store::get_version(
            &self.pool,
            &run.agent_key,
            run.indicator_definition_id,
            run.indicator_version_id,
        )
        .await
        .map_err(|error| AttemptFailure::Retryable(error.to_string()))?
        .ok_or_else(|| {
            AttemptFailure::Terminal("indicator version no longer exists".to_string())
        })?;
        let candles = self
            .candles
            .fetch_closed_candles(
                &run.instrument_id,
                &run.timeframe,
                run.scheduled_for,
                DEFAULT_INDICATOR_HISTORY_BARS,
            )
            .await
            .map_err(|error| match error.kind {
                CandleFetchErrorKind::Retryable => AttemptFailure::Retryable(error.to_string()),
                CandleFetchErrorKind::Terminal => AttemptFailure::Terminal(error.to_string()),
            })?;
        let output = execute_indicator(
            Arc::clone(&self.execution_limit),
            version.source,
            version.input_values,
            Arc::clone(&candles),
            run.instrument_id.clone(),
            run.timeframe.clone(),
        )
        .await
        .map_err(|error| AttemptFailure::Terminal(error.to_string()))?;
        let candle_data = json!(candles.as_ref());
        let plot_data = json!(output.plots);
        let visual_data = json!(output.visual_data);
        if serde_json::to_vec(&json!({
            "candles": candle_data,
            "plots": plot_data,
            "visuals": visual_data,
        }))
        .map_or(true, |data| data.len() > MAX_INDICATOR_RESULT_BYTES)
        {
            return Err(AttemptFailure::Terminal(
                "indicator result exceeds the maximum stored size".to_string(),
            ));
        }
        Ok(store::PersistedIndicatorOutput {
            candle_data,
            plot_data,
            visual_data,
            latest_values: json!(output.latest_values),
            diagnostics: json!(output.diagnostics),
        })
    }

    async fn fail(&self, run: &IndicatorRun, claim_token: Uuid, error: &str) {
        let summary = sanitize_error_summary(error);
        match store::finish_run_failed(&self.pool, run.id, claim_token, json!([]), &summary).await {
            Ok(true) => {
                info!(run_id = %run.id, attempt = run.attempt_count, "indicator run failed terminally");
                crate::indicators::coordination::notify_completion();
            }
            Ok(false) => {
                warn!(run_id = %run.id, claim_token = %claim_token, "indicator failure lost ownership")
            }
            Err(persist_error) => {
                warn!(run_id = %run.id, error = ?persist_error, "failed to persist indicator failure")
            }
        }
    }

    async fn retry_or_fail(&self, run: &IndicatorRun, claim_token: Uuid, error: &str) {
        if run.attempt_count >= MAX_RUN_ATTEMPTS {
            self.fail(run, claim_token, error).await;
            return;
        }
        let summary = sanitize_error_summary(error);
        match store::requeue_retryable_run(
            &self.pool,
            run.id,
            claim_token,
            run.attempt_count,
            &summary,
            MAX_RUN_ATTEMPTS,
        )
        .await
        {
            Ok(true) => info!(
                run_id = %run.id,
                attempt = run.attempt_count,
                retry_delay_seconds = store::retry_delay(run.attempt_count).num_seconds(),
                "scheduled indicator run retry"
            ),
            Ok(false) => {
                warn!(run_id = %run.id, claim_token = %claim_token, "indicator retry lost ownership")
            }
            Err(persist_error) => {
                warn!(run_id = %run.id, error = ?persist_error, "failed to schedule indicator retry")
            }
        }
    }
}

fn sanitize_error_summary(error: &str) -> String {
    error
        .chars()
        .filter(|character| !character.is_control())
        .take(500)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use rust_decimal::Decimal;
    use serde_json::json;

    use super::*;
    use crate::{
        agents::{
            model::AgentRegistryRow,
            store::{insert_agent, replace_agent_analysis_instruments},
        },
        indicators::{
            market_data::CandleFetchError,
            model::Candle,
            store::{CreateIndicatorDefinition, NewIndicatorVersion},
        },
        test_db,
    };

    #[test]
    fn retry_backoff_is_exponential_and_capped() {
        assert_eq!(store::retry_delay(1), chrono::Duration::seconds(1));
        assert_eq!(store::retry_delay(2), chrono::Duration::seconds(2));
        assert_eq!(store::retry_delay(3), chrono::Duration::seconds(4));
        assert_eq!(store::retry_delay(20), chrono::Duration::seconds(30));
    }

    struct BarrierCandleClient {
        barrier: tokio::sync::Barrier,
        active: AtomicUsize,
        peak: AtomicUsize,
    }

    struct RetryOnceCandleClient {
        calls: AtomicUsize,
    }

    fn complete_candles(
        timeframe: &str,
        boundary: DateTime<Utc>,
        requested_bars: usize,
    ) -> Vec<Candle> {
        let seconds =
            crate::harness::timeframe::parse_timeframe_seconds(timeframe).expect("test timeframe");
        (0..requested_bars)
            .map(|index| Candle {
                opened_at: boundary
                    - chrono::Duration::seconds(
                        seconds
                            * i64::try_from(requested_bars - index)
                                .expect("history length fits i64"),
                    ),
                open: Decimal::ONE,
                high: Decimal::ONE,
                low: Decimal::ONE,
                close: Decimal::ONE,
                volume: Decimal::ONE,
            })
            .collect()
    }

    #[async_trait]
    impl IndicatorCandleClient for BarrierCandleClient {
        async fn fetch_closed_candles(
            &self,
            _instrument_id: &str,
            timeframe: &str,
            boundary: DateTime<Utc>,
            requested_bars: usize,
        ) -> Result<Vec<Candle>, CandleFetchError> {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(active, Ordering::SeqCst);
            self.barrier.wait().await;
            self.active.fetch_sub(1, Ordering::SeqCst);
            Ok(complete_candles(timeframe, boundary, requested_bars))
        }
    }

    #[async_trait]
    impl IndicatorCandleClient for RetryOnceCandleClient {
        async fn fetch_closed_candles(
            &self,
            _instrument_id: &str,
            timeframe: &str,
            boundary: DateTime<Utc>,
            requested_bars: usize,
        ) -> Result<Vec<Candle>, CandleFetchError> {
            if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                return Err(CandleFetchError::retryable("temporary test failure"));
            }
            Ok(complete_candles(timeframe, boundary, requested_bars))
        }
    }

    async fn seed_scheduler_agent(pool: &DbPool, key: &str, timeframe: &str) {
        let now = Utc::now();
        insert_agent(
            pool,
            &AgentRegistryRow {
                agent_key: key.to_string(),
                user_id: test_db::test_user_id(),
                created_at: now,
                updated_at: now,
                enabled: true,
                lifecycle: "active".to_string(),
                display_name: key.to_string(),
                trading_account_address: Some(format!("0x{:040x}", Uuid::new_v4().as_u128())),
                environment: "live".to_string(),
                api_key: format!("key-{key}"),
                api_key_last_used_at: None,
                runtime_config: json!({}),
            },
        )
        .await
        .expect("insert test agent");
        sqlx::query("INSERT INTO hyperliquid.instruments (instrument_id, name, market_type, base_asset, quote_asset, settlement_asset, asset_index, price_decimals, size_decimals, lot_size, max_leverage, is_hip3, active, created_at, updated_at) VALUES ('BTC', 'BTC', 'perp', 'BTC', 'USD', 'USDC', 1, 2, 3, 0.001, 50, false, true, now(), now()) ON CONFLICT (instrument_id) DO NOTHING")
            .execute(pool)
            .await
            .expect("seed test instrument");
        replace_agent_analysis_instruments(pool, key, &["BTC".to_string()])
            .await
            .expect("select analysis instrument");
        store::create_definition_with_initial_version(
            pool,
            key,
            &CreateIndicatorDefinition {
                name: "Test".to_string(),
                description: String::new(),
                timeframes: vec![timeframe.to_string()],
                enabled: true,
                instrument_ids: vec!["BTC".to_string()],
                version: NewIndicatorVersion {
                    source: "indicator(\"Test\")\nplot(close)".to_string(),
                    compiler_version: "test".to_string(),
                    metadata: json!({}),
                    input_values: json!({}),
                    created_by_kind: "operator".to_string(),
                    created_by_run_id: None,
                    created_by_conversation_id: None,
                },
            },
        )
        .await
        .expect("create test indicator");
    }

    #[tokio::test]
    async fn scheduler_starts_multiple_workers_up_to_the_configured_limit() {
        let pool = test_db::pool().await;
        seed_scheduler_agent(&pool, "indicator-concurrent-a", "1h").await;
        seed_scheduler_agent(&pool, "indicator-concurrent-b", "4h").await;
        let client = Arc::new(BarrierCandleClient {
            barrier: tokio::sync::Barrier::new(3),
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        });
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let (_force_tx, force_rx) = watch::channel(false);
        let scheduler = IndicatorScheduler::with_client(
            pool.as_ref().clone(),
            shutdown_rx,
            force_rx,
            IndicatorQueueNotifier::new(),
            2,
            client.clone(),
        );
        let handle = tokio::spawn(scheduler.run());

        tokio::time::timeout(Duration::from_secs(5), client.barrier.wait())
            .await
            .expect("two candle fetches should run concurrently");
        assert_eq!(client.peak.load(Ordering::SeqCst), 2);
        shutdown_tx.send(true).expect("signal shutdown");
        tokio::time::timeout(Duration::from_secs(30), handle)
            .await
            .expect("scheduler should drain")
            .expect("scheduler task")
            .expect("scheduler result");
    }

    #[tokio::test]
    async fn one_agent_can_fill_all_worker_slots() {
        let pool = test_db::pool().await;
        seed_scheduler_agent(&pool, "indicator-one-agent", "1h").await;
        let definition = store::list_definitions(&pool, "indicator-one-agent")
            .await
            .expect("list definitions")
            .pop()
            .expect("definition");
        let version_id = definition.active_version_id.expect("active version");
        let older_boundary = crate::harness::timeframe::canonical_boundary_at_or_before(
            Utc::now() - chrono::Duration::hours(1),
            "1h",
        )
        .expect("older boundary");
        store::enqueue_run(
            &pool,
            "indicator-one-agent",
            definition.id,
            version_id,
            "BTC",
            "1h",
            older_boundary,
        )
        .await
        .expect("enqueue second run");
        let client = Arc::new(BarrierCandleClient {
            barrier: tokio::sync::Barrier::new(3),
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        });
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let (_force_tx, force_rx) = watch::channel(false);
        let scheduler = IndicatorScheduler::with_client(
            pool.as_ref().clone(),
            shutdown_rx,
            force_rx,
            IndicatorQueueNotifier::new(),
            2,
            client.clone(),
        );
        let handle = tokio::spawn(scheduler.run());

        tokio::time::timeout(Duration::from_secs(5), client.barrier.wait())
            .await
            .expect("one agent should fill both worker slots");
        assert_eq!(client.peak.load(Ordering::SeqCst), 2);
        shutdown_tx.send(true).expect("signal shutdown");
        handle
            .await
            .expect("scheduler task")
            .expect("scheduler result");
    }

    #[tokio::test]
    async fn queue_notification_wakes_scheduler_before_fallback_poll() {
        let pool = test_db::pool().await;
        let client = Arc::new(BarrierCandleClient {
            barrier: tokio::sync::Barrier::new(2),
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        });
        let notifier = IndicatorQueueNotifier::new();
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let (_force_tx, force_rx) = watch::channel(false);
        let scheduler = IndicatorScheduler::with_client(
            pool.as_ref().clone(),
            shutdown_rx,
            force_rx,
            notifier.clone(),
            1,
            client.clone(),
        );
        let handle = tokio::spawn(scheduler.run());
        tokio::time::sleep(Duration::from_millis(100)).await;

        seed_scheduler_agent(&pool, "indicator-notified", "1h").await;
        notifier.notify();
        tokio::time::timeout(Duration::from_secs(2), client.barrier.wait())
            .await
            .expect("notification should wake scheduler before ten-second poll");
        shutdown_tx.send(true).expect("signal shutdown");
        tokio::time::timeout(Duration::from_secs(30), handle)
            .await
            .expect("scheduler should drain")
            .expect("scheduler task")
            .expect("scheduler result");
    }

    #[tokio::test]
    async fn round_robin_cursor_persists_across_single_slot_refills() {
        let pool = test_db::pool().await;
        seed_scheduler_agent(&pool, "indicator-round-robin-a", "1h").await;
        seed_scheduler_agent(&pool, "indicator-round-robin-b", "1h").await;
        let client = Arc::new(BarrierCandleClient {
            barrier: tokio::sync::Barrier::new(100),
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        });
        let (_shutdown_tx, shutdown_rx) = watch::channel(false);
        let (_force_tx, force_rx) = watch::channel(false);
        let mut scheduler = IndicatorScheduler::with_client(
            pool.as_ref().clone(),
            shutdown_rx,
            force_rx,
            IndicatorQueueNotifier::new(),
            1,
            client,
        );

        let mut workers = JoinSet::new();
        scheduler
            .refill(&mut workers, RefillReason::Startup)
            .await
            .expect("first refill");
        let first: (Uuid, String, Uuid) = sqlx::query_as(
            "SELECT id, agent_key, claim_token FROM agent_indicator_runs WHERE status = 'running'",
        )
        .fetch_one(&pool)
        .await
        .expect("first running claim");
        assert!(
            store::release_owned_claim(&pool, first.0, first.2)
                .await
                .expect("release first")
        );
        workers.abort_all();
        while workers.join_next().await.is_some() {}

        scheduler
            .refill(&mut workers, RefillReason::WorkerCompletion)
            .await
            .expect("second refill");
        let second: (Uuid, String, Uuid) = sqlx::query_as(
            "SELECT id, agent_key, claim_token FROM agent_indicator_runs WHERE status = 'running'",
        )
        .fetch_one(&pool)
        .await
        .expect("second running claim");
        assert_ne!(first.1, second.1);
        assert!(
            store::release_owned_claim(&pool, second.0, second.2)
                .await
                .expect("release second")
        );
        workers.abort_all();
        while workers.join_next().await.is_some() {}
    }

    #[tokio::test]
    async fn persisted_retry_deadline_wakes_scheduler_before_poll_interval() {
        let pool = test_db::pool().await;
        seed_scheduler_agent(&pool, "indicator-retry-wake", "1h").await;
        let client = Arc::new(RetryOnceCandleClient {
            calls: AtomicUsize::new(0),
        });
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let (_force_tx, force_rx) = watch::channel(false);
        let scheduler = IndicatorScheduler::with_client(
            pool.as_ref().clone(),
            shutdown_rx,
            force_rx,
            IndicatorQueueNotifier::new(),
            1,
            client.clone(),
        );
        let handle = tokio::spawn(scheduler.run());

        tokio::time::timeout(Duration::from_secs(4), async {
            loop {
                let succeeded: bool = sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM agent_indicator_runs WHERE status = 'succeeded')",
                )
                .fetch_one(&pool)
                .await
                .expect("check retry completion");
                if succeeded {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("persisted retry deadline should wake before fallback poll");
        assert_eq!(client.calls.load(Ordering::SeqCst), 2);
        shutdown_tx.send(true).expect("signal shutdown");
        handle
            .await
            .expect("scheduler task")
            .expect("scheduler result");
    }
}
