use std::{collections::HashMap, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use tokio::{sync::watch, task::JoinHandle};
use tracing::{error, info, warn};

use crate::{
    db::DbPool,
    hyperliquid::{
        account_sync::{
            InstrumentLookupMap, build_instrument_lookup, sync_account_once,
            sync_historical_orders_once,
        },
        config::{AccountSyncConfig, HyperliquidEnvironment},
        instruments::{load_instruments, upsert_instruments},
        live_state::LiveAccountStore,
        live_ws::{LiveWsOptions, run_account_live_ws},
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
    live_accounts: Arc<LiveAccountStore>,
}

impl AgentOrchestrator {
    pub fn new(
        pool: DbPool,
        shutdown_rx: watch::Receiver<bool>,
        live_accounts: Arc<LiveAccountStore>,
    ) -> Self {
        Self {
            pool,
            shutdown_rx,
            tasks: HashMap::new(),
            lookup: Arc::new(HashMap::new()),
            live_accounts,
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
                    Arc::clone(&self.live_accounts),
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
    live_accounts: Arc<LiveAccountStore>,
) -> (String, AgentTaskHandle) {
    let wallet_address = agent.wallet_address.clone();
    let environment = agent.environment.clone();
    let agent_key = agent.agent_key.clone();

    let task = tokio::spawn(async move {
        run_agent_task(pool, agent, shutdown_rx, lookup, live_accounts).await;
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
    shutdown_rx: watch::Receiver<bool>,
    lookup: Arc<InstrumentLookupMap>,
    live_accounts: Arc<LiveAccountStore>,
) {
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

    let raw_http = Arc::new(RawHyperliquidHttpClient::new(RawHttpConfig {
        environment,
        account_address: agent.wallet_address.clone(),
    }));

    info!(
        agent_key = %agent.agent_key,
        wallet_address = %agent.wallet_address,
        environment = %config.environment.as_journal_str(),
        "agent live loop starting (startup HTTP sync + WebSocket)"
    );

    // Set the initial visible state for the API.
    live_accounts.set_status(
        &crate::hyperliquid::live_state::AccountKey::new(
            config.account_address.clone(),
            config.environment.as_journal_str(),
        ),
        crate::hyperliquid::live_state::LiveConnectionStatus::StartupSyncing,
    );

    if *shutdown_rx.borrow() {
        live_accounts.set_status(
            &crate::hyperliquid::live_state::AccountKey::new(
                config.account_address.clone(),
                config.environment.as_journal_str(),
            ),
            crate::hyperliquid::live_state::LiveConnectionStatus::Stopped,
        );
        return;
    }

    if let Err(e) = sync_account_once(&pool, &config, &raw_http, &lookup).await {
        error!(
            agent_key = %agent.agent_key,
            wallet_address = %config.account_address,
            environment = %config.environment.as_journal_str(),
            error = ?e,
            "startup account sync failed"
        );
    }

    if let Err(e) = sync_historical_orders_once(&pool, &config, &raw_http, &lookup).await {
        error!(
            agent_key = %agent.agent_key,
            wallet_address = %config.account_address,
            environment = %config.environment.as_journal_str(),
            error = ?e,
            "startup historical orders sync failed"
        );
    }

    // Run the WebSocket loop until shutdown. The `run_account_live_ws`
    // implementation handles reconnect catch-up internally.
    if let Err(e) = run_account_live_ws(
        pool.clone(),
        config.clone(),
        Arc::clone(&lookup),
        Arc::clone(&raw_http),
        Arc::clone(&live_accounts),
        shutdown_rx.clone(),
        LiveWsOptions::default(),
    )
    .await
    {
        error!(
            agent_key = %agent.agent_key,
            wallet_address = %config.account_address,
            environment = %config.environment.as_journal_str(),
            error = ?e,
            "live WebSocket loop exited with error"
        );
    }

    info!(agent_key = %agent.agent_key, "agent live loop stopped");
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
