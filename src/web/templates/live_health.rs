use askama::Template;

use crate::hyperliquid::live_state::{
    AccountLiveState, LiveAccountHealthStatus, LiveConnectionStatus, account_live_health,
};

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
