use crate::hyperliquid::raw_http::{
    RawHistoricalOrder, RawHyperliquidHttpClient, RawLedgerUpdate, RawUserFill, RawUserFunding,
};
use anyhow::Result;

pub async fn fetch_user_fills_window(
    client: &RawHyperliquidHttpClient,
    start_time: u64,
    end_time: Option<u64>,
) -> Result<Vec<RawUserFill>> {
    client.user_fills_by_time(start_time, end_time).await
}

pub async fn fetch_funding_window(
    client: &RawHyperliquidHttpClient,
    start_time: u64,
    end_time: Option<u64>,
) -> Result<Vec<RawUserFunding>> {
    client.user_funding(start_time, end_time).await
}

pub async fn fetch_ledger_window(
    client: &RawHyperliquidHttpClient,
    start_time: u64,
    end_time: Option<u64>,
) -> Result<Vec<RawLedgerUpdate>> {
    client.non_user_funding_updates(start_time, end_time).await
}

#[allow(dead_code)]
pub async fn fetch_historical_orders(
    client: &RawHyperliquidHttpClient,
) -> Result<Vec<RawHistoricalOrder>> {
    client.historical_orders().await
}

/// Fetch all transaction-relevant streams for the same window.
///
/// Prefer [`sync_account_once`](crate::hyperliquid::account_sync::sync_account_once) for
/// incremental, per-stream reconciliation.
#[allow(dead_code)]
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
    let fills = fetch_user_fills_window(client, start_time, end_time).await?;
    let funding = fetch_funding_window(client, start_time, end_time).await?;
    let ledger = fetch_ledger_window(client, start_time, end_time).await?;
    let orders = fetch_historical_orders(client).await?;
    Ok((fills, funding, ledger, orders))
}
