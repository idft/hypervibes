use std::{collections::BTreeMap, path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

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
}

#[derive(Debug, Clone)]
pub struct ModelsDevCatalogSnapshot {
    pub providers: BTreeMap<String, ModelsDevProvider>,
    pub fetched_at: DateTime<Utc>,
    pub stale: bool,
}

pub struct ModelsDevCatalog {
    cache_dir: PathBuf,
    http: reqwest::Client,
    api_url: String,
    state: RwLock<Option<ModelsDevCatalogSnapshot>>,
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
            cache_dir,
            http,
            api_url,
            state: RwLock::new(None),
        })
    }

    pub async fn snapshot(&self) -> Result<ModelsDevCatalogSnapshot> {
        {
            let state = self.state.read().await;
            if let Some(snapshot) = state.as_ref() {
                if Utc::now()
                    .signed_duration_since(snapshot.fetched_at)
                    .to_std()
                    .unwrap_or_default()
                    <= CATALOG_TTL
                {
                    return Ok(snapshot.clone());
                }
            }
        }

        let disk_snapshot = self.load_snapshot_from_disk().await?;
        if let Some(snapshot) = disk_snapshot.as_ref() {
            let is_fresh = Utc::now()
                .signed_duration_since(snapshot.fetched_at)
                .to_std()
                .unwrap_or_default()
                <= CATALOG_TTL;
            if is_fresh {
                let mut state = self.state.write().await;
                *state = Some(snapshot.clone());
                return Ok(snapshot.clone());
            }
        }

        match self.fetch_snapshot().await {
            Ok(snapshot) => {
                let mut state = self.state.write().await;
                *state = Some(snapshot.clone());
                Ok(snapshot)
            }
            Err(error) => {
                if let Some(mut snapshot) = disk_snapshot {
                    snapshot.stale = true;
                    let mut state = self.state.write().await;
                    *state = Some(snapshot.clone());
                    Ok(snapshot)
                } else {
                    Err(error)
                }
            }
        }
    }

    pub async fn get_provider(&self, provider_id: &str) -> Option<ModelsDevProvider> {
        self.snapshot().await.ok()?.providers.get(provider_id).cloned()
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
        let response = self
            .http
            .get(&self.api_url)
            .send()
            .await
            .map_err(|error| anyhow!("models.dev catalog fetch failed: {error}"))?;
        if !response.status().is_success() {
            return Err(anyhow!(
                "models.dev catalog fetch failed with status {}",
                response.status()
            ));
        }

        let bytes = response
            .bytes()
            .await
            .context("failed to read models.dev catalog response")?;
        let providers = parse_catalog_bytes(&bytes)?;
        let snapshot = ModelsDevCatalogSnapshot {
            providers,
            fetched_at: Utc::now(),
            stale: false,
        };
        self.write_snapshot_to_disk(&snapshot, &bytes).await?;
        Ok(snapshot)
    }

    async fn load_snapshot_from_disk(&self) -> Result<Option<ModelsDevCatalogSnapshot>> {
        let api_path = self.cache_dir.join("models-dev/api.json");
        let meta_path = self.cache_dir.join("models-dev/api.meta.json");

        let bytes = match tokio::fs::read(&api_path).await {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to read {}", api_path.display()));
            }
        };

        let providers = parse_catalog_bytes(&bytes)?;
        let fetched_at = match tokio::fs::read(&meta_path).await {
            Ok(meta_bytes) => serde_json::from_slice::<ModelsDevCatalogMeta>(&meta_bytes)
                .map(|meta| meta.fetched_at)
                .unwrap_or_else(|_| Utc::now()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Utc::now(),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to read {}", meta_path.display()));
            }
        };

        Ok(Some(ModelsDevCatalogSnapshot {
            providers,
            fetched_at,
            stale: false,
        }))
    }

    async fn write_snapshot_to_disk(
        &self,
        snapshot: &ModelsDevCatalogSnapshot,
        raw_bytes: &[u8],
    ) -> Result<()> {
        let root = self.cache_dir.join("models-dev");
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
            serde_json::to_vec_pretty(&ModelsDevCatalogMeta {
                fetched_at: snapshot.fetched_at,
            })?,
        )
        .await
        .with_context(|| format!("failed to write {}", meta_path.display()))?;
        Ok(())
    }
}

fn parse_catalog_bytes(bytes: &[u8]) -> Result<BTreeMap<String, ModelsDevProvider>> {
    serde_json::from_slice(bytes).context("failed to decode models.dev catalog json")
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf, process, time::UNIX_EPOCH};

    use super::*;

    fn temp_dir(prefix: &str) -> PathBuf {
        let suffix = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time after epoch")
            .as_nanos();
        let path = PathBuf::from("/tmp/opencode").join(format!("{prefix}-{}-{suffix}", process::id()));
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
        assert_eq!(providers["anthropic"].models["claude-sonnet-4"].name, "Claude Sonnet 4");
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
            })
            .unwrap(),
        )
        .unwrap();

        let catalog = ModelsDevCatalog::new(root).unwrap();
        assert_eq!(catalog.get_provider("anthropic").await.unwrap().name, "Anthropic");
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
            })
            .unwrap(),
        )
        .unwrap();

        let catalog = ModelsDevCatalog::new_with_api_url(root, "http://127.0.0.1:9/api.json".to_string()).unwrap();
        let snapshot = catalog.snapshot().await.unwrap();
        assert!(snapshot.stale);
        assert!(snapshot.providers.contains_key("anthropic"));
    }
}
