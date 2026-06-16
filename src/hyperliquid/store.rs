use std::time::Duration;

use anyhow::Result;

use crate::{
    db::{connect, migrate},
    hyperliquid::{
        account_sync::{build_instrument_lookup, sync_account_once, sync_historical_orders_once},
        config::AppConfig,
        instruments::{load_instruments, upsert_instruments},
        polling::PollingSchedule,
        raw_http::{RawHttpConfig, RawHyperliquidHttpClient},
        sync_state::SyncStream,
    },
};

/// Standalone journal store for the manual/sync binary path.
///
/// The web server orchestrator uses [`sync_account_once`] directly instead.
#[allow(dead_code)]
pub struct HyperliquidJournalStore {
    config: AppConfig,
}

impl HyperliquidJournalStore {
    #[allow(dead_code)]
    pub fn new(config: AppConfig) -> Self {
        Self { config }
    }

    #[allow(dead_code)]
    pub async fn run(&self) -> Result<()> {
        let pool = connect(&self.config.database_url).await?;
        migrate(&pool).await?;

        let instruments = load_instruments(self.config.environment).await?;
        upsert_instruments(&pool, &instruments).await?;

        let raw_http = RawHyperliquidHttpClient::new(RawHttpConfig {
            environment: self.config.environment,
            account_address: self.config.account_address.clone(),
        });

        let lookup = build_instrument_lookup(&instruments);
        let account_config = self.config.to_account_sync_config();

        sync_account_once(&pool, &account_config, &raw_http, &lookup).await?;
        sync_historical_orders_once(&pool, &account_config, &raw_http, &lookup).await?;

        if self.config.poll_once {
            return Ok(());
        }

        let schedule = PollingSchedule::default();
        let mut historical_orders_elapsed = Duration::from_secs(0);
        loop {
            tokio::time::sleep(schedule.fills).await;
            sync_account_once(&pool, &account_config, &raw_http, &lookup).await?;

            historical_orders_elapsed += schedule.fills;
            if historical_orders_elapsed >= schedule.historical_orders {
                historical_orders_elapsed = Duration::from_secs(0);
                sync_historical_orders_once(&pool, &account_config, &raw_http, &lookup).await?;
            }
        }
    }
}

#[allow(dead_code)]
fn _sync_state_row_example() -> crate::hyperliquid::sync_state::SyncStateRow {
    crate::hyperliquid::sync_state::SyncStateRow::new(
        "0x0".to_string(),
        "live".to_string(),
        SyncStream::Fills,
    )
}
