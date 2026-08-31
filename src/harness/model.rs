use std::collections::BTreeSet;

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

pub const RUN_CONTEXT_SNAPSHOT_SCHEMA_VERSION: i32 = 1;
pub const CAPABILITY_SCHEMA_VERSION: i32 = 1;
pub const CAPABILITY_NOTIFICATION_SEND: &str = "hypervibes:notification_send";
pub const MAX_RUN_CONTEXT_SNAPSHOT_BYTES: usize = 1024 * 1024;

/// API actions attached to a short-lived run credential. These are separate
/// from OpenCode permissions so direct HTTP calls cannot bypass role policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunApiScope {
    AccountRead,
    MemoryRead,
    MemoryWrite,
    OrderRead,
    OrderWrite,
    TransactionRead,
    PromptRead,
    PromptRevisionSubmit,
    NotificationSend,
}

impl RunApiScope {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AccountRead => "hypervibes:account_read",
            Self::MemoryRead => "hypervibes:memory_read",
            Self::MemoryWrite => "hypervibes:memory_write",
            Self::OrderRead => "hypervibes:order_read",
            Self::OrderWrite => "hypervibes:order_write",
            Self::TransactionRead => "hypervibes:transaction_read",
            Self::PromptRead => "hypervibes:prompt_read",
            Self::PromptRevisionSubmit => "hypervibes:prompt_revision_submit",
            Self::NotificationSend => CAPABILITY_NOTIFICATION_SEND,
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "hypervibes:account_read" => Some(Self::AccountRead),
            "hypervibes:memory_read" => Some(Self::MemoryRead),
            "hypervibes:memory_write" => Some(Self::MemoryWrite),
            "hypervibes:order_read" => Some(Self::OrderRead),
            "hypervibes:order_write" => Some(Self::OrderWrite),
            "hypervibes:transaction_read" => Some(Self::TransactionRead),
            "hypervibes:prompt_read" => Some(Self::PromptRead),
            "hypervibes:prompt_revision_submit" => Some(Self::PromptRevisionSubmit),
            CAPABILITY_NOTIFICATION_SEND => Some(Self::NotificationSend),
            _ => None,
        }
    }
}

pub fn run_api_scopes_for_sub_agent(
    sub_agent_kind: &str,
    notification_send_enabled: bool,
) -> anyhow::Result<Vec<RunApiScope>> {
    let mut scopes = match sub_agent_kind {
        SUB_AGENT_KIND_ANALYSIS => vec![
            RunApiScope::AccountRead,
            RunApiScope::MemoryRead,
            RunApiScope::MemoryWrite,
        ],
        SUB_AGENT_KIND_MARKET_ANALYSIS => {
            vec![RunApiScope::MemoryRead, RunApiScope::MemoryWrite]
        }
        SUB_AGENT_KIND_TRADING => vec![
            RunApiScope::AccountRead,
            RunApiScope::MemoryRead,
            RunApiScope::OrderRead,
            RunApiScope::OrderWrite,
        ],
        SUB_AGENT_KIND_DAILY_REVIEW => vec![
            RunApiScope::MemoryRead,
            RunApiScope::MemoryWrite,
            RunApiScope::OrderRead,
            RunApiScope::TransactionRead,
            RunApiScope::PromptRead,
            RunApiScope::PromptRevisionSubmit,
        ],
        _ => anyhow::bail!("unsupported sub-agent kind for run API scopes"),
    };
    if notification_send_enabled {
        scopes.push(RunApiScope::NotificationSend);
    }
    Ok(scopes)
}

// Schema version one intentionally has no catch-all object. Adding a new run
// input is an explicit snapshot-schema change rather than an unreviewed place
// to put runtime configuration or secrets.
const RUN_CONTEXT_SNAPSHOT_V1_FIELDS: &[&str] = &[
    "account_snapshot_metadata",
    "additional_instructions",
    "accumulated_learning_memory_id",
    "mcp_installations",
    "model_id",
    "model_variant",
    "notification_send_enabled",
    "provider_id",
    "quantitative_package",
    "scheduled_candle_boundary",
    "selected_instruments",
    "strategy_prompt_revisions",
    "system_prompt_version",
    "timeout_seconds",
];

