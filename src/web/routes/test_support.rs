//! Shared test helpers for route tests.
use std::sync::{Arc, Mutex};

use anyhow::Result;
use async_trait::async_trait;
use axum::body::Body;
use chrono::Utc;
use rust_decimal::Decimal;

use crate::agents::{
    keys::derive_wallet_address,
    model::slugify_agent_key,
    store::{get_agent, insert_agent},
};
use crate::{
    agentic::backend::{AgenticBackend, DispatchRequest, DispatchResult},
    agents::{
        crypto::EncryptionKey,
        store::update_agent_runtime_config,
        strategy_prompts::{
            PROMPT_KIND_ANALYSIS, PROMPT_KIND_TRADING, insert_default_strategy_prompts_for_agent,
            upsert_agent_strategy_prompt,
        },
    },
    memory::CreateMemory,
    opencode::workspace::{
        OpenCodeWorkspaceAgent, WorkspaceGenerationMode, generate_agent_workspace,
        runtime_config_for_generated_workspace,
    },
    test_db,
    web::{AppState, run_detail_events::RunDetailEventHub, ui_events::UiEventHub},
};
use axum::response::Response;
use http_body_util::BodyExt as _;

pub(in crate::web::routes) struct NoopAgenticBackend;
#[async_trait]
impl AgenticBackend for NoopAgenticBackend {
    async fn dispatch(&self, _request: DispatchRequest) -> Result<DispatchResult> {
        Ok(DispatchResult {
            backend_run_ref: "ses_test".to_string(),
        })
    }
}
pub(in crate::web::routes) struct RecordingAgenticBackend {
    pub calls: Arc<Mutex<Vec<DispatchRequest>>>,
}
#[async_trait]
impl AgenticBackend for RecordingAgenticBackend {
    async fn dispatch(&self, request: DispatchRequest) -> Result<DispatchResult> {
        self.calls.lock().unwrap().push(request);
        Ok(DispatchResult {
            backend_run_ref: "ses_recorded".to_string(),
        })
    }
}
pub(in crate::web::routes) async fn test_state() -> Arc<AppState> {
    test_state_with_backend(Arc::new(NoopAgenticBackend)).await
}
pub(in crate::web::routes) async fn test_state_with_backend(
    agentic_backend: Arc<dyn AgenticBackend>,
) -> Arc<AppState> {
    test_state_with_backend_and_shutdown(agentic_backend, false).await
}

