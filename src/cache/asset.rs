use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use anyhow::{Context, Result, anyhow, bail};

#[derive(Debug, Clone)]
pub struct AssetCachePolicy {
    pub ttl: Duration,
    pub max_bytes: usize,
    pub allowed_content_types: Vec<&'static str>,
}

#[derive(Debug, Clone)]
pub struct CachedAsset {
    pub bytes: Vec<u8>,
    pub stale: bool,
}

#[derive(Clone)]
pub struct AssetCache {
    root: PathBuf,
    http: reqwest::Client,
}

impl AssetCache {
    pub fn new(root: PathBuf) -> Result<Self> {
        let http = reqwest::Client::builder()
            .build()
            .context("failed to build asset cache HTTP client")?;
        Ok(Self { root, http })
    }

    pub async fn get_or_fetch(
        &self,
        namespace: &str,
        key: &str,
        url: &str,
        policy: AssetCachePolicy,
    ) -> Result<CachedAsset> {
        validate_namespace(namespace)?;
        validate_key(key)?;

        let path = self.root.join(namespace).join(key);
        let stale_bytes = read_if_exists(&path).await?;

        if let Some(bytes) = stale_bytes.as_ref() {
            if is_fresh(&path, policy.ttl).await? {
                return Ok(CachedAsset {
                    bytes: bytes.clone(),
                    stale: false,
                });
            }
        }

        match self.fetch_and_store(&path, url, &policy).await {
            Ok(bytes) => Ok(CachedAsset {
                bytes,
                stale: false,
            }),
            Err(error) => {
                if let Some(bytes) = stale_bytes {
                    Ok(CachedAsset { bytes, stale: true })
                } else {
                    Err(error)
                }
            }
        }
    }

    async fn fetch_and_store(
        &self,
        path: &Path,
        url: &str,
        policy: &AssetCachePolicy,
    ) -> Result<Vec<u8>> {
        let response = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|error| anyhow!("asset fetch failed: {error}"))?;

        if !response.status().is_success() {
            bail!("asset fetch failed with status {}", response.status());
        }

        if let Some(content_type) = response.headers().get(reqwest::header::CONTENT_TYPE) {
            let content_type = content_type.to_str().unwrap_or_default();
            if !policy.allowed_content_types.is_empty()
                && !policy
                    .allowed_content_types
                    .iter()
                    .any(|allowed| content_type.starts_with(allowed))
            {
                bail!("asset fetch returned unsupported content type {content_type}");
            }
        }

        let body = response
            .bytes()
            .await
            .context("failed to read asset response body")?;
        if body.len() > policy.max_bytes {
            bail!(
                "asset fetch exceeded max size ({} > {})",
                body.len(),
                policy.max_bytes
            );
        }

        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .with_context(|| format!("failed to create cache directory {}", parent.display()))?;
        }

        let temp_path = path.with_extension(format!(
            "{}.tmp",
            path.extension().and_then(|ext| ext.to_str()).unwrap_or("cache")
        ));
        tokio::fs::write(&temp_path, body.as_ref())
            .await
            .with_context(|| format!("failed to write temporary cache file {}", temp_path.display()))?;
        tokio::fs::rename(&temp_path, path)
            .await
            .with_context(|| format!("failed to move cache file into place {}", path.display()))?;

        Ok(body.to_vec())
    }
}

pub fn validate_namespace(namespace: &str) -> Result<()> {
    if namespace.is_empty() {
        bail!("cache namespace must not be empty");
    }
    for segment in namespace.split('/') {
        if segment.is_empty()
            || segment == "."
            || segment == ".."
            || !segment
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            bail!("cache namespace contains unsafe path segments");
        }
    }
    Ok(())
}

pub fn validate_key(key: &str) -> Result<()> {
    if key.is_empty() || key.contains('/') || key == "." || key == ".." {
        bail!("cache key contains unsafe path characters");
    }
    if !key
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        bail!("cache key contains unsafe path characters");
    }
    Ok(())
}

async fn read_if_exists(path: &Path) -> Result<Option<Vec<u8>>> {
    match tokio::fs::read(path).await {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("failed to read cache file {}", path.display())),
    }
}

async fn is_fresh(path: &Path, ttl: Duration) -> Result<bool> {
    if ttl.is_zero() {
        return Ok(false);
    }
    let metadata = tokio::fs::metadata(path)
        .await
        .with_context(|| format!("failed to stat cache file {}", path.display()))?;
    let modified = metadata
        .modified()
        .with_context(|| format!("failed to read cache mtime {}", path.display()))?;
    let age = SystemTime::now()
        .duration_since(modified)
        .unwrap_or_else(|_| Duration::from_secs(0));
    Ok(age <= ttl)
}

#[cfg(test)]
mod tests {
    use std::{fs, process, time::UNIX_EPOCH};

    use super::*;

    fn temp_dir(prefix: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time after epoch")
            .as_nanos();
        let path = PathBuf::from("/tmp/opencode").join(format!("{prefix}-{}-{suffix}", process::id()));
        fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    #[tokio::test]
    async fn rejects_unsafe_namespace_and_key() {
        let cache = AssetCache::new(temp_dir("asset-cache-unsafe")).unwrap();
        let policy = AssetCachePolicy {
            ttl: Duration::from_secs(60),
            max_bytes: 64,
            allowed_content_types: Vec::new(),
        };

        assert!(cache
            .get_or_fetch("../bad", "logo.svg", "http://127.0.0.1:9/logo.svg", policy.clone())
            .await
            .is_err());
        assert!(cache
            .get_or_fetch("models-dev", "../logo.svg", "http://127.0.0.1:9/logo.svg", policy)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn returns_cached_content_when_fresh() {
        let root = temp_dir("asset-cache-fresh");
        let namespace = "models-dev/logos";
        let key = "anthropic.svg";
        let path = root.join(namespace).join(key);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"cached").unwrap();

        let cache = AssetCache::new(root).unwrap();
        let asset = cache
            .get_or_fetch(
                namespace,
                key,
                "http://127.0.0.1:9/logo.svg",
                AssetCachePolicy {
                    ttl: Duration::from_secs(3600),
                    max_bytes: 64,
                    allowed_content_types: Vec::new(),
                },
            )
            .await
            .unwrap();

        assert_eq!(asset.bytes, b"cached");
        assert!(!asset.stale);
    }

    #[tokio::test]
    async fn returns_stale_content_when_fetch_fails() {
        let root = temp_dir("asset-cache-stale");
        let namespace = "models-dev/logos";
        let key = "anthropic.svg";
        let path = root.join(namespace).join(key);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"stale").unwrap();

        let cache = AssetCache::new(root).unwrap();
        let asset = cache
            .get_or_fetch(
                namespace,
                key,
                "http://127.0.0.1:9/logo.svg",
                AssetCachePolicy {
                    ttl: Duration::ZERO,
                    max_bytes: 64,
                    allowed_content_types: Vec::new(),
                },
            )
            .await
            .unwrap();

        assert_eq!(asset.bytes, b"stale");
        assert!(asset.stale);
    }
}