const STRATEGY_PROMPT_REVISION_KEYS: &[&str] = &[
    SUB_AGENT_KIND_ANALYSIS,
    SUB_AGENT_KIND_MARKET_ANALYSIS,
    SUB_AGENT_KIND_TRADING,
    SUB_AGENT_KIND_DAILY_REVIEW,
    SUB_AGENT_KIND_ANALYSIS_CODING,
];

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
    pub notification_send_enabled: bool,
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

/// Durable, versioned inputs for an isolated run. Runtime credentials and
/// gateway destinations are deliberately not representable in this snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct RunContextSnapshot {
    pub schema_version: i32,
    pub context: Value,
    pub capability_schema_version: i32,
    pub enabled_capabilities: Vec<String>,
}

impl RunContextSnapshot {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.schema_version != RUN_CONTEXT_SNAPSHOT_SCHEMA_VERSION {
            anyhow::bail!("unsupported run context schema version");
        }
        if self.capability_schema_version != CAPABILITY_SCHEMA_VERSION {
            anyhow::bail!("unsupported capability schema version");
        }
        if !self.context.is_object() {
            anyhow::bail!("run context snapshot must be a JSON object");
        }
        if serde_json::to_vec(&self.context)?.len() > MAX_RUN_CONTEXT_SNAPSHOT_BYTES {
            anyhow::bail!("run context snapshot exceeds the size limit");
        }
        let capabilities = self.normalized_enabled_capabilities()?;
        validate_context_snapshot_v1(
            &self.context,
            capabilities
                .iter()
                .any(|capability| capability == CAPABILITY_NOTIFICATION_SEND),
        )?;
        Ok(())
    }

    pub fn normalized_enabled_capabilities(&self) -> anyhow::Result<Vec<String>> {
        let mut capabilities = BTreeSet::new();
        for capability in &self.enabled_capabilities {
            let capability = capability.trim();
            if capability.is_empty()
                || capability.len() > 128
                || capability.chars().any(char::is_control)
            {
                anyhow::bail!("invalid capability in run context snapshot");
            }
            if capability != CAPABILITY_NOTIFICATION_SEND {
                anyhow::bail!("unsupported capability in run context snapshot");
            }
            if !capabilities.insert(capability.to_string()) {
                anyhow::bail!("duplicate capability in run context snapshot");
            }
        }
        Ok(capabilities.into_iter().collect())
    }

    pub fn notification_send_enabled(&self) -> bool {
        self.enabled_capabilities
            .iter()
            .any(|capability| capability == CAPABILITY_NOTIFICATION_SEND)
    }
}

