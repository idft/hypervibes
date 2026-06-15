mod agents;
mod config;
mod db;
mod hyperliquid;
mod web;

use anyhow::Result;
use config::AppConfig;
use db::{connect, migrate};

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    let config = AppConfig::from_env()?;
    println!("Starting Vibetrading web server");
    println!("Connecting to database");
    let pool = connect(&config.database_url).await?;
    println!("Running migrations");
    migrate(&pool).await?;
    println!("Listening on http://{}", config.bind_addr);

    web::serve(
        &config.bind_addr,
        pool,
        crate::agents::crypto::EncryptionKey::new(
            config.agents_encryption_key_id.clone(),
            config.agents_encryption_key,
        ),
    )
    .await
}
