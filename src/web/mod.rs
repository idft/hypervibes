mod routes;
mod templates;

use std::sync::Arc;

use anyhow::{Context, Result};
use axum::Router;
use tower_http::{services::ServeDir, trace::TraceLayer};

use crate::{agents::crypto::EncryptionKey, db::DbPool};

#[derive(Clone)]
pub struct AppState {
    pub db_pool: DbPool,
    pub encryption_key: EncryptionKey,
}

pub async fn serve(bind_addr: &str, db_pool: DbPool, encryption_key: EncryptionKey) -> Result<()> {
    let state = Arc::new(AppState {
        db_pool,
        encryption_key,
    });
    let app = router(state);
    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .with_context(|| format!("failed to bind web server to {bind_addr}"))?;

    axum::serve(listener, app)
        .await
        .context("web server terminated unexpectedly")
}

fn router(state: Arc<AppState>) -> Router {
    routes::router(state)
        .nest_service("/static", ServeDir::new("static"))
        .layer(TraceLayer::new_for_http())
}
