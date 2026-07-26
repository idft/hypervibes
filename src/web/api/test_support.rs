use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::Utc;
use uuid::Uuid;

use crate::{
    agentic::backend::{AgenticBackend, DispatchRequest, DispatchResult},
    agents::{
        crypto::EncryptionKey,
        keys::derive_wallet_address,
        model::AgentRegistryRow,
        store::{insert_agent, replace_agent_instruments},
        strategy_prompts::{
            PROMPT_KIND_ANALYSIS, PROMPT_KIND_TRADING, insert_default_strategy_prompts_for_agent,
            upsert_agent_strategy_prompt,
        },
    },
    hyperliquid::builder_fee::{BuilderFeeCache, BuilderFeeLookup, LookupFuture},
    test_db,
    web::{AppState, api, run_detail_events::RunDetailEventHub, ui_events::UiEventHub},
};

pub struct NoopAgenticBackend;

struct TestBuilderFeeLookup;

impl BuilderFeeLookup for TestBuilderFeeLookup {
    fn max_builder_fee<'a>(&'a self, _user: &'a str, _builder: &'a str) -> LookupFuture<'a> {
        Box::pin(async { Ok(10) })
    }
}

#[async_trait]
impl AgenticBackend for NoopAgenticBackend {
    async fn dispatch(&self, _request: DispatchRequest) -> Result<DispatchResult> {
        Ok(DispatchResult {
            backend_run_ref: "ses_test".to_string(),
        })
    }
}

pub async fn test_state() -> Arc<AppState> {
    let pool = Arc::new(test_db::pool().await);
    let cache_dir = std::path::PathBuf::from("/tmp/opencode/vibetrading-api-cache");
    let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    Arc::new(AppState {
        db_pool: pool.as_ref().as_ref().clone(),
        _test_db_guard: Some(Arc::clone(&pool)),
        agentic_backend: Arc::new(NoopAgenticBackend),
        encryption_key: EncryptionKey::new(
            "test",
            [
                0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22,
                23, 24, 25, 26, 27, 28, 29, 30, 31,
            ],
        ),
        live_accounts: Arc::new(crate::hyperliquid::live_state::LiveAccountStore::new()),
        ui_events: Arc::new(UiEventHub::new()),
        run_detail_events: Arc::new(RunDetailEventHub::new()),
        opencode_workspace_config: crate::opencode::workspace::OpenCodeWorkspaceConfig {
            source_root: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join(crate::opencode::workspace::PROFILE_SOURCE_RELATIVE_PATH),
            host_workspaces_root: std::path::PathBuf::from("/tmp/opencode/vibetrading-api"),
            container_workspaces_root: "/workspaces".to_string(),
            api_base_url: "http://host.containers.internal:3003".to_string(),
        },
        workspace_controller: Arc::new(
            crate::opencode::workspace_control_client::LocalWorkspaceController::new(
                crate::opencode::workspace::OpenCodeWorkspaceConfig {
                    source_root: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                        .join(crate::opencode::workspace::PROFILE_SOURCE_RELATIVE_PATH),
                    host_workspaces_root: std::path::PathBuf::from("/tmp/opencode/vibetrading-api"),
                    container_workspaces_root: "/workspaces".to_string(),
                    api_base_url: "http://host.containers.internal:3003".to_string(),
                },
            ),
        ),
        vibetrading_agent_api_base_url: "http://host.containers.internal:3003".to_string(),
        opencode_container_workspaces_root: "/workspaces".to_string(),
        opencode_base_url: "http://localhost:14096".to_string(),
        opencode_client: Arc::new(
            crate::opencode::client::OpenCodeClient::new(
                crate::opencode::client::OpenCodeClientConfig::new("opencode".to_string(), None),
            )
            .unwrap(),
        ),
        model_catalog: crate::model_catalog::models_dev::ModelsDevCatalog::shared(
            cache_dir.clone(),
        )
        .unwrap(),
        asset_cache: Arc::new(crate::cache::asset::AssetCache::new(cache_dir).unwrap()),
        builder_fee_cache: Arc::new(BuilderFeeCache::new(Arc::new(TestBuilderFeeLookup))),
        in_flight: crate::agentic::in_flight::InFlightTracker::new(),
        workspace_leases: crate::agentic::workspace_lease::WorkspaceLeaseManager::new(),
        conversation_turns: crate::agent_conversations::service::ConversationTurnTracker::default(),
        shutdown_rx,
        provider_connections: crate::web::provider_connections::ProviderConnectionsState::new(),
    })
}

pub fn random_private_key() -> String {
    use rand::RngExt;
    format!("0x{}", hex::encode(rand::rng().random::<[u8; 32]>()))
}

pub async fn seed_agent(state: &Arc<AppState>, suffix: &str) -> (String, String) {
    seed_agent_with_prompts(state, suffix, "", "").await
}

