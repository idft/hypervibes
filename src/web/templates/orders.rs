use std::time::Duration;

use askama::Template;
use chrono::{DateTime, Utc};

use crate::hyperliquid::live_state::{AccountLiveState, LiveConnectionStatus, LiveOpenOrder};

use super::shared::{
    MoneyCell, currency_logo_url, format_money_cell, format_neutral_money_cell_with_decimals,
    format_timestamp_iso, format_timestamp_utc,
};

/// Per-row view of an open resting order for the agent detail page.
#[derive(Debug, Clone)]
pub struct OpenOrderView {
    pub coin: String,
    pub logo_url: String,
    pub side: String,
    pub order_type: String,
    pub size: String,
    pub orig_size: String,
    pub price: MoneyCell,
    pub tif: String,
    pub reduce_only: bool,
    pub trigger_px: MoneyCell,
    pub age: String,
    pub is_trigger: bool,
    pub is_position_tpsl: bool,
}

/// View-model bundle handed to the open-orders partial template.
#[derive(Debug, Clone)]
pub struct OpenOrdersView {
    pub orders: Vec<OpenOrderView>,
    pub has_any_state: bool,
}

impl OpenOrdersView {
    pub fn from_live_state(state: AccountLiveState) -> Self {
        let has_any_state = state.status != LiveConnectionStatus::Starting
            || state.updated_at.is_some()
            || !state.open_positions.is_empty()
            || !state.open_orders.is_empty()
            || state.margin.is_some()
            || !state.spot_balances.is_empty();

        let mut indexed: Vec<(Option<rust_decimal::Decimal>, u64, &LiveOpenOrder)> = state
            .open_orders
            .iter()
            .map(|o| (o.limit_px, o.timestamp.unwrap_or(0), o))
            .collect();
        // Sort highest price first; missing prices sort to the end. When prices
        // match, newer orders come first, then coin for deterministic output.
        indexed.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| b.1.cmp(&a.1))
                .then_with(|| a.2.coin.cmp(&b.2.coin))
        });

        let orders: Vec<OpenOrderView> = indexed
            .into_iter()
            .map(|(_, _, order)| order_view(order))
            .collect();

        Self {
            orders,
            has_any_state,
        }
    }
}

pub(super) fn order_view(order: &LiveOpenOrder) -> OpenOrderView {
    let side = order.side.clone().unwrap_or_else(|| "-".to_string());
    let order_type = order.order_type.clone().unwrap_or_else(|| "-".to_string());
    let tif = order.tif.clone().unwrap_or_else(|| "-".to_string());
    let size = match order.sz {
        Some(v) => format!("{:.4}", v),
        None => "-".to_string(),
    };
    let orig_size = match order.orig_sz {
        Some(v) => format!("{:.4}", v),
        None => "-".to_string(),
    };
    let reduce_only = order.reduce_only.unwrap_or(false);
    let is_trigger = order.is_trigger.unwrap_or(false);
    let is_position_tpsl = order.is_position_tpsl.unwrap_or(false);
    let age = format_order_age(order.timestamp);
    OpenOrderView {
        coin: order.coin.clone(),
        logo_url: currency_logo_url(&order.coin),
        side,
        order_type,
        size,
        orig_size,
        price: format_neutral_money_cell_with_decimals(order.limit_px, 0),
        tif,
        reduce_only,
        trigger_px: format_money_cell(order.trigger_px),
        age,
        is_trigger,
        is_position_tpsl,
    }
}

pub(super) fn format_order_age(timestamp_ms: Option<u64>) -> String {
    let Some(ts_ms) = timestamp_ms else {
        return "-".to_string();
    };
    let now_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
    let age_ms = now_ms.saturating_sub(ts_ms);
    let duration = Duration::from_millis(age_ms);
    let total_seconds = duration.as_secs();
    if total_seconds < 60 {
        return format!("{total_seconds}s");
    }
    let total_minutes = total_seconds / 60;
    if total_minutes < 60 {
        let seconds = total_seconds % 60;
        return format!("{total_minutes}m {seconds}s");
    }
    let total_hours = total_minutes / 60;
    if total_hours < 24 {
        let minutes = total_minutes % 60;
        return format!("{total_hours}h {minutes}m");
    }
    let days = total_hours / 24;
    let hours = total_hours % 24;
    format!("{days}d {hours}h")
}

