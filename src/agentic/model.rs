use chrono::{DateTime, Utc};
use serde_json::Value;

pub const JOB_KIND_ANALYSIS: &str = "analysis";
pub const JOB_KIND_MARKET_ANALYSIS: &str = "market_analysis";
pub const JOB_KIND_TRADING: &str = "trading";
pub const JOB_KIND_DAILY_REVIEW: &str = "daily_review";
pub const JOB_KIND_ANALYSIS_CODING: &str = "analysis_coding";

pub const HOOK_EVENT_ANALYSIS_BATCH_COMPLETED: &str = "analysis_batch_completed";
pub const HOOK_EVENT_DAILY_REVIEW_COMPLETED: &str = "daily_review_completed";

pub const RUN_STATUS_QUEUED: &str = "queued";
pub const RUN_STATUS_RUNNING: &str = "running";
pub const RUN_STATUS_SUCCEEDED: &str = "succeeded";
pub const RUN_STATUS_FAILED: &str = "failed";
#[cfg(test)]
pub const RUN_STATUS_ABORTED: &str = "aborted";
pub const RUN_STATUS_SKIPPED: &str = "skipped";

pub const MAINTENANCE_TASK_KIND_WORKSPACE_REGENERATE: &str = "workspace_regenerate";
pub const MAINTENANCE_TASK_KIND_ANALYSIS_CODING: &str = "analysis_coding";

pub const MAINTENANCE_STATUS_QUEUED: &str = "queued";
pub const MAINTENANCE_STATUS_RUNNING: &str = "running";
pub const MAINTENANCE_STATUS_SUCCEEDED: &str = "succeeded";
pub const MAINTENANCE_STATUS_FAILED: &str = "failed";
pub const MAINTENANCE_STATUS_ABORTED: &str = "aborted";

/// Phase enum values for `agentic_maintenance_tasks.phase`. The
/// workspace regeneration flow only ever enters `queued`/
/// `running`/`completed`; coding uses the full state machine.
pub const MAINTENANCE_PHASE_QUEUED: &str = "queued";
pub const MAINTENANCE_PHASE_PREPARING: &str = "preparing";
pub const MAINTENANCE_PHASE_GENERATING: &str = "generating";
pub const MAINTENANCE_PHASE_VALIDATING: &str = "validating";
pub const MAINTENANCE_PHASE_WAITING_FOR_PROMOTION: &str = "waiting_for_promotion";
pub const MAINTENANCE_PHASE_PROMOTING: &str = "promoting";
pub const MAINTENANCE_PHASE_SMOKE_TESTING: &str = "smoke_testing";
pub const MAINTENANCE_PHASE_ROLLING_BACK: &str = "rolling_back";
pub const MAINTENANCE_PHASE_COMPLETED: &str = "completed";

/// Engineering phases that hold an exclusive live-workspace lease and
/// therefore block normal scheduled/manual dispatch while they
/// execute.
pub const CODING_PROMOTION_PHASES: &[&str] = &[
    MAINTENANCE_PHASE_WAITING_FOR_PROMOTION,
    MAINTENANCE_PHASE_PROMOTING,
    MAINTENANCE_PHASE_SMOKE_TESTING,
    MAINTENANCE_PHASE_ROLLING_BACK,
];

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
    pub task_kind: String,
    pub parameters: Value,
    pub status: String,
    pub phase: String,
    pub error_summary: Option<String>,
    pub run_id: Option<i64>,
    pub source_run_id: Option<i64>,
    pub source_memory_id: Option<uuid::Uuid>,
}

impl AgentMaintenanceTaskRow {
    pub fn parameter_bool(&self, name: &str) -> bool {
        self.parameters
            .get(name)
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    pub fn parameter_i64(&self, name: &str) -> Option<i64> {
        self.parameters.get(name).and_then(Value::as_i64)
    }

    /// Convenience accessor for the coding promotion phase check.
    pub fn is_in_promotion_window(&self) -> bool {
        CODING_PROMOTION_PHASES.contains(&self.phase.as_str())
    }
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
