use std::time::Duration;

use askama::Template;

use crate::hyperliquid::live_state::{
    AccountLiveState, LiveDataStatus, LiveOpenOrder, account_live_health,
};

use super::live_health::LiveAccountDisplayState;
use super::shared::{
    MoneyCell, currency_logo_url, format_money_cell, format_neutral_money_cell_with_decimals,
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
    pub is_loading: bool,
    pub unavailable_message: Option<String>,
}

impl OpenOrdersView {
    pub fn from_live_state(state: AccountLiveState) -> Self {
        let health = account_live_health(&state);
        let display = LiveAccountDisplayState::from_live_state(&state);
        let (is_loading, unavailable_message) = if display.orders_available {
            (false, None)
        } else {
            section_message(health.open_orders)
        };
        if !display.orders_available {
            return Self {
                orders: Vec::new(),
                is_loading,
                unavailable_message,
            };
        }

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
            is_loading,
            unavailable_message,
        }
    }
}

fn section_message(status: LiveDataStatus) -> (bool, Option<String>) {
    match status {
        LiveDataStatus::Current => (false, None),
        LiveDataStatus::Loading => (true, None),
        LiveDataStatus::Stale => (
            false,
            Some(
                "Live open-orders data is stale. Waiting for a current exchange snapshot."
                    .to_string(),
            ),
        ),
        LiveDataStatus::Degraded => (
            false,
            Some(
                "Live open-orders data is unavailable while exchange monitoring is not connected."
                    .to_string(),
            ),
        ),
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
