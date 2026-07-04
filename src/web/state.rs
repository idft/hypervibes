use std::sync::Arc;

use crate::{
    agentic::backend::AgenticBackend,
    agents::crypto::EncryptionKey,
    cache::asset::AssetCache,
    db::DbPool,
    hyperliquid::live_state::LiveAccountStore,
    model_catalog::models_dev::ModelsDevCatalog,
    opencode::{client::OpenCodeClient, workspace::OpenCodeWorkspaceConfig},
};

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
    pub opencode_workspace_config: OpenCodeWorkspaceConfig,
    pub opencode_client: Arc<OpenCodeClient>,
    pub model_catalog: Arc<ModelsDevCatalog>,
    pub asset_cache: Arc<AssetCache>,
}
