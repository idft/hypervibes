use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

const RESPONSE_SNIPPET_MAX_CHARS: usize = 200;
const PROVIDER_CACHE_TTL: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone)]
pub struct OpenCodeClientConfig {
    pub base_url: String,
    pub username: String,
    pub password: Option<String>,
    pub create_session_timeout: Duration,
    pub status_timeout: Duration,
    pub oauth_callback_timeout: Duration,
}

impl OpenCodeClientConfig {
    #[cfg(test)]
    pub fn new(username: String, password: Option<String>) -> Self {
        Self::new_with_base_url(username, password, "http://localhost:14096".to_string())
    }

    pub fn new_with_base_url(username: String, password: Option<String>, base_url: String) -> Self {
        Self {
            base_url,
            username,
            password,
            create_session_timeout: Duration::from_secs(15),
            status_timeout: Duration::from_secs(15),
            oauth_callback_timeout: Duration::from_secs(15 * 60),
        }
    }
}

#[derive(Clone)]
pub struct OpenCodeClient {
    http: reqwest::Client,
    config: OpenCodeClientConfig,
    // OpenCode provider configuration belongs to the shared backend, not to an
    // individual agent workspace. The workspace directory is only required to
    // make the initial API request.
    provider_cache: Arc<RwLock<HashMap<String, CachedProvidersResponse>>>,
}

struct CachedProvidersResponse {
    fetched_at: Instant,
    response: OpenCodeProvidersResponse,
}

/// Session lifecycle status returned by OpenCode's
/// `/session/status` endpoint. The value reflects whether the session
/// is actively executing (`Busy`/`Retry`) or idle (`Idle`).
///
/// Vibetrading treats `Idle` as terminal for the purposes of run
/// cancellation; a dispatch that has timed out keeps the underlying
/// `agentic_runs` row in `running` until a probe confirms `Idle`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionStatusKind {
    Idle,
    Busy,
    Retry { message: String },
}

impl SessionStatusKind {
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            SessionStatusKind::Busy | SessionStatusKind::Retry { .. }
        )
    }
}

/// Deserialized entry from `/session/status` -- a tagged union with the
/// kind string (`idle`/`busy`/`retry`) at `type`. Only the `type` field
/// is consulted; retry-specific details are dropped.
#[derive(Debug, Clone, Deserialize)]
struct SessionStatusResponse {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    message: String,
}

impl SessionStatusResponse {
    fn into_kind(self) -> Result<SessionStatusKind> {
        match self.kind.as_str() {
            "idle" => Ok(SessionStatusKind::Idle),
            "busy" => Ok(SessionStatusKind::Busy),
            "retry" => Ok(SessionStatusKind::Retry {
                message: self.message,
            }),
            other => Err(anyhow!("unknown session status: {other}")),
        }
    }
}

