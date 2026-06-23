use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use reqwest::header::{HeaderMap, HeaderValue};
use tokio::sync::RwLock;

use super::model::{ActiveProfile, ProfileList, StatusResponse};

pub const HERMES_SESSION_HEADER: &str = "X-Hermes-Session-Token";

#[derive(Debug, Clone)]
pub struct HermesHealth {
    pub reachable: bool,
    pub version: Option<String>,
}

#[derive(Clone)]
pub struct HermesClient {
    client: reqwest::Client,
    base_url: String,
    /// Optional fixed session token supplied by the operator. If set, this
    /// is used directly and the dashboard index is never scraped.
    provided_token: Option<String>,
    /// Lazily populated session token scraped from the served SPA HTML when
    /// no fixed token is configured. Stored in an `Arc<RwLock<...>>` so the
    /// client remains `Clone`.
    resolved_token: Arc<RwLock<Option<String>>>,
}

impl HermesClient {
    pub fn new(base_url: String, session_token: Option<String>) -> Result<Self> {
        let client = reqwest::Client::builder()
            .build()
            .context("failed to build Hermes HTTP client")?;
        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            provided_token: session_token.clone(),
            resolved_token: Arc::new(RwLock::new(session_token)),
        })
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Return the active dashboard session token, resolving it lazily if
    /// necessary. In loopback / ``--insecure`` mode the token is injected
    /// into ``index.html``; we fetch that page and cache the value. In gated
    /// (OAuth) mode a fixed token must be supplied via
    /// ``HERMES_DASHBOARD_SESSION_TOKEN``.
    async fn session_token(&self) -> Result<String> {
        if let Some(token) = &self.provided_token {
            return Ok(token.clone());
        }

        {
            let cached = self.resolved_token.read().await;
            if let Some(token) = cached.as_ref() {
                return Ok(token.clone());
            }
        }

        let token = self.scrape_session_token().await?;
        let mut cached = self.resolved_token.write().await;
        *cached = Some(token.clone());
        Ok(token)
    }

    async fn scrape_session_token(&self) -> Result<String> {
        let url = format!("{}/", self.base_url);
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| anyhow!("failed to fetch Hermes dashboard index at {url}: {e}"))?;

        if !response.status().is_success() {
            return Err(anyhow!("GET {url} returned status {}", response.status()));
        }

        let text = response
            .text()
            .await
            .map_err(|e| anyhow!("failed to read Hermes dashboard index: {e}"))?;

        const MARKER: &str = "window.__HERMES_SESSION_TOKEN__=\"";
        let start = text.find(MARKER).ok_or_else(|| {
            anyhow!(
                "Hermes dashboard index did not contain a session token. \
                 If the dashboard is running in gated/OAuth mode, set HERMES_DASHBOARD_SESSION_TOKEN."
            )
        })?;
        let rest = &text[start + MARKER.len()..];
        let end = rest.find('"').ok_or_else(|| {
            anyhow!("malformed session token injection in Hermes dashboard index")
        })?;
        let token = &rest[..end];
        if token.is_empty() {
            return Err(anyhow!("Hermes dashboard returned an empty session token"));
        }
        Ok(token.to_string())
    }

    async fn auth_headers(&self) -> Result<HeaderMap> {
        let token = self.session_token().await?;
        let mut headers = HeaderMap::new();
        let value = HeaderValue::from_str(&token).context("invalid Hermes session token")?;
        headers.insert(HERMES_SESSION_HEADER, value);
        Ok(headers)
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    /// Probe Hermes by calling the public ``/api/status`` endpoint. Returns
    /// ``HermesHealth { reachable: false }`` when the dashboard is not
    /// reachable, so the UI can render "disconnected" without surfacing an
    /// error as a hard failure.
    pub async fn health(&self) -> Result<HermesHealth> {
        let url = self.url("/api/status");
        let response = match self.client.get(&url).send().await {
            Ok(resp) => resp,
            Err(_) => {
                return Ok(HermesHealth {
                    reachable: false,
                    version: None,
                });
            }
        };

        if !response.status().is_success() {
            return Ok(HermesHealth {
                reachable: false,
                version: None,
            });
        }

        match response.json::<StatusResponse>().await {
            Ok(status) => Ok(HermesHealth {
                reachable: true,
                version: Some(status.version),
            }),
            Err(_) => Ok(HermesHealth {
                reachable: true,
                version: None,
            }),
        }
    }

    pub async fn list_profiles(&self) -> Result<Vec<super::model::Profile>> {
        let url = self.url("/api/profiles");
        let response = self
            .client
            .get(&url)
            .headers(self.auth_headers().await?)
            .send()
            .await
            .map_err(|e| anyhow!("failed to GET {url}: {e}"))?;

        if !response.status().is_success() {
            return Err(anyhow!("GET {url} returned status {}", response.status()));
        }

        let body: ProfileList = response
            .json()
            .await
            .map_err(|e| anyhow!("failed to decode profiles response: {e}"))?;
        Ok(body.profiles)
    }

    pub async fn get_active_profile(&self) -> Result<Option<ActiveProfile>> {
        let url = self.url("/api/profiles/active");
        let response = self
            .client
            .get(&url)
            .headers(self.auth_headers().await?)
            .send()
            .await
            .map_err(|e| anyhow!("failed to GET {url}: {e}"))?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }

        if !response.status().is_success() {
            return Err(anyhow!("GET {url} returned status {}", response.status()));
        }

        let body: ActiveProfile = response
            .json()
            .await
            .map_err(|e| anyhow!("failed to decode active profile response: {e}"))?;
        Ok(Some(body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_builds_without_token() {
        let client = HermesClient::new("http://127.0.0.1:19119".to_string(), None);
        assert!(client.is_ok());
    }

    #[test]
    fn client_builds_with_token() {
        let client = HermesClient::new(
            "http://127.0.0.1:19119".to_string(),
            Some("abc123".to_string()),
        );
        assert!(client.is_ok());
    }

    #[test]
    fn extracts_injected_session_token() {
        let html = r#"<!DOCTYPE html><html><head><script>window.__HERMES_SESSION_TOKEN__="s3cr3t-token";window.__HERMES_BASE_PATH__="";</script></head><body></body></html>"#;
        const MARKER: &str = "window.__HERMES_SESSION_TOKEN__=\"";
        let start = html.find(MARKER).unwrap();
        let rest = &html[start + MARKER.len()..];
        let end = rest.find('"').unwrap();
        let token = &rest[..end];
        assert_eq!(token, "s3cr3t-token");
    }
}
