use std::{collections::BTreeMap, sync::Arc, time::Duration};

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;
use workspace_store::{
    coding_workspace::{CandidateInspection, PromotionJournalPhase, PromotionResult},
    isolated_workspace::{
        IsolatedWorkspaceCreated, IsolatedWorkspaceInspection, RuntimeSecretsScrubbed,
    },
    workspace::{
        MaterializedRunWorkspace, QuantitativePackageSnapshot, RunWorkspaceMaterializationInput,
        WorkspaceBrowserListing, WorkspaceFilePreview, WorkspaceTemplateDrift,
    },
};

#[cfg(test)]
use workspace_store::isolated_workspace::{
    ConversationWorkspacePath, RunWorkspacePath, create_conversation_workspace,
    create_run_workspace, delete_conversation_workspace, delete_run_workspace,
    inspect_conversation_workspace, inspect_run_workspace,
    scrub_conversation_workspace_runtime_secrets, scrub_run_workspace_runtime_secrets,
};

#[cfg(test)]
use workspace_store::workspace::{
    OpenCodeWorkspaceAgent, OpenCodeWorkspaceConfig, WorkspaceGenerationMode,
};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct WorkspaceAgentInput {
    pub agent_key: String,
    pub display_name: String,
    pub agent_api_key: String,
    pub api_base_url: String,
}

#[derive(Debug, Clone)]
pub struct WorkspaceCreated {
    pub workspace_container_path: String,
    pub profile_source: String,
}

#[derive(Debug, Clone)]
pub struct CodingCandidateCreated {
    pub workspace_container_path: String,
    pub base_manifest: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RecoveryResult {
    pub agent_key: String,
    pub task_id: i64,
    pub phase: PromotionJournalPhase,
}

/// Phase 2 exposes isolated-workspace operations while Phase 3 and Phase 4
/// retain the existing active-workspace dispatch and conversation callers.
#[allow(
    dead_code,
    reason = "Phase 3 and Phase 4 will invoke the new isolated-workspace controller methods"
)]
#[async_trait]
pub trait WorkspaceController: Send + Sync {
    async fn create_workspace(
        &self,
        input: WorkspaceAgentInput,
        regenerate: bool,
        idempotency_key: &str,
    ) -> Result<WorkspaceCreated>;
    async fn delete_workspace(&self, agent_key: &str, idempotency_key: &str) -> Result<bool>;
    async fn create_run_workspace(
        &self,
        agent_key: &str,
        run_id: i64,
        idempotency_key: &str,
    ) -> Result<IsolatedWorkspaceCreated>;
    async fn inspect_active_quantitative_package(
        &self,
        agent_key: &str,
    ) -> Result<Option<QuantitativePackageSnapshot>>;
    async fn materialize_run_workspace(
        &self,
        agent_key: &str,
        run_id: i64,
        input: RunWorkspaceMaterializationInput,
        idempotency_key: &str,
    ) -> Result<MaterializedRunWorkspace>;
    async fn inspect_run_workspace(
        &self,
        agent_key: &str,
        run_id: i64,
    ) -> Result<IsolatedWorkspaceInspection>;
    async fn scrub_run_workspace_runtime_secrets(
        &self,
        agent_key: &str,
        run_id: i64,
        idempotency_key: &str,
    ) -> Result<RuntimeSecretsScrubbed>;
    async fn delete_run_workspace(
        &self,
        agent_key: &str,
        run_id: i64,
        idempotency_key: &str,
    ) -> Result<bool>;
    async fn create_conversation_workspace(
        &self,
        agent_key: &str,
        conversation_id: Uuid,
        idempotency_key: &str,
    ) -> Result<IsolatedWorkspaceCreated>;
    async fn inspect_conversation_workspace(
        &self,
        agent_key: &str,
        conversation_id: Uuid,
    ) -> Result<IsolatedWorkspaceInspection>;
    async fn scrub_conversation_workspace_runtime_secrets(
        &self,
        agent_key: &str,
        conversation_id: Uuid,
        idempotency_key: &str,
    ) -> Result<RuntimeSecretsScrubbed>;
    async fn delete_conversation_workspace(
        &self,
        agent_key: &str,
        conversation_id: Uuid,
        idempotency_key: &str,
    ) -> Result<bool>;
    async fn list_workspace_browser_entries(
        &self,
        agent_key: &str,
    ) -> Result<WorkspaceBrowserListing>;
    async fn read_workspace_browser_file(
        &self,
        agent_key: &str,
        relative_path: &str,
    ) -> Result<WorkspaceFilePreview>;
    async fn template_drift(&self, input: WorkspaceAgentInput) -> Result<WorkspaceTemplateDrift>;
    async fn create_candidate(
        &self,
        input: WorkspaceAgentInput,
        task_id: i64,
    ) -> Result<CodingCandidateCreated>;
    async fn store_report(&self, agent_key: &str, task_id: i64, report: Value) -> Result<()>;
    async fn inspect_candidate(&self, agent_key: &str, task_id: i64)
    -> Result<CandidateInspection>;
    async fn promote_candidate(
        &self,
        agent_key: &str,
        task_id: i64,
        expected_base_manifest: BTreeMap<String, String>,
        candidate_manifest_hash: String,
    ) -> Result<PromotionResult>;
    async fn delete_candidate(&self, agent_key: &str, task_id: i64) -> Result<bool>;
    async fn recover_promotions(&self) -> Result<Vec<RecoveryResult>>;
}

