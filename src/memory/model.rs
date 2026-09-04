use chrono::{DateTime, Utc};
use serde::Deserialize;
use uuid::Uuid;

pub const MEMORY_SCOPE_AGENT: &str = "agent";
pub const MEMORY_SCOPE_INSTRUMENTS: &str = "instruments";

/// Memory types owned by the framework. Analysis may write any valid
/// non-reserved type; Trading writes `trading_decision`; Review writes
/// review/learnings records; Coding result records stay application owned.
pub const RESERVED_MEMORY_TYPES: &[&str] = &["trading_decision", "agent_learnings"];

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct MemoryRecord {
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
    pub agent_key: String,
    pub scope_kind: String,
    pub timeframe: Option<String>,
    pub memory_type: String,
    pub summary: String,
    pub content: String,
    pub metadata: serde_json::Value,
    pub source_run_id: Option<i64>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct MemoryTimelineRecord {
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
    pub scope_kind: String,
    pub timeframe: Option<String>,
    pub memory_type: String,
    pub summary: String,
}

impl From<&MemoryRecord> for MemoryTimelineRecord {
    fn from(row: &MemoryRecord) -> Self {
        Self {
            id: row.id,
            created_at: row.created_at,
            scope_kind: row.scope_kind.clone(),
            timeframe: row.timeframe.clone(),
            memory_type: row.memory_type.clone(),
            summary: row.summary.clone(),
        }
    }
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
    pub scope_kind: String,
    /// One or more unique canonical Hyperliquid instrument IDs. Required for
    /// `instruments` scope; must be empty for `agent` scope.
    #[serde(default)]
    pub instrument_ids: Vec<String>,
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

        match self.scope_kind.as_str() {
            MEMORY_SCOPE_AGENT => {
                if !self.instrument_ids.is_empty() {
                    errors.push(
                        "instrument_ids must be empty for agent-scoped memories.".to_string(),
                    );
                }
            }
            MEMORY_SCOPE_INSTRUMENTS => {
                if self.instrument_ids.is_empty() {
                    errors.push(
                        "instrument_ids is required for instrument-scoped memories.".to_string(),
                    );
                }
                let mut seen = std::collections::BTreeSet::new();
                for instrument_id in &self.instrument_ids {
                    if instrument_id.trim().is_empty() {
                        errors.push("instrument_ids must not contain blank values.".to_string());
                    } else if !seen.insert(instrument_id.trim().to_string()) {
                        errors.push("instrument_ids must not contain duplicates.".to_string());
                    }
                }
            }
            _ => errors.push("scope_kind must be `agent` or `instruments`.".to_string()),
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
    pub scope_kind: Option<String>,
    pub instrument_id: Option<String>,
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