impl OpenCodeClient {
    pub fn new(config: OpenCodeClientConfig) -> Result<Self> {
        let http = reqwest::Client::builder()
            .build()
            .context("failed to build OpenCode HTTP client")?;
        Ok(Self {
            http,
            config,
            provider_cache: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    pub fn base_url(&self) -> &str {
        &self.config.base_url
    }

    /// Create a new session bound to the given workspace container path.
    pub async fn create_session(
        &self,
        base_url: &str,
        workspace_container_path: &str,
        title: Option<&str>,
    ) -> Result<OpenCodeSession> {
        let url = build_url(
            base_url,
            "session",
            &[("directory", workspace_container_path)],
        );
        let body = match title {
            Some(title) => serde_json::json!({ "title": title }),
            None => serde_json::json!({}),
        };
        let response = self
            .http
            .post(url)
            .timeout(self.config.create_session_timeout)
            .apply_basic_auth(&self.config)
            .json(&body)
            .send()
            .await
            .map_err(|error| anyhow!("OpenCode create_session request failed: {error}"))?;
        let response = parse_opencode_response(response).await?;
        let session: OpenCodeSession = response
            .json()
            .await
            .context("failed to decode OpenCode create_session response")?;
        Ok(session)
    }

    /// Run a slash command against an existing session.
    ///
    /// For the current OpenCode integration, this HTTP request blocks until the
    /// command finishes. Vibetrading therefore marks the run succeeded only
    /// after this request returns successfully.
    pub async fn run_command(
        &self,
        base_url: &str,
        session_id: &str,
        request: OpenCodeCommandRequest,
    ) -> Result<()> {
        let url = build_url(base_url, &format!("session/{}/command", session_id), &[]);
        let response = self
            .http
            .post(url)
            .apply_basic_auth(&self.config)
            .json(&request)
            .send()
            .await
            .map_err(|error| anyhow!("OpenCode run_command request failed: {error}"))?;
        let _ = parse_opencode_response(response).await?;
        Ok(())
    }

    /// Abort an in-flight OpenCode session. The OpenCode `1.17.11`
    /// HTTP API is `POST /session/{sessionID}/abort` which returns a
    /// boolean indicating that the abort signal was accepted. We
    /// additionally surface `false` for HTTP 4xx/5xx responses (which
    /// are reported via [`parse_opencode_response`] errors).
    pub async fn abort_session(&self, base_url: &str, session_id: &str) -> Result<bool> {
        let url = build_url(base_url, &format!("session/{}/abort", session_id), &[]);
        let response = self
            .http
            .post(url)
            .timeout(self.config.status_timeout)
            .apply_basic_auth(&self.config)
            .json(&serde_json::json!({}))
            .send()
            .await
            .map_err(|error| anyhow!("OpenCode abort_session request failed: {error}"))?;
        let response = parse_opencode_response(response).await?;
        let aborted: bool = response
            .json()
            .await
            .context("failed to decode OpenCode abort_session response")?;
        Ok(aborted)
    }

    /// Probe the live status of a single OpenCode session by querying
    /// `/session/status` (which returns a map keyed by session ID) and
    /// looking up `session_id`. Returns `Ok(None)` when the session is
    /// not present in the response -- typically because it has been
    /// forgotten by OpenCode and should be treated as terminal.
    pub async fn get_session_status(
        &self,
        base_url: &str,
        session_id: &str,
    ) -> Result<Option<SessionStatusKind>> {
        self.get_session_status_in_directory(base_url, session_id, None)
            .await
    }

    pub async fn get_session_status_in_directory(
        &self,
        base_url: &str,
        session_id: &str,
        workspace_container_path: Option<&str>,
    ) -> Result<Option<SessionStatusKind>> {
        let query = workspace_container_path
            .map(|directory| vec![("directory", directory)])
            .unwrap_or_default();
        let url = build_url(base_url, "session/status", &query);
        let response = self
            .http
            .get(url)
            .timeout(self.config.status_timeout)
            .apply_basic_auth(&self.config)
            .send()
            .await
            .map_err(|error| anyhow!("OpenCode get_session_status request failed: {error}"))?;
        let response = parse_opencode_response(response).await?;
        let statuses: HashMap<String, SessionStatusResponse> = response
            .json()
            .await
            .context("failed to decode OpenCode session status response")?;
        statuses
            .get(session_id)
            .cloned()
            .map(SessionStatusResponse::into_kind)
            .transpose()
    }

    pub async fn list_providers(
        &self,
        base_url: &str,
        workspace_container_path: &str,
    ) -> Result<OpenCodeProvidersResponse> {
        let cache_key = base_url.trim_end_matches('/').to_string();
        if let Some(response) = self
            .provider_cache
            .read()
            .await
            .get(&cache_key)
            .filter(|entry| entry.fetched_at.elapsed() < PROVIDER_CACHE_TTL)
            .map(|entry| entry.response.clone())
        {
            return Ok(response);
        }

        let url = build_url(
            base_url,
            "provider",
            &[("directory", workspace_container_path)],
        );
        let response = self
            .http
            .get(url)
            .timeout(self.config.status_timeout)
            .apply_basic_auth(&self.config)
            .send()
            .await
            .map_err(|error| anyhow!("OpenCode list_providers request failed: {error}"))?;
        let response = parse_status_response(response, "provider discovery").await?;
        let response: OpenCodeProvidersResponse = response
            .json()
            .await
            .context("failed to decode OpenCode provider discovery response")?;
        let mut cache = self.provider_cache.write().await;
        cache.retain(|_, entry| entry.fetched_at.elapsed() < PROVIDER_CACHE_TTL);
        cache.insert(
            cache_key,
            CachedProvidersResponse {
                fetched_at: Instant::now(),
                response: response.clone(),
            },
        );
        Ok(response)
    }

    pub async fn list_provider_auth_methods(
        &self,
        base_url: &str,
        directory: &str,
    ) -> Result<HashMap<String, Vec<OpenCodeProviderAuthMethod>>> {
        let url = build_url(base_url, "provider/auth", &[("directory", directory)]);
        let response = self
            .http
            .get(url)
            .timeout(self.config.status_timeout)
            .apply_basic_auth(&self.config)
            .send()
            .await
            .map_err(|error| anyhow!("OpenCode provider auth discovery request failed: {error}"))?;
        let response = parse_status_response(response, "provider auth discovery").await?;
        response
            .json()
            .await
            .context("failed to decode OpenCode provider auth methods response")
    }

    pub async fn authorize_provider_oauth(
        &self,
        base_url: &str,
        directory: &str,
        provider_id: &str,
        method: usize,
        inputs: BTreeMap<String, String>,
    ) -> Result<OpenCodeOAuthAuthorization> {
        let url = build_url(
            base_url,
            &format!("provider/{}/oauth/authorize", percent_encode(provider_id)),
            &[("directory", directory)],
        );
        let body = OpenCodeOAuthAuthorizeRequest {
            method,
            inputs: (!inputs.is_empty()).then_some(inputs),
        };
        let response = self
            .http
            .post(url)
            .timeout(self.config.status_timeout)
            .apply_basic_auth(&self.config)
            .json(&body)
            .send()
            .await
            .map_err(|error| anyhow!("OpenCode OAuth authorization request failed: {error}"))?;
        let response = parse_credential_response(response, "OAuth authorization").await?;
        response
            .json()
            .await
            .context("failed to decode OpenCode OAuth authorization response")
    }

    pub async fn complete_provider_oauth(
        &self,
        base_url: &str,
        directory: &str,
        provider_id: &str,
        method: usize,
        code: Option<&str>,
    ) -> Result<()> {
        let url = build_url(
            base_url,
            &format!("provider/{}/oauth/callback", percent_encode(provider_id)),
            &[("directory", directory)],
        );
        let body = OpenCodeOAuthCallbackRequest {
            method,
            code: code.map(ToOwned::to_owned),
        };
        let response = self
            .http
            .post(url)
            .timeout(self.config.oauth_callback_timeout)
            .apply_basic_auth(&self.config)
            .json(&body)
            .send()
            .await
            .map_err(|error| anyhow!("OpenCode OAuth callback request failed: {error}"))?;
        let _ = parse_credential_response(response, "OAuth callback").await?;
        Ok(())
    }

    pub async fn set_provider_api_auth(
        &self,
        base_url: &str,
        provider_id: &str,
        api_key: &str,
        metadata: BTreeMap<String, String>,
    ) -> Result<()> {
        let url = build_url(
            base_url,
            &format!("auth/{}", percent_encode(provider_id)),
            &[],
        );
        let body = OpenCodeApiAuthRequest {
            auth_type: "api",
            key: api_key,
            metadata: (!metadata.is_empty()).then_some(metadata),
        };
        let response = self
            .http
            .put(url)
            .timeout(self.config.status_timeout)
            .apply_basic_auth(&self.config)
            .json(&body)
            .send()
            .await
            .map_err(|error| anyhow!("OpenCode API credential request failed: {error}"))?;
        let _ = parse_credential_response(response, "API credential storage").await?;
        Ok(())
    }

    pub async fn remove_provider_auth(&self, base_url: &str, provider_id: &str) -> Result<()> {
        let url = build_url(
            base_url,
            &format!("auth/{}", percent_encode(provider_id)),
            &[],
        );
        let response = self
            .http
            .delete(url)
            .timeout(self.config.status_timeout)
            .apply_basic_auth(&self.config)
            .send()
            .await
            .map_err(|error| anyhow!("OpenCode credential removal request failed: {error}"))?;
        let _ = parse_credential_response(response, "credential removal").await?;
        Ok(())
    }

    pub async fn invalidate_provider_cache(&self) {
        self.provider_cache.write().await.clear();
    }

    /// Dispose all OpenCode instances, invalidating the in-memory
    /// provider/runtime cache so newly stored credentials are reflected
    /// in the next `/provider` response. This is the HTTP equivalent of
    /// the TUI's `global.dispose` call after `auth.set`. It interrupts
    /// in-flight OpenCode sessions, so callers should ensure no agent
    /// work is active before invoking it.
    pub async fn dispose_instances(&self, base_url: &str) -> Result<()> {
        let url = build_url(base_url, "global/dispose", &[]);
        let response = self
            .http
            .post(url)
            .timeout(self.config.status_timeout)
            .apply_basic_auth(&self.config)
            .send()
            .await
            .map_err(|error| anyhow!("OpenCode dispose request failed: {error}"))?;
        let _ = parse_credential_response(response, "instance dispose").await?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenCodeSession {
    pub id: String,
    #[serde(default)]
    pub directory: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OpenCodeCommandRequest {
    pub command: String,
    pub arguments: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpenCodeProvidersResponse {
    #[serde(default)]
    pub all: Vec<OpenCodeProviderInfo>,
    #[serde(default)]
    pub connected: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpenCodeProviderInfo {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub models: BTreeMap<String, OpenCodeModelInfo>,
    #[serde(default)]
    pub env: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpenCodeProviderAuthMethod {
    #[serde(rename = "type")]
    pub auth_type: String,
    pub label: String,
    #[serde(default)]
    pub prompts: Vec<OpenCodeProviderAuthPrompt>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(tag = "type")]
pub enum OpenCodeProviderAuthPrompt {
    #[serde(rename = "text")]
    Text {
        key: String,
        message: String,
        #[serde(default)]
        placeholder: Option<String>,
        #[serde(default)]
        when: Option<OpenCodeProviderAuthWhen>,
    },
    #[serde(rename = "select")]
    Select {
        key: String,
        message: String,
        #[serde(default)]
        options: Vec<OpenCodeProviderAuthOption>,
        #[serde(default)]
        when: Option<OpenCodeProviderAuthWhen>,
    },
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct OpenCodeProviderAuthOption {
    pub label: String,
    pub value: String,
    #[serde(default)]
    pub hint: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct OpenCodeProviderAuthWhen {
    pub key: String,
    pub op: OpenCodeProviderAuthWhenOp,
    pub value: String,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OpenCodeProviderAuthWhenOp {
    Eq,
    Neq,
}

#[derive(Clone, Deserialize)]
pub struct OpenCodeOAuthAuthorization {
    pub url: String,
    #[serde(rename = "method")]
    pub completion_mode: OpenCodeOAuthCompletionMode,
    pub instructions: String,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OpenCodeOAuthCompletionMode {
    Auto,
    Code,
}

#[derive(Serialize)]
struct OpenCodeOAuthAuthorizeRequest {
    method: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    inputs: Option<BTreeMap<String, String>>,
}

#[derive(Serialize)]
struct OpenCodeOAuthCallbackRequest {
    method: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<String>,
}

#[derive(Serialize)]
struct OpenCodeApiAuthRequest<'a> {
    #[serde(rename = "type")]
    auth_type: &'static str,
    key: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpenCodeModelInfo {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

trait ApplyBasicAuth {
    fn apply_basic_auth(self, config: &OpenCodeClientConfig) -> Self;
}

impl ApplyBasicAuth for reqwest::RequestBuilder {
    fn apply_basic_auth(self, config: &OpenCodeClientConfig) -> reqwest::RequestBuilder {
        match &config.password {
            Some(password) => self.basic_auth(&config.username, Some(password)),
            None => self,
        }
    }
}

fn build_url(base_url: &str, path: &str, query: &[(&str, &str)]) -> String {
    let trimmed = base_url.trim_end_matches('/');
    let mut url = format!("{trimmed}/{path}");
    if !query.is_empty() {
        url.push('?');
        for (index, (key, value)) in query.iter().enumerate() {
            if index > 0 {
                url.push('&');
            }
            url.push_str(key);
            url.push('=');
            url.push_str(&percent_encode(value));
        }
    }
    url
}

fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(*byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

async fn parse_opencode_response(response: reqwest::Response) -> Result<reqwest::Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let snippet = snippet_from_response(response).await;
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return Err(anyhow!("OpenCode authentication failed: {}", snippet));
    }
    Err(anyhow!("OpenCode request failed ({}): {}", status, snippet))
}

async fn parse_credential_response(
    response: reqwest::Response,
    operation: &str,
) -> Result<reqwest::Response> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return Err(anyhow!("OpenCode authentication failed during {operation}"));
    }
    Err(anyhow!("OpenCode {operation} failed with status {status}"))
}

async fn parse_status_response(
    response: reqwest::Response,
    operation: &str,
) -> Result<reqwest::Response> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return Err(anyhow!("OpenCode authentication failed during {operation}"));
    }
    Err(anyhow!("OpenCode {operation} failed with status {status}"))
}

async fn snippet_from_response(response: reqwest::Response) -> String {
    let status = response.status();
    let url = response.url().to_string();
    let body = response.text().await.unwrap_or_default();
    let snippet: String = body.chars().take(RESPONSE_SNIPPET_MAX_CHARS).collect();
    if body.chars().count() > RESPONSE_SNIPPET_MAX_CHARS {
        format!(
            "{status} for {url} body={}…",
            snippet.replace('\n', " ").trim()
        )
    } else {
        format!(
            "{status} for {url} body={}",
            snippet.replace('\n', " ").trim()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_status_retry_retains_provider_message() {
        let status: SessionStatusResponse =
            serde_json::from_str(r#"{"type":"retry","message":"You exceeded your current quota"}"#)
                .expect("parse retry status");
        assert_eq!(
            status.into_kind().expect("status kind"),
            SessionStatusKind::Retry {
                message: "You exceeded your current quota".to_string(),
            }
        );
    }

    #[tokio::test]
    async fn session_status_uses_workspace_directory() {
        use std::sync::Mutex;

        use axum::{Json, Router, extract::Query, routing::get};

        let directory = Arc::new(Mutex::new(None));
        let app = Router::new().route(
            "/session/status",
            get({
                let directory = Arc::clone(&directory);
                move |Query(query): Query<HashMap<String, String>>| {
                    let directory = Arc::clone(&directory);
                    async move {
                        *directory.lock().expect("lock directory") =
                            query.get("directory").cloned();
                        Json(serde_json::json!({
                            "ses_test": {
                                "type": "retry",
                                "message": "You exceeded your current quota"
                            }
                        }))
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind status test server");
        let base_url = format!("http://{}", listener.local_addr().expect("local address"));
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve status test server");
        });
        let client = OpenCodeClient::new(OpenCodeClientConfig::new("opencode".to_string(), None))
            .expect("client");

        let status = client
            .get_session_status_in_directory(
                &base_url,
                "ses_test",
                Some("/workspaces/coding/btc-1/7/workspace"),
            )
            .await
            .expect("get status");

        assert_eq!(
            status,
            Some(SessionStatusKind::Retry {
                message: "You exceeded your current quota".to_string(),
            })
        );
        assert_eq!(
            directory.lock().expect("lock directory").as_deref(),
            Some("/workspaces/coding/btc-1/7/workspace")
        );
        server.abort();
    }

    #[test]
    fn build_url_trims_trailing_slash_and_encodes_directory() {
        let url = build_url(
            "http://localhost:14096/",
            "session",
            &[("directory", "/workspaces/agents/btc 2")],
        );
        assert_eq!(
            url,
            "http://localhost:14096/session?directory=%2Fworkspaces%2Fagents%2Fbtc%202"
        );
    }

    #[test]
    fn build_url_with_no_query_drops_question_mark() {
        let url = build_url("http://localhost:14096", "session/abc/command", &[]);
        assert_eq!(url, "http://localhost:14096/session/abc/command");
    }

    #[test]
    fn snippet_truncates_long_bodies() {
        let mut body = String::with_capacity(RESPONSE_SNIPPET_MAX_CHARS * 2);
        for _ in 0..(RESPONSE_SNIPPET_MAX_CHARS * 2) {
            body.push('a');
        }
        let snippet: String = body.chars().take(RESPONSE_SNIPPET_MAX_CHARS).collect();
        assert_eq!(snippet.chars().count(), RESPONSE_SNIPPET_MAX_CHARS);
    }

    #[test]
    fn client_is_clone_and_basic_auth_omitted_when_password_missing() {
        let config = OpenCodeClientConfig::new("opencode".to_string(), None);
        let client = OpenCodeClient::new(config).expect("client");
        let cloned = client.clone();
        assert!(cloned.config.password.is_none());
    }

    #[test]
    fn client_uses_short_timeouts_for_metadata_calls() {
        let config = OpenCodeClientConfig::new("opencode".to_string(), None);
        assert_eq!(config.create_session_timeout, Duration::from_secs(15));
        assert_eq!(config.status_timeout, Duration::from_secs(15));
    }

    #[tokio::test]
    async fn list_providers_reuses_cached_runtime_response() {
        let client = OpenCodeClient::new(OpenCodeClientConfig::new("opencode".to_string(), None))
            .expect("client");
        let cache_key = "http://127.0.0.1:1".to_string();
        client.provider_cache.write().await.insert(
            cache_key,
            CachedProvidersResponse {
                fetched_at: Instant::now(),
                response: OpenCodeProvidersResponse {
                    all: vec![OpenCodeProviderInfo {
                        id: "anthropic".to_string(),
                        name: Some("Anthropic".to_string()),
                        models: BTreeMap::new(),
                        env: Vec::new(),
                    }],
                    connected: vec!["anthropic".to_string()],
                },
            },
        );

        let response = client
            .list_providers("http://127.0.0.1:1/", "/workspaces/agents/btc-1")
            .await
            .expect("cached response");

        assert_eq!(response.connected, vec!["anthropic"]);
    }

    #[tokio::test]
    async fn list_providers_shares_one_cache_across_workspaces() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        use axum::{Json, Router, routing::get};

        let requests = Arc::new(AtomicUsize::new(0));
        let app = Router::new().route(
            "/provider",
            get({
                let requests = Arc::clone(&requests);
                move || {
                    let requests = Arc::clone(&requests);
                    async move {
                        requests.fetch_add(1, Ordering::Relaxed);
                        Json(serde_json::json!({
                            "all": [],
                            "connected": []
                        }))
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind provider test server");
        let base_url = format!("http://{}", listener.local_addr().expect("local address"));
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve provider test server");
        });
        let client = OpenCodeClient::new(OpenCodeClientConfig::new("opencode".to_string(), None))
            .expect("client");

        client
            .list_providers(&base_url, "/workspaces/agents/btc")
            .await
            .expect("first provider request");
        client
            .list_providers(&base_url, "/workspaces/agents/eth")
            .await
            .expect("cached provider request");

        assert_eq!(requests.load(Ordering::Relaxed), 1);
        server.abort();
    }

    #[test]
    fn command_request_serializes_model_as_provider_and_model_string() {
        let request = OpenCodeCommandRequest {
            command: "vibetrading-analysis".to_string(),
            arguments: "Agent key: btc-2".to_string(),
            agent: Some("analysis".to_string()),
            model: Some("anthropic/claude-sonnet-4".to_string()),
        };

        let json = serde_json::to_value(&request).expect("serialize command request");

        assert_eq!(json["model"], "anthropic/claude-sonnet-4");
        assert!(json.get("provider_id").is_none());
        assert!(json.get("model_id").is_none());
    }

    #[test]
    fn provider_response_parses_models_and_connected_providers() {
        let response: OpenCodeProvidersResponse = serde_json::from_str(
            r#"{
                "all": [{
                    "id": "anthropic",
                    "name": "Anthropic",
                    "models": {
                        "claude-sonnet-4": { "name": "Claude Sonnet 4" }
                    }
                }],
                "connected": ["anthropic"],
                "default": { "analysis": "anthropic/claude-sonnet-4" }
            }"#,
        )
        .unwrap();

        assert_eq!(response.connected, vec!["anthropic"]);
        assert_eq!(
            response.all[0].models["claude-sonnet-4"].name.as_deref(),
            Some("Claude Sonnet 4")
        );
    }

    #[test]
    fn provider_auth_methods_deserialize_safe_dynamic_prompts() {
        let methods: HashMap<String, Vec<OpenCodeProviderAuthMethod>> = serde_json::from_str(
            r#"{
                "provider": [{
                    "type": "api",
                    "label": "API key",
                    "prompts": [
                        {"type":"text","key":"account","message":"Account","when":{"key":"mode","op":"eq","value":"team"}},
                        {"type":"select","key":"mode","message":"Mode","options":[{"label":"Team","value":"team","hint":"shared"}]}
                    ],
                    "secret": "must not be modeled"
                }]
            }"#,
        )
        .expect("provider auth methods");

        assert_eq!(methods["provider"][0].auth_type, "api");
        assert_eq!(methods["provider"][0].prompts.len(), 2);
        assert_eq!(
            methods["provider"][0].prompts[0],
            OpenCodeProviderAuthPrompt::Text {
                key: "account".to_string(),
                message: "Account".to_string(),
                placeholder: None,
                when: Some(OpenCodeProviderAuthWhen {
                    key: "mode".to_string(),
                    op: OpenCodeProviderAuthWhenOp::Eq,
                    value: "team".to_string(),
                }),
            }
        );
    }

    #[tokio::test]
    async fn invalidate_provider_cache_forces_the_next_discovery() {
        let client = OpenCodeClient::new(OpenCodeClientConfig::new("opencode".to_string(), None))
            .expect("client");
        client.provider_cache.write().await.insert(
            "http://example.test".to_string(),
            CachedProvidersResponse {
                fetched_at: Instant::now(),
                response: OpenCodeProvidersResponse {
                    all: Vec::new(),
                    connected: Vec::new(),
                },
            },
        );

        client.invalidate_provider_cache().await;

        assert!(client.provider_cache.read().await.is_empty());
    }
}
