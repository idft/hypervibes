use std::sync::Arc;

use crate::{
    agent_conversations::service::ConversationTurnTracker,
    agentic::{
        backend::AgenticBackend, in_flight::InFlightTracker, workspace_lease::WorkspaceLeaseManager,
    },
    agents::crypto::EncryptionKey,
    cache::asset::AssetCache,
    db::DbPool,
    hyperliquid::builder_fee::BuilderFeeCache,
    hyperliquid::live_state::LiveAccountStore,
    model_catalog::models_dev::ModelsDevCatalog,
    opencode::{client::OpenCodeClient, workspace_control_client::WorkspaceController},
};

use super::provider_connections::ProviderConnectionsState;
use super::run_detail_events::RunDetailEventHub;
use super::ui_events::UiEventHub;

#[derive(Clone)]
pub struct AppState {
    pub db_pool: DbPool,
    #[cfg(test)]
    pub _test_db_guard: Option<Arc<crate::test_db::TestDb>>,
    pub agentic_backend: Arc<dyn AgenticBackend>,
    pub encryption_key: EncryptionKey,
    pub live_accounts: Arc<LiveAccountStore>,
    pub ui_events: Arc<UiEventHub>,
    pub run_detail_events: Arc<RunDetailEventHub>,
    pub workspace_controller: Arc<dyn WorkspaceController>,
    pub vibetrading_agent_api_base_url: String,
    pub opencode_container_workspaces_root: String,
    #[cfg(test)]
    pub opencode_workspace_config: crate::opencode::workspace::OpenCodeWorkspaceConfig,
    pub opencode_base_url: String,
    pub opencode_client: Arc<OpenCodeClient>,
    pub model_catalog: Arc<ModelsDevCatalog>,
    pub asset_cache: Arc<AssetCache>,
    pub builder_fee_cache: Arc<BuilderFeeCache>,
    pub in_flight: InFlightTracker,
    pub workspace_leases: WorkspaceLeaseManager,
    pub conversation_turns: ConversationTurnTracker,
    pub shutdown_rx: tokio::sync::watch::Receiver<bool>,
    pub provider_connections: ProviderConnectionsState,
}
