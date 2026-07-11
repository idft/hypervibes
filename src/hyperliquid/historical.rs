use crate::hyperliquid::raw_http::{
    RawHyperliquidHttpClient, RawLedgerUpdate, RawUserFill, RawUserFunding,
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
