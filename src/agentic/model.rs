use chrono::{DateTime, Utc};
use serde_json::Value;

pub const JOB_KIND_ANALYSIS: &str = "analysis";
pub const JOB_KIND_TRADING: &str = "trading";

pub const RUN_STATUS_QUEUED: &str = "queued";
pub const RUN_STATUS_RUNNING: &str = "running";
pub const RUN_STATUS_SUCCEEDED: &str = "succeeded";
pub const RUN_STATUS_FAILED: &str = "failed";
pub const RUN_STATUS_ABORTED: &str = "aborted";
pub const RUN_STATUS_SKIPPED: &str = "skipped";

#[derive(Debug, Clone, sqlx::FromRow)]
#[allow(dead_code)]
pub struct AgenticJobScheduleRow {
    pub id: i64,
    pub agent_key: String,
    pub job_key: String,
    pub job_kind: String,
    pub enabled: bool,
    pub timeframe: String,
    pub trigger_delay_seconds: i32,
    pub next_run_at: DateTime<Utc>,
    pub model_provider_id: Option<String>,
    pub model_id: Option<String>,
    pub timeout_seconds: i32,
    pub operator_prompt: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
#[allow(dead_code)]
pub struct AgenticRunRow {
    pub id: i64,
    pub schedule_id: Option<i64>,
    pub agent_key: String,
    pub job_key: String,
    pub job_kind: String,
    pub timeframe: String,
    pub status: String,
    pub backend_run_ref: Option<String>,
    pub model_provider_id: Option<String>,
    pub model_id: Option<String>,
    pub scheduled_for: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub timeout_seconds: i32,
    pub error_summary: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
#[allow(dead_code)]
pub struct DueOpenCodeScheduleRow {
    pub schedule_id: i64,
    pub agent_key: String,
    pub display_name: String,
    pub job_key: String,
    pub job_kind: String,
    pub timeframe: String,
    pub trigger_delay_seconds: i32,
    pub next_run_at: DateTime<Utc>,
    pub model_provider_id: Option<String>,
    pub model_id: Option<String>,
    pub timeout_seconds: i32,
    pub operator_prompt: String,
    pub runtime_id: String,
    pub runtime_name: String,
    pub runtime_base_url: String,
    pub runtime_config: Value,
}
