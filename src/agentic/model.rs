use chrono::{DateTime, Utc};
use serde_json::Value;

pub const JOB_KIND_ANALYSIS: &str = "analysis";
pub const JOB_KIND_MARKET_ANALYSIS: &str = "market_analysis";
pub const JOB_KIND_TRADING: &str = "trading";
pub const JOB_KIND_DAILY_REVIEW: &str = "daily_review";

pub const HOOK_EVENT_ANALYSIS_BATCH_COMPLETED: &str = "analysis_batch_completed";

pub const RUN_STATUS_QUEUED: &str = "queued";
pub const RUN_STATUS_RUNNING: &str = "running";
pub const RUN_STATUS_SUCCEEDED: &str = "succeeded";
pub const RUN_STATUS_FAILED: &str = "failed";
#[cfg(test)]
pub const RUN_STATUS_ABORTED: &str = "aborted";
pub const RUN_STATUS_SKIPPED: &str = "skipped";

pub const MAINTENANCE_TASK_KIND_WORKSPACE_REGENERATE: &str = "workspace_regenerate";

pub const MAINTENANCE_STATUS_QUEUED: &str = "queued";
pub const MAINTENANCE_STATUS_RUNNING: &str = "running";
pub const MAINTENANCE_STATUS_SUCCEEDED: &str = "succeeded";
pub const MAINTENANCE_STATUS_FAILED: &str = "failed";
pub const MAINTENANCE_STATUS_ABORTED: &str = "aborted";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AgenticJobScheduleRow {
    pub id: i64,
    pub agent_key: String,
    pub job_key: String,
    pub job_kind: String,
    pub enabled: bool,
    pub timeframe: String,
    pub next_run_at: DateTime<Utc>,
    pub model_provider_id: Option<String>,
    pub model_id: Option<String>,
    pub timeout_seconds: i32,
    pub operator_prompt: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AgenticJobHookRow {
    pub id: i64,
    pub agent_key: String,
    pub job_key: String,
    pub job_kind: String,
    pub hook_event: String,
    pub enabled: bool,
    pub model_provider_id: Option<String>,
    pub model_id: Option<String>,
    pub timeout_seconds: i32,
    pub operator_prompt: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AgenticRunRow {
    pub id: i64,
    pub schedule_id: Option<i64>,
    pub hook_id: Option<i64>,
    pub agent_key: String,
    pub job_key: String,
    pub timeframe: Option<String>,
    pub status: String,
    pub backend_run_ref: Option<String>,
    pub scheduled_for: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub timeout_seconds: i32,
    pub error_summary: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AgentMaintenanceTaskRow {
    pub id: i64,
    pub agent_key: String,
    pub hard_reset: bool,
    pub status: String,
    pub error_summary: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
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
    pub runtime_base_url: String,
    pub runtime_config: Value,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DueOpenCodeHookRow {
    pub hook_id: i64,
    pub agent_key: String,
    pub display_name: String,
    pub job_key: String,
    pub job_kind: String,
    pub model_provider_id: Option<String>,
    pub model_id: Option<String>,
    pub timeout_seconds: i32,
    pub operator_prompt: String,
    pub runtime_base_url: String,
    pub runtime_config: Value,
}