pub(in crate::web::routes) async fn test_state_with_backend_and_shutdown(
    agentic_backend: Arc<dyn AgenticBackend>,
    shutdown_signaled: bool,
) -> Arc<AppState> {
    let pool = Arc::new(test_db::pool().await);
    let cache_dir = std::path::PathBuf::from("/tmp/opencode/vibetrading-routes-cache");
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(shutdown_signaled);
    // Keep the sender alive for the test by leaking it; tests are
    // short-lived and the watch is shared with the AppState clone.
    let _ = Box::leak(Box::new(shutdown_tx));
    Arc::new(AppState {
        db_pool: pool.as_ref().as_ref().clone(),
        _test_db_guard: Some(Arc::clone(&pool)),
        agentic_backend,
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
            host_workspaces_root: std::path::PathBuf::from("/tmp/opencode/vibetrading-routes"),
            container_workspaces_root: "/workspaces".to_string(),
            api_base_url: "http://host.containers.internal:3003".to_string(),
        },
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
        in_flight: crate::agentic::in_flight::InFlightTracker::new(),
        workspace_leases: crate::agentic::workspace_lease::WorkspaceLeaseManager::new(),
        shutdown_rx,
    })
}
pub(in crate::web::routes) async fn read_sse_chunk(body: Body, timeout_ms: u64) -> String {
    let mut body = body;
    let mut buf = Vec::<u8>::new();
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_millis(timeout_ms);

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, body.frame()).await {
            Ok(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    buf.extend_from_slice(data);
                }
            }
            Ok(Some(Err(e))) => panic!("failed to read SSE body: {e}"),
            Ok(None) => break,
            Err(_) => break,
        }
    }

    if buf.is_empty() {
        panic!("timed out reading SSE body");
    }

    String::from_utf8_lossy(&buf).into_owned()
}
pub(in crate::web::routes) async fn response_text(response: Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8_lossy(&bytes).into_owned()
}
pub(in crate::web::routes) async fn seed_ledger_event(
    state: &Arc<AppState>,
    wallet_address: &str,
    hash: &str,
    event_time: chrono::DateTime<Utc>,
    usdc: Decimal,
) {
    sqlx::query(
        "INSERT INTO hyperliquid.ledger_events
            (hash, account_address, environment, event_time, event_type,
             source_stream, ledger_type, usdc, ingest_source, inserted_at)
         VALUES ($1, $2, 'live', $3, 'ledger', 'test', 'deposit', $4, 'test', NOW())",
    )
    .bind(hash)
    .bind(wallet_address)
    .bind(event_time)
    .bind(usdc)
    .execute(&state.db_pool)
    .await
    .expect("insert ledger event");
}
pub(in crate::web::routes) async fn seed_memory(
    state: &Arc<AppState>,
    agent_key: &str,
    summary: &str,
    content: &str,
) -> crate::memory::MemoryRecord {
    seed_memory_with_type(state, agent_key, "plan", summary, content).await
}
pub(in crate::web::routes) async fn seed_memory_with_type(
    state: &Arc<AppState>,
    agent_key: &str,
    memory_type: &str,
    summary: &str,
    content: &str,
) -> crate::memory::MemoryRecord {
    crate::memory::insert_memory(
        &state.db_pool,
        agent_key,
        &CreateMemory {
            symbol: "BTC".to_string(),
            timeframe: Some("1h".to_string()),
            memory_type: memory_type.to_string(),
            summary: summary.to_string(),
            content: content.to_string(),
            metadata: Some(serde_json::json!({ "confidence": 0.8 })),
            links: None,
        },
    )
    .await
    .expect("insert memory")
}
pub(in crate::web::routes) async fn seed_instrument(
    state: &Arc<AppState>,
    instrument_id: &str,
    active: bool,
) {
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
pub(in crate::web::routes) async fn insert_test_agent(
    state: &Arc<AppState>,
) -> Option<(String, String)> {
    insert_test_agent_with_text(state, String::new(), String::new()).await
}
pub(in crate::web::routes) async fn insert_test_opencode_agent(
    state: &Arc<AppState>,
) -> Option<(String, String)> {
    let timestamp = chrono::Utc::now().timestamp_millis();
    let display_name = format!("OpenCodeScheduleTest{}", timestamp);
    let agent_key = slugify_agent_key(&display_name);
    let private_key = random_private_key();
    let wallet_address = match derive_wallet_address(&private_key) {
        Ok(addr) => addr,
        Err(_) => return None,
    };
    let now = Utc::now();
    let row = crate::agents::model::AgentRegistryRow {
        agent_key: agent_key.clone(),
        user_id: crate::test_db::test_user_id(),
        created_at: now,
        updated_at: now,
        enabled: true,
        lifecycle: crate::agents::model::AGENT_LIFECYCLE_ACTIVE.to_string(),
        display_name,
        trading_account_address: Some(wallet_address.clone()),
        environment: "live".to_string(),
        api_key: format!("opencode-schedule-test-{timestamp}"),
        api_key_last_used_at: None,
        runtime_config: serde_json::json!({}),
    };
    if insert_agent(&state.db_pool, &row).await.is_err() {
        return None;
    }
    insert_default_strategy_prompts_for_agent(&state.db_pool, &agent_key)
        .await
        .expect("insert default prompts");
    crate::agentic::store::insert_default_opencode_schedules(&state.db_pool, &agent_key)
        .await
        .expect("insert default schedules");
    sqlx::query(
        "UPDATE agentic_job_schedules
            SET model_provider_id = 'anthropic', model_id = 'claude-sonnet-test'
          WHERE agent_key = $1",
    )
    .bind(&agent_key)
    .execute(&state.db_pool)
    .await
    .expect("set test schedule models");
    sqlx::query(
        "UPDATE agentic_job_hooks
            SET model_provider_id = 'anthropic', model_id = 'claude-sonnet-test'
          WHERE agent_key = $1",
    )
    .bind(&agent_key)
    .execute(&state.db_pool)
    .await
    .expect("set test hook models");
    Some((agent_key, wallet_address))
}
pub(in crate::web::routes) async fn seed_workspace_runtime_config(
    state: &Arc<AppState>,
    agent_key: &str,
) {
    update_agent_runtime_config(
        &state.db_pool,
        agent_key,
        serde_json::json!({
            "workspace_host_path": format!("workspaces/agents/{agent_key}"),
            "workspace_container_path": format!("/workspaces/agents/{agent_key}"),
            "profile_source": "agent-runtime/workspace-template"
        }),
    )
    .await
    .expect("seed workspace runtime config");
}

pub(in crate::web::routes) async fn generate_test_agent_workspace(
    state: &Arc<AppState>,
    agent_key: &str,
) {
    let agent = get_agent(&state.db_pool, agent_key)
        .await
        .expect("load test agent")
        .expect("test agent exists");
    let generated = generate_agent_workspace(
        &state.opencode_workspace_config,
        &OpenCodeWorkspaceAgent {
            agent_key: agent.agent_key,
            display_name: agent.display_name,
            api_key: agent.api_key,
        },
        WorkspaceGenerationMode::CreateNew,
    )
    .expect("generate test workspace");
    update_agent_runtime_config(
        &state.db_pool,
        agent_key,
        runtime_config_for_generated_workspace(&generated).into_value(),
    )
    .await
    .expect("store test workspace metadata");
}
pub(in crate::web::routes) async fn insert_test_agent_with_text(
    state: &Arc<AppState>,
    analysis_prompt: String,
    trading_prompt: String,
) -> Option<(String, String)> {
    let timestamp = chrono::Utc::now().timestamp_millis();
    let display_name = format!("BalanceStreamTest{}", timestamp);
    let agent_key = slugify_agent_key(&display_name);
    let private_key = random_private_key();
    let wallet_address = match derive_wallet_address(&private_key) {
        Ok(addr) => addr,
        Err(_) => return None,
    };
    let now = Utc::now();
    let row = crate::agents::model::AgentRegistryRow {
        agent_key: agent_key.clone(),
        user_id: crate::test_db::test_user_id(),
        created_at: now,
        updated_at: now,
        enabled: true,
        lifecycle: crate::agents::model::AGENT_LIFECYCLE_ACTIVE.to_string(),
        display_name,
        trading_account_address: Some(wallet_address.clone()),
        environment: "live".to_string(),
        api_key: format!("balance-stream-test-{timestamp}"),
        api_key_last_used_at: None,
        runtime_config: serde_json::json!({}),
    };
    if insert_agent(&state.db_pool, &row).await.is_err() {
        return None;
    }
    insert_default_strategy_prompts_for_agent(&state.db_pool, &agent_key)
        .await
        .expect("insert default prompts");
    upsert_agent_strategy_prompt(
        &state.db_pool,
        &agent_key,
        PROMPT_KIND_ANALYSIS,
        &analysis_prompt,
    )
    .await
    .expect("seed analysis prompt");
    upsert_agent_strategy_prompt(
        &state.db_pool,
        &agent_key,
        PROMPT_KIND_TRADING,
        &trading_prompt,
    )
    .await
    .expect("seed trading prompt");
    Some((agent_key, wallet_address))
}
pub(in crate::web::routes) fn random_private_key() -> String {
    use rand::RngExt;
    format!("0x{}", hex::encode(rand::rng().random::<[u8; 32]>()))
}