pub struct HttpWorkspaceController {
    client: Client,
    base_url: Url,
    api_key: Arc<str>,
}

#[cfg(test)]
pub struct LocalWorkspaceController {
    config: OpenCodeWorkspaceConfig,
}

#[cfg(test)]
impl LocalWorkspaceController {
    pub fn new(config: OpenCodeWorkspaceConfig) -> Self {
        Self { config }
    }

    fn config_for(&self, api_base_url: String) -> OpenCodeWorkspaceConfig {
        let mut config = self.config.clone();
        config.api_base_url = api_base_url;
        config
    }
}

#[cfg(test)]
#[async_trait]
impl WorkspaceController for LocalWorkspaceController {
    async fn create_workspace(
        &self,
        input: WorkspaceAgentInput,
        regenerate: bool,
        _idempotency_key: &str,
    ) -> Result<WorkspaceCreated> {
        let config = self.config_for(input.api_base_url);
        let generated = workspace_store::workspace::generate_agent_workspace(
            &config,
            &OpenCodeWorkspaceAgent {
                agent_key: input.agent_key,
                display_name: input.display_name,
                api_key: input.agent_api_key,
            },
            if regenerate {
                WorkspaceGenerationMode::Regenerate
            } else {
                WorkspaceGenerationMode::CreateNew
            },
        )?;
        Ok(WorkspaceCreated {
            workspace_container_path: generated.workspace_container_path,
            profile_source: "agent-runtime/workspace-template".to_string(),
        })
    }
    async fn delete_workspace(&self, agent_key: &str, _idempotency_key: &str) -> Result<bool> {
        workspace_store::workspace::delete_agent_workspace(&self.config, agent_key)
    }
    async fn create_run_workspace(
        &self,
        agent_key: &str,
        run_id: i64,
        _idempotency_key: &str,
    ) -> Result<IsolatedWorkspaceCreated> {
        let path = RunWorkspacePath::new(agent_key, run_id)?;
        create_run_workspace(&self.config, &path)
    }
    async fn inspect_active_quantitative_package(
        &self,
        agent_key: &str,
    ) -> Result<Option<QuantitativePackageSnapshot>> {
        workspace_store::workspace::inspect_active_quantitative_package(&self.config, agent_key)
    }
    async fn materialize_run_workspace(
        &self,
        agent_key: &str,
        run_id: i64,
        input: RunWorkspaceMaterializationInput,
        _idempotency_key: &str,
    ) -> Result<MaterializedRunWorkspace> {
        let path = RunWorkspacePath::new(agent_key, run_id)?;
        workspace_store::workspace::materialize_run_workspace(&self.config, &path, &input)
    }
    async fn inspect_run_workspace(
        &self,
        agent_key: &str,
        run_id: i64,
    ) -> Result<IsolatedWorkspaceInspection> {
        let path = RunWorkspacePath::new(agent_key, run_id)?;
        inspect_run_workspace(&self.config, &path)
    }
    async fn scrub_run_workspace_runtime_secrets(
        &self,
        agent_key: &str,
        run_id: i64,
        _idempotency_key: &str,
    ) -> Result<RuntimeSecretsScrubbed> {
        let path = RunWorkspacePath::new(agent_key, run_id)?;
        scrub_run_workspace_runtime_secrets(&self.config, &path)
    }
    async fn delete_run_workspace(
        &self,
        agent_key: &str,
        run_id: i64,
        _idempotency_key: &str,
    ) -> Result<bool> {
        let path = RunWorkspacePath::new(agent_key, run_id)?;
        delete_run_workspace(&self.config, &path)
    }
    async fn create_conversation_workspace(
        &self,
        agent_key: &str,
        conversation_id: Uuid,
        _idempotency_key: &str,
    ) -> Result<IsolatedWorkspaceCreated> {
        let path = ConversationWorkspacePath::new(agent_key, conversation_id)?;
        create_conversation_workspace(&self.config, &path)
    }
    async fn inspect_conversation_workspace(
        &self,
        agent_key: &str,
        conversation_id: Uuid,
    ) -> Result<IsolatedWorkspaceInspection> {
        let path = ConversationWorkspacePath::new(agent_key, conversation_id)?;
        inspect_conversation_workspace(&self.config, &path)
    }
    async fn scrub_conversation_workspace_runtime_secrets(
        &self,
        agent_key: &str,
        conversation_id: Uuid,
        _idempotency_key: &str,
    ) -> Result<RuntimeSecretsScrubbed> {
        let path = ConversationWorkspacePath::new(agent_key, conversation_id)?;
        scrub_conversation_workspace_runtime_secrets(&self.config, &path)
    }
    async fn delete_conversation_workspace(
        &self,
        agent_key: &str,
        conversation_id: Uuid,
        _idempotency_key: &str,
    ) -> Result<bool> {
        let path = ConversationWorkspacePath::new(agent_key, conversation_id)?;
        delete_conversation_workspace(&self.config, &path)
    }
    async fn list_workspace_browser_entries(
        &self,
        agent_key: &str,
    ) -> Result<WorkspaceBrowserListing> {
        workspace_store::workspace::list_workspace_browser_entries(&self.config, agent_key)
    }
    async fn read_workspace_browser_file(
        &self,
        agent_key: &str,
        relative_path: &str,
    ) -> Result<WorkspaceFilePreview> {
        workspace_store::workspace::read_workspace_browser_file(
            &self.config,
            agent_key,
            relative_path,
        )
    }
    async fn template_drift(&self, input: WorkspaceAgentInput) -> Result<WorkspaceTemplateDrift> {
        let config = self.config_for(input.api_base_url);
        workspace_store::workspace::diff_agent_workspace_from_template(
            &config,
            &OpenCodeWorkspaceAgent {
                agent_key: input.agent_key,
                display_name: input.display_name,
                api_key: input.agent_api_key,
            },
        )
    }
    async fn create_candidate(
        &self,
        input: WorkspaceAgentInput,
        task_id: i64,
    ) -> Result<CodingCandidateCreated> {
        let config = self.config_for(input.api_base_url);
        let candidate = workspace_store::coding_workspace::prepare_coding_candidate(
            &config,
            &input.agent_key,
            task_id,
            &input.display_name,
            &input.agent_api_key,
        )?;
        Ok(CodingCandidateCreated {
            workspace_container_path: workspace_store::coding_workspace::candidate_container_root(
                &config,
                &input.agent_key,
                task_id,
            )?,
            base_manifest: candidate.base_manifest,
        })
    }
    async fn store_report(&self, agent_key: &str, task_id: i64, report: Value) -> Result<()> {
        workspace_store::coding_workspace::store_coding_report(
            &self.config,
            agent_key,
            task_id,
            &report,
        )
    }
    async fn inspect_candidate(
        &self,
        agent_key: &str,
        task_id: i64,
    ) -> Result<CandidateInspection> {
        workspace_store::coding_workspace::inspect_coding_candidate(
            &self.config,
            agent_key,
            task_id,
        )
    }
    async fn promote_candidate(
        &self,
        agent_key: &str,
        task_id: i64,
        expected_base_manifest: BTreeMap<String, String>,
        candidate_manifest_hash: String,
    ) -> Result<PromotionResult> {
        workspace_store::coding_workspace::promote_coding_candidate(
            &self.config,
            agent_key,
            task_id,
            &expected_base_manifest,
            &candidate_manifest_hash,
        )
    }
    async fn delete_candidate(&self, agent_key: &str, task_id: i64) -> Result<bool> {
        workspace_store::coding_workspace::delete_coding_candidate(&self.config, agent_key, task_id)
    }
    async fn recover_promotions(&self) -> Result<Vec<RecoveryResult>> {
        workspace_store::coding_workspace::list_promotion_journals(&self.config)?
            .into_iter()
            .map(|journal| {
                Ok(RecoveryResult {
                    agent_key: journal.agent_key.clone(),
                    task_id: journal.task_id,
                    phase: workspace_store::coding_workspace::recover_promotion_journal(
                        &self.config,
                        &journal,
                    )?,
                })
            })
            .collect()
    }
}

