mod agents;
mod backends;
#[cfg(test)]
mod backends_tests;
mod model_catalog;
mod root;
mod settings;
mod shared;
#[cfg(test)]
pub(in crate::web::routes) mod test_support;

use self::{agents::*, backends::*, model_catalog::*, root::*, settings::*};

use std::sync::Arc;

use axum::{
    Router,
    routing::{get, post},
};

use crate::web::AppState;

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(root))
        .route("/healthz", get(healthz))
        .route("/model-catalog/logos/{provider}", get(model_catalog_logo))
        .route("/agents", get(agents_index).post(create_agent))
        .route("/agents/new", get(agents_new))
        .route("/backends", get(backends_index).post(create_backend))
        .route("/backends/new", get(backends_new))
        .route("/agents/{agent_key}", get(agents_show))
        .route(
            "/agents/{agent_key}/transactions",
            get(agents_show_transactions),
        )
        .route("/agents/{agent_key}/memories", get(agents_show_memories))
        .route(
            "/agents/{agent_key}/memories/stream",
            get(agent_memories_stream),
        )
        .route(
            "/agents/{agent_key}/memories/{memory_id}",
            get(agents_show_memory_detail),
        )
        .route("/agents/{agent_key}/prompts", get(agents_show_prompts))
        .route(
            "/agents/{agent_key}/prompts/analysis",
            post(agents_update_analysis_prompt),
        )
        .route(
            "/agents/{agent_key}/prompts/trading",
            post(agents_update_trading_prompt),
        )
        .route("/agents/{agent_key}/settings", get(agents_show_settings))
        .route(
            "/agents/{agent_key}/settings/instruments",
            post(agents_update_instruments),
        )
        .route(
            "/agents/{agent_key}/settings/regenerate-workspace",
            post(agents_regenerate_workspace),
        )
        .route(
            "/agents/{agent_key}/settings/workspace-maintenance-status",
            get(agents_workspace_maintenance_status),
        )
        .route(
            "/agents/{agent_key}/jobs",
            get(agents_show_jobs).post(agents_create_job),
        )
        .route("/agents/{agent_key}/jobs/new", get(agents_new_job))
        .route(
            "/agents/{agent_key}/jobs/toggle-all",
            post(agents_toggle_all_jobs),
        )
        .route(
            "/agents/{agent_key}/jobs/{job_id}",
            get(agents_show_job_detail),
        )
        .route(
            "/agents/{agent_key}/hooks/{hook_id}",
            get(agents_show_hook_detail),
        )
        .route(
            "/agents/{agent_key}/jobs/{job_id}/toggle",
            post(agents_toggle_job),
        )
        .route(
            "/agents/{agent_key}/jobs/{job_id}/delete",
            post(agents_delete_job),
        )
        .route(
            "/agents/{agent_key}/jobs/{job_id}/run",
            post(agents_run_job_now),
        )
        .route(
            "/agents/{agent_key}/jobs/{job_id}/model",
            post(agents_update_job_model),
        )
        .route(
            "/agents/{agent_key}/jobs/{job_id}/timeout",
            post(agents_update_job_timeout),
        )
        .route("/agents/{agent_key}/hooks/new", get(agents_new_hook))
        .route("/agents/{agent_key}/hooks", post(agents_create_hook))
        .route(
            "/agents/{agent_key}/hooks/{hook_id}/run",
            post(agents_run_hook_now),
        )
        .route(
            "/agents/{agent_key}/hooks/{hook_id}/toggle",
            post(agents_toggle_hook),
        )
        .route(
            "/agents/{agent_key}/hooks/{hook_id}/delete",
            post(agents_delete_hook),
        )
        .route(
            "/agents/{agent_key}/hooks/{hook_id}/model",
            post(agents_update_hook_model),
        )
        .route(
            "/agents/{agent_key}/hooks/{hook_id}/timeout",
            post(agents_update_hook_timeout),
        )
        .route(
            "/agents/{agent_key}/runs/{run_id}",
            get(agents_show_run_detail),
        )
        .route("/agents/{agent_key}/delete", post(delete_agent))
        .route("/agents/{agent_key}/live/stream", get(agent_live_stream))
        .route("/settings", get(settings_index).post(settings_update))
        .with_state(state)
}
