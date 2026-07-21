mod api;
pub(crate) mod auth;
mod error;
mod routes;
mod state;
pub(crate) mod templates;
pub(crate) mod ui_events;

pub use state::AppState;

use std::sync::Arc;

use anyhow::{Context, Result};
use axum::Router;
use tokio::sync::watch;
use tower_http::{services::ServeDir, trace::TraceLayer};
use tracing::Level;

use crate::{
    agentic::{
        backend::AgenticBackend,
        in_flight::{InFlightTracker, SHUTDOWN_IN_FLIGHT_GRACE},
        workspace_lease::WorkspaceLeaseManager,
    },
    agents::crypto::EncryptionKey,
    cache::asset::AssetCache,
    db::DbPool,
    hyperliquid::live_state::LiveAccountStore,
    model_catalog::models_dev::ModelsDevCatalog,
    opencode::{client::OpenCodeClient, workspace::OpenCodeWorkspaceConfig},
};

use self::ui_events::UiEventHub;

#[allow(clippy::too_many_arguments)]
pub async fn serve(
    bind_addr: &str,
    db_pool: DbPool,
    agentic_backend: Arc<dyn AgenticBackend>,
    encryption_key: EncryptionKey,
    live_accounts: Arc<LiveAccountStore>,
    opencode_workspace_config: OpenCodeWorkspaceConfig,
    opencode_base_url: String,
    opencode_client: Arc<OpenCodeClient>,
    model_catalog: Arc<ModelsDevCatalog>,
    asset_cache: Arc<AssetCache>,
    shutdown_rx: watch::Receiver<bool>,
    force_shutdown_rx: watch::Receiver<bool>,
    in_flight: InFlightTracker,
    workspace_leases: WorkspaceLeaseManager,
) -> Result<()> {
    let shutdown_rx_for_state = shutdown_rx.clone();
    let in_flight_for_state = in_flight.clone();
    let state = Arc::new(AppState {
        db_pool,
        #[cfg(test)]
        _test_db_guard: None,
        agentic_backend,
        encryption_key,
        live_accounts,
        ui_events: Arc::new(UiEventHub::new()),
        opencode_workspace_config,
        opencode_base_url,
        opencode_client,
        model_catalog,
        asset_cache,
        in_flight: in_flight_for_state,
        workspace_leases,
        shutdown_rx: shutdown_rx_for_state,
    });
    let app = router(state);
    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .with_context(|| format!("failed to bind web server to {bind_addr}"))?;

    let shutdown_signal = shutdown_signal_future(shutdown_rx, force_shutdown_rx, in_flight);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal)
        .await
        .context("web server terminated unexpectedly")
}

/// Drive the web server's `with_graceful_shutdown` future. After the
/// first shutdown signal, the future waits for the shared
/// `InFlightTracker` to drain (or hit `grace`) before returning,
/// because the agent's MCP server makes HTTP calls back into this
/// API during a dispatch. A second shutdown signal flips a separate
/// `force` watch that short-circuits the wait so the API is cut
/// immediately and in-flight dispatches fail fast on their next MCP
/// call.
async fn shutdown_signal_future_with_grace(
    mut shutdown_rx: watch::Receiver<bool>,
    force_shutdown_rx: watch::Receiver<bool>,
    in_flight: InFlightTracker,
    grace: std::time::Duration,
) {
    loop {
        if *shutdown_rx.borrow() {
            break;
        }
        if shutdown_rx.changed().await.is_err() {
            // Sender was dropped; treat as shutdown.
            return;
        }
    }

    let in_flight_count = in_flight.in_flight();
    if in_flight_count > 0 {
        tracing::info!(
            in_flight = in_flight_count,
            grace_seconds = grace.as_secs(),
            "web server holding for in-flight agentic dispatches to complete"
        );
    }

    let in_flight_for_wait = in_flight.clone();
    let mut force_for_wait = force_shutdown_rx.clone();
    tokio::select! {
        _ = async {
            let drained = in_flight_for_wait
                .wait_idle_with_timeout(grace)
                .await;
            if drained {
                tracing::info!("web server proceeding with graceful shutdown (in-flight agentic dispatches drained)");
            } else {
                tracing::warn!(
                    remaining = in_flight_for_wait.in_flight(),
                    "web server proceeding with graceful shutdown; in-flight agentic dispatches did not drain within grace period"
                );
            }
        } => {}
        _ = async {
            loop {
                if *force_for_wait.borrow() { break; }
                if force_for_wait.changed().await.is_err() { return; }
            }
            tracing::warn!("web server force-shutdown: cutting API to in-flight agentic dispatches");
        } => {}
    }
}

fn shutdown_signal_future(
    shutdown_rx: watch::Receiver<bool>,
    force_shutdown_rx: watch::Receiver<bool>,
    in_flight: InFlightTracker,
) -> impl std::future::Future<Output = ()> {
    shutdown_signal_future_with_grace(
        shutdown_rx,
        force_shutdown_rx,
        in_flight,
        SHUTDOWN_IN_FLIGHT_GRACE,
    )
}

