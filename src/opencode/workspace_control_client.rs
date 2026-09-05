use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use reqwest::{Client, Url};
use serde::Deserialize;
use uuid::Uuid;
use workspace_store::{
    isolated_workspace::{
        IsolatedWorkspaceCreated, IsolatedWorkspaceInspection, RuntimeSecretsScrubbed,
    },
    workspace::{
        ConversationWorkspaceMaterializationInput, MaterializedConversationWorkspace,
        MaterializedRunWorkspace, RunWorkspaceMaterializationInput,
    },
};

#[cfg(test)]
use workspace_store::isolated_workspace::{
    ConversationWorkspacePath, RunWorkspacePath, create_conversation_workspace,
    create_run_workspace, delete_conversation_workspace, delete_run_workspace,
    inspect_conversation_workspace, inspect_run_workspace,
    scrub_conversation_workspace_runtime_secrets, scrub_run_workspace_runtime_secrets,
};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[async_trait]
pub trait WorkspaceController: Send + Sync {
    async fn create_run_workspace(
        &self,
        agent_key: &str,
        run_id: i64,
        idempotency_key: &str,
    ) -> Result<IsolatedWorkspaceCreated>;
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
    async fn materialize_conversation_workspace(
        &self,
        agent_key: &str,
        conversation_id: Uuid,
        input: ConversationWorkspaceMaterializationInput,
        idempotency_key: &str,
    ) -> Result<MaterializedConversationWorkspace>;
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
}

pub struct HttpWorkspaceController {
    client: Client,
    base_url: Url,
    api_key: Arc<str>,
}

#[cfg(test)]
pub struct LocalWorkspaceController {
    config: workspace_store::workspace::OpenCodeWorkspaceConfig,
}

#[cfg(test)]
impl LocalWorkspaceController {
    pub fn new(config: workspace_store::workspace::OpenCodeWorkspaceConfig) -> Self {
        Self { config }
    }
}

#[cfg(test)]
#[async_trait]
impl WorkspaceController for LocalWorkspaceController {
    async fn create_run_workspace(
        &self,
        agent_key: &str,
        run_id: i64,
        _idempotency_key: &str,
    ) -> Result<IsolatedWorkspaceCreated> {
        let path = RunWorkspacePath::new(agent_key, run_id)?;
        create_run_workspace(&self.config, &path)
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
    async fn materialize_conversation_workspace(
        &self,
        agent_key: &str,
        conversation_id: Uuid,
        input: ConversationWorkspaceMaterializationInput,
        _idempotency_key: &str,
    ) -> Result<MaterializedConversationWorkspace> {
        let path = ConversationWorkspacePath::new(agent_key, conversation_id)?;
        workspace_store::workspace::materialize_conversation_workspace(&self.config, &path, &input)
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

    async fn materialize_conversation_workspace(
        &self,
        agent_key: &str,
        conversation_id: Uuid,
        input: ConversationWorkspaceMaterializationInput,
        idempotency_key: &str,
    ) -> Result<MaterializedConversationWorkspace> {
        let response = self
            .request(
                reqwest::Method::POST,
                &format!("v1/conversation-workspaces/{agent_key}/{conversation_id}/materialize"),
            )?
            .header("Idempotency-Key", idempotency_key)
            .json(&input)
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
}
#[derive(Deserialize)]
struct DeleteResponse {
    deleted: bool,
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

    fn test_config(root: PathBuf) -> workspace_store::workspace::OpenCodeWorkspaceConfig {
        workspace_store::workspace::OpenCodeWorkspaceConfig {
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
