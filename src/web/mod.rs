mod api;
mod error;
mod routes;
mod state;
mod templates;
pub(crate) mod ui_events;

pub use state::AppState;

use std::sync::Arc;

use anyhow::{Context, Result};
use axum::Router;
use tokio::sync::watch;
use tower_http::{services::ServeDir, trace::TraceLayer};
use tracing::Level;

use crate::{
    agentic::backend::AgenticBackend,
    agents::crypto::EncryptionKey,
    cache::asset::AssetCache,
    db::DbPool,
    hyperliquid::live_state::LiveAccountStore,
    model_catalog::models_dev::ModelsDevCatalog,
    opencode::{client::OpenCodeClient, workspace::OpenCodeWorkspaceConfig},
};

use self::ui_events::UiEventHub;

pub async fn serve(
    bind_addr: &str,
    db_pool: DbPool,
    agentic_backend: Arc<dyn AgenticBackend>,
    encryption_key: EncryptionKey,
    live_accounts: Arc<LiveAccountStore>,
    opencode_workspace_config: OpenCodeWorkspaceConfig,
    opencode_client: Arc<OpenCodeClient>,
    model_catalog: Arc<ModelsDevCatalog>,
    asset_cache: Arc<AssetCache>,
    shutdown_rx: watch::Receiver<bool>,
    in_flight: crate::agentic::in_flight::InFlightTracker,
) -> Result<()> {
    let shutdown_rx_for_state = shutdown_rx.clone();
    let state = Arc::new(AppState {
        db_pool,
        #[cfg(test)]
        _test_db_guard: None,
        agentic_backend,
        encryption_key,
        live_accounts,
        ui_events: Arc::new(UiEventHub::new()),
        opencode_workspace_config,
        opencode_client,
        model_catalog,
        asset_cache,
        in_flight,
        shutdown_rx: shutdown_rx_for_state,
    });
    let app = router(state);
    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .with_context(|| format!("failed to bind web server to {bind_addr}"))?;

    let mut shutdown_rx = shutdown_rx;
    let shutdown_signal = async move {
        // Wait for the shared shutdown signal set by `main` (in response
        // to SIGINT/SIGTERM) instead of installing our own signal
        // handler. This keeps the shutdown path single-sourced: a
        // single signal handler in `main` flips the watch, and every
        // long-running task observes the same flag.
        loop {
            if *shutdown_rx.borrow() {
                return;
            }
            if shutdown_rx.changed().await.is_err() {
                // Sender was dropped; treat as shutdown.
                return;
            }
        }
    };

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal)
        .await
        .context("web server terminated unexpectedly")
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
