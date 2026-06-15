use std::{collections::HashMap, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use tokio::{sync::watch, task::JoinHandle};
use tracing::{error, info, warn};

use crate::{
    db::DbPool,
    hyperliquid::{
        account_sync::{InstrumentLookupMap, build_instrument_lookup, sync_account_once, sync_historical_orders_once},
        config::{AccountSyncConfig, HyperliquidEnvironment},
        instruments::load_instruments,
        normalize::InstrumentRow,
        polling::PollingSchedule,
        raw_http::{RawHttpConfig, RawHyperliquidHttpClient},
    },
};

const REGISTRY_REFRESH_INTERVAL: Duration = Duration::from_secs(30);
const INSTRUMENT_LOAD_RETRY_INTERVAL: Duration = Duration::from_secs(30);
const DEFAULT_OVERLAP_MS: u64 = 300_000;

#[derive(Debug, Clone)]
struct EnabledAgent {
    agent_key: String,
    wallet_address: String,
    environment: String,
    history_start_ms: u64,
}

struct AgentTaskHandle {
    wallet_address: String,
    environment: String,
    #[allow(dead_code)]
    task: JoinHandle<()>,
}

/// Supervises one background reconciliation task per enabled agent.
pub struct AgentOrchestrator {
    pool: DbPool,
    shutdown_rx: watch::Receiver<bool>,
    tasks: HashMap<String, AgentTaskHandle>,
    lookup: Arc<InstrumentLookupMap>,
}

impl AgentOrchestrator {
    pub fn new(pool: DbPool, shutdown_rx: watch::Receiver<bool>) -> Self {
        Self {
            pool,
            shutdown_rx,
            tasks: HashMap::new(),
            lookup: Arc::new(HashMap::new()),
        }
    }

    /// Run the orchestrator loop until the shutdown signal is received.
    pub async fn run(mut self) -> Result<()> {
        info!("agent orchestrator starting");
        self.lookup = load_instruments_with_retry(&self.pool, &mut self.shutdown_rx).await?;
        info!("agent orchestrator loaded instruments");
        loop {
            if *self.shutdown_rx.borrow() {
                break;
            }

            let agents = match load_enabled_agents(&self.pool).await {
                Ok(agents) => agents,
                Err(e) => {
                    error!(error = ?e, "orchestrator failed to load enabled agents");
                    tokio::select! {
                        _ = tokio::time::sleep(REGISTRY_REFRESH_INTERVAL) => continue,
                        _ = self.shutdown_rx.changed() => break,
                    };
                }
            };

            self.reconcile_tasks(agents).await;

            tokio::select! {
                _ = tokio::time::sleep(REGISTRY_REFRESH_INTERVAL) => {}
                _ = self.shutdown_rx.changed() => break,
            }
        }

        self.shutdown_all().await;
        info!("agent orchestrator stopped");
        Ok(())
    }

    async fn reconcile_tasks(&mut self, agents: Vec<EnabledAgent>) {
        // Stop tasks that are no longer enabled or whose wallet/environment changed.
        let mut to_stop = Vec::new();
        for (key, handle) in &self.tasks {
            if let Some(agent) = agents.iter().find(|a| a.agent_key == *key) {
                if handle.wallet_address != agent.wallet_address
                    || handle.environment != agent.environment
                {
                    to_stop.push(key.clone());
                }
            } else {
                to_stop.push(key.clone());
            }
        }
        for key in to_stop {
            if let Some(handle) = self.tasks.remove(&key) {
                info!(agent_key = %key, "stopping agent sync task");
                handle.task.abort();
            }
        }

        // Start missing tasks.
        for agent in agents {
            if !self.tasks.contains_key(&agent.agent_key) {
                info!(
                    agent_key = %agent.agent_key,
                    wallet_address = %agent.wallet_address,
                    environment = %agent.environment,
                    "starting agent sync task"
                );
                let (key, handle) = spawn_agent_task(
                    self.pool.clone(),
                    agent,
                    self.shutdown_rx.clone(),
                    Arc::clone(&self.lookup),
                );
                self.tasks.insert(key, handle);
            }
        }
    }

    async fn shutdown_all(mut self) {
        for (_key, handle) in self.tasks.drain() {
            handle.task.abort();
        }
    }
}

fn spawn_agent_task(
    pool: DbPool,
    agent: EnabledAgent,
    shutdown_rx: watch::Receiver<bool>,
    lookup: Arc<InstrumentLookupMap>,
) -> (String, AgentTaskHandle) {
    let wallet_address = agent.wallet_address.clone();
    let environment = agent.environment.clone();
    let agent_key = agent.agent_key.clone();

    let task = tokio::spawn(async move {
        run_agent_task(pool, agent, shutdown_rx, lookup).await;
    });

    (
        agent_key,
        AgentTaskHandle {
            wallet_address,
            environment,
            task,
        },
    )
}

async fn run_agent_task(
    pool: DbPool,
    agent: EnabledAgent,
    mut shutdown_rx: watch::Receiver<bool>,
    lookup: Arc<InstrumentLookupMap>,
) {
    let schedule = PollingSchedule::default();
    let environment = match agent.environment.parse::<HyperliquidEnvironment>() {
        Ok(env) => env,
        Err(e) => {
            eprintln!(
                "agent {} has invalid environment '{}': {e}",
                agent.agent_key, agent.environment
            );
            return;
        }
    };

    let config = AccountSyncConfig {
        account_address: agent.wallet_address.clone(),
        environment,
        history_start_ms: agent.history_start_ms,
        overlap_ms: DEFAULT_OVERLAP_MS,
    };

    let raw_http = RawHyperliquidHttpClient::new(RawHttpConfig {
        environment,
        account_address: agent.wallet_address.clone(),
    });

    let mut historical_orders_accumulator = Duration::from_secs(0);

    info!(
        agent_key = %agent.agent_key,
        wallet_address = %agent.wallet_address,
        environment = %config.environment.as_journal_str(),
        "agent sync loop started"
    );

    loop {
        if *shutdown_rx.borrow() {
            break;
        }

        match sync_account_once(&pool, &config, &raw_http, &lookup).await {
            Ok(summary) => {
                for stream in summary.streams {
                    if let Some(error) = &stream.error {
                        warn!(
                            agent_key = %agent.agent_key,
                            wallet_address = %config.account_address,
                            environment = %summary.environment,
                            stream = %stream.stream.as_str(),
                            error = %error,
                            "agent account stream sync failed"
                        );
                    }
                    info!(
                        agent_key = %agent.agent_key,
                        wallet_address = %config.account_address,
                        environment = %summary.environment,
                        stream = %stream.stream.as_str(),
                        status = %stream.status.as_str(),
                        fetched = stream.count,
                        last_event_time = ?stream.last_event_time,
                        "agent account stream sync completed"
                    );
                }
            }
            Err(e) => {
                error!(
                    agent_key = %agent.agent_key,
                    wallet_address = %config.account_address,
                    environment = %config.environment.as_journal_str(),
                    error = ?e,
                    "agent account sync failed"
                );
            }
        }

        // Sync historical orders on a slower cadence.
        historical_orders_accumulator += schedule.fills;
        if historical_orders_accumulator >= schedule.historical_orders {
            historical_orders_accumulator = Duration::from_secs(0);
            if let Err(e) =
                sync_historical_orders_once(&pool, &config, &raw_http, &lookup).await
            {
                error!(
                    agent_key = %agent.agent_key,
                    wallet_address = %config.account_address,
                    environment = %config.environment.as_journal_str(),
                    error = ?e,
                    "agent historical orders sync failed"
                );
            } else {
                info!(
                    agent_key = %agent.agent_key,
                    wallet_address = %config.account_address,
                    environment = %config.environment.as_journal_str(),
                    "agent historical orders sync completed"
                );
            }
        }

        tokio::select! {
            _ = tokio::time::sleep(schedule.fills) => {}
            _ = shutdown_rx.changed() => break,
        }
    }

    info!(agent_key = %agent.agent_key, "agent sync loop stopped");
}

async fn load_instruments_with_retry(
    pool: &DbPool,
    shutdown_rx: &mut watch::Receiver<bool>,
) -> Result<Arc<InstrumentLookupMap>> {
    loop {
        if *shutdown_rx.borrow() {
            anyhow::bail!("shutdown requested before instruments could be loaded");
        }

        match load_instruments(HyperliquidEnvironment::Mainnet).await {
            Ok(instruments) => {
                upsert_instruments(pool, &instruments).await?;
                return Ok(Arc::new(build_instrument_lookup(&instruments)));
            }
            Err(e) => {
                warn!(
                    retry_in = ?INSTRUMENT_LOAD_RETRY_INTERVAL,
                    error = ?e,
                    "orchestrator failed to load instruments"
                );
                tokio::select! {
                    _ = tokio::time::sleep(INSTRUMENT_LOAD_RETRY_INTERVAL) => {}
                    _ = shutdown_rx.changed() => anyhow::bail!("shutdown requested while loading instruments"),
                }
            }
        }
    }
}

async fn upsert_instruments(pool: &DbPool, instruments: &[InstrumentRow]) -> Result<()> {
    for instrument in instruments {
        sqlx::query(
            "INSERT INTO hyperliquid.instruments (instrument_id, symbol, raw_symbol, market_type, base_asset, quote_asset, settlement_asset, asset_index, price_decimals, size_decimals, tick_size, lot_size, is_hip3, active, created_at, updated_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16) ON CONFLICT (instrument_id) DO UPDATE SET symbol = EXCLUDED.symbol, raw_symbol = EXCLUDED.raw_symbol, market_type = EXCLUDED.market_type, base_asset = EXCLUDED.base_asset, quote_asset = EXCLUDED.quote_asset, settlement_asset = EXCLUDED.settlement_asset, asset_index = EXCLUDED.asset_index, price_decimals = EXCLUDED.price_decimals, size_decimals = EXCLUDED.size_decimals, tick_size = EXCLUDED.tick_size, lot_size = EXCLUDED.lot_size, is_hip3 = EXCLUDED.is_hip3, active = EXCLUDED.active, updated_at = EXCLUDED.updated_at",
        )
        .bind(&instrument.instrument_id)
        .bind(&instrument.symbol)
        .bind(&instrument.raw_symbol)
        .bind(match instrument.market_type {
            crate::hyperliquid::normalize::MarketType::Perp => "perp",
            crate::hyperliquid::normalize::MarketType::Spot => "spot",
            crate::hyperliquid::normalize::MarketType::Outcome => "outcome",
        })
        .bind(&instrument.base_asset)
        .bind(&instrument.quote_asset)
        .bind(&instrument.settlement_asset)
        .bind(instrument.asset_index)
        .bind(instrument.price_decimals)
        .bind(instrument.size_decimals)
        .bind(instrument.tick_size)
        .bind(instrument.lot_size)
        .bind(instrument.is_hip3)
        .bind(instrument.active)
        .bind(instrument.created_at)
        .bind(instrument.updated_at)
        .execute(pool)
        .await?;
    }

    Ok(())
}

async fn load_enabled_agents(pool: &DbPool) -> Result<Vec<EnabledAgent>> {
    let rows = sqlx::query_as::<_, EnabledAgentRow>(
        "SELECT agent_key, wallet_address, environment FROM agents.registry WHERE enabled = true",
    )
    .fetch_all(pool)
    .await
    .context("failed to load enabled agents")?;

    Ok(rows
        .into_iter()
        .map(|r| EnabledAgent {
            agent_key: r.agent_key,
            wallet_address: r.wallet_address,
            environment: r.environment,
            history_start_ms: 0,
        })
        .collect())
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct EnabledAgentRow {
    agent_key: String,
    wallet_address: String,
    environment: String,
}