#[derive(Template)]
#[template(path = "agents/fragments/open-orders.html")]
pub struct OpenOrdersPartialTemplate {
    pub view: OpenOrdersView,
}

impl OpenOrdersPartialTemplate {
    pub fn render_view(view: OpenOrdersView) -> Result<String, askama::Error> {
        Self { view }.render()
    }
}

#[derive(Template)]
#[template(path = "agents/fragments/latest-trade-execution-summary.html")]
pub struct LatestTradeExecutionSummaryPartialTemplate {
    pub summary: Option<String>,
    /// `created_at` of the latest `trade_execution` memory formatted as
    /// an ISO 8601 / RFC 3339 string with a `Z` suffix, suitable for the
    /// `datetime` attribute of a `<time>` element consumed by
    /// `timeago.js`. Empty when no memory exists yet.
    pub created_at_iso: String,
    /// `created_at` of the latest `trade_execution` memory formatted as
    /// `YYYY-MM-DD HH:MM UTC`. Used as the timeago fallback so the
    /// timestamp is meaningful even before client-side JS hydrates.
    /// Empty when no memory exists yet.
    pub created_at_fallback_text: String,
}

impl LatestTradeExecutionSummaryPartialTemplate {
    pub fn render_view(
        summary: Option<String>,
        created_at: Option<DateTime<Utc>>,
    ) -> Result<String, askama::Error> {
        Self {
            summary,
            created_at_iso: created_at.map(format_timestamp_iso).unwrap_or_default(),
            created_at_fallback_text: created_at.map(format_timestamp_utc).unwrap_or_default(),
        }
        .render()
    }
}

#[derive(Template)]
#[template(path = "agents/fragments/latest-analysis-summary.html")]
pub struct LatestAnalysisSummaryPartialTemplate {
    pub summary: Option<String>,
    pub detail_url: Option<String>,
    /// `created_at` of the latest `market_analysis` memory formatted as an
    /// ISO 8601 / RFC 3339 string with a `Z` suffix, suitable for the
    /// `datetime` attribute of a `<time>` element consumed by
    /// `timeago.js`. Empty when no memory exists yet.
    pub created_at_iso: String,
    /// `created_at` of the latest `market_analysis` memory formatted as
    /// `YYYY-MM-DD HH:MM UTC`. Used as the timeago fallback so the
    /// timestamp is meaningful even before client-side JS hydrates.
    /// Empty when no memory exists yet.
    pub created_at_fallback_text: String,
    /// `expires_at` of the latest `market_analysis` memory (resolved from
    /// `stale_after` / `valid_for_seconds` / per-timeframe defaults)
    /// formatted as an ISO 8601 / RFC 3339 string, suitable for the
    /// `title` attribute of a `<time>` element. Empty when the row has
    /// no explicit or implicit expiration.
    pub expires_at_iso: String,
    /// `true` when `expires_at` is at or before the server's `now`. The
    /// agent page uses this to highlight the timestamp and surface a
    /// warning icon, since the trading loop will treat the market analysis
    /// as stale and the operator should investigate the gap.
    pub is_expired: bool,
}

impl LatestAnalysisSummaryPartialTemplate {
    pub fn render_view(
        summary: Option<String>,
        detail_url: Option<String>,
        created_at: Option<DateTime<Utc>>,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<String, askama::Error> {
        let is_expired = expires_at.is_some_and(|value| value <= Utc::now());
        Self {
            summary,
            detail_url,
            created_at_iso: created_at.map(format_timestamp_iso).unwrap_or_default(),
            created_at_fallback_text: created_at.map(format_timestamp_utc).unwrap_or_default(),
            expires_at_iso: expires_at.map(format_timestamp_iso).unwrap_or_default(),
            is_expired,
        }
        .render()
    }
}
