use std::collections::{HashMap, HashSet};

use crate::hyperliquid::live_state::{
    AccountLiveState, LiveDataStatus, LivePosition, account_live_health,
};
use askama::Template;
use rust_decimal::Decimal;

use super::shared::{
    MoneyCell, currency_logo_url, dash_cell, format_decimal_with_commas,
    format_money_text_with_decimals, format_neutral_money_cell_with_decimals,
};

/// Per-row view of an open perpetual position for the agent detail page.
#[derive(Debug, Clone)]
pub struct OpenPositionView {
    pub coin: String,
    pub logo_url: String,
    pub market_url: String,
    pub has_position: bool,
    pub side: &'static str,
    pub size: String,
    pub entry_px: MoneyCell,
    pub mark_px_or_value: String,
    pub unrealized_pnl: MoneyCell,
    pub liquidation_px: MoneyCell,
    pub margin_used: MoneyCell,
    pub return_on_equity: String,
    pub roe_color_class: &'static str,
}

/// View-model bundle handed to the open-positions partial template.
#[derive(Debug, Clone)]
pub struct OpenPositionsView {
    pub positions: Vec<OpenPositionView>,
    pub is_loading: bool,
    pub unavailable_message: Option<String>,
    pub has_open_positions: bool,
    pub agent_key: String,
}

impl OpenPositionsView {
    #[cfg(test)]
    pub fn from_live_state(state: AccountLiveState) -> Self {
        Self::from_live_state_with_configured_coins(state, &[])
    }

    pub fn from_live_state_with_configured_coins(
        state: AccountLiveState,
        configured_coins: &[String],
    ) -> Self {
        let health = account_live_health(&state);
        let (is_loading, unavailable_message) = section_message(health.positions, "positions");

        if !health.positions.is_current() {
            return Self {
                positions: Vec::new(),
                is_loading,
                unavailable_message,
                has_open_positions: false,
                agent_key: String::new(),
            };
        }

        let mut visible: Vec<&LivePosition> = state
            .open_positions
            .iter()
            .filter(|p| p.szi.is_some_and(|s| !s.is_zero()))
            .collect();
        visible.sort_by(|a, b| {
            let a_abs = a.szi.map(|s| s.abs()).unwrap_or_default();
            let b_abs = b.szi.map(|s| s.abs()).unwrap_or_default();
            b_abs
                .partial_cmp(&a_abs)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.coin.cmp(&b.coin))
        });

        let mut visible_by_coin: HashMap<&str, &LivePosition> = visible
            .iter()
            .copied()
            .map(|position| (position.coin.as_str(), position))
            .collect();
        let mut configured_seen = HashSet::new();
        let mut positions = Vec::new();

        for coin in configured_coins {
            if !configured_seen.insert(coin.as_str()) {
                continue;
            }

            if let Some(position) = visible_by_coin.remove(coin.as_str()) {
                positions.push(position_view(position));
            } else {
                positions.push(empty_position_view(coin));
            }
        }

        for position in &visible {
            if visible_by_coin.contains_key(position.coin.as_str()) {
                positions.push(position_view(position));
            }
        }

        Self {
            has_open_positions: !visible.is_empty(),
            positions,
            is_loading,
            unavailable_message,
            agent_key: String::new(),
        }
    }

    pub fn with_agent_key(mut self, agent_key: &str) -> Self {
        self.agent_key = agent_key.to_string();
        self
    }
}

fn section_message(status: LiveDataStatus, section: &str) -> (bool, Option<String>) {
    match status {
        LiveDataStatus::Current => (false, None),
        LiveDataStatus::Loading => (true, None),
        LiveDataStatus::Stale => (
            false,
            Some(format!(
                "Live {section} data is stale. Waiting for a current exchange snapshot."
            )),
        ),
        LiveDataStatus::Degraded => (
            false,
            Some(format!(
                "Live {section} data is unavailable while exchange monitoring is not connected."
            )),
        ),
    }
}

pub(super) fn position_view(pos: &LivePosition) -> OpenPositionView {
    let szi = pos.szi.unwrap_or_default();
    let abs_szi = szi.abs();
    let side: &'static str = if szi.is_sign_negative() {
        "short"
    } else {
        "long"
    };

    let roe = match pos.return_on_equity {
        Some(v) => format_signed_percent(v, 2),
        None => "-".to_string(),
    };
    let roe_color_class = match pos.return_on_equity {
        Some(v) if v.is_sign_negative() => "text-red-400",
        Some(_) => "text-emerald-400",
        None => "text-zinc-500",
    };

    OpenPositionView {
        coin: pos.coin.clone(),
        logo_url: currency_logo_url(&pos.coin),
        market_url: hyperliquid_market_url(&pos.coin),
        has_position: true,
        side,
        size: format_size(abs_szi),
        entry_px: format_neutral_money_cell_with_decimals(pos.entry_px, 0),
        mark_px_or_value: format_money_text_with_decimals(pos.position_value, 0),
        unrealized_pnl: money_cell_for_pnl(pos.unrealized_pnl.unwrap_or_default()),
        liquidation_px: format_neutral_money_cell_with_decimals(pos.liquidation_px, 0),
        margin_used: format_neutral_money_cell_with_decimals(pos.margin_used, 0),
        return_on_equity: roe,
        roe_color_class,
    }
}

fn empty_position_view(coin: &str) -> OpenPositionView {
    OpenPositionView {
        coin: coin.to_string(),
        logo_url: currency_logo_url(coin),
        market_url: hyperliquid_market_url(coin),
        has_position: false,
        side: "No position",
        size: "-".to_string(),
        entry_px: dash_cell(),
        mark_px_or_value: "-".to_string(),
        unrealized_pnl: dash_cell(),
        liquidation_px: dash_cell(),
        margin_used: dash_cell(),
        return_on_equity: "-".to_string(),
        roe_color_class: "text-zinc-500",
    }
}

fn hyperliquid_market_url(coin: &str) -> String {
    format!("https://app.hyperliquid.xyz/trade/{coin}")
}

pub(super) fn money_cell_for_pnl(value: Decimal) -> MoneyCell {
    if value.is_zero() {
        dash_cell()
    } else {
        let abs = value.abs();
        let formatted = format_decimal_with_commas(abs, 4);
        if value.is_sign_negative() {
            MoneyCell {
                value: format!("({formatted})"),
                color_class: "text-red-400",
            }
        } else {
            MoneyCell {
                value: formatted,
                color_class: "text-emerald-400",
            }
        }
    }
}

pub(super) fn format_size(value: Decimal) -> String {
    format_decimal_with_commas(value, 4)
}

pub fn format_signed_percent(value: Decimal, decimals: usize) -> String {
    let abs = value.abs();
    let formatted = match decimals {
        0 => format_decimal_with_commas(abs, 0),
        1 => format_decimal_with_commas(abs, 1),
        2 => format_decimal_with_commas(abs, 2),
        3 => format_decimal_with_commas(abs, 3),
        _ => format_decimal_with_commas(abs, 4),
    };
    if value.is_sign_negative() {
        format!("-{formatted}%")
    } else {
        format!("+{formatted}%")
    }
}

#[derive(Template)]
#[template(path = "agents/fragments/open-positions.html")]
pub struct OpenPositionsPartialTemplate {
    pub view: OpenPositionsView,
}

impl OpenPositionsPartialTemplate {
    pub fn render_view(view: OpenPositionsView) -> Result<String, askama::Error> {
        Self { view }.render()
    }
}