impl HttpWorkspaceController {
    pub fn new(base_url: &str, api_key: String) -> Result<Self> {
        if api_key.trim().is_empty() {
            bail!("WORKSPACE_CONTROL_API_KEY must not be empty");
        }
        let base_url = Url::parse(base_url)
            .context("WORKSPACE_CONTROL_BASE_URL must be a valid absolute URL")?;
        if base_url.scheme().is_empty() || base_url.host_str().is_none() {
            bail!("WORKSPACE_CONTROL_BASE_URL must include a scheme and host");
        }
        Ok(Self {
            client: Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .context("failed to build workspace controller client")?,
            base_url,
            api_key: Arc::from(api_key),
        })
    }

    fn request(&self, method: reqwest::Method, path: &str) -> Result<reqwest::RequestBuilder> {
        let url = self
            .base_url
            .join(path)
            .context("failed to build workspace controller request URL")?;
        Ok(self
            .client
            .request(method, url)
            .bearer_auth(self.api_key.as_ref()))
    }

    async fn response<T: for<'de> Deserialize<'de>>(response: reqwest::Response) -> Result<T> {
        let status = response.status();
        if !status.is_success() {
            bail!("workspace controller returned {}", status.as_u16());
        }
        response
            .json()
            .await
            .context("workspace controller returned an invalid response")
    }

