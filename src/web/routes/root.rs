use crate::web::AppState;
use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Redirect},
};
use serde::Serialize;
use std::sync::Arc;
pub(in crate::web::routes) async fn root() -> Redirect {
    Redirect::to("/agents")
}
#[derive(Debug, Serialize)]
pub(in crate::web::routes) struct HealthResponse {
    pub status: &'static str,
}
pub(in crate::web::routes) async fn healthz(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let _ = &state.db_pool;
    (StatusCode::OK, Json(HealthResponse { status: "ok" }))
}
