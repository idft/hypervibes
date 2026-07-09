use chrono::{DateTime, Utc};
use serde::Deserialize;
use uuid::Uuid;

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct MemoryRecord {
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
    pub agent_key: String,
    pub symbol: String,
    pub timeframe: Option<String>,
    pub memory_type: String,
    pub summary: String,
    pub content: String,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct MemoryLinkRecord {
    pub id: i64,
    pub agent_key: String,
    pub source_memory_id: Uuid,
    pub target_memory_id: Uuid,
    pub link_type: String,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateMemoryLink {
    pub target_memory_id: Uuid,
    pub link_type: String,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateMemory {
    pub symbol: String,
    pub timeframe: Option<String>,
    pub memory_type: String,
    pub summary: String,
    pub content: String,
    pub metadata: Option<serde_json::Value>,
    pub links: Option<Vec<CreateMemoryLink>>,
}

impl CreateMemory {
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();

        if self.symbol.trim().is_empty() {
            errors.push("symbol is required.".to_string());
        }
        if self.memory_type.trim().is_empty() {
            errors.push("memory_type is required.".to_string());
        }
        if self.summary.trim().is_empty() {
            errors.push("summary is required.".to_string());
        }
        if self.content.trim().is_empty() {
            errors.push("content is required.".to_string());
        }
        if let Some(tf) = &self.timeframe
            && tf.trim().is_empty()
        {
            errors.push("timeframe must not be empty if provided.".to_string());
        }
        if let Some(value) = &self.metadata
            && !value.is_object()
        {
            errors.push("metadata must be a JSON object.".to_string());
        }
        if let Some(links) = &self.links {
            for (index, link) in links.iter().enumerate() {
                if link.link_type.trim().is_empty() {
                    errors.push(format!("links[{index}].link_type is required."));
                }
                if let Some(metadata) = &link.metadata
                    && !metadata.is_object()
                {
                    errors.push(format!("links[{index}].metadata must be a JSON object."));
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    pub fn metadata_or_default(&self) -> serde_json::Value {
        self.metadata
            .clone()
            .unwrap_or_else(|| serde_json::json!({}))
    }

    pub fn links_or_empty(&self) -> Vec<CreateMemoryLink> {
        self.links.clone().unwrap_or_default()
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct MemoryListFilter {
    pub symbol: Option<String>,
    pub timeframe: Option<String>,
    pub memory_type: Option<String>,
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
    pub limit: Option<i64>,
    /// If `true`, the list endpoint returns rows whose `expires_at` is at
    /// or before "now" as well. Defaults to `false` so the trading loop
    /// (and the default agent debug query) never sees stale analysis.
    /// Set to `true` from operator tooling / debugging endpoints only.
    #[serde(default)]
    pub include_expired: bool,
}