    async fn delete_response(&self, path: &str, idempotency_key: String) -> Result<bool> {
        let response = self
            .request(reqwest::Method::DELETE, path)?
            .header("Idempotency-Key", idempotency_key)
            .send()
            .await
            .context("workspace controller request failed")?;
        Ok(Self::response::<DeleteResponse>(response).await?.deleted)
    }
}

#[async_trait]
impl WorkspaceController for HttpWorkspaceController {
    async fn create_workspace(
        &self,
        input: WorkspaceAgentInput,
        regenerate: bool,
        idempotency_key: &str,
    ) -> Result<WorkspaceCreated> {
        let response = self
            .request(
                reqwest::Method::POST,
                &format!("v1/agent-workspaces/{}", input.agent_key),
            )?
            .header("Idempotency-Key", idempotency_key)
            .json(&WorkspaceRequest::from_input(input, regenerate))
            .send()
            .await
            .context("workspace controller request failed")?;
        let response: WorkspaceResponse = Self::response(response).await?;
        Ok(WorkspaceCreated {
            workspace_container_path: response.workspace_container_path,
            profile_source: response.profile_source,
        })
    }

    async fn delete_workspace(&self, agent_key: &str, idempotency_key: &str) -> Result<bool> {
        self.delete_response(
            &format!("v1/agent-workspaces/{agent_key}"),
            idempotency_key.to_string(),
        )
        .await
    }

    async fn create_run_workspace(
        &self,
        agent_key: &str,
        run_id: i64,
        idempotency_key: &str,
    ) -> Result<IsolatedWorkspaceCreated> {
        let response = self
            .request(
                reqwest::Method::POST,
                &format!("v1/run-workspaces/{agent_key}/{run_id}"),
            )?
            .header("Idempotency-Key", idempotency_key)
            .send()
            .await
            .context("workspace controller request failed")?;
        Self::response(response).await
    }

