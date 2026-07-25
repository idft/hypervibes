use askama::Template;
use rust_decimal::Decimal;

use crate::hyperliquid::queries::BalancePoint;

use super::shared::{AnimatedNumber, MoneyCell, dash_cell, format_signed_money_cell};

/// View-model for the live account balance card shown on the agent detail
/// page and as a column on the agents index page. The same struct drives
/// both the initial render (when the page is first loaded) and the
/// live-updating SSE swaps (when the orchestrator pushes a new value).
///
/// The value shown mirrors the "Balance" shown in the Hyperliquid UI:
/// the perps `crossMarginSummary.accountValue` plus the spot USDC that is
/// *available* to use. Adding only the available spot (not the spot
/// total) avoids double-counting the USDC that has been transferred to
/// the perps account as initial margin — that money is already counted
/// inside `accountValue`. When the live state has no margin snapshot
/// yet, the spot USDC available is used on its own.
#[derive(Debug, Clone)]
pub struct AccountBalanceView {
    pub total_balance: Option<Decimal>,
    pub total_u_pnl: AnimatedNumber,
}

impl AccountBalanceView {
    pub fn from_live_state(state: crate::hyperliquid::live_state::AccountLiveState) -> Self {
        let perps_account_value = state
            .margin
            .as_ref()
            .and_then(|m| m.account_value)
            .filter(|v| !v.is_sign_negative());

        let spot_usdc = state
            .spot_balances
            .iter()
            .find(|b| b.coin.eq_ignore_ascii_case("USDC"));
        let spot_usdc_available = spot_usdc
            .and_then(|b| b.available)
            .filter(|v| !v.is_sign_negative());
        let spot_usdc_total = spot_usdc
            .and_then(|b| b.total)
            .filter(|v| !v.is_sign_negative());

        let total_balance = match (perps_account_value, spot_usdc_available) {
            (Some(perps), Some(spot_available)) => Some(perps + spot_available),
            (Some(perps), None) => Some(perps),
            (None, Some(spot_available)) => Some(spot_available),
            (None, None) => spot_usdc_total,
        };
        let total_u_pnl = state
            .open_positions
            .iter()
            .filter_map(|position| position.unrealized_pnl)
            .fold(Decimal::ZERO, |acc, value| acc + value);

        Self {
            total_balance,
            total_u_pnl: AnimatedNumber::for_pnl(total_u_pnl),
        }
    }

    /// The animated total balance value.
    ///
    /// Returns `None` when the value is not yet known; the template uses
    /// this to render a `Loading…` placeholder.
    pub fn total(&self) -> Option<AnimatedNumber> {
        self.total_balance
            .map(|v| AnimatedNumber::from_decimal(v, "text-zinc-100"))
    }
}

#[derive(Template)]
#[template(path = "account/balance.html")]
pub struct AccountBalancePartialTemplate {
    pub view: AccountBalanceView,
}

impl AccountBalancePartialTemplate {
    pub fn render_view(view: AccountBalanceView) -> Result<String, askama::Error> {
        Self { view }.render()
    }
}

/// View-model for a single server-rendered sparkline. The actual SVG is
/// computed by [`SparklineView::from_series`] and stored as a
/// pre-formatted `<polyline points="...">` attribute string so the
/// template can drop it straight into the markup.
#[derive(Debug, Clone)]
pub struct SparklineView {
    pub label: &'static str,
    pub polyline: String,
    pub change: MoneyCell,
    pub is_empty: bool,
    pub width: u32,
    pub height: u32,
}

impl SparklineView {
    /// Build a sparkline from a balance series. Pure function: no DB,
    /// no I/O, easy to unit test.
    ///
    /// * `points.len() < 2` → empty sparkline (`is_empty = true`); if
    ///   exactly one point is supplied its value is shown as
    ///   `last_value` and the change cell is a dash.
    /// * Otherwise the x-axis maps evenly across `0..width` and the
    ///   y-axis is normalized against `min..max` of the balances with a
    ///   small vertical padding, inverted for SVG (y grows downward).
    ///   The change cell is `last - first` formatted via
    ///   [`format_money_cell`].
    pub fn from_series(
        label: &'static str,
        points: &[BalancePoint],
        width: u32,
        height: u32,
    ) -> Self {
        if points.is_empty() {
            return Self {
                label,
                polyline: String::new(),
                change: dash_cell(),
                is_empty: true,
                width,
                height,
            };
        }

        if points.len() == 1 {
            return Self {
                label,
                polyline: String::new(),
                change: dash_cell(),
                is_empty: true,
                width,
                height,
            };
        }

        let first = points.first().expect("non-empty").balance;
        let last = points.last().expect("non-empty").balance;
        let change = format_signed_money_cell(Some(last - first));

        let mut min = first;
        let mut max = first;
        for p in points {
            let _bucket = p.bucket;
            if p.balance < min {
                min = p.balance;
            }
            if p.balance > max {
                max = p.balance;
            }
        }
        // Always leave a small vertical gutter so flat lines don't sit
        // exactly on the edge of the viewBox.
        let span = (max - min).abs();
        let pad = if span.is_zero() {
            Decimal::ONE
        } else {
            span * Decimal::new(1, 1)
        };
        let y_min = min - pad;
        let y_max = max + pad;
        let y_range = (y_max - y_min).abs();

        let count = points.len();
        // Map index 0..count-1 evenly across 0..width. Guard against
        // a single-point series, which we already short-circuited above.
        let denom = (count - 1) as i64;
        let x_for = |i: usize| -> f64 {
            if denom == 0 {
                width as f64 / 2.0
            } else {
                (i as f64 / denom as f64) * (width as f64)
            }
        };
        let y_for = |balance: Decimal| -> f64 {
            let normalized = if y_range.is_zero() {
                0.5
            } else {
                ((balance - y_min) / y_range)
                    .to_string()
                    .parse::<f64>()
                    .unwrap_or(0.5)
            };
            // Invert (SVG y grows downward) and leave a 1px gutter.
            let clamped = normalized.clamp(0.0, 1.0);
            (1.0 - clamped) * (height as f64 - 1.0) + 0.5
        };

        let mut buf = String::new();
        for (i, p) in points.iter().enumerate() {
            if i > 0 {
                buf.push(' ');
            }
            let x = x_for(i);
            let y = y_for(p.balance);
            buf.push_str(&format!("{x:.2},{y:.2}"));
        }

        Self {
            label,
            polyline: buf,
            change,
            is_empty: false,
            width,
            height,
        }
    }
}

#[derive(Template)]
#[template(path = "agents/fragments/balance-sparklines.html")]
pub struct BalanceSparklinesPartialTemplate {
    pub sparklines: Vec<SparklineView>,
}

impl BalanceSparklinesPartialTemplate {
    pub fn render_view(sparklines: Vec<SparklineView>) -> Result<String, askama::Error> {
        Self { sparklines }.render()
    }
}
