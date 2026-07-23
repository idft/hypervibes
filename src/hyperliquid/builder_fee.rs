use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc};

use alloy::primitives::Address;
use hypersdk::hypercore;
use tokio::sync::Mutex;

/// Hyperliquid builder fee recipient. This is the only builder address the
/// server accepts and submits.
pub const BUILDER_RECIPIENT: &str = "0x2ebba955c61116e1c249efb1e39d25cb4a79ea05";
pub const MAX_BUILDER_FEE_TENTHS_OF_BP: u32 = 100;

pub type LookupFuture<'a> = Pin<Box<dyn Future<Output = Result<u32, String>> + Send + 'a>>;

/// Unsigned client used to query the current builder-fee approval maximum.
pub trait BuilderFeeLookup: Send + Sync {
    fn max_builder_fee<'a>(&'a self, user: &'a str, builder: &'a str) -> LookupFuture<'a>;
}

/// Real unsigned Hyperliquid `/info` client for builder-fee lookups.
pub struct HyperliquidBuilderFeeLookup {
    client: hypercore::HttpClient,
}

impl HyperliquidBuilderFeeLookup {
    pub fn mainnet() -> Self {
        Self {
            client: hypercore::mainnet(),
        }
    }
}

impl BuilderFeeLookup for HyperliquidBuilderFeeLookup {
    fn max_builder_fee<'a>(&'a self, user: &'a str, builder: &'a str) -> LookupFuture<'a> {
        Box::pin(async move {
            let user = user.parse::<Address>().map_err(|error| error.to_string())?;
            let builder = builder
                .parse::<Address>()
                .map_err(|error| error.to_string())?;
            self.client
                .max_builder_fee(user, builder)
                .await
                .map_err(|error| error.to_string())
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct BuilderFeeKey {
    user: String,
    builder: String,
}

impl BuilderFeeKey {
    fn new(user: &str, builder: &str) -> Self {
        Self {
            user: user.trim().to_ascii_lowercase(),
            builder: builder.trim().to_ascii_lowercase(),
        }
    }
}

/// Process-local cache of successful Hyperliquid builder-fee lookups.
///
/// The mutex is deliberately held over a cache miss and its HTTP request. A
/// lookup is rare, and this keeps concurrent misses for one key from issuing
/// duplicate requests without adding a separate single-flight dependency.
pub struct BuilderFeeCache {
    lookup: Arc<dyn BuilderFeeLookup>,
    values: Mutex<HashMap<BuilderFeeKey, u32>>,
}

impl BuilderFeeCache {
    pub fn new(lookup: Arc<dyn BuilderFeeLookup>) -> Self {
        Self {
            lookup,
            values: Mutex::new(HashMap::new()),
        }
    }

    pub fn mainnet() -> Self {
        Self::new(Arc::new(HyperliquidBuilderFeeLookup::mainnet()))
    }

    /// Return a cached maximum or perform the first lookup for this key.
    pub async fn max_builder_fee(&self, user: &str, builder: &str) -> Result<u32, String> {
        let key = BuilderFeeKey::new(user, builder);
        let mut values = self.values.lock().await;
        if let Some(maximum) = values.get(&key) {
            return Ok(*maximum);
        }
        let maximum = self.lookup.max_builder_fee(&key.user, &key.builder).await?;
        values.insert(key, maximum);
        Ok(maximum)
    }

    /// Force a fresh lookup after Hyperliquid rejects an order. A failed
    /// refresh leaves any existing value untouched.
    pub async fn refresh(&self, user: &str, builder: &str) -> Result<u32, String> {
        let key = BuilderFeeKey::new(user, builder);
        let mut values = self.values.lock().await;
        let maximum = self.lookup.max_builder_fee(&key.user, &key.builder).await?;
        values.insert(key, maximum);
        Ok(maximum)
    }

    /// Record an accepted user-signed approval. The action establishes the
    /// approved maximum without an immediate follow-up request.
    pub async fn record_approval(&self, user: &str, builder: &str, fee: u32) {
        let key = BuilderFeeKey::new(user, builder);
        self.values.lock().await.insert(key, fee);
    }

    /// Record a successful user-signed revocation without another lookup.
    pub async fn record_revocation(&self, user: &str, builder: &str) {
        let key = BuilderFeeKey::new(user, builder);
        self.values.lock().await.insert(key, 0);
    }

    /// Record a successful order as lower-bound evidence without reducing a
    /// larger maximum learned from Hyperliquid.
    pub async fn record_order_evidence(&self, user: &str, builder: &str, fee: u32) {
        let key = BuilderFeeKey::new(user, builder);
        let mut values = self.values.lock().await;
        values
            .entry(key)
            .and_modify(|maximum| *maximum = (*maximum).max(fee))
            .or_insert(fee);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use super::*;

    struct FakeLookup {
        calls: AtomicUsize,
        result: Mutex<Result<u32, String>>,
        delay: Option<Duration>,
    }

    impl FakeLookup {
        fn new(result: Result<u32, &str>) -> Arc<Self> {
            Arc::new(Self {
                calls: AtomicUsize::new(0),
                result: Mutex::new(result.map_err(str::to_string)),
                delay: None,
            })
        }
    }

    impl BuilderFeeLookup for FakeLookup {
        fn max_builder_fee<'a>(&'a self, _user: &'a str, _builder: &'a str) -> LookupFuture<'a> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                if let Some(delay) = self.delay {
                    tokio::time::sleep(delay).await;
                }
                self.result.lock().await.clone()
            })
        }
    }

    #[tokio::test]
    async fn caches_successful_lookup_by_normalized_user_and_builder() {
        let fake = FakeLookup::new(Ok(17));
        let cache = BuilderFeeCache::new(fake.clone());

        assert_eq!(cache.max_builder_fee("0xUSER", "0xBUILDER").await, Ok(17));
        assert_eq!(cache.max_builder_fee("0xuser", "0xbuilder").await, Ok(17));
        assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn concurrent_misses_issue_one_lookup() {
        let fake = Arc::new(FakeLookup {
            calls: AtomicUsize::new(0),
            result: Mutex::new(Ok(17)),
            delay: Some(Duration::from_millis(10)),
        });
        let cache = Arc::new(BuilderFeeCache::new(fake.clone()));
        let first = {
            let cache = Arc::clone(&cache);
            tokio::spawn(async move { cache.max_builder_fee("user", "builder").await })
        };
        let second = {
            let cache = Arc::clone(&cache);
            tokio::spawn(async move { cache.max_builder_fee("USER", "BUILDER").await })
        };
        assert_eq!(first.await.expect("first task"), Ok(17));
        assert_eq!(second.await.expect("second task"), Ok(17));
        assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn failures_are_not_cached_and_keys_do_not_share_values() {
        let fake = FakeLookup::new(Err("unavailable"));
        let cache = BuilderFeeCache::new(fake.clone());
        assert!(cache.max_builder_fee("user-a", "builder").await.is_err());
        assert!(cache.max_builder_fee("user-a", "builder").await.is_err());
        assert!(cache.max_builder_fee("user-b", "builder").await.is_err());
        assert_eq!(fake.calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn known_values_update_as_approval_evidence_and_refreshes() {
        let fake = FakeLookup::new(Ok(21));
        let cache = BuilderFeeCache::new(fake);
        cache.record_approval("user", "builder", 10).await;
        assert_eq!(cache.max_builder_fee("user", "builder").await, Ok(10));
        cache.record_order_evidence("user", "builder", 7).await;
        assert_eq!(cache.max_builder_fee("user", "builder").await, Ok(10));
        cache.record_order_evidence("user", "builder", 12).await;
        assert_eq!(cache.max_builder_fee("user", "builder").await, Ok(12));
        cache.record_revocation("user", "builder").await;
        assert_eq!(cache.max_builder_fee("user", "builder").await, Ok(0));
        assert_eq!(cache.refresh("user", "builder").await, Ok(21));
        assert_eq!(cache.max_builder_fee("user", "builder").await, Ok(21));
    }
}
