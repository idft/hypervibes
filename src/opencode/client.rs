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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStatusKind {
    Idle,
    Busy,
    Retry,
}

impl SessionStatusKind {
    pub fn is_active(self) -> bool {
        matches!(self, SessionStatusKind::Busy | SessionStatusKind::Retry)
    }
}

/// Deserialized entry from `/session/status` -- a tagged union with the
/// kind string (`idle`/`busy`/`retry`) at `type`. Only the `type` field
/// is consulted; retry-specific details are dropped.
#[derive(Debug, Clone, Deserialize)]
struct SessionStatusResponse {
    #[serde(rename = "type")]
    kind: String,
}

impl SessionStatusResponse {
    fn into_kind(self) -> Result<SessionStatusKind> {
        match self.kind.as_str() {
            "idle" => Ok(SessionStatusKind::Idle),
            "busy" => Ok(SessionStatusKind::Busy),
            "retry" => Ok(SessionStatusKind::Retry),
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
        let url = build_url(base_url, "session/status", &[]);
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
        let response = parse_opencode_response(response).await?;
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
}
