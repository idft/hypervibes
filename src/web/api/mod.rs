mod account;
mod error;
mod memories;
mod orders;

#[cfg(test)]
mod test_support;

#[cfg(test)]
mod account_tests;
#[cfg(test)]
mod memories_tests;
#[cfg(test)]
mod orders_tests;

// Re-export so handlers are reachable by bare name from `router()` below,
// and so test code can reference them via `super::*` if needed.
use self::{account::*, memories::*, orders::*};

use std::sync::Arc;

use axum::{Router, routing::{get, post}};
use crate::web::AppState;

/// Build the `/api/v1` sub-router. Merged into the main router in
/// `src/web/routes.rs`.
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/memories", post(create_memory).get(list_memories))
        .route("/memories/latest", get(list_latest_memories))
        .route("/memories/{id}", get(get_memory_by_id))
        .route("/account", get(get_account))
        .route(
            "/orders",
            post(place_orders_handler).get(list_orders_handler),
        )
        .route("/orders/cancel", post(cancel_orders_handler))
        .route("/orders/cancel-all", post(cancel_all_handler))
        .route("/orders/{id}", get(get_order_handler))
        .with_state(state)
}

/// Wire the `/api/v1` sub-router into a parent router.
pub fn merge(parent: Router, state: Arc<AppState>) -> Router {
    parent.nest("/api/v1", router(state))
}