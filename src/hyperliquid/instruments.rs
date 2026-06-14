use anyhow::Result;
use chrono::Utc;
use nautilus_hyperliquid::http::{client::HyperliquidHttpClient, parse::HyperliquidInstrumentDef};

use crate::hyperliquid::{config::HyperliquidEnvironment, normalize::{InstrumentRow, MarketType}};

pub async fn load_instruments(environment: HyperliquidEnvironment) -> Result<Vec<InstrumentRow>> {
    let client = HyperliquidHttpClient::new(environment.as_nt_environment(), 60, None)?;
    let defs = client.request_instrument_defs().await?;
    Ok(defs.into_iter().map(map_instrument_def).collect())
}

fn map_instrument_def(def: HyperliquidInstrumentDef) -> InstrumentRow {
    let now = Utc::now();

    InstrumentRow {
        instrument_id: format!("{}.HYPERLIQUID", def.symbol),
        symbol: def.symbol.to_string(),
        raw_symbol: def.raw_symbol.to_string(),
        market_type: match def.market_type {
            nautilus_hyperliquid::http::parse::HyperliquidMarketType::Perp => MarketType::Perp,
            nautilus_hyperliquid::http::parse::HyperliquidMarketType::Spot => MarketType::Spot,
            nautilus_hyperliquid::http::parse::HyperliquidMarketType::Outcome => {
                MarketType::Outcome
            }
        },
        base_asset: def.base.to_string(),
        quote_asset: def.quote.to_string(),
        settlement_asset: def.settlement.map(|value| value.to_string()),
        asset_index: Some(def.asset_index as i32),
        price_decimals: def.price_decimals as i32,
        size_decimals: def.size_decimals as i32,
        tick_size: def.tick_size,
        lot_size: def.lot_size,
        is_hip3: def.is_hip3,
        active: def.active,
        created_at: now,
        updated_at: now,
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal::Decimal;

    use super::map_instrument_def;

    #[test]
    fn maps_perp_definition_to_journal_row() {
        let row = map_instrument_def(nautilus_hyperliquid::http::parse::HyperliquidInstrumentDef {
            symbol: "BTC-USD-PERP".into(),
            raw_symbol: "BTC".into(),
            base: "BTC".into(),
            quote: "USD".into(),
            settlement: Some("USDC".into()),
            market_type: nautilus_hyperliquid::http::parse::HyperliquidMarketType::Perp,
            asset_index: 0,
            price_decimals: 1,
            size_decimals: 5,
            tick_size: Decimal::new(1, 1),
            lot_size: Decimal::new(1, 5),
            max_leverage: None,
            only_isolated: false,
            is_hip3: false,
            active: true,
            outcome: None,
            raw_data: "{}".to_string(),
        });

        assert_eq!(row.instrument_id, "BTC-USD-PERP.HYPERLIQUID");
        assert_eq!(row.raw_symbol, "BTC");
        assert_eq!(row.base_asset, "BTC");
        assert_eq!(row.quote_asset, "USD");
        assert_eq!(row.settlement_asset.as_deref(), Some("USDC"));
    }
}
