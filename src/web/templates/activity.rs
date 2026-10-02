use askama::Template;
use chrono::Utc;

use crate::memory::{MemoryRecord, memory_expires_at};

use super::shared::{format_timestamp_iso, format_timestamp_utc};

pub struct AgentActivityItem {
    pub summary: String,
    pub memory_type: String,
    pub detail_url: String,
    pub created_at_iso: String,
    pub created_at_fallback_text: String,
    pub expires_at_iso: String,
    pub is_expired: bool,
}

#[derive(Template)]
#[template(path = "agents/fragments/activity.html")]
pub struct AgentActivityPartialTemplate {
    pub items: Vec<AgentActivityItem>,
}

impl AgentActivityPartialTemplate {
    pub fn render_view(
        agent_key: &str,
        memories: Vec<MemoryRecord>,
    ) -> Result<String, askama::Error> {
        let now = Utc::now();
        let items = memories
            .into_iter()
            .map(|memory| {
                let expires_at = memory_expires_at(&memory);
                AgentActivityItem {
                    summary: memory.summary,
                    memory_type: memory.memory_type.replace('_', " "),
                    detail_url: format!("/agents/{agent_key}/memories/{}", memory.id),
                    created_at_iso: format_timestamp_iso(memory.created_at),
                    created_at_fallback_text: format_timestamp_utc(memory.created_at),
                    expires_at_iso: expires_at.map(format_timestamp_iso).unwrap_or_default(),
                    is_expired: expires_at.is_some_and(|value| value <= now),
                }
            })
            .collect();
        Self { items }.render()
    }
}
