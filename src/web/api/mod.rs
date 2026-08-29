mod account;
mod coding;
mod error;
mod memories;
mod notifications;
pub(crate) mod orders;
mod strategy_prompts;
mod transactions;

#[cfg(test)]
mod test_support;

#[cfg(test)]
mod account_tests;
#[cfg(test)]
mod coding_tests;
#[cfg(test)]
mod memories_tests;
#[cfg(test)]
mod notifications_tests;
#[cfg(test)]
mod orders_tests;
#[cfg(test)]
mod strategy_prompts_tests;
#[cfg(test)]
mod transactions_tests;

// Re-export so handlers are reachable by bare name from `router()` below,
// and so test code can reference them via `super::*` if needed.
use self::notifications::create as create_notification;
use self::{account::*, coding::*, memories::*, orders::*, strategy_prompts::*, transactions::*};

use std::sync::Arc;

use crate::web::AppState;
use crate::{agents::AuthenticatedAgent, harness::model::RunApiScope};
use axum::{
    Router,
    routing::{get, post},
};

/// Build the `/api/v1` sub-router. Merged into the main router in
/// `src/web/routes.rs`.
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/memories", post(create_memory).get(list_memories))
        .route("/memories/latest", get(list_latest_memories))
        .route("/memories/{id}", get(get_memory_by_id))
        .route("/notifications", post(create_notification))
        .route("/account", get(get_account))
        .route("/account/transactions", get(list_account_transactions))
        .route("/coding/requests", post(request_analysis_coding))
        .route("/coding/report", post(submit_coding_report))
        .route("/strategy-prompts", get(list_strategy_prompts))
        .route(
            "/strategy-prompts/{prompt_kind}",
            get(get_strategy_prompt).put(update_strategy_prompt),
        )
        .route(
            "/orders",
            post(place_orders_handler).get(list_orders_handler),
        )
        .route("/orders/cancel", post(cancel_orders_handler))
        .route("/orders/cancel-all", post(cancel_all_handler))
        .route("/orders/{id}", get(get_order_handler))
        .with_state(state)
}

pub(in crate::web::api) fn require_run_api_scope(
    agent: &AuthenticatedAgent,
    scope: RunApiScope,
) -> Result<(), error::ApiError> {
    if agent.permits(scope) {
        Ok(())
    } else {
        Err(error::ApiError::Forbidden(
            "run credential does not permit this API action",
        ))
    }
}

pub(in crate::web::api) fn require_permanent_agent_credential(
    agent: &AuthenticatedAgent,
) -> Result<(), error::ApiError> {
    if agent.is_run_credential() {
        Err(error::ApiError::Forbidden(
            "run credentials do not permit this API action",
        ))
    } else {
        Ok(())
    }
}

/// Wire the `/api/v1` sub-router into a parent router.
pub fn merge(parent: Router, state: Arc<AppState>) -> Router {
    parent.nest("/api/v1", router(state))
}
