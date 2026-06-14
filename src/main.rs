mod hyperliquid;
mod db;

use anyhow::Result;
use hyperliquid::config::AppConfig;
use hyperliquid::store::HyperliquidJournalStore;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    let config = AppConfig::from_env()?;
    let store = HyperliquidJournalStore::new(config);

    store.run().await
}