// V1 is deliberately an exact, closed JSON shape. The database retains JSONB
// for forwards-compatible storage, but no arbitrary nested configuration can
// enter a durable artifact under the first schema version.
fn validate_context_snapshot_v1(
    value: &Value,
    notification_send_enabled: bool,
) -> anyhow::Result<()> {
    let fields = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("run context snapshot must be a JSON object"))?;
    if fields.len() != RUN_CONTEXT_SNAPSHOT_V1_FIELDS.len()
        || RUN_CONTEXT_SNAPSHOT_V1_FIELDS
            .iter()
            .any(|field| !fields.contains_key(*field))
    {
        anyhow::bail!("run context snapshot does not match schema version one");
    }

    validate_identifier(
        required_context_field(fields, "provider_id")?,
        "provider_id",
    )?;
    validate_identifier(required_context_field(fields, "model_id")?, "model_id")?;
    validate_optional_identifier(
        required_context_field(fields, "model_variant")?,
        "model_variant",
    )?;
    validate_positive_integer(
        required_context_field(fields, "timeout_seconds")?,
        "timeout_seconds",
    )?;
    validate_identifier_array(
        required_context_field(fields, "selected_instruments")?,
        "selected_instruments",
    )?;
    validate_strategy_prompt_revisions(required_context_field(
        fields,
        "strategy_prompt_revisions",
    )?)?;
    validate_text(
        required_context_field(fields, "additional_instructions")?,
        "additional_instructions",
    )?;
    validate_optional_uuid(
        required_context_field(fields, "accumulated_learning_memory_id")?,
        "accumulated_learning_memory_id",
    )?;
    validate_identifier(
        required_context_field(fields, "system_prompt_version")?,
        "system_prompt_version",
    )?;
    validate_quantitative_package(required_context_field(fields, "quantitative_package")?)?;
    validate_mcp_installations(required_context_field(fields, "mcp_installations")?)?;
    let declared_notification_send = required_context_field(fields, "notification_send_enabled")?
        .as_bool()
        .ok_or_else(|| anyhow::anyhow!("notification_send_enabled must be a boolean"))?;
    if declared_notification_send != notification_send_enabled {
        anyhow::bail!("run context notification authority does not match capabilities");
    }
    validate_optional_timestamp(
        required_context_field(fields, "scheduled_candle_boundary")?,
        "scheduled_candle_boundary",
    )?;
    validate_account_snapshot_metadata(required_context_field(
        fields,
        "account_snapshot_metadata",
    )?)?;
    Ok(())
}

fn required_context_field<'a>(
    fields: &'a serde_json::Map<String, Value>,
    field: &str,
) -> anyhow::Result<&'a Value> {
    fields
        .get(field)
        .ok_or_else(|| anyhow::anyhow!("run context snapshot is missing {field}"))
}

fn validate_identifier(value: &Value, field: &str) -> anyhow::Result<()> {
    let value = value
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("{field} must be a string"))?;
    if value.is_empty()
        || value.len() > 512
        || value.chars().any(char::is_control)
        || looks_like_runtime_secret(value)
    {
        anyhow::bail!("{field} is invalid");
    }
    Ok(())
}

fn validate_optional_identifier(value: &Value, field: &str) -> anyhow::Result<()> {
    if value.is_null() {
        return Ok(());
    }
    validate_identifier(value, field)
}

fn validate_positive_integer(value: &Value, field: &str) -> anyhow::Result<()> {
    let Some(value) = value.as_u64() else {
        anyhow::bail!("{field} must be a positive integer");
    };
    if value == 0 {
        anyhow::bail!("{field} must be a positive integer");
    }
    Ok(())
}

fn validate_optional_timestamp(value: &Value, field: &str) -> anyhow::Result<()> {
    if value.is_null() {
        return Ok(());
    }
    if value.as_u64().is_none() {
        anyhow::bail!("{field} must be a Unix timestamp in milliseconds or null");
    }
    Ok(())
}

fn validate_identifier_array(value: &Value, field: &str) -> anyhow::Result<()> {
    let values = value
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("{field} must be an array"))?;
    if values.len() > 256 {
        anyhow::bail!("{field} contains too many values");
    }
    for value in values {
        validate_identifier(value, field)?;
    }
    Ok(())
}

fn validate_strategy_prompt_revisions(value: &Value) -> anyhow::Result<()> {
    let revisions = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("strategy_prompt_revisions must be an object"))?;
    for (kind, revision) in revisions {
        if !STRATEGY_PROMPT_REVISION_KEYS.contains(&kind.as_str()) {
            anyhow::bail!("strategy_prompt_revisions contains an unsupported agent kind");
        }
        validate_positive_integer(revision, "strategy prompt revision")?;
    }
    Ok(())
}

fn validate_text(value: &Value, field: &str) -> anyhow::Result<()> {
    let value = value
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("{field} must be a string"))?;
    if value.len() > 65_536 || looks_like_runtime_secret(value) {
        anyhow::bail!("{field} is invalid");
    }
    Ok(())
}

