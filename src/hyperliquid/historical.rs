use anyhow::Result;
use crate::hyperliquid::raw_http::{
    RawHistoricalOrder, RawHyperliquidHttpClient, RawLedgerUpdate, RawUserFill, RawUserFunding,
};

pub async fn fetch_historical_batch(
    client: &RawHyperliquidHttpClient,
    start_time: u64,
    end_time: Option<u64>,
) -> Result<(
    Vec<RawUserFill>,
    Vec<RawUserFunding>,
    Vec<RawLedgerUpdate>,
    Vec<RawHistoricalOrder>,
)> {
    let fills = client.user_fills_by_time(start_time, end_time).await?;
    let funding = client.user_funding(start_time, end_time).await?;
    let ledger = client.non_user_funding_updates(start_time, end_time).await?;
    let orders = client.historical_orders().await?;
    Ok((fills, funding, ledger, orders))
}
