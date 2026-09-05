mod account;
mod agents;
mod currency;
mod model_catalog;
mod providers;
mod root;
mod shared;
#[cfg(test)]
pub(in crate::web::routes) mod test_support;

use self::{account::*, agents::*, currency::*, model_catalog::*, providers::*, root::*};
use crate::web::auth::{login, login_challenge, login_verify, logout};
use crate::web::error::not_found;

use std::sync::Arc;

use axum::middleware;
use axum::{
    Router,
    response::Redirect,
    routing::{get, post},
};

use crate::web::AppState;

pub fn router(state: Arc<AppState>) -> Router {
    let public = Router::new()
        .route("/healthz", get(healthz))
        .route(
            "/favicon.ico",
            get(|| async { Redirect::permanent("/static/favicon.ico") }),
        )
        .route("/login", get(login))
        .route("/auth/challenge", post(login_challenge))
        .route("/auth/verify", post(login_verify))
        .route("/auth/logout", post(logout))
        .with_state(Arc::clone(&state));

    let protected = Router::new()
        .route("/", get(root))
        .route("/model-catalog/logos/{provider}", get(model_catalog_logo))
        .route("/currency/{file}", get(currency_logo))
        .route("/agents", get(agents_index).post(create_agent))
        .route("/agents/new", get(agents_new))
        .route("/agents/new/account-choices", get(agent_account_choices))
        .route("/account/subaccounts", post(create_user_subaccount))
        .route("/agents/{agent_key}", get(agents_show))
        .route(
            "/agents/{agent_key}/notifications",
            get(agents_show_notifications).post(agents_delete_notifications),
        )
        .route(
            "/agents/{agent_key}/notifications/count/stream",
            get(agent_notification_count_stream),
        )
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
            "/agents/{agent_key}/memories/timeline",
            get(agent_memory_timeline_page),
        )
        .route(
            "/agents/{agent_key}/memories/{memory_id}",
            get(agents_show_memory_detail),
        )
        .route(
            "/agents/{agent_key}/memories/{memory_id}/delete",
            post(agents_delete_memory),
        )
        .route(
            "/agents/{agent_key}/sub-agents/recent-runs/stream",
            get(agent_sub_agent_recent_runs_stream),
        )
        .route(
            "/agents/{agent_key}/analysis",
            get(agents_show_analysis).post(agents_create_analysis_job),
        )
        .route(
            "/agents/{agent_key}/analysis/new",
            get(agents_new_analysis_job),
        )
        .route(
            "/agents/{agent_key}/trading",
            get(agents_show_trading).post(agents_create_trading_singleton),
        )
        .route(
            "/agents/{agent_key}/trading/new",
            get(agents_new_trading_singleton),
        )
        .route(
            "/agents/{agent_key}/trading/edit",
            get(agents_edit_trading_singleton),
        )
        .route(
            "/agents/{agent_key}/review",
            get(agents_show_review).post(agents_create_review_singleton),
        )
        .route(
            "/agents/{agent_key}/review/new",
            get(agents_new_review_singleton),
        )
        .route(
            "/agents/{agent_key}/review/edit",
            get(agents_edit_review_singleton),
        )
        .route("/agents/{agent_key}/settings", get(agents_show_settings))
        .route(
            "/agents/{agent_key}/settings/gateway/telegram/status",
            get(telegram_gateway_status),
        )
        .route(
            "/agents/{agent_key}/settings/toggle-enabled",
            post(agents_set_enabled),
        )
        .route(
            "/agents/{agent_key}/emergency-stop",
            post(agents_emergency_stop),
        )
        .route(
            "/agents/{agent_key}/positions/{symbol}/close",
            post(agents_close_position),
        )
        .route(
            "/agents/{agent_key}/positions/close-all",
            post(agents_close_all_positions),
        )
        .route("/agents/{agent_key}/chat", get(agents_show_chat))
        .route("/agents/{agent_key}/chat/new", get(agents_new_chat))
        .route(
            "/agents/{agent_key}/chat/conversations",
            post(agents_create_conversation),
        )
        .route(
            "/agents/{agent_key}/chat/{conversation_id}",
            get(agents_show_chat_detail),
        )
        .route(
            "/agents/{agent_key}/chat/{conversation_id}/stream",
            get(agent_conversation_stream),
        )
        .route(
            "/agents/{agent_key}/chat/{conversation_id}/messages",
            post(agents_send_conversation_message),
        )
        .route(
            "/agents/{agent_key}/chat/{conversation_id}/stop",
            post(agents_stop_conversation),
        )
        .route(
            "/agents/{agent_key}/chat/{conversation_id}/compact",
            post(agents_compact_conversation),
        )
        .route(
            "/agents/{agent_key}/chat/{conversation_id}/delete",
            post(agents_delete_conversation),
        )
        .route(
            "/agents/{agent_key}/chat/{conversation_id}/settings",
            post(agents_update_conversation_settings),
        )
        .route(
            "/agents/{agent_key}/chat/{conversation_id}/permissions/{request_id}/reply",
            post(agents_reply_to_conversation_permission),
        )
        .route(
            "/agents/{agent_key}/settings/instruments",
            post(agents_update_instruments),
        )
        .route(
            "/agents/{agent_key}/settings/reset-memories",
            post(agents_reset_memories),
        )
        .route(
            "/agents/{agent_key}/settings/gateway/telegram/token",
            post(add_telegram_token),
        )
        .route(
            "/agents/{agent_key}/settings/gateway/telegram/link",
            post(start_telegram_link),
        )
        .route(
            "/agents/{agent_key}/settings/gateway/telegram/link/{token}/confirm",
            post(confirm_telegram_link),
        )
        .route(
            "/agents/{agent_key}/settings/gateway/telegram/link/{token}/reject",
            post(reject_telegram_link),
        )
        .route(
            "/agents/{agent_key}/settings/gateway/telegram/disconnect",
            post(disconnect_telegram_gateway),
        )
        .route(
            "/agents/{agent_key}/sub-agents/toggle-all",
            post(agents_toggle_all_sub_agents),
        )
        .route(
            "/agents/{agent_key}/sub-agents/{sub_agent_id}",
            get(agents_show_sub_agent_detail),
        )
        .route(
            "/agents/{agent_key}/sub-agents/{sub_agent_id}/model-picker",
            get(agents_sub_agent_model_picker),
        )
        .route(
            "/agents/{agent_key}/sub-agents/{sub_agent_id}/toggle",
            post(agents_toggle_sub_agent),
        )
        .route(
            "/agents/{agent_key}/sub-agents/{sub_agent_id}/delete",
            post(agents_delete_sub_agent),
        )
        .route(
            "/agents/{agent_key}/sub-agents/{sub_agent_id}/run",
            post(agents_run_sub_agent_now),
        )
        .route(
            "/agents/{agent_key}/sub-agents/{sub_agent_id}/model",
            post(agents_update_sub_agent_model),
        )
        .route(
            "/agents/{agent_key}/sub-agents/{sub_agent_id}/timeout",
            post(agents_update_sub_agent_timeout),
        )
        .route(
            "/agents/{agent_key}/sub-agents/{sub_agent_id}/notification-capability",
            post(agents_update_sub_agent_notification_capability),
        )
        .route(
            "/agents/{agent_key}/sub-agents/{sub_agent_id}/timeframe",
            post(agents_update_sub_agent_timeframe),
        )
        .route(
            "/agents/{agent_key}/sub-agents/{sub_agent_id}/prompt",
            post(agents_update_sub_agent_prompt),
        )
        .route(
            "/agents/{agent_key}/sub-agents/{sub_agent_id}/prompt/rollback",
            post(agents_rollback_sub_agent_prompt),
        )
        .route(
            "/agents/{agent_key}/sub-agents/{sub_agent_id}/review-prompt-update",
            post(agents_update_sub_agent_review_prompt_update),
        )
        .route(
            "/agents/{agent_key}/runs/{run_id}",
            get(agents_show_run_detail),
        )
        .route(
            "/agents/{agent_key}/runs/{run_id}/cancel",
            post(agents_cancel_run),
        )
        .route(
            "/agents/{agent_key}/runs/{run_id}/retry",
            post(agents_retry_run),
        )
        .route(
            "/agents/{agent_key}/runs/{run_id}/stream",
            get(agent_run_detail_stream),
        )
        .route("/agents/{agent_key}/delete", post(delete_agent))
        .route("/agents/{agent_key}/live/stream", get(agent_live_stream))
        .route("/account", get(account_index))
        .route("/providers", get(providers_index))
        .route(
            "/providers/{provider_id}/connect",
            get(provider_connect_form).post(provider_connect),
        )
        .route(
            "/providers/{provider_id}/connect/pending",
            get(provider_pending),
        )
        .route(
            "/providers/{provider_id}/connect/callback",
            post(provider_callback),
        )
        .route(
            "/providers/{provider_id}/connect/cancel",
            post(provider_cancel),
        )
        .route(
            "/providers/{provider_id}/disconnect",
            post(provider_disconnect),
        )
        .route("/providers/reload", post(provider_reload_config))
        .route("/providers/reload-status", get(provider_reload_status))
        .route("/account/approve-builder-fee", post(approve_builder_fee))
        .route("/account/cancel-builder-fee", post(cancel_builder_fee))
        .route("/account/api-wallet", post(setup_user_api_wallet))
        .route("/account/approve-api-wallet", post(approve_user_api_wallet))
        .route("/account/transfers", post(transfer_between_accounts))
        .route(
            "/account/referral/signing-payload",
            post(referral_signing_payload),
        )
        .route("/account/referral/claim", post(claim_referral_discount))
        .with_state(Arc::clone(&state))
        .layer(middleware::from_fn_with_state(
            state,
            crate::web::auth::require_operator,
        ));

    public.merge(protected).fallback(not_found)
}
