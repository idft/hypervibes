mod agentic;
mod agents;
mod config;
mod db;
mod hermes;
mod hyperliquid;
mod memory;
mod opencode;
mod settings;
mod web;

#[cfg(test)]
mod test_db;

use std::sync::Arc;

use anyhow::{Context, Result};
use config::AppConfig;
use db::{connect, migrate};
use hermes::HermesClient;
use tokio::sync::watch;
use tracing::{info, warn};
use tracing_subscriber::{EnvFilter, fmt};

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

    let hermes = match HermesClient::new(
        config.hermes_dashboard_url.clone(),
        config.hermes_dashboard_session_token.clone(),
    ) {
        Ok(client) => {
            info!("Hermes client configured");
            Some(client)
        }
        Err(e) => {
            warn!(error = ?e, "Hermes client disabled");
            None
        }
    };

    let repo_root =
        std::env::current_dir().context("failed to resolve current working directory")?;
    let opencode_workspace_config = opencode::workspace::OpenCodeWorkspaceConfig {
        source_root: repo_root.join(opencode::workspace::PROFILE_SOURCE_RELATIVE_PATH),
        host_workspaces_root: config.opencode_workspaces_root.clone(),
        container_workspaces_root: config.opencode_container_workspaces_root.clone(),
        api_base_url: config.vibetrading_agent_api_base_url.clone(),
    };

    let opencode_client =
        opencode::client::OpenCodeClient::new(opencode::client::OpenCodeClientConfig::new(
            config.opencode_server_username.clone(),
            config.opencode_server_password.clone(),
        ))
        .context("failed to build OpenCode HTTP client")?;
    let opencode_backend: Arc<dyn agentic::backend::AgenticBackend> = Arc::new(
        agentic::backend::OpenCodeBackend::new(Arc::new(opencode_client)),
    );

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
        hermes,
        config.hermes_dashboard_link_url.clone(),
        opencode_workspace_config,
        shutdown_tx,
    );
    tokio::pin!(server_future);

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

    println!("Shutdown complete");
    Ok(())
}
