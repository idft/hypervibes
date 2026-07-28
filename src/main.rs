pub mod agent_conversations;
mod agents;
mod cache;
mod config;
mod db;
mod harness;
mod hyperliquid;
mod memory;
mod model_catalog;
mod opencode;
mod web;

#[cfg(test)]
mod test_db;

use std::sync::Arc;

use anyhow::{Context, Result};
use config::AppConfig;
use db::{connect, migrate};
use tokio::sync::watch;
use tracing::{error, info, warn};
use tracing_subscriber::{EnvFilter, fmt};

use crate::harness::in_flight::{InFlightTracker, SHUTDOWN_IN_FLIGHT_GRACE};
use crate::hyperliquid::live_state::LiveAccountStore;

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
    info!("starting Vibetrading web server");
    info!("connecting to database");
    let pool = connect(&config.database_url).await?;
    info!("running migrations");
    migrate(&pool).await?;

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (force_shutdown_tx, force_shutdown_rx) = watch::channel(false);
    let live_accounts = Arc::new(LiveAccountStore::new());
    let encryption_key = agents::crypto::EncryptionKey::new(
        config.agents_encryption_key_id.clone(),
        config.agents_encryption_key,
    );
    let in_flight = InFlightTracker::new();
    let workspace_leases = harness::workspace_lease::WorkspaceLeaseManager::new();
    let in_flight_for_scheduler = in_flight.clone();
    let in_flight_for_web = in_flight.clone();

    let workspace_controller: Arc<dyn opencode::workspace_control_client::WorkspaceController> =
        Arc::new(
            opencode::workspace_control_client::HttpWorkspaceController::new(
                &config.workspace_control_base_url,
                config.workspace_control_api_key.clone(),
            )?,
        );

    let opencode_client = Arc::new(
        opencode::client::OpenCodeClient::new(
            opencode::client::OpenCodeClientConfig::new_with_base_url(
                config.opencode_server_username.clone(),
                config.opencode_server_password.clone(),
                config.opencode_base_url.clone(),
            ),
        )
        .context("failed to build OpenCode HTTP client")?,
    );
    let opencode_backend: Arc<dyn harness::backend::HarnessBackend> = Arc::new(
        harness::backend::OpenCodeBackend::new(pool.clone(), Arc::clone(&opencode_client)),
    );
    let asset_cache = cache::asset::AssetCache::new(config.app_cache_dir.clone())?;
    let model_catalog =
        model_catalog::models_dev::ModelsDevCatalog::shared(config.app_cache_dir.clone())?;
    let warm_model_catalog = Arc::clone(&model_catalog);
    tokio::spawn(async move {
        if let Err(error) = warm_model_catalog.snapshot().await {
            warn!(error = ?error, "models.dev catalog warmup failed");
        }
    });
    let warm_provider_client = Arc::clone(&opencode_client);
    let warm_opencode_base_url = config.opencode_base_url.clone();
    let warm_provider_workspace = config.opencode_container_workspaces_root.clone();
    tokio::spawn(async move {
        info!(workspace = %warm_provider_workspace, "warming shared OpenCode provider cache");
        match warm_provider_client
            .list_providers(&warm_opencode_base_url, &warm_provider_workspace)
            .await
        {
            Ok(response) => info!(
                providers = response.all.len(),
                connected = response.connected.len(),
                "OpenCode provider cache warmed"
            ),
            Err(error) => warn!(error = ?error, "OpenCode provider warmup failed"),
        }
    });

    info!("starting Hyperliquid agent monitor");
    let hyperliquid_monitor = agents::HyperliquidAgentMonitor::new(
        pool.clone(),
        shutdown_rx.clone(),
        Arc::clone(&live_accounts),
        encryption_key.clone(),
    );
    let mut hyperliquid_monitor_handle = tokio::spawn(async move {
        if let Err(e) = hyperliquid_monitor.run().await {
            error!(error = ?e, "hyperliquid agent monitor exited with error");
        }
    });

    info!("starting harness scheduler");
    let harness_scheduler = harness::scheduler::HarnessScheduler::new_with_workspace_leases(
        pool.clone(),
        shutdown_rx.clone(),
        force_shutdown_rx.clone(),
        opencode_backend.clone(),
        Arc::clone(&live_accounts),
        harness::scheduler::HarnessSchedulerRuntime {
            workspace_controller: Arc::clone(&workspace_controller),
            agent_api_base_url: config.vibetrading_agent_api_base_url.clone(),
            container_workspaces_root: config.opencode_container_workspaces_root.clone(),
            opencode_client: Arc::clone(&opencode_client),
            in_flight: in_flight_for_scheduler,
        },
        workspace_leases.clone(),
    );
    let mut harness_scheduler_handle = tokio::spawn(async move {
        if let Err(e) = harness_scheduler.run().await {
            error!(error = ?e, "harness scheduler exited with error");
        }
    });

    info!(address = %config.bind_addr, "listening for web requests");

    let server_future = web::serve(
        &config.bind_addr,
        pool,
        opencode_backend,
        encryption_key,
        Arc::clone(&live_accounts),
        workspace_controller,
        config.vibetrading_agent_api_base_url,
        config.opencode_container_workspaces_root,
        config.opencode_base_url,
        opencode_client,
        model_catalog,
        Arc::new(asset_cache),
        shutdown_rx.clone(),
        force_shutdown_rx.clone(),
        in_flight_for_web,
        workspace_leases,
    );
    tokio::pin!(server_future);

    // Single source of truth for shutdown: when SIGINT or SIGTERM
    // arrives, set the shared flag. Every long-running task
    // (web server's graceful shutdown, harness scheduler's loop,
    // hyperliquid monitor's loop) observes it and drains. A second
    // signal flips a separate `force` watch that short-circuits
    // the web server's in-flight hold and the scheduler's drain
    // wait, so the API is cut immediately and in-flight dispatches
    // fail fast on their next MCP call.
    spawn_shutdown_listener(shutdown_tx, force_shutdown_tx);

    tokio::select! {
        result = &mut server_future => {
            result?;
        }
        _ = &mut hyperliquid_monitor_handle => {
            warn!("hyperliquid agent monitor exited early");
            return Ok(());
        }
        _ = &mut harness_scheduler_handle => {
            warn!("harness scheduler exited early");
            return Ok(());
        }
    }

    // Wait for the background tasks to finish their graceful shutdown.
    let _ = hyperliquid_monitor_handle.await;
    let _ = harness_scheduler_handle.await;

    // The scheduler drains the trackers it knows about inside its
    // own `run`, but manual event dispatches that started after the
    // scheduler already drained are only visible here. One more
    // bounded wait covers them, and the wait is short-circuited by
    // the force watch so a second Ctrl-C exits immediately.
    if in_flight.in_flight() > 0 {
        info!(
            in_flight = in_flight.in_flight(),
            grace_seconds = SHUTDOWN_IN_FLIGHT_GRACE.as_secs(),
            "main waiting for in-flight harness dispatches to complete"
        );
        let mut force_rx = force_shutdown_rx;
        let drained = tokio::select! {
            drained = in_flight.wait_idle_with_timeout(SHUTDOWN_IN_FLIGHT_GRACE) => drained,
            _ = async {
                loop {
                    if *force_rx.borrow() { break; }
                    if force_rx.changed().await.is_err() { return; }
                }
            } => false,
        };
        if !drained {
            warn!(
                remaining = in_flight.in_flight(),
                "in-flight harness dispatches did not drain; \
                 leaving them orphaned for the next start to recover"
            );
        } else {
            info!("all in-flight harness dispatches completed");
        }
    }

    info!("shutdown complete");
    Ok(())
}

/// Spawn a background task that listens for `SIGINT` and `SIGTERM` and
/// flips the shutdown watch on the first signal received. A second
/// signal flips a separate `force` watch that the web server and
/// scheduler observe to short-circuit the graceful drain path. After
/// the second signal, further signals are ignored.
fn spawn_shutdown_listener(
    shutdown_tx: watch::Sender<bool>,
    force_shutdown_tx: watch::Sender<bool>,
) {
    tokio::spawn(async move {
        #[cfg(unix)]
        let mut sigterm =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(stream) => stream,
                Err(error) => {
                    error!(error = ?error, "failed to install SIGTERM handler");
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

        info!(signal = signal_name, "shutdown signal received; draining");
        let _ = shutdown_tx.send(true);

        // Wait for a second signal to flip the force flag. The
        // web server observes it to cut the API immediately, and
        // the scheduler observes it to abandon its in-flight wait.
        #[cfg(unix)]
        {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = sigterm.recv() => {}
            }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
        }
        warn!("second shutdown signal received; force-shutting down");
        let _ = force_shutdown_tx.send(true);
    });
}
