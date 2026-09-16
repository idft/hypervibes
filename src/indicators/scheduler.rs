use std::{sync::Arc, time::Duration};

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::json;
use tokio::sync::{Semaphore, watch};
use tracing::{error, warn};

use crate::{
    db::DbPool,
    harness::timeframe::{
        DEFAULT_TRIGGER_DELAY_SECONDS, boundary_for_due_at, latest_due_at_or_before,
    },
    indicators::{
        market_data::IndicatorMarketDataClient,
        model::{Candle, IndicatorRun},
        runtime::{
            DEFAULT_INDICATOR_HISTORY_BARS, MAX_CONCURRENT_INDICATOR_EXECUTIONS,
            MAX_INDICATOR_RESULT_BYTES, execute_indicator,
        },
        store,
    },
};

const POLL_INTERVAL: Duration = Duration::from_secs(10);
const RUN_CLAIM_LIMIT: usize = MAX_CONCURRENT_INDICATOR_EXECUTIONS;

#[async_trait]
pub trait IndicatorCandleClient: Send + Sync {
    async fn fetch_closed_candles(
        &self,
        instrument_id: &str,
        timeframe: &str,
        boundary: DateTime<Utc>,
        requested_bars: usize,
    ) -> Result<Vec<Candle>>;
}

#[async_trait]
impl IndicatorCandleClient for IndicatorMarketDataClient {
    async fn fetch_closed_candles(
        &self,
        instrument_id: &str,
        timeframe: &str,
        boundary: DateTime<Utc>,
        requested_bars: usize,
    ) -> Result<Vec<Candle>> {
        IndicatorMarketDataClient::fetch_closed_candles(
            self,
            instrument_id,
            timeframe,
            boundary,
            requested_bars,
        )
        .await
    }
}

/// Deterministic indicator work loop. Database claims make concurrent process
/// instances safe; the local semaphore bounds interpreter CPU consumption.
pub struct IndicatorScheduler {
    pool: DbPool,
    shutdown_rx: watch::Receiver<bool>,
    candles: Arc<dyn IndicatorCandleClient>,
    execution_limit: Arc<Semaphore>,
}

impl IndicatorScheduler {
    pub fn new(pool: DbPool, shutdown_rx: watch::Receiver<bool>) -> Self {
        Self::with_client(
            pool,
            shutdown_rx,
            Arc::new(IndicatorMarketDataClient::new()),
        )
    }

    pub fn with_client(
        pool: DbPool,
        shutdown_rx: watch::Receiver<bool>,
        candles: Arc<dyn IndicatorCandleClient>,
    ) -> Self {
        Self {
            pool,
            shutdown_rx,
            candles,
            execution_limit: Arc::new(Semaphore::new(MAX_CONCURRENT_INDICATOR_EXECUTIONS)),
        }
    }

    pub async fn run(mut self) -> Result<()> {
        let mut interval = tokio::time::interval(POLL_INTERVAL);
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if let Err(error) = self.tick(Utc::now()).await { error!(error = ?error, "indicator scheduler tick failed"); }
                }
                changed = self.shutdown_rx.changed() => {
                    if changed.is_err() || *self.shutdown_rx.borrow() { return Ok(()); }
                }
            }
        }
    }

    pub async fn tick(&self, now: DateTime<Utc>) -> Result<()> {
        for target in store::list_enabled_targets(&self.pool).await? {
            let Some(due_at) =
                latest_due_at_or_before(now, &target.timeframe, DEFAULT_TRIGGER_DELAY_SECONDS)?
            else {
                continue;
            };
            let boundary = boundary_for_due_at(due_at, DEFAULT_TRIGGER_DELAY_SECONDS);
            store::enqueue_run(
                &self.pool,
                &target.agent_key,
                target.definition_id,
                target.version_id,
                &target.instrument_id,
                &target.timeframe,
                boundary,
            )
            .await?;
        }
        for _ in 0..RUN_CLAIM_LIMIT {
            let Some(run) = store::claim_next_queued_run(&self.pool).await? else {
                break;
            };
            self.execute_run(run).await;
        }
        Ok(())
    }

    async fn execute_run(&self, run: IndicatorRun) {
        let version = match store::get_version(
            &self.pool,
            &run.agent_key,
            run.indicator_definition_id,
            run.indicator_version_id,
        )
        .await
        {
            Ok(Some(version)) => version,
            Ok(None) => {
                self.fail(&run, "indicator version no longer exists").await;
                return;
            }
            Err(error) => {
                self.fail(&run, &error.to_string()).await;
                return;
            }
        };
        let candles = match self
            .candles
            .fetch_closed_candles(
                &run.instrument_id,
                &run.timeframe,
                run.scheduled_for,
                DEFAULT_INDICATOR_HISTORY_BARS,
            )
            .await
        {
            Ok(candles) => candles,
            Err(error) => {
                self.fail(&run, &error.to_string()).await;
                return;
            }
        };
        let output = match execute_indicator(
            Arc::clone(&self.execution_limit),
            version.source,
            version.input_values,
            candles.clone(),
            run.instrument_id.clone(),
            run.timeframe.clone(),
        )
        .await
        {
            Ok(output) => output,
            Err(error) => {
                self.fail(&run, &error.to_string()).await;
                return;
            }
        };
        let candle_data = json!(candles);
        let plot_data = json!(output.plots);
        let latest_values = json!(output.latest_values);
        if serde_json::to_vec(&json!({"candles": candle_data, "plots": plot_data}))
            .map_or(true, |data| data.len() > MAX_INDICATOR_RESULT_BYTES)
        {
            self.fail(&run, "indicator result exceeds the maximum stored size")
                .await;
            return;
        }
        if let Err(error) = store::finish_run_succeeded(
            &self.pool,
            &run.agent_key,
            run.id,
            candle_data,
            plot_data,
            latest_values,
            json!(output.diagnostics),
        )
        .await
        {
            warn!(run_id = %run.id, error = ?error, "failed to persist indicator result");
        }
    }

    async fn fail(&self, run: &IndicatorRun, error: &str) {
        let summary: String = error
            .chars()
            .filter(|character| !character.is_control())
            .take(500)
            .collect();
        if let Err(persist_error) =
            store::finish_run_failed(&self.pool, &run.agent_key, run.id, json!([]), &summary).await
        {
            warn!(run_id = %run.id, error = ?persist_error, "failed to persist indicator failure");
        }
    }
}