    async fn inspect_active_quantitative_package(
        &self,
        agent_key: &str,
    ) -> Result<Option<QuantitativePackageSnapshot>> {
        let response = self
            .request(
                reqwest::Method::GET,
                &format!("v1/agent-workspaces/{agent_key}/quantitative-package"),
            )?
            .send()
            .await
            .context("workspace controller request failed")?;
        Self::response(response).await
    }

    async fn materialize_run_workspace(
        &self,
        agent_key: &str,
        run_id: i64,
        input: RunWorkspaceMaterializationInput,
        idempotency_key: &str,
    ) -> Result<MaterializedRunWorkspace> {
        let response = self
            .request(
                reqwest::Method::POST,
                &format!("v1/run-workspaces/{agent_key}/{run_id}/materialize"),
            )?
            .header("Idempotency-Key", idempotency_key)
            .json(&input)
            .send()
            .await
            .context("workspace controller request failed")?;
        Self::response(response).await
    }

    async fn inspect_run_workspace(
        &self,
        agent_key: &str,
        run_id: i64,
    ) -> Result<IsolatedWorkspaceInspection> {
        let response = self
            .request(
                reqwest::Method::GET,
                &format!("v1/run-workspaces/{agent_key}/{run_id}/inspection"),
            )?
            .send()
            .await
            .context("workspace controller request failed")?;
        Self::response(response).await
    }

    async fn scrub_run_workspace_runtime_secrets(
        &self,
        agent_key: &str,
        run_id: i64,
        idempotency_key: &str,
    ) -> Result<RuntimeSecretsScrubbed> {
        let response = self
            .request(
                reqwest::Method::POST,
                &format!("v1/run-workspaces/{agent_key}/{run_id}/runtime-secrets"),
            )?
            .header("Idempotency-Key", idempotency_key)
            .send()
            .await
            .context("workspace controller request failed")?;
        Self::response(response).await
    }

    async fn delete_run_workspace(
        &self,
        agent_key: &str,
        run_id: i64,
        idempotency_key: &str,
    ) -> Result<bool> {
        self.delete_response(
            &format!("v1/run-workspaces/{agent_key}/{run_id}"),
            idempotency_key.to_string(),
        )
        .await
    }

    async fn create_conversation_workspace(
        &self,
        agent_key: &str,
        conversation_id: Uuid,
        idempotency_key: &str,
    ) -> Result<IsolatedWorkspaceCreated> {
        let response = self
            .request(
                reqwest::Method::POST,
                &format!("v1/conversation-workspaces/{agent_key}/{conversation_id}"),
            )?
            .header("Idempotency-Key", idempotency_key)
            .send()
            .await
            .context("workspace controller request failed")?;
        Self::response(response).await
    }

    async fn inspect_conversation_workspace(
        &self,
        agent_key: &str,
        conversation_id: Uuid,
    ) -> Result<IsolatedWorkspaceInspection> {
        let response = self
            .request(
                reqwest::Method::GET,
                &format!("v1/conversation-workspaces/{agent_key}/{conversation_id}/inspection"),
            )?
            .send()
            .await
            .context("workspace controller request failed")?;
        Self::response(response).await
    }

    async fn scrub_conversation_workspace_runtime_secrets(
        &self,
        agent_key: &str,
        conversation_id: Uuid,
        idempotency_key: &str,
    ) -> Result<RuntimeSecretsScrubbed> {
        let response = self
            .request(
                reqwest::Method::POST,
                &format!(
                    "v1/conversation-workspaces/{agent_key}/{conversation_id}/runtime-secrets"
                ),
            )?
            .header("Idempotency-Key", idempotency_key)
            .send()
            .await
            .context("workspace controller request failed")?;
        Self::response(response).await
    }

    async fn delete_conversation_workspace(
        &self,
        agent_key: &str,
        conversation_id: Uuid,
        idempotency_key: &str,
    ) -> Result<bool> {
        self.delete_response(
            &format!("v1/conversation-workspaces/{agent_key}/{conversation_id}"),
            idempotency_key.to_string(),
        )
        .await
    }

