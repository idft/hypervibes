use std::{collections::BTreeMap, sync::Arc, time::Duration};

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use workspace_store::{
    coding_workspace::{CandidateInspection, PromotionJournalPhase, PromotionResult},
    workspace::WorkspaceTemplateDrift,
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

#[async_trait]
pub trait WorkspaceController: Send + Sync {
    async fn create_workspace(
        &self,
        input: WorkspaceAgentInput,
        regenerate: bool,
        idempotency_key: &str,
    ) -> Result<WorkspaceCreated>;
    async fn delete_workspace(&self, agent_key: &str, idempotency_key: &str) -> Result<bool>;
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
