use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use reqwest::{
    StatusCode,
    header::{ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED},
};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::warn;

const MODELS_DEV_API_URL: &str = "https://models.dev/api.json";
const CATALOG_TTL: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Clone, Deserialize)]
pub struct ModelsDevProvider {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub env: Vec<String>,
    #[serde(default)]
    pub models: BTreeMap<String, ModelsDevModel>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelsDevModel {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub reasoning: Option<bool>,
    #[serde(default, rename = "tool_call")]
    pub tool_call: Option<bool>,
    #[serde(default)]
    pub limit: Option<ModelsDevLimit>,
    #[serde(default)]
    pub cost: Option<ModelsDevCost>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelsDevLimit {
    #[serde(default, alias = "context")]
    pub context_window: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelsDevCost {
    #[serde(default)]
    pub input: Option<f64>,
    #[serde(default)]
    pub output: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelsDevCatalogMeta {
    pub fetched_at: DateTime<Utc>,
    #[serde(default)]
    pub etag: Option<String>,
    #[serde(default)]
    pub last_modified: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ModelsDevCatalogSnapshot {
    pub providers: BTreeMap<String, ModelsDevProvider>,
    pub fetched_at: DateTime<Utc>,
    pub stale: bool,
}

struct ModelsDevCatalogInner {
    cache_dir: PathBuf,
    http: reqwest::Client,
    api_url: String,
    state: RwLock<Option<ModelsDevCatalogSnapshot>>,
    meta: RwLock<Option<ModelsDevCatalogMeta>>,
    refresh_in_flight: AtomicBool,
}

#[derive(Clone)]
pub struct ModelsDevCatalog {
    inner: Arc<ModelsDevCatalogInner>,
}

impl ModelsDevCatalog {
    pub fn new(cache_dir: PathBuf) -> Result<Self> {
        Self::new_with_api_url(cache_dir, MODELS_DEV_API_URL.to_string())
    }

    pub fn shared(cache_dir: PathBuf) -> Result<Arc<Self>> {
        Ok(Arc::new(Self::new(cache_dir)?))
    }

    pub fn new_with_api_url(cache_dir: PathBuf, api_url: String) -> Result<Self> {
        let http = reqwest::Client::builder()
            .build()
            .context("failed to build models.dev HTTP client")?;
        Ok(Self {
            inner: Arc::new(ModelsDevCatalogInner {
                cache_dir,
                http,
                api_url,
                state: RwLock::new(None),
                meta: RwLock::new(None),
                refresh_in_flight: AtomicBool::new(false),
            }),
        })
    }

    pub async fn snapshot(&self) -> Result<ModelsDevCatalogSnapshot> {
        {
            let state = self.inner.state.read().await;
            if let Some(snapshot) = state.as_ref() {
                if snapshot_is_fresh(snapshot) {
                    return Ok(snapshot.clone());
                }

                let mut stale_snapshot = snapshot.clone();
                stale_snapshot.stale = true;
                drop(state);
                self.spawn_refresh_if_needed();
                return Ok(stale_snapshot);
            }
        }

        let disk_snapshot = self.load_snapshot_from_disk().await?;
        if let Some((mut snapshot, meta)) = disk_snapshot {
            snapshot.stale = !snapshot_is_fresh(&snapshot);
            let mut state = self.inner.state.write().await;
            *state = Some(snapshot.clone());
            drop(state);
            let mut cached_meta = self.inner.meta.write().await;
            *cached_meta = Some(meta);
            drop(cached_meta);
            if snapshot.stale {
                self.spawn_refresh_if_needed();
            }
            return Ok(snapshot);
        }

        let snapshot = self.fetch_snapshot().await?;
        let mut state = self.inner.state.write().await;
        *state = Some(snapshot.clone());
        Ok(snapshot)
    }

    pub async fn get_provider(&self, provider_id: &str) -> Option<ModelsDevProvider> {
        self.snapshot()
            .await
            .ok()?
            .providers
            .get(provider_id)
            .cloned()
    }

    pub async fn get_model(&self, provider_id: &str, model_id: &str) -> Option<ModelsDevModel> {
        self.snapshot()
            .await
            .ok()?
            .providers
            .get(provider_id)?
            .models
            .get(model_id)
            .cloned()
    }

    async fn fetch_snapshot(&self) -> Result<ModelsDevCatalogSnapshot> {
        let request = self.inner.http.get(&self.inner.api_url);
        let request = if let Some(meta) = self.inner.meta.read().await.clone() {
            let request = match meta.etag.as_deref() {
                Some(etag) if !etag.trim().is_empty() => request.header(IF_NONE_MATCH, etag),
                _ => request,
            };
            match meta.last_modified.as_deref() {
                Some(last_modified) if !last_modified.trim().is_empty() => {
                    request.header(IF_MODIFIED_SINCE, last_modified)
                }
                _ => request,
            }
        } else {
            request
        };
        let response = request
            .send()
            .await
            .map_err(|error| anyhow!("models.dev catalog fetch failed: {error}"))?;
        if response.status() == StatusCode::NOT_MODIFIED {
            return self.handle_not_modified_response(response).await;
        }
        if !response.status().is_success() {
            return Err(anyhow!(
                "models.dev catalog fetch failed with status {}",
                response.status()
            ));
        }

        let etag = header_value_to_string(response.headers().get(ETAG));
        let last_modified = header_value_to_string(response.headers().get(LAST_MODIFIED));
        let bytes = response
            .bytes()
            .await
            .context("failed to read models.dev catalog response")?;
        let providers = parse_catalog_bytes(&bytes)?;
        let fetched_at = Utc::now();
        let snapshot = ModelsDevCatalogSnapshot {
            providers,
            fetched_at,
            stale: false,
        };
        let meta = ModelsDevCatalogMeta {
            fetched_at,
            etag,
            last_modified,
        };
        self.write_snapshot_to_disk(&snapshot, &meta, &bytes).await?;
        let mut cached_meta = self.inner.meta.write().await;
        *cached_meta = Some(meta);
        Ok(snapshot)
    }

    async fn load_snapshot_from_disk(
        &self,
    ) -> Result<Option<(ModelsDevCatalogSnapshot, ModelsDevCatalogMeta)>> {
        let api_path = self.inner.cache_dir.join("models-dev/api.json");
        let meta_path = self.inner.cache_dir.join("models-dev/api.meta.json");

        let bytes = match tokio::fs::read(&api_path).await {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to read {}", api_path.display()));
            }
        };

        let providers = parse_catalog_bytes(&bytes)?;
        let meta = match tokio::fs::read(&meta_path).await {
            Ok(meta_bytes) => serde_json::from_slice::<ModelsDevCatalogMeta>(&meta_bytes)
                .unwrap_or_else(|_| ModelsDevCatalogMeta {
                    fetched_at: Utc::now(),
                    etag: None,
                    last_modified: None,
                }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => ModelsDevCatalogMeta {
                fetched_at: Utc::now(),
                etag: None,
                last_modified: None,
            },
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to read {}", meta_path.display()));
            }
        };

        Ok(Some((
            ModelsDevCatalogSnapshot {
                providers,
                fetched_at: meta.fetched_at,
                stale: false,
            },
            meta,
        )))
    }

    async fn write_snapshot_to_disk(
        &self,
        _snapshot: &ModelsDevCatalogSnapshot,
        meta: &ModelsDevCatalogMeta,
        raw_bytes: &[u8],
    ) -> Result<()> {
        let root = self.inner.cache_dir.join("models-dev");
        tokio::fs::create_dir_all(&root)
            .await
            .with_context(|| format!("failed to create {}", root.display()))?;

        let api_path = root.join("api.json");
        let meta_path = root.join("api.meta.json");
        tokio::fs::write(&api_path, raw_bytes)
            .await
            .with_context(|| format!("failed to write {}", api_path.display()))?;
        tokio::fs::write(
            &meta_path,
            serde_json::to_vec_pretty(meta)?,
        )
        .await
        .with_context(|| format!("failed to write {}", meta_path.display()))?;
        Ok(())
    }

    async fn handle_not_modified_response(
        &self,
        response: reqwest::Response,
    ) -> Result<ModelsDevCatalogSnapshot> {
        let existing_snapshot = self
            .inner
            .state
            .read()
            .await
            .clone()
            .ok_or_else(|| anyhow!("models.dev returned 304 without a cached snapshot"))?;
        let existing_meta = self.inner.meta.read().await.clone().unwrap_or(ModelsDevCatalogMeta {
            fetched_at: existing_snapshot.fetched_at,
            etag: None,
            last_modified: None,
        });
        let fetched_at = Utc::now();
        let snapshot = ModelsDevCatalogSnapshot {
            providers: existing_snapshot.providers,
            fetched_at,
            stale: false,
        };
        let meta = ModelsDevCatalogMeta {
            fetched_at,
            etag: header_value_to_string(response.headers().get(ETAG)).or(existing_meta.etag),
            last_modified: header_value_to_string(response.headers().get(LAST_MODIFIED))
                .or(existing_meta.last_modified),
        };

        let api_path = self.inner.cache_dir.join("models-dev/api.json");
        let bytes = tokio::fs::read(&api_path)
            .await
            .with_context(|| format!("failed to read {}", api_path.display()))?;
        self.write_snapshot_to_disk(&snapshot, &meta, &bytes).await?;

        let mut cached_meta = self.inner.meta.write().await;
        *cached_meta = Some(meta);
        Ok(snapshot)
    }
}

impl ModelsDevCatalog {
    fn spawn_refresh_if_needed(&self) {
        if self
            .inner
            .refresh_in_flight
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }

        let this = self.clone();
        tokio::spawn(async move {
            match this.fetch_snapshot().await {
                Ok(snapshot) => {
                    let mut state = this.inner.state.write().await;
                    *state = Some(snapshot);
                }
                Err(error) => {
                    warn!(error = ?error, "failed to refresh models.dev catalog in background");
                }
            }

            this.inner
                .refresh_in_flight
                .store(false, Ordering::Release);
        });
    }
}

fn snapshot_is_fresh(snapshot: &ModelsDevCatalogSnapshot) -> bool {
    Utc::now()
        .signed_duration_since(snapshot.fetched_at)
        .to_std()
        .unwrap_or_default()
        <= CATALOG_TTL
}

fn parse_catalog_bytes(bytes: &[u8]) -> Result<BTreeMap<String, ModelsDevProvider>> {
    serde_json::from_slice(bytes).context("failed to decode models.dev catalog json")
}

fn header_value_to_string(value: Option<&reqwest::header::HeaderValue>) -> Option<String> {
    value
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf, process, time::UNIX_EPOCH};

    use anyhow::Context;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    fn temp_dir(prefix: &str) -> PathBuf {
        let suffix = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time after epoch")
            .as_nanos();
        let path =
            PathBuf::from("/tmp/opencode").join(format!("{prefix}-{}-{suffix}", process::id()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn parses_provider_keyed_fixture() {
        let providers = parse_catalog_bytes(
            br#"{
                "anthropic": {
                    "id": "anthropic",
                    "name": "Anthropic",
                    "models": {
                        "claude-sonnet-4": {
                            "id": "claude-sonnet-4",
                            "name": "Claude Sonnet 4",
                            "tool_call": true
                        }
                    }
                }
            }"#,
        )
        .unwrap();

        assert_eq!(providers["anthropic"].name, "Anthropic");
        assert_eq!(
            providers["anthropic"].models["claude-sonnet-4"].name,
            "Claude Sonnet 4"
        );
    }

    #[tokio::test]
    async fn looks_up_provider_and_model() {
        let root = temp_dir("models-dev-lookup");
        fs::create_dir_all(root.join("models-dev")).unwrap();
        fs::write(
            root.join("models-dev/api.json"),
            br#"{"anthropic":{"id":"anthropic","name":"Anthropic","models":{"claude-sonnet-4":{"id":"claude-sonnet-4","name":"Claude Sonnet 4"}}}}"#,
        )
        .unwrap();
        fs::write(
            root.join("models-dev/api.meta.json"),
            serde_json::to_vec(&ModelsDevCatalogMeta {
                fetched_at: Utc::now(),
                etag: None,
                last_modified: None,
            })
            .unwrap(),
        )
        .unwrap();

        let catalog = ModelsDevCatalog::new(root).unwrap();
        assert_eq!(
            catalog.get_provider("anthropic").await.unwrap().name,
            "Anthropic"
        );
        assert_eq!(
            catalog
                .get_model("anthropic", "claude-sonnet-4")
                .await
                .unwrap()
                .name,
            "Claude Sonnet 4"
        );
    }

    #[tokio::test]
    async fn falls_back_to_stale_disk_cache_when_fetch_fails() {
        let root = temp_dir("models-dev-stale");
        fs::create_dir_all(root.join("models-dev")).unwrap();
        fs::write(
            root.join("models-dev/api.json"),
            br#"{"anthropic":{"id":"anthropic","name":"Anthropic","models":{}}}"#,
        )
        .unwrap();
        fs::write(
            root.join("models-dev/api.meta.json"),
            serde_json::to_vec(&ModelsDevCatalogMeta {
                fetched_at: Utc::now() - chrono::Duration::days(2),
                etag: None,
                last_modified: None,
            })
            .unwrap(),
        )
        .unwrap();

        let catalog =
            ModelsDevCatalog::new_with_api_url(root, "http://127.0.0.1:9/api.json".to_string())
                .unwrap();
        let snapshot = catalog.snapshot().await.unwrap();
        assert!(snapshot.stale);
        assert!(snapshot.providers.contains_key("anthropic"));
    }

    #[tokio::test]
    async fn returns_stale_disk_cache_without_waiting_for_refresh() {
        let root = temp_dir("models-dev-background-refresh");
        fs::create_dir_all(root.join("models-dev")).unwrap();
        fs::write(
            root.join("models-dev/api.json"),
            br#"{"anthropic":{"id":"anthropic","name":"Anthropic","models":{}}}"#,
        )
        .unwrap();
        fs::write(
            root.join("models-dev/api.meta.json"),
            serde_json::to_vec(&ModelsDevCatalogMeta {
                fetched_at: Utc::now() - chrono::Duration::days(2),
                etag: Some("stale-tag".to_string()),
                last_modified: Some("Tue, 01 Jul 2025 12:00:00 GMT".to_string()),
            })
            .unwrap(),
        )
        .unwrap();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.context("accept request").unwrap();
            tokio::time::sleep(Duration::from_secs(5)).await;
        });

        let catalog = ModelsDevCatalog::new_with_api_url(
            root,
            format!("http://{address}/api.json"),
        )
        .unwrap();

        let snapshot = tokio::time::timeout(Duration::from_millis(100), catalog.snapshot())
            .await
            .expect("snapshot should not wait for background refresh")
            .unwrap();
        assert!(snapshot.stale);
        assert!(snapshot.providers.contains_key("anthropic"));
    }

    #[tokio::test]
    async fn conditional_refresh_uses_cached_validators_and_handles_304() {
        let root = temp_dir("models-dev-304");
        fs::create_dir_all(root.join("models-dev")).unwrap();
        let api_bytes = br#"{"anthropic":{"id":"anthropic","name":"Anthropic","models":{}}}"#;
        fs::write(root.join("models-dev/api.json"), api_bytes).unwrap();
        fs::write(
            root.join("models-dev/api.meta.json"),
            serde_json::to_vec(&ModelsDevCatalogMeta {
                fetched_at: Utc::now() - chrono::Duration::days(2),
                etag: Some("\"etag-123\"".to_string()),
                last_modified: Some("Tue, 01 Jul 2025 12:00:00 GMT".to_string()),
            })
            .unwrap(),
        )
        .unwrap();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.context("accept request")?;
            let mut request = Vec::new();
            let mut buffer = [0u8; 1024];
            loop {
                let read = stream.read(&mut buffer).await.context("read request")?;
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let request = String::from_utf8(request).context("request should be utf8")?;
            let request = request.to_ascii_lowercase();
            anyhow::ensure!(request.contains("if-none-match: \"etag-123\""));
            anyhow::ensure!(
                request.contains("if-modified-since: tue, 01 jul 2025 12:00:00 gmt")
            );
            stream
                .write_all(
                    b"HTTP/1.1 304 Not Modified\r\nETag: \"etag-123\"\r\nLast-Modified: Tue, 01 Jul 2025 12:00:00 GMT\r\nContent-Length: 0\r\n\r\n",
                )
                .await
                .context("write response")?;
            Result::<_, anyhow::Error>::Ok(())
        });

        let catalog = ModelsDevCatalog::new_with_api_url(root.clone(), format!("http://{address}/api.json")).unwrap();
        let (snapshot, meta) = catalog.load_snapshot_from_disk().await.unwrap().unwrap();
        *catalog.inner.state.write().await = Some(snapshot);
        *catalog.inner.meta.write().await = Some(meta);

        let snapshot = catalog.fetch_snapshot().await.unwrap();
        assert!(!snapshot.stale);
        assert!(snapshot.providers.contains_key("anthropic"));
        assert!(snapshot.fetched_at > Utc::now() - chrono::Duration::minutes(1));

        server.await.unwrap().unwrap();

        let saved_meta: ModelsDevCatalogMeta = serde_json::from_slice(
            &fs::read(root.join("models-dev/api.meta.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(saved_meta.etag.as_deref(), Some("\"etag-123\""));
        assert_eq!(
            saved_meta.last_modified.as_deref(),
            Some("Tue, 01 Jul 2025 12:00:00 GMT")
        );
        assert!(saved_meta.fetched_at > Utc::now() - chrono::Duration::minutes(1));
    }
}