    async fn list_workspace_browser_entries(
        &self,
        agent_key: &str,
    ) -> Result<WorkspaceBrowserListing> {
        let response = self
            .request(
                reqwest::Method::GET,
                &format!("v1/agent-workspaces/{agent_key}/browser"),
            )?
            .send()
            .await
            .context("workspace controller request failed")?;
        Self::response(response).await
    }

    async fn read_workspace_browser_file(
        &self,
        agent_key: &str,
        relative_path: &str,
    ) -> Result<WorkspaceFilePreview> {
        let mut url = self
            .base_url
            .join(&format!("v1/agent-workspaces/{agent_key}/browser/file"))
            .context("failed to build workspace controller request URL")?;
        url.query_pairs_mut().append_pair("path", relative_path);
        let response = self
            .client
            .request(reqwest::Method::GET, url)
            .bearer_auth(self.api_key.as_ref())
            .send()
            .await
            .context("workspace controller request failed")?;
        Self::response(response).await
    }

    async fn template_drift(&self, input: WorkspaceAgentInput) -> Result<WorkspaceTemplateDrift> {
        let path = format!("v1/agent-workspaces/{}/template-drift", input.agent_key);
        let response = self
            .request(reqwest::Method::POST, &path)?
            .json(&CandidateRequest::from_input(input))
            .send()
            .await
            .context("workspace controller request failed")?;
        Self::response(response).await
    }

    async fn create_candidate(
        &self,
        input: WorkspaceAgentInput,
        task_id: i64,
    ) -> Result<CodingCandidateCreated> {
        let path = format!("v1/coding-candidates/{}/{}", input.agent_key, task_id);
        let response = self
            .request(reqwest::Method::POST, &path)?
            .header(
                "Idempotency-Key",
                format!("maintenance:{task_id}:candidate"),
            )
            .json(&CandidateRequest::from_input(input))
            .send()
            .await
            .context("workspace controller request failed")?;
        let response: CandidateResponse = Self::response(response).await?;
        Ok(CodingCandidateCreated {
            workspace_container_path: response.workspace_container_path,
            base_manifest: response.base_manifest,
        })
    }

    async fn store_report(&self, agent_key: &str, task_id: i64, report: Value) -> Result<()> {
        let response = self
            .request(
                reqwest::Method::PUT,
                &format!("v1/coding-candidates/{agent_key}/{task_id}/report"),
            )?
            .header("Idempotency-Key", format!("maintenance:{task_id}:report"))
            .json(&ReportRequest { report })
            .send()
            .await
            .context("workspace controller request failed")?;
        if response.status() != StatusCode::CREATED {
            bail!(
                "workspace controller returned {}",
                response.status().as_u16()
            );
        }
        Ok(())
    }

    async fn inspect_candidate(
        &self,
        agent_key: &str,
        task_id: i64,
    ) -> Result<CandidateInspection> {
        let response = self
            .request(
                reqwest::Method::GET,
                &format!("v1/coding-candidates/{agent_key}/{task_id}/inspection"),
            )?
            .send()
            .await
            .context("workspace controller request failed")?;
        Self::response(response).await
    }

    async fn promote_candidate(
        &self,
        agent_key: &str,
        task_id: i64,
        expected_base_manifest: BTreeMap<String, String>,
        candidate_manifest_hash: String,
    ) -> Result<PromotionResult> {
        let response = self
            .request(
                reqwest::Method::POST,
                &format!("v1/coding-candidates/{agent_key}/{task_id}/promote"),
            )?
            .header("Idempotency-Key", format!("maintenance:{task_id}:promote"))
            .json(&PromoteRequest {
                expected_base_manifest,
                candidate_manifest_hash,
            })
            .send()
            .await
            .context("workspace controller request failed")?;
        Self::response(response).await
    }

    async fn delete_candidate(&self, agent_key: &str, task_id: i64) -> Result<bool> {
        self.delete_response(
            &format!("v1/coding-candidates/{agent_key}/{task_id}"),
            format!("maintenance:{task_id}:cleanup"),
        )
        .await
    }

    async fn recover_promotions(&self) -> Result<Vec<RecoveryResult>> {
        let response = self
            .request(reqwest::Method::POST, "v1/promotion-recovery")?
            .header("Idempotency-Key", "promotion-recovery")
            .send()
            .await
            .context("workspace controller request failed")?;
        Ok(Self::response::<RecoveryResponse>(response)
            .await?
            .recovered)
    }
}