fn validate_optional_uuid(value: &Value, field: &str) -> anyhow::Result<()> {
    if value.is_null() {
        return Ok(());
    }
    let value = value
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("{field} must be a UUID or null"))?;
    uuid::Uuid::parse_str(value).map_err(|_| anyhow::anyhow!("{field} must be a UUID or null"))?;
    Ok(())
}

fn validate_quantitative_package(value: &Value) -> anyhow::Result<()> {
    if value.is_null() {
        return Ok(());
    }
    let package = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("quantitative_package must be an object or null"))?;
    if package.len() != 2
        || !package.contains_key("version")
        || !package.contains_key("manifest_hash")
    {
        anyhow::bail!("quantitative_package does not match schema version one");
    }
    validate_identifier(&package["version"], "quantitative_package.version")?;
    validate_identifier(
        &package["manifest_hash"],
        "quantitative_package.manifest_hash",
    )
}

fn validate_mcp_installations(value: &Value) -> anyhow::Result<()> {
    let installations = value
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("mcp_installations must be an array"))?;
    if installations.len() > 64 {
        anyhow::bail!("mcp_installations contains too many values");
    }
    for installation in installations {
        let installation = installation
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("mcp installation must be an object"))?;
        if installation.len() != 3
            || !installation.contains_key("installation_id")
            || !installation.contains_key("version")
            || !installation.contains_key("tools")
        {
            anyhow::bail!("mcp installation does not match schema version one");
        }
        validate_identifier(&installation["installation_id"], "mcp installation id")?;
        validate_identifier(&installation["version"], "mcp installation version")?;
        validate_identifier_array(&installation["tools"], "mcp installation tools")?;
    }
    Ok(())
}

fn validate_account_snapshot_metadata(value: &Value) -> anyhow::Result<()> {
    if value.is_null() {
        return Ok(());
    }
    let metadata = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("account_snapshot_metadata must be an object or null"))?;
    if metadata.len() != 2
        || !metadata.contains_key("captured_at_ms")
        || !metadata.contains_key("account_address")
    {
        anyhow::bail!("account_snapshot_metadata does not match schema version one");
    }
    validate_optional_timestamp(
        &metadata["captured_at_ms"],
        "account_snapshot_metadata.captured_at_ms",
    )?;
    validate_identifier(
        &metadata["account_address"],
        "account_snapshot_metadata.account_address",
    )
}

fn looks_like_runtime_secret(value: &str) -> bool {
    value.contains("vta_")
        || value.contains("vtr_")
        || value.contains("-----BEGIN")
        || contains_telegram_bot_token(value)
}

fn contains_telegram_bot_token(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut start = 0;
    while start < bytes.len() {
        if !bytes[start].is_ascii_digit() {
            start += 1;
            continue;
        }
        let mut bot_id_end = start;
        while bot_id_end < bytes.len() && bytes[bot_id_end].is_ascii_digit() {
            bot_id_end += 1;
        }
        if bot_id_end - start < 5 || bytes.get(bot_id_end) != Some(&b':') {
            start = bot_id_end;
            continue;
        }
        let mut token_end = bot_id_end + 1;
        while token_end < bytes.len()
            && (bytes[token_end].is_ascii_alphanumeric() || matches!(bytes[token_end], b'_' | b'-'))
        {
            token_end += 1;
        }
        if token_end - (bot_id_end + 1) >= 20 {
            return true;
        }
        start = token_end;
    }
    false
}

#[derive(Debug, Clone, PartialEq)]
pub struct RunWorkspaceArtifactRow {
    pub run_id: i64,
    pub agent_key: String,
    pub context: RunContextSnapshot,
    pub workspace_status: String,
    pub workspace_created_at: Option<DateTime<Utc>>,
    pub runtime_secrets_scrubbed_at: Option<DateTime<Utc>>,
    pub terminalized_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub deletion_started_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
    pub observed_size_bytes: Option<i64>,
    pub observed_file_count: Option<i64>,
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
    pub notification_send_enabled: bool,
    pub opencode_base_url: String,
    pub runtime_config: Value,
}
