mod api;
mod routes;
mod templates;
pub(crate) mod ui_events;

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

#[derive(Clone)]
pub struct AppState {
    pub db_pool: DbPool,
    #[cfg(test)]
    pub _test_db_guard: Option<Arc<crate::test_db::TestDb>>,
    pub agentic_backend: Arc<dyn AgenticBackend>,
    pub encryption_key: EncryptionKey,
    pub live_accounts: Arc<LiveAccountStore>,
    pub ui_events: Arc<UiEventHub>,
    pub opencode_workspace_config: OpenCodeWorkspaceConfig,
    pub opencode_client: Arc<OpenCodeClient>,
    pub model_catalog: Arc<ModelsDevCatalog>,
    pub asset_cache: Arc<AssetCache>,
}

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
    shutdown_tx: watch::Sender<bool>,
) -> Result<()> {
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
    });
    let app = router(state);
    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .with_context(|| format!("failed to bind web server to {bind_addr}"))?;

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            tokio::signal::ctrl_c().await.ok();
            let _ = shutdown_tx.send(true);
        })
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
