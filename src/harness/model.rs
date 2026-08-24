use chrono::{DateTime, Utc};
use serde_json::Value;

pub const SUB_AGENT_KIND_ANALYSIS: &str = "analysis";
pub const SUB_AGENT_KIND_MARKET_ANALYSIS: &str = "market_analysis";
pub const SUB_AGENT_KIND_TRADING: &str = "trading";
pub const SUB_AGENT_KIND_DAILY_REVIEW: &str = "daily_review";
pub const SUB_AGENT_KIND_ANALYSIS_CODING: &str = "analysis_coding";

pub const RUN_STATUS_QUEUED: &str = "queued";
pub const RUN_STATUS_RUNNING: &str = "running";
pub const RUN_STATUS_SUCCEEDED: &str = "succeeded";
pub const RUN_STATUS_FAILED: &str = "failed";
pub const RUN_STATUS_ABORTED: &str = "aborted";
pub const RUN_STATUS_SKIPPED: &str = "skipped";

pub const MAINTENANCE_TASK_KIND_WORKSPACE_REGENERATE: &str = "workspace_regenerate";
pub const MAINTENANCE_TASK_KIND_ANALYSIS_CODING: &str = "analysis_coding";
pub const MAINTENANCE_TASK_KIND_PROVIDER_CONFIG_RELOAD: &str = "provider_config_reload";

pub const MAINTENANCE_STATUS_QUEUED: &str = "queued";
pub const MAINTENANCE_STATUS_RUNNING: &str = "running";
pub const MAINTENANCE_STATUS_SUCCEEDED: &str = "succeeded";
pub const MAINTENANCE_STATUS_FAILED: &str = "failed";
pub const MAINTENANCE_STATUS_ABORTED: &str = "aborted";

/// Phase enum values for `harness_maintenance_tasks.phase`. The
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

/// Coding phases that hold an exclusive live-workspace lease and
/// therefore block normal scheduled/manual dispatch while they
/// execute.
pub const CODING_PROMOTION_PHASES: &[&str] = &[
    MAINTENANCE_PHASE_WAITING_FOR_PROMOTION,
    MAINTENANCE_PHASE_PROMOTING,
    MAINTENANCE_PHASE_SMOKE_TESTING,
    MAINTENANCE_PHASE_ROLLING_BACK,
];

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct HarnessSubAgentRow {
    pub id: i64,
    pub agent_key: String,
    pub sub_agent_key: String,
    pub sub_agent_kind: String,
    pub enabled: bool,
    pub timeframe: Option<String>,
    pub next_run_at: Option<DateTime<Utc>>,
    pub model_provider_id: Option<String>,
    pub model_id: Option<String>,
    pub model_variant: Option<String>,
    pub timeout_seconds: i32,
    pub operator_prompt: String,
    // Retained on the row so callers can use persistence timestamps without a new query.
    #[expect(
        dead_code,
        reason = "job views do not currently display persistence timestamps"
    )]
    pub created_at: DateTime<Utc>,
    #[expect(
        dead_code,
        reason = "job views do not currently display persistence timestamps"
    )]
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct HarnessSubAgentRunRow {
    pub id: i64,
    pub sub_agent_id: i64,
    pub agent_key: String,
    pub sub_agent_key: String,
    pub sub_agent_kind: String,
    pub timeframe: Option<String>,
    pub status: String,
    pub backend_run_ref: Option<String>,
    pub model_provider_id: Option<String>,
    pub model_id: Option<String>,
    pub model_variant: Option<String>,
    pub scheduled_for: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub timeout_seconds: i32,
    pub error_summary: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
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
    pub sub_agent_id: Option<i64>,
    pub run_id: Option<i64>,
    pub source_sub_agent_run_id: Option<i64>,
    pub source_memory_id: Option<uuid::Uuid>,
}

impl AgentMaintenanceTaskRow {
    pub fn parameter_bool(&self, name: &str) -> bool {
        self.parameters
            .get(name)
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    /// Convenience accessor for the coding promotion phase check.
    pub fn is_in_promotion_window(&self) -> bool {
        CODING_PROMOTION_PHASES.contains(&self.phase.as_str())
    }
}

/// Row for a global (non-agent-scoped) maintenance task such as
/// `provider_config_reload`.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct GlobalMaintenanceTaskRow {
    pub id: i64,
    pub status: String,
    pub error_summary: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct HarnessDispatchSubAgentRow {
    pub sub_agent_id: i64,
    pub agent_key: String,
    pub display_name: String,
    pub sub_agent_key: String,
    pub sub_agent_kind: String,
    pub timeframe: Option<String>,
    pub trigger_delay_seconds: Option<i32>,
    pub next_run_at: Option<DateTime<Utc>>,
    pub model_provider_id: Option<String>,
    pub model_id: Option<String>,
    pub model_variant: Option<String>,
    pub timeout_seconds: i32,
    pub operator_prompt: String,
    pub opencode_base_url: String,
    pub runtime_config: Value,
}
