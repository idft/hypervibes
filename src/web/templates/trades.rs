use rust_decimal::Decimal;

use crate::hyperliquid::trade_store::{TradeFillRow, TradeRow};

use super::shared::{
    LocalTimestampView, MoneyCell, currency_logo_url, format_decimal_with_commas,
    format_money_cell_with_decimals, local_timestamp_view,
};

fn trade_number(value: Decimal) -> String {
    let rounded = value.round_dp(4).normalize();
    format_decimal_with_commas(rounded, rounded.scale() as usize)
}

fn trade_pnl(value: Decimal) -> MoneyCell {
    let rounded = value.round_dp(4).normalize();
    format_money_cell_with_decimals(Some(rounded), rounded.scale() as usize)
}

#[derive(Debug, Clone)]
pub struct TradeView {
    pub row: TradeRow,
    pub logo_url: String,
    pub last_activity: LocalTimestampView,
    pub entry_price: String,
    pub exit_price: String,
    pub amount: String,
    pub gross_pnl: MoneyCell,
    pub usdc_fees: String,
    pub net_realized: MoneyCell,
    pub funding: MoneyCell,
}

impl TradeView {
    pub fn from_row(row: TradeRow) -> Self {
        Self {
            logo_url: currency_logo_url(&row.symbol),
            last_activity: local_timestamp_view(row.last_activity_at),
            entry_price: row
                .entry_price()
                .map(trade_number)
                .unwrap_or_else(|| "—".into()),
            exit_price: row
                .exit_price()
                .map(trade_number)
                .unwrap_or_else(|| "—".into()),
            amount: trade_number(row.entry_size),
            gross_pnl: trade_pnl(row.gross_pnl),
            usdc_fees: trade_number(row.usdc_fees),
            net_realized: row
                .funding_usdc
                .filter(|_| row.fee_coverage)
                .map(|funding| trade_pnl(row.net_pnl + funding))
                .unwrap_or_else(|| MoneyCell {
                    value: "Incomplete".into(),
                    color_class: "text-amber-300",
                }),
            funding: row
                .funding_usdc
                .map(trade_pnl)
                .unwrap_or_else(|| MoneyCell {
                    value: "Incomplete".into(),
                    color_class: "text-amber-300",
                }),
            row,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TradeFillView {
    pub row: TradeFillRow,
    pub quantity: String,
    pub price: String,
    pub fee_usdc: Option<String>,
    pub realized_pnl: MoneyCell,
}

impl TradeFillView {
    pub fn from_row(row: TradeFillRow) -> Self {
        Self {
            quantity: trade_number(row.quantity),
            price: trade_number(row.price),
            fee_usdc: row.fee_usdc.map(trade_number),
            realized_pnl: trade_pnl(row.realized_pnl_usdc),
            row,
        }
    }
}