#[derive(Serialize)]
struct WorkspaceRequest {
    mode: &'static str,
    display_name: String,
    agent_api_key: String,
    api_base_url: String,
}
impl WorkspaceRequest {
    fn from_input(input: WorkspaceAgentInput, regenerate: bool) -> Self {
        Self {
            mode: if regenerate {
                "regenerate"
            } else {
                "create_new"
            },
            display_name: input.display_name,
            agent_api_key: input.agent_api_key,
            api_base_url: input.api_base_url,
        }
    }
}
#[derive(Serialize)]
struct CandidateRequest {
    display_name: String,
    agent_api_key: String,
    api_base_url: String,
}
impl CandidateRequest {
    fn from_input(input: WorkspaceAgentInput) -> Self {
        Self {
            display_name: input.display_name,
            agent_api_key: input.agent_api_key,
            api_base_url: input.api_base_url,
        }
    }
}
#[derive(Deserialize)]
struct WorkspaceResponse {
    workspace_container_path: String,
    profile_source: String,
}
#[derive(Deserialize)]
struct CandidateResponse {
    workspace_container_path: String,
    base_manifest: BTreeMap<String, String>,
}
#[derive(Serialize)]
struct ReportRequest {
    report: Value,
}
#[derive(Serialize)]
struct PromoteRequest {
    expected_base_manifest: BTreeMap<String, String>,
    candidate_manifest_hash: String,
}
#[derive(Deserialize)]
struct DeleteResponse {
    deleted: bool,
}
#[derive(Deserialize)]
struct RecoveryResponse {
    recovered: Vec<RecoveryResult>,
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        process,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    fn test_config(root: PathBuf) -> OpenCodeWorkspaceConfig {
        OpenCodeWorkspaceConfig {
            source_root: root.clone(),
            host_workspaces_root: root,
            container_workspaces_root: "/workspaces".to_string(),
            api_base_url: String::new(),
        }
    }

    fn temp_root() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time after unix epoch")
            .as_nanos();
        PathBuf::from("/tmp/opencode").join(format!("workspace-client-{}-{suffix}", process::id()))
    }

    #[tokio::test]
    async fn local_controller_implements_isolated_workspace_contract() {
        let root = temp_root();
        fs::create_dir_all(&root).expect("create temporary workspace root");
        let controller = LocalWorkspaceController::new(test_config(root.clone()));

        let run = controller
            .create_run_workspace("agent", 7, "run-create")
            .await
            .expect("create local run workspace");
        assert_eq!(
            run.workspace_container_path,
            "/workspaces/runs/agent/7/workspace"
        );
        fs::write(root.join("runs/agent/7/workspace/.env"), "secret")
            .expect("write local runtime secret");
        assert!(
            controller
                .inspect_run_workspace("agent", 7)
                .await
                .expect("inspect local run workspace")
                .runtime_secrets_present
        );
        assert!(
            controller
                .scrub_run_workspace_runtime_secrets("agent", 7, "run-scrub")
                .await
                .expect("scrub local run workspace")
                .removed
        );
        assert!(
            controller
                .delete_run_workspace("agent", 7, "run-delete")
                .await
                .expect("delete local run workspace")
        );

        let conversation_id = Uuid::new_v4();
        let conversation = controller
            .create_conversation_workspace("agent", conversation_id, "conversation-create")
            .await
            .expect("create local conversation workspace");
        assert_eq!(
            conversation.workspace_container_path,
            format!("/workspaces/conversations/agent/{conversation_id}/workspace")
        );
        assert!(
            controller
                .inspect_conversation_workspace("agent", conversation_id)
                .await
                .expect("inspect local conversation workspace")
                .workspace_exists
        );
        assert!(
            controller
                .scrub_conversation_workspace_runtime_secrets(
                    "agent",
                    conversation_id,
                    "conversation-scrub",
                )
                .await
                .expect("scrub local conversation workspace")
                .workspace_exists
        );
        assert!(
            controller
                .delete_conversation_workspace("agent", conversation_id, "conversation-delete")
                .await
                .expect("delete local conversation workspace")
        );

        fs::remove_dir_all(root).expect("remove temporary workspace root");
    }
}
