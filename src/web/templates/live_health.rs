use askama::Template;
use chrono::{DateTime, Utc};

use crate::hyperliquid::live_state::{
    ACCOUNT_DATA_MAX_AGE, AccountLiveState, LiveAccountHealthStatus, LiveConnectionStatus,
    account_live_health, account_live_health_at,
};

const RECONNECT_DISPLAY_GRACE: chrono::Duration = chrono::Duration::seconds(30);

/// Presentation-only tolerance for brief interruptions. Trading and API
/// consumers continue to use the strict live-account authority checks.
pub(super) struct LiveAccountDisplayState {
    pub positions_available: bool,
    pub orders_available: bool,
    pub balance_available: bool,
    pub showing_last_known_data: bool,
}

impl LiveAccountDisplayState {
    pub fn from_live_state(state: &AccountLiveState) -> Self {
        Self::from_live_state_at(state, Utc::now())
    }

    fn from_live_state_at(state: &AccountLiveState, now: DateTime<Utc>) -> Self {
        let health = account_live_health_at(state, now);
        let reconnecting = matches!(
            state.status,
            LiveConnectionStatus::Reconnecting
                | LiveConnectionStatus::Disconnected
                | LiveConnectionStatus::Connecting
                | LiveConnectionStatus::StartupSyncing
        ) && state
            .connection_interrupted_at
            .is_some_and(|interrupted_at| now - interrupted_at < RECONNECT_DISPLAY_GRACE);
        let recent_snapshot = |updated_at: Option<DateTime<Utc>>| {
            reconnecting
                && updated_at.is_some_and(|updated_at| now - updated_at <= ACCOUNT_DATA_MAX_AGE)
        };
        let positions_available =
            health.positions.is_current() || recent_snapshot(state.clearinghouse_updated_at);
        let orders_available =
            health.open_orders.is_current() || recent_snapshot(state.open_orders_updated_at);
        let balance_available = health.balance.is_current()
            || (recent_snapshot(state.clearinghouse_updated_at)
                && recent_snapshot(state.spot_updated_at));
        Self {
            positions_available,
            orders_available,
            balance_available,
            showing_last_known_data: reconnecting
                && (positions_available || orders_available || balance_available),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LiveAccountHealthView {
    pub status_label: &'static str,
    pub status_dot_class: &'static str,
    pub is_healthy: bool,
    pub description: Option<String>,
    pub last_error: Option<String>,
}

impl LiveAccountHealthView {
    pub fn from_live_state(state: &AccountLiveState) -> Self {
        let health = account_live_health(state);
        if LiveAccountDisplayState::from_live_state(state).showing_last_known_data {
            return Self {
                status_label: "Reconnecting",
                status_dot_class: "bg-amber-400",
                is_healthy: false,
                description: Some(
                    "Showing last known data while Hyperliquid reconnects.".to_string(),
                ),
                last_error: None,
            };
        }
        let (status_label, status_dot_class, description) = match health.status {
            LiveAccountHealthStatus::Healthy => (
                "Connected",
                "bg-emerald-400",
                None,
            ),
            LiveAccountHealthStatus::Loading => (
                "Connecting",
                "bg-zinc-500",
                Some("Waiting for the initial exchange account snapshots.".to_string()),
            ),
            LiveAccountHealthStatus::Degraded => (
                "Reconnecting",
                "bg-amber-400",
                Some(
                    "Recent account data is not authoritative while the exchange connection reconnects. New exposure is blocked."
                        .to_string(),
                ),
            ),
            LiveAccountHealthStatus::Stale => (
                "Data stale",
                "bg-amber-400",
                Some(
                    "The exchange has not supplied a current account snapshot. New exposure is blocked."
                        .to_string(),
                ),
            ),
            LiveAccountHealthStatus::Failed => (
                "Disconnected",
                "bg-red-400",
                Some("Live exchange monitoring failed and is retrying. New exposure is blocked.".to_string()),
            ),
            LiveAccountHealthStatus::Stopped => (
                "Monitoring stopped",
                "bg-zinc-500",
                None,
            ),
        };
        let connection = match health.connection_status {
            LiveConnectionStatus::Starting => "starting",
            LiveConnectionStatus::StartupSyncing => "synchronizing",
            LiveConnectionStatus::Connecting => "connecting",
            LiveConnectionStatus::Connected => "connected",
            LiveConnectionStatus::Reconnecting => "reconnecting",
            LiveConnectionStatus::Disconnected => "disconnected",
            LiveConnectionStatus::Failed => "failed",
            LiveConnectionStatus::Stopped => "stopped",
        };

        Self {
            status_label,
            status_dot_class,
            is_healthy: health.status == LiveAccountHealthStatus::Healthy,
            description: description
                .map(|description| format!("{description} Connection: {connection}.")),
            last_error: health.last_error,
        }
    }
}

#[derive(Template)]
#[template(path = "agents/fragments/live-account-health.html")]
pub struct LiveAccountHealthPartialTemplate {
    pub view: LiveAccountHealthView,
}

impl LiveAccountHealthPartialTemplate {
    pub fn render_view(view: LiveAccountHealthView) -> Result<String, askama::Error> {
        Self { view }.render()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hyperliquid::live_state::{
        LiveDataStatus, LiveMarginState, LiveOpenOrder, LivePosition,
    };
    use crate::web::templates::{
        AccountBalancePartialTemplate, AccountBalanceView, OpenOrdersPartialTemplate,
        OpenOrdersView, OpenPositionsPartialTemplate, OpenPositionsView,
    };
    use rust_decimal::Decimal;

    fn reconnecting_state(now: DateTime<Utc>) -> AccountLiveState {
        AccountLiveState {
            status: LiveConnectionStatus::Reconnecting,
            connection_interrupted_at: Some(now - chrono::Duration::seconds(2)),
            clearinghouse_updated_at: Some(now - chrono::Duration::minutes(1)),
            open_orders_updated_at: Some(now - chrono::Duration::minutes(1)),
            spot_updated_at: Some(now - chrono::Duration::minutes(1)),
            margin: Some(LiveMarginState {
                account_value: Some(Decimal::new(1000, 0)),
                ..Default::default()
            }),
            open_positions: vec![LivePosition {
                coin: "BTC".to_string(),
                szi: Some(Decimal::ONE),
                unrealized_pnl: Some(Decimal::new(125, 0)),
                ..Default::default()
            }],
            open_orders: vec![LiveOpenOrder {
                coin: "ETH".to_string(),
                oid: Some("42".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn brief_reconnect_preserves_rendered_data_with_a_compact_warning() {
        let now = Utc::now();
        for status in [
            LiveConnectionStatus::Reconnecting,
            LiveConnectionStatus::Disconnected,
            LiveConnectionStatus::Connecting,
            LiveConnectionStatus::StartupSyncing,
        ] {
            let mut state = reconnecting_state(now);
            state.status = status;
            let positions = OpenPositionsView::from_live_state(state.clone());
            assert_eq!(positions.positions.len(), 1);
            assert!(positions.unavailable_message.is_none());
            let html =
                OpenPositionsPartialTemplate::render_view(positions).expect("positions render");
            assert!(html.contains("BTC"));
            let orders = OpenOrdersView::from_live_state(state.clone());
            assert_eq!(orders.orders.len(), 1);
            assert!(orders.unavailable_message.is_none());
            let html = OpenOrdersPartialTemplate::render_view(orders).expect("orders render");
            assert!(html.contains("ETH"));
            let balance = AccountBalanceView::from_live_state(state.clone());
            assert_eq!(balance.total_balance, Some(Decimal::new(1000, 0)));
            assert!(balance.unrealized_pnl().is_some());
            let html =
                AccountBalancePartialTemplate::render_view(balance).expect("balance renders");
            assert!(html.contains("data-animate-key=\"total-upnl\""));
            let view = LiveAccountHealthView::from_live_state(&state);
            assert_eq!(view.status_label, "Reconnecting");
            let html = LiveAccountHealthPartialTemplate::render_view(view).expect("health renders");
            assert!(html.contains("Showing last known data"));

            // UI grace must not grant trading authority to the cached data.
            let health = account_live_health_at(&state, now);
            assert!(!health.positions.is_current());
            assert!(!health.open_orders.is_current());
            assert!(!health.balance.is_current());
        }
    }

    #[test]
    fn display_grace_expires_without_another_exchange_message() {
        let now = Utc::now();
        let mut state = reconnecting_state(now);
        state.connection_interrupted_at = Some(now);
        let display = LiveAccountDisplayState::from_live_state_at(
            &state,
            now + RECONNECT_DISPLAY_GRACE - chrono::Duration::milliseconds(1),
        );
        assert!(display.positions_available);
        assert!(display.orders_available);
        assert!(display.balance_available);
        let display =
            LiveAccountDisplayState::from_live_state_at(&state, now + RECONNECT_DISPLAY_GRACE);
        assert!(!display.positions_available);
        assert!(!display.orders_available);
        assert!(!display.balance_available);
        assert!(!display.showing_last_known_data);

        state.connection_interrupted_at = Some(now - RECONNECT_DISPLAY_GRACE);
        assert!(
            OpenPositionsView::from_live_state(state.clone())
                .unavailable_message
                .is_some()
        );
        assert!(
            OpenOrdersView::from_live_state(state.clone())
                .unavailable_message
                .is_some()
        );
        assert!(
            AccountBalanceView::from_live_state(state.clone())
                .total()
                .is_none()
        );
        let view = LiveAccountHealthView::from_live_state(&state);
        assert!(
            !view
                .description
                .expect("outage description")
                .contains("Showing last known data")
        );
    }

    #[test]
    fn grace_requires_recent_snapshots_and_a_recoverable_connection() {
        let now = Utc::now();
        let mut state = reconnecting_state(now);
        state.open_orders_updated_at = None;
        state.spot_updated_at = Some(now - ACCOUNT_DATA_MAX_AGE - chrono::Duration::seconds(1));
        let display = LiveAccountDisplayState::from_live_state_at(&state, now);
        assert!(display.positions_available);
        assert!(!display.orders_available);
        assert!(!display.balance_available);

        state.clearinghouse_updated_at = state.spot_updated_at;
        let display = LiveAccountDisplayState::from_live_state_at(&state, now);
        assert!(!display.positions_available);
        assert!(!display.showing_last_known_data);

        for status in [LiveConnectionStatus::Failed, LiveConnectionStatus::Stopped] {
            let mut state = reconnecting_state(now);
            state.status = status;
            let display = LiveAccountDisplayState::from_live_state_at(&state, now);
            assert!(!display.positions_available);
            assert!(!display.orders_available);
            assert!(!display.balance_available);
        }

        let mut state = reconnecting_state(now);
        state.connection_interrupted_at = None;
        let display = LiveAccountDisplayState::from_live_state_at(&state, now);
        assert!(!display.showing_last_known_data);
        assert!(!display.positions_available);
        state.status = LiveConnectionStatus::Connected;
        let display = LiveAccountDisplayState::from_live_state_at(&state, now);
        assert!(display.positions_available);
        assert!(!display.showing_last_known_data);
        assert_eq!(
            account_live_health_at(&state, now).positions,
            LiveDataStatus::Current
        );
        assert!(LiveAccountHealthView::from_live_state(&state).is_healthy);
    }
}
