use std::time::Duration;

use anyhow::Result;

use crate::{
    db::{DbPool, connect, migrate},
    hyperliquid::{
        account_sync::{build_instrument_lookup, sync_account_once, sync_historical_orders_once},
        config::AppConfig,
        instruments::load_instruments,
        normalize::InstrumentRow,
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
async fn upsert_instruments(
    pool: &DbPool,
    instruments: &[InstrumentRow],
) -> Result<()> {
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

#[allow(dead_code)]
fn _sync_state_row_example() -> crate::hyperliquid::sync_state::SyncStateRow {
    crate::hyperliquid::sync_state::SyncStateRow::new(
        "0x0".to_string(),
        "live".to_string(),
        SyncStream::Fills,
    )
}