pub async fn seed_agent_with_prompts(
    state: &Arc<AppState>,
    suffix: &str,
    analysis_prompt: &str,
    trading_prompt: &str,
) -> (String, String) {
    let timestamp = Utc::now().timestamp_millis();
    let display_name = format!("MemApiTest{}{}", suffix, timestamp);
    let agent_key = crate::agents::model::slugify_agent_key(&display_name);
    let private_key = random_private_key();
    let wallet_address = derive_wallet_address(&private_key).expect("derives");
    let api_key = format!("vta_memapi-{suffix}-{timestamp}");
    let now = Utc::now();
    let row = AgentRegistryRow {
        agent_key: agent_key.clone(),
        user_id: crate::test_db::test_user_id(),
        created_at: now,
        updated_at: now,
        enabled: true,
        lifecycle: crate::agents::model::AGENT_LIFECYCLE_ACTIVE.to_string(),
        display_name,
        trading_account_address: Some(wallet_address),
        environment: "live".to_string(),
        api_key: api_key.clone(),
        api_key_last_used_at: None,
        runtime_config: serde_json::json!({}),
    };
    insert_agent(&state.db_pool, &row)
        .await
        .expect("insert agent");
    insert_default_strategy_prompts_for_agent(&state.db_pool, &agent_key)
        .await
        .expect("insert default prompts");
    upsert_agent_strategy_prompt(
        &state.db_pool,
        &agent_key,
        PROMPT_KIND_ANALYSIS,
        analysis_prompt,
    )
    .await
    .expect("seed analysis prompt");
    upsert_agent_strategy_prompt(
        &state.db_pool,
        &agent_key,
        PROMPT_KIND_TRADING,
        trading_prompt,
    )
    .await
    .expect("seed trading prompt");
    (agent_key, api_key)
}

pub async fn seed_instrument(state: &Arc<AppState>, instrument_id: &str, active: bool) {
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO hyperliquid.instruments (
            instrument_id,
            name,
            market_type,
            base_asset,
            quote_asset,
            settlement_asset,
            asset_index,
            price_decimals,
            size_decimals,
            lot_size,
            max_leverage,
            is_hip3,
            active,
            created_at,
            updated_at
        ) VALUES (
            $1, $1, 'perp', $1, 'USD', 'USDC', 1, 2, 3, 0.001, 50, false, $2, $3, $3
        )
        ON CONFLICT (instrument_id) DO UPDATE
            SET market_type = EXCLUDED.market_type,
                active = EXCLUDED.active,
                updated_at = EXCLUDED.updated_at",
    )
    .bind(instrument_id)
    .bind(active)
    .bind(now)
    .execute(&state.db_pool)
    .await
    .expect("insert instrument");
}

pub async fn select_instruments(state: &Arc<AppState>, agent_key: &str, instrument_ids: &[&str]) {
    let instrument_ids: Vec<String> = instrument_ids.iter().map(|id| (*id).to_string()).collect();
    replace_agent_instruments(&state.db_pool, agent_key, &instrument_ids)
        .await
        .expect("replace agent instruments");
}

pub fn app(state: Arc<AppState>) -> Router {
    api::router(state)
}

pub fn json_body<T: serde::Serialize>(value: &T) -> (Option<(&'static str, String)>, Body) {
    let body = serde_json::to_vec(value).expect("serialize");
    (
        Some(("content-type", "application/json".to_string())),
        Body::from(body),
    )
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_memory_at(
    state: &Arc<AppState>,
    agent_key: &str,
    created_at: chrono::DateTime<Utc>,
    symbol: &str,
    timeframe: Option<&str>,
    memory_type: &str,
    summary: &str,
    metadata: serde_json::Value,
) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO memory.records (
            id, created_at, agent_key, symbol, timeframe, memory_type, summary, content, metadata
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(id)
    .bind(created_at)
    .bind(agent_key)
    .bind(symbol)
    .bind(timeframe)
    .bind(memory_type)
    .bind(summary)
    .bind(format!("body for {summary}"))
    .bind(metadata)
    .execute(&state.db_pool)
    .await
    .expect("insert memory record");
    id
}

pub async fn latest_memories_response(
    state: &Arc<AppState>,
    api_key: &str,
    uri: &str,
) -> (StatusCode, serde_json::Value) {
    let request = Request::builder()
        .method("GET")
        .uri(uri)
        .header("authorization", format!("Bearer {api_key}"))
        .body(Body::empty())
        .unwrap();
    let response = app(Arc::clone(state)).oneshot(request).await.unwrap();
    let status = response.status();
    let body_bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    let body = serde_json::from_slice(&body_bytes).unwrap();
    (status, body)
}

pub async fn get_json_response(
    state: &Arc<AppState>,
    api_key: &str,
    uri: &str,
) -> (StatusCode, serde_json::Value) {
    let request = Request::builder()
        .uri(uri)
        .header("authorization", format!("Bearer {api_key}"))
        .body(Body::empty())
        .unwrap();
    let response = app(Arc::clone(state)).oneshot(request).await.unwrap();
    let status = response.status();
    let body_bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    let body = serde_json::from_slice(&body_bytes).unwrap();
    (status, body)
}

// Re-export items used by test helpers but not directly imported in tests
// (kept here so the `use` list above is self-contained).
#[allow(unused_imports)]
use tower::util::ServiceExt;
