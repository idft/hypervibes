use anyhow::{Context, Result};
use chrono::Utc;
use hypersdk::hypercore::{self, PerpMarket};
use rust_decimal::Decimal;
use sqlx::Postgres;

use crate::{
    db::DbPool,
    hyperliquid::{
        config::HyperliquidEnvironment,
        normalize::{InstrumentRow, MarketType},
    },
};

/// Load perpetual instrument definitions from Hyperliquid via `hypersdk`.
///
/// Only perpetual markets on the default DEX are loaded today. Spot, outcome,
/// and HIP-3 markets can be added later as a code-only change (the schema is
/// already venue-agnostic): also call `client.spot()`, `client.outcomes()`,
/// and iterate `client.perp_dexes()` + `client.perps_from(dex)`, mapping each
/// into `InstrumentRow` with the appropriate `MarketType`/`is_hip3`.
pub async fn load_instruments(environment: HyperliquidEnvironment) -> Result<Vec<InstrumentRow>> {
    let client = match environment {
        HyperliquidEnvironment::Mainnet => hypercore::mainnet(),
    };
    let perps = client
        .perps()
        .await
        .context("failed to fetch Hyperliquid perp markets")?;
    perps.iter().map(map_perp_market).collect()
}

fn map_perp_market(market: &PerpMarket) -> Result<InstrumentRow> {
    let now = Utc::now();
    let sz_decimals = u32::try_from(market.sz_decimals)
        .with_context(|| format!("negative sz_decimals for {}", market.name))?;
    // Perp price decimals: 6 - sz_decimals (matches PriceTick::for_perp).
    let price_decimals = 6i32 - market.sz_decimals as i32;
    // Lot size = 10^(-sz_decimals).
    let lot_size = Decimal::new(1, sz_decimals);

    Ok(InstrumentRow {
        instrument_id: market.name.clone(),
        name: market.name.clone(),
        market_type: MarketType::Perp,
        base_asset: market.name.clone(),
        quote_asset: market.collateral.name.clone(),
        settlement_asset: Some(market.collateral.name.clone()),
        asset_index: i32::try_from(market.index).ok(),
        price_decimals,
        size_decimals: market.sz_decimals as i32,
        lot_size,
        max_leverage: i32::try_from(market.max_leverage).ok(),
        is_hip3: false,
        active: true,
        created_at: now,
        updated_at: now,
    })
}

/// Upsert instrument rows into `hyperliquid.instruments`.
///
/// Canonical helper shared by the orchestrator and the standalone store path.
pub async fn upsert_instruments(pool: &DbPool, instruments: &[InstrumentRow]) -> Result<()> {
    for instrument in instruments {
        sqlx::query::<Postgres>(
            "INSERT INTO hyperliquid.instruments \
             (instrument_id, name, market_type, base_asset, quote_asset, settlement_asset, \
              asset_index, price_decimals, size_decimals, lot_size, max_leverage, is_hip3, \
              active, created_at, updated_at) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15) \
             ON CONFLICT (instrument_id) DO UPDATE SET \
              name = EXCLUDED.name, market_type = EXCLUDED.market_type, \
              base_asset = EXCLUDED.base_asset, quote_asset = EXCLUDED.quote_asset, \
              settlement_asset = EXCLUDED.settlement_asset, asset_index = EXCLUDED.asset_index, \
              price_decimals = EXCLUDED.price_decimals, size_decimals = EXCLUDED.size_decimals, \
              lot_size = EXCLUDED.lot_size, max_leverage = EXCLUDED.max_leverage, \
              is_hip3 = EXCLUDED.is_hip3, active = EXCLUDED.active, updated_at = EXCLUDED.updated_at",
        )
        .bind(&instrument.instrument_id)
        .bind(&instrument.name)
        .bind(match instrument.market_type {
            MarketType::Perp => "perp",
            MarketType::Spot => "spot",
            MarketType::Outcome => "outcome",
        })
        .bind(&instrument.base_asset)
        .bind(&instrument.quote_asset)
        .bind(&instrument.settlement_asset)
        .bind(instrument.asset_index)
        .bind(instrument.price_decimals)
        .bind(instrument.size_decimals)
        .bind(instrument.lot_size)
        .bind(instrument.max_leverage)
        .bind(instrument.is_hip3)
        .bind(instrument.active)
        .bind(instrument.created_at)
        .bind(instrument.updated_at)
        .execute(pool)
        .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use hypersdk::hypercore::{PerpMarket, PriceTick, SpotToken};
    use rust_decimal::Decimal;

    use super::map_perp_market;

    fn usdc() -> SpotToken {
        SpotToken {
            name: "USDC".into(),
            index: 0,
            token_id: Default::default(),
            evm_contract: None,
            cross_chain_address: None,
            sz_decimals: 8,
            wei_decimals: 8,
            evm_extra_decimals: 0,
        }
    }

    fn perp(name: &str, sz_decimals: i64, index: usize, max_leverage: u64) -> PerpMarket {
        PerpMarket {
            name: name.into(),
            index,
            sz_decimals,
            collateral: usdc(),
            max_leverage,
            isolated_margin: false,
            margin_mode: None,
            growth_mode: false,
            aligned_quote_token: false,
            table: PriceTick::for_perp(sz_decimals),
        }
    }

    #[test]
    fn maps_perp_market_to_instrument_row() {
        let row = map_perp_market(&perp("BTC", 5, 0, 40)).expect("row");
        assert_eq!(row.instrument_id, "BTC");
        assert_eq!(row.name, "BTC");
        assert_eq!(row.base_asset, "BTC");
        assert_eq!(row.quote_asset, "USDC");
        assert_eq!(row.settlement_asset.as_deref(), Some("USDC"));
        assert_eq!(row.asset_index, Some(0));
        assert_eq!(row.size_decimals, 5);
        assert_eq!(row.price_decimals, 1); // 6 - 5
        assert_eq!(row.lot_size, Decimal::new(1, 5));
        assert_eq!(row.max_leverage, Some(40));
        assert!(!row.is_hip3);
        assert!(row.active);
    }
}
