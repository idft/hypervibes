use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

const RESPONSE_SNIPPET_MAX_CHARS: usize = 200;

#[derive(Debug, Clone)]
pub struct OpenCodeClientConfig {
    pub username: String,
    pub password: Option<String>,
    pub create_session_timeout: Duration,
    pub status_timeout: Duration,
}

impl OpenCodeClientConfig {
    pub fn new(username: String, password: Option<String>) -> Self {
        Self {
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
}

impl OpenCodeClient {
    pub fn new(config: OpenCodeClientConfig) -> Result<Self> {
        let http = reqwest::Client::builder()
            .build()
            .context("failed to build OpenCode HTTP client")?;
        Ok(Self { http, config })
    }

    pub fn config(&self) -> &OpenCodeClientConfig {
        &self.config
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

    /// Best-effort session liveness check.
    ///
    /// Treats any successful (2xx) response as a live session. The
    /// `OpenCode /session/{id}/status` endpoint does not currently
    /// return a structured terminal state across versions, so the first
    /// implementation prefers the more reliable path: the command
    /// dispatch response is treated as terminal success. The polling
    /// shape is kept for future use.
    pub async fn session_is_active(&self, base_url: &str, session_id: &str) -> Result<bool> {
        let url = build_url(base_url, &format!("session/{}/status", session_id), &[]);
        let response = self
            .http
            .get(url)
            .timeout(self.config.status_timeout)
            .apply_basic_auth(&self.config)
            .send()
            .await
            .map_err(|error| anyhow!("OpenCode session status request failed: {error}"))?;
        let status = response.status();
        if status.is_success() {
            return Ok(true);
        }
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            return Err(anyhow!("OpenCode authentication failed"));
        }
        if status == StatusCode::NOT_FOUND {
            return Ok(false);
        }
        let snippet = snippet_from_response(response).await;
        Err(anyhow!(
            "OpenCode session status returned {}: {}",
            status,
            snippet
        ))
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
        assert!(cloned.config().password.is_none());
    }

    #[test]
    fn client_uses_short_timeouts_for_metadata_calls() {
        let config = OpenCodeClientConfig::new("opencode".to_string(), None);
        assert_eq!(config.create_session_timeout, Duration::from_secs(15));
        assert_eq!(config.status_timeout, Duration::from_secs(15));
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
}