fn router(state: Arc<AppState>) -> Router {
    let app = api::merge(routes::router(Arc::clone(&state)), Arc::clone(&state));
    app.nest_service("/static", ServeDir::new("static")).layer(
        TraceLayer::new_for_http()
            .make_span_with(tower_http::trace::DefaultMakeSpan::new().level(Level::INFO))
            .on_response(tower_http::trace::DefaultOnResponse::new().level(Level::INFO))
            .on_failure(tower_http::trace::DefaultOnFailure::new().level(Level::ERROR)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Short grace used by these tests so the timeout path runs
    /// quickly. The real `SHUTDOWN_IN_FLIGHT_GRACE` is 30 minutes.
    const TEST_GRACE: Duration = Duration::from_millis(200);

    async fn run_with_grace(
        shutdown_rx: watch::Receiver<bool>,
        force_shutdown_rx: watch::Receiver<bool>,
        in_flight: InFlightTracker,
    ) {
        shutdown_signal_future_with_grace(shutdown_rx, force_shutdown_rx, in_flight, TEST_GRACE)
            .await;
    }

    #[tokio::test]
    async fn shutdown_signal_waits_for_shutdown_rx_forever() {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let (_force_tx, force_rx) = watch::channel(false);
        let in_flight = InFlightTracker::new();
        let handle = tokio::spawn(run_with_grace(shutdown_rx, force_rx, in_flight));
        // The future must not resolve while the shutdown watch is unset.
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            !handle.is_finished(),
            "future should still be waiting for the shutdown watch"
        );
        // Cleanup: drop the sender so the spawned task returns.
        drop(shutdown_tx);
    }

    #[tokio::test]
    async fn shutdown_signal_resolves_quickly_when_in_flight_empty() {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let (_force_tx, force_rx) = watch::channel(false);
        let in_flight = InFlightTracker::new();
        let started = std::time::Instant::now();
        let handle = tokio::spawn(run_with_grace(shutdown_rx, force_rx, in_flight));
        // Give the future a moment to enter the outer watch loop.
        tokio::time::sleep(Duration::from_millis(20)).await;
        shutdown_tx.send(true).expect("send shutdown");
        tokio::time::timeout(Duration::from_secs(1), handle)
            .await
            .expect("future should resolve after shutdown")
            .expect("join");
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "shutdown with no in-flight work should return promptly; took {:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn shutdown_signal_holds_for_in_flight_guard() {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let (_force_tx, force_rx) = watch::channel(false);
        let in_flight = InFlightTracker::new();
        let guard = in_flight.track();
        let handle = tokio::spawn(run_with_grace(shutdown_rx, force_rx, in_flight.clone()));
        tokio::time::sleep(Duration::from_millis(20)).await;
        shutdown_tx.send(true).expect("send shutdown");
        // The future must not resolve while the guard is held.
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            !handle.is_finished(),
            "future should still be waiting for in_flight drain"
        );
        drop(guard);
        tokio::time::timeout(Duration::from_secs(1), handle)
            .await
            .expect("future should resolve after guard drop")
            .expect("join");
    }

    #[tokio::test]
    async fn shutdown_signal_resolves_via_grace_timeout_when_guard_persists() {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let (_force_tx, force_rx) = watch::channel(false);
        let in_flight = InFlightTracker::new();
        let _guard = in_flight.track();
        let started = std::time::Instant::now();
        let handle = tokio::spawn(run_with_grace(shutdown_rx, force_rx, in_flight));
        tokio::time::sleep(Duration::from_millis(20)).await;
        shutdown_tx.send(true).expect("send shutdown");
        tokio::time::timeout(Duration::from_secs(1), handle)
            .await
            .expect("future should resolve after grace elapses")
            .expect("join");
        // Confirm we waited at least roughly the grace period.
        assert!(
            started.elapsed() >= TEST_GRACE,
            "future should wait at least the grace period; took {:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn shutdown_signal_resolves_via_force_immediately_even_with_guard_held() {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let (force_tx, force_rx) = watch::channel(false);
        let in_flight = InFlightTracker::new();
        let _guard = in_flight.track();
        let started = std::time::Instant::now();
        let handle = tokio::spawn(run_with_grace(shutdown_rx, force_rx, in_flight));
        tokio::time::sleep(Duration::from_millis(20)).await;
        shutdown_tx.send(true).expect("send shutdown");
        // Force fires *before* the grace expires. The future must
        // resolve promptly without waiting for the held guard.
        tokio::time::sleep(Duration::from_millis(20)).await;
        force_tx.send(true).expect("send force");
        tokio::time::timeout(Duration::from_millis(500), handle)
            .await
            .expect("future should resolve via force before grace expires")
            .expect("join");
        assert!(
            started.elapsed() < TEST_GRACE,
            "force path should bypass grace; took {:?} (grace={:?})",
            started.elapsed(),
            TEST_GRACE
        );
    }
}
