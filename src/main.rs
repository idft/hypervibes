pub mod agent_conversations;
mod agents;
mod cache;
mod config;
mod db;
mod gateway;
mod harness;
mod hyperliquid;
pub mod indicators;
mod memory;
mod model_catalog;
mod notifications;
mod opencode;
mod web;

#[cfg(test)]
mod test_db;

use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result};
use config::AppConfig;
use dashmap::DashMap;
use db::{connect, migrate};
use tokio::sync::watch;
use tracing::{error, info, warn};
use tracing_subscriber::{EnvFilter, fmt};
use uuid::Uuid;

use crate::gateway::model::PendingLink;
use crate::harness::in_flight::{InFlightTracker, SHUTDOWN_IN_FLIGHT_GRACE};
use crate::hyperliquid::live_state::LiveAccountStore;
use crate::opencode::retry::retry_until_ready;

const OPENCODE_STARTUP_RETRY_INTERVAL: Duration = Duration::from_secs(5);

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
    info!(
        indicator_max_concurrent_executions = config.indicators.max_concurrent_executions,
        indicator_concurrency_source = if config.indicators.concurrency_overridden {
            "environment"
        } else {
            "automatic"
        },
        indicator_analysis_wait_timeout_seconds = config.indicators.analysis_wait_timeout.as_secs(),
        "resolved indicator settings"
    );
    indicators::coordination::set_analysis_wait_timeout(config.indicators.analysis_wait_timeout);
    info!("starting HyperVibes web server");
    info!("connecting to database");
    let pool = connect(&config.database_url).await?;
    info!("running migrations");
    migrate(&pool).await?;

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (force_shutdown_tx, force_shutdown_rx) = watch::channel(false);
    let indicator_queue_notifier = indicators::coordination::IndicatorQueueNotifier::new();
    indicators::coordination::install_queue_notifier(indicator_queue_notifier.clone());
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
    let mut warm_shutdown_rx = shutdown_rx.clone();
    tokio::spawn(async move {
        info!(workspace = %warm_provider_workspace, "warming shared OpenCode provider cache");
        let response = retry_until_ready(
            "OpenCode provider cache warmup",
            OPENCODE_STARTUP_RETRY_INTERVAL,
            &mut warm_shutdown_rx,
            || {
                let client = Arc::clone(&warm_provider_client);
                let base_url = warm_opencode_base_url.clone();
                let workspace = warm_provider_workspace.clone();
                async move { client.list_providers(&base_url, &workspace).await }
            },
        )
        .await;
        if let Some(response) = response {
            info!(
                providers = response.all.len(),
                connected = response.connected.len(),
                "OpenCode provider cache warmed"
            );
        }
    });

    info!("starting Hyperliquid agent monitor");
    let hyperliquid_monitor = agents::HyperliquidAgentMonitor::new(
        pool.clone(),
        shutdown_rx.clone(),
        Arc::clone(&live_accounts),
        encryption_key.clone(),
    );
    let hyperliquid_monitor_handle = tokio::spawn(async move {
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
            agent_api_base_url: config.hypervibes_agent_api_base_url.clone(),
            container_workspaces_root: config.opencode_container_workspaces_root.clone(),
            opencode_client: Arc::clone(&opencode_client),
            in_flight: in_flight_for_scheduler,
        },
        workspace_leases.clone(),
    );
    let harness_scheduler_handle = tokio::spawn(async move {
        if let Err(e) = harness_scheduler.run().await {
            error!(error = ?e, "harness scheduler exited with error");
        }
    });

    info!("starting indicator scheduler");
    let indicator_scheduler = indicators::scheduler::IndicatorScheduler::new(
        pool.clone(),
        shutdown_rx.clone(),
        force_shutdown_rx.clone(),
        indicator_queue_notifier,
        config.indicators.max_concurrent_executions,
    );
    let indicator_scheduler_handle = tokio::spawn(async move {
        if let Err(e) = indicator_scheduler.run().await {
            error!(error = ?e, "indicator scheduler exited with error");
        }
    });

    info!(address = %config.bind_addr, "listening for web requests");

    let gateway_pending_links: Arc<DashMap<Uuid, PendingLink>> = Arc::new(DashMap::new());
    let gateway_service = gateway::GatewayService::new(
        pool.clone(),
        Arc::clone(&opencode_client),
        config.opencode_base_url.clone(),
        config.hypervibes_agent_api_base_url.clone(),
        encryption_key.clone(),
        shutdown_rx.clone(),
        in_flight.clone(),
        Arc::clone(&workspace_controller),
        crate::agent_conversations::service::ConversationTurnTracker::default(),
        Arc::clone(&gateway_pending_links),
    );
    let gateway_handle = tokio::spawn(async move {
        if let Err(e) = gateway_service.run().await {
            error!(error = ?e, "gateway service exited with error");
        }
    });

    let server_future = web::serve(
        &config.bind_addr,
        pool,
        opencode_backend,
        encryption_key,
        Arc::clone(&live_accounts),
        workspace_controller,
        config.hypervibes_agent_api_base_url,
        config.opencode_container_workspaces_root,
        config.opencode_base_url,
        opencode_client,
        model_catalog,
        Arc::new(asset_cache),
        shutdown_rx.clone(),
        force_shutdown_rx.clone(),
        in_flight_for_web,
        workspace_leases,
        gateway_pending_links,
    );
    // Single source of truth for shutdown: when SIGINT or SIGTERM
    // arrives, set the shared flag. Every long-running task
    // (web server's graceful shutdown, harness scheduler's loop,
    // hyperliquid monitor's loop) observes it and drains. A second
    // signal flips a separate `force` watch that short-circuits
    // the web server's in-flight hold and the scheduler's drain
    // wait, so the API is cut immediately and in-flight dispatches
    // fail fast on their next MCP call.
    spawn_shutdown_listener(shutdown_tx, force_shutdown_tx);

    wait_for_services(
        server_future,
        shutdown_rx,
        force_shutdown_rx.clone(),
        hyperliquid_monitor_handle,
        harness_scheduler_handle,
        indicator_scheduler_handle,
        gateway_handle,
    )
    .await?;

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

