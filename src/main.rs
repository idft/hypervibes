mod agents;
mod config;
mod db;
mod hyperliquid;
mod memory;
mod web;

#[cfg(test)]
mod test_db;

use std::sync::Arc;

use anyhow::Result;
use config::AppConfig;
use db::{connect, migrate};
use tokio::sync::watch;
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

    println!("Starting agent orchestrator");
    let orchestrator = agents::AgentOrchestrator::new(
        pool.clone(),
        shutdown_rx,
        Arc::clone(&live_accounts),
        encryption_key.clone(),
    );
    let mut orchestrator_handle = tokio::spawn(async move {
        if let Err(e) = orchestrator.run().await {
            eprintln!("orchestrator exited with error: {e}");
        }
    });

    println!("Listening on http://{}", config.bind_addr);

    let server_future = web::serve(
        &config.bind_addr,
        pool,
        encryption_key,
        Arc::clone(&live_accounts),
        shutdown_tx,
    );
    tokio::pin!(server_future);

    tokio::select! {
        result = &mut server_future => {
            result?;
        }
        _ = &mut orchestrator_handle => {
            eprintln!("orchestrator exited early");
            return Ok(());
        }
    }

    // Wait for the orchestrator to finish its graceful shutdown.
    let _ = orchestrator_handle.await;

    println!("Shutdown complete");
    Ok(())
}
