mod agentic;
mod agents;
mod cache;
mod config;
mod db;
mod hyperliquid;
mod memory;
mod model_catalog;
mod opencode;
mod settings;
mod web;

#[cfg(test)]
mod test_db;

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use config::AppConfig;
use db::{connect, migrate};
use tokio::sync::watch;
use tracing_subscriber::{EnvFilter, fmt};

use crate::agentic::in_flight::InFlightTracker;
use crate::hyperliquid::live_state::LiveAccountStore;

/// Maximum time `main` will wait for in-flight agentic dispatches to
/// drain after the scheduler and web server have both stopped, before
/// exiting anyway. Sized to be larger than the longest configured
/// `agentic_job_schedules.timeout_seconds` (currently 900s) so a
/// hung-but-still-progressing dispatch can complete, plus a 15m
/// buffer for cleanup. The scheduler enforces the same ceiling, so
/// this is a belt-and-suspenders check.
const SHUTDOWN_IN_FLIGHT_GRACE: Duration = Duration::from_secs(30 * 60);

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,tower_http=info")),
        )
        .init();

    let config = AppConfig::from_env()?;
    println!("Starting Vibetrading web server");
    println!("Connecting to database");
    let pool = connect(&config.database_url).await?;
    println!("Running migrations");
    migrate(&pool).await?;

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let live_accounts = Arc::new(LiveAccountStore::new());
    let encryption_key = agents::crypto::EncryptionKey::new(
        config.agents_encryption_key_id.clone(),
        config.agents_encryption_key,
    );
    let in_flight = InFlightTracker::new();
    let in_flight_for_scheduler = in_flight.clone();
    let in_flight_for_web = in_flight.clone();

    let repo_root =
        std::env::current_dir().context("failed to resolve current working directory")?;
    let opencode_workspace_config = opencode::workspace::OpenCodeWorkspaceConfig {
        source_root: repo_root.join(opencode::workspace::PROFILE_SOURCE_RELATIVE_PATH),
        host_workspaces_root: config.opencode_workspaces_root.clone(),
        container_workspaces_root: config.opencode_container_workspaces_root.clone(),
        api_base_url: config.vibetrading_agent_api_base_url.clone(),
    };

    let opencode_client = Arc::new(
        opencode::client::OpenCodeClient::new(opencode::client::OpenCodeClientConfig::new(
            config.opencode_server_username.clone(),
            config.opencode_server_password.clone(),
        ))
        .context("failed to build OpenCode HTTP client")?,
    );
    let opencode_backend: Arc<dyn agentic::backend::AgenticBackend> = Arc::new(
        agentic::backend::OpenCodeBackend::new(pool.clone(), Arc::clone(&opencode_client)),
    );
    let asset_cache = cache::asset::AssetCache::new(config.app_cache_dir.clone())?;
    let model_catalog =
        model_catalog::models_dev::ModelsDevCatalog::shared(config.app_cache_dir.clone())?;
    let warm_model_catalog = Arc::clone(&model_catalog);
    tokio::spawn(async move {
        if let Err(error) = warm_model_catalog.snapshot().await {
            eprintln!("models.dev catalog warmup failed: {error:#}");
        }
    });

    println!("Starting Hyperliquid agent monitor");
    let hyperliquid_monitor = agents::HyperliquidAgentMonitor::new(
        pool.clone(),
        shutdown_rx.clone(),
        Arc::clone(&live_accounts),
        encryption_key.clone(),
    );
    let mut hyperliquid_monitor_handle = tokio::spawn(async move {
        if let Err(e) = hyperliquid_monitor.run().await {
            eprintln!("hyperliquid agent monitor exited with error: {e}");
        }
    });

    println!("Starting agentic scheduler");
    let agentic_scheduler = agentic::scheduler::AgenticScheduler::new(
        pool.clone(),
        shutdown_rx.clone(),
        opencode_backend.clone(),
        Arc::clone(&live_accounts),
        opencode_workspace_config.clone(),
        Arc::clone(&opencode_client),
        in_flight_for_scheduler,
    );
    let mut agentic_scheduler_handle = tokio::spawn(async move {
        if let Err(e) = agentic_scheduler.run().await {
            eprintln!("agentic scheduler exited with error: {e}");
        }
    });

    println!("Listening on http://{}", config.bind_addr);

    let server_future = web::serve(
        &config.bind_addr,
        pool,
        opencode_backend,
        encryption_key,
        Arc::clone(&live_accounts),
        opencode_workspace_config,
        opencode_client,
        model_catalog,
        Arc::new(asset_cache),
        shutdown_rx.clone(),
        in_flight_for_web,
    );
    tokio::pin!(server_future);

    // Single source of truth for shutdown: when SIGINT or SIGTERM
    // arrives, set the shared flag. Every long-running task
    // (web server's graceful shutdown, agentic scheduler's loop,
    // hyperliquid monitor's loop) observes it and drains.
    spawn_shutdown_listener(shutdown_tx);

    tokio::select! {
        result = &mut server_future => {
            result?;
        }
        _ = &mut hyperliquid_monitor_handle => {
            eprintln!("hyperliquid agent monitor exited early");
            return Ok(());
        }
        _ = &mut agentic_scheduler_handle => {
            eprintln!("agentic scheduler exited early");
            return Ok(());
        }
    }

    // Wait for the background tasks to finish their graceful shutdown.
    let _ = hyperliquid_monitor_handle.await;
    let _ = agentic_scheduler_handle.await;

    // The scheduler drains the trackers it knows about inside its
    // own `run`, but manual hook dispatches that started after the
    // scheduler already drained are only visible here. One more
    // bounded wait covers them.
    if in_flight.in_flight() > 0 {
        tracing::info!(
            in_flight = in_flight.in_flight(),
            grace_seconds = SHUTDOWN_IN_FLIGHT_GRACE.as_secs(),
            "main waiting for in-flight agentic dispatches to complete"
        );
        let drained = in_flight
            .wait_idle_with_timeout(SHUTDOWN_IN_FLIGHT_GRACE)
            .await;
        if !drained {
            tracing::warn!(
                remaining = in_flight.in_flight(),
                "in-flight agentic dispatches did not drain within grace period; \
                 leaving them orphaned for the next start to recover"
            );
        } else {
            tracing::info!("all in-flight agentic dispatches completed");
        }
    }

    println!("Shutdown complete");
    Ok(())
}

/// Spawn a background task that listens for `SIGINT` and `SIGTERM`
/// and flips the shutdown watch on the first signal received. The
/// second signal is logged but ignored so operators can see it; force
/// kill the process if they need a hard exit.
fn spawn_shutdown_listener(shutdown_tx: watch::Sender<bool>) {
    tokio::spawn(async move {
        #[cfg(unix)]
        let mut sigterm =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(stream) => stream,
                Err(error) => {
                    tracing::error!(error = ?error, "failed to install SIGTERM handler");
                    return;
                }
            };

        let signal_name;
        #[cfg(unix)]
        {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    signal_name = "SIGINT";
                }
                _ = sigterm.recv() => {
                    signal_name = "SIGTERM";
                }
            }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
            signal_name = "ctrl_c";
        }

        tracing::info!(signal = signal_name, "shutdown signal received; draining");
        let _ = shutdown_tx.send(true);
    });
}