async fn wait_for_services<F>(
    server_future: F,
    shutdown_rx: watch::Receiver<bool>,
    mut force_shutdown_rx: watch::Receiver<bool>,
    hyperliquid_monitor_handle: tokio::task::JoinHandle<()>,
    harness_scheduler_handle: tokio::task::JoinHandle<()>,
    indicator_scheduler_handle: tokio::task::JoinHandle<()>,
    gateway_handle: tokio::task::JoinHandle<()>,
) -> Result<()>
where
    F: std::future::Future<Output = Result<()>>,
{
    tokio::select! {
        result = wait_for_services_gracefully(
            server_future,
            shutdown_rx,
            hyperliquid_monitor_handle,
            harness_scheduler_handle,
            indicator_scheduler_handle,
            gateway_handle,
        ) => result,
        _ = async {
            loop {
                if *force_shutdown_rx.borrow() { break; }
                if force_shutdown_rx.changed().await.is_err() {
                    std::future::pending::<()>().await;
                }
            }
        } => {
            warn!("force shutdown: abandoning pending web and background task drains");
            Ok(())
        }
    }
}

async fn wait_for_services_gracefully<F>(
    server_future: F,
    shutdown_rx: watch::Receiver<bool>,
    mut hyperliquid_monitor_handle: tokio::task::JoinHandle<()>,
    mut harness_scheduler_handle: tokio::task::JoinHandle<()>,
    mut indicator_scheduler_handle: tokio::task::JoinHandle<()>,
    mut gateway_handle: tokio::task::JoinHandle<()>,
) -> Result<()>
where
    F: std::future::Future<Output = Result<()>>,
{
    tokio::pin!(server_future);
    let server_finished = tokio::select! {
        result = &mut server_future => {
            result?;
            true
        }
        _ = &mut hyperliquid_monitor_handle => {
            if !*shutdown_rx.borrow() {
                warn!("hyperliquid agent monitor exited early");
                return Ok(());
            }
            false
        }
        _ = &mut harness_scheduler_handle => {
            if !*shutdown_rx.borrow() {
                warn!("harness scheduler exited early");
                return Ok(());
            }
            false
        }
        _ = &mut indicator_scheduler_handle => {
            if !*shutdown_rx.borrow() {
                warn!("indicator scheduler exited early");
                return Ok(());
            }
            false
        }
        _ = &mut gateway_handle => {
            if !*shutdown_rx.borrow() {
                warn!("gateway service exited early");
                return Ok(());
            }
            false
        }
    };

    // A background task may finish first during normal shutdown. Keep the
    // web API available until dispatches drain, regardless of which task won
    // the select. Do not poll a JoinHandle already consumed by that select.
    if !server_finished {
        server_future.await?;
    }
    if !hyperliquid_monitor_handle.is_finished() {
        let _ = hyperliquid_monitor_handle.await;
    }
    if !harness_scheduler_handle.is_finished() {
        let _ = harness_scheduler_handle.await;
    }
    if !indicator_scheduler_handle.is_finished() {
        let _ = indicator_scheduler_handle.await;
    }
    if !gateway_handle.is_finished() {
        let _ = gateway_handle.await;
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn normal_worker_exit_during_shutdown_does_not_interrupt_web_or_harness_drain() {
        let (_shutdown_tx, shutdown_rx) = watch::channel(true);
        let (_force_tx, force_rx) = watch::channel(false);
        let (web_tx, web_rx) = tokio::sync::oneshot::channel::<()>();
        let (harness_tx, harness_rx) = tokio::sync::oneshot::channel::<()>();
        let waiter = tokio::spawn(wait_for_services(
            async move {
                web_rx.await.expect("release web drain");
                Ok(())
            },
            shutdown_rx,
            force_rx,
            tokio::spawn(async {}),
            tokio::spawn(async move {
                harness_rx.await.expect("release harness drain");
            }),
            tokio::spawn(async {}),
            tokio::spawn(async {}),
        ));

        tokio::task::yield_now().await;
        assert!(!waiter.is_finished(), "web drain should still be pending");
        web_tx.send(()).expect("release web drain");
        tokio::task::yield_now().await;
        assert!(
            !waiter.is_finished(),
            "harness drain should still be pending"
        );
        harness_tx.send(()).expect("release harness drain");
        tokio::time::timeout(std::time::Duration::from_secs(1), waiter)
            .await
            .expect("shutdown finishes")
            .expect("join waiter")
            .expect("wait for services");
    }

    #[tokio::test]
    async fn force_shutdown_interrupts_stuck_services() {
        let (_shutdown_tx, shutdown_rx) = watch::channel(true);
        let (force_tx, force_rx) = watch::channel(false);
        let mut waiter = tokio::spawn(wait_for_services(
            std::future::pending::<Result<()>>(),
            shutdown_rx,
            force_rx,
            tokio::spawn(async {}),
            tokio::spawn(std::future::pending()),
            tokio::spawn(async {}),
            tokio::spawn(async {}),
        ));

        tokio::task::yield_now().await;
        assert!(!waiter.is_finished(), "services should still be draining");
        force_tx.send(true).expect("force shutdown");
        let result = tokio::time::timeout(std::time::Duration::from_millis(200), &mut waiter).await;
        if result.is_err() {
            waiter.abort();
        }
        result
            .expect("force should interrupt the service drain")
            .expect("join waiter")
            .expect("wait for services");
    }
}
