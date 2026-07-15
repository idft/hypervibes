use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::{
    agents::model::{AgentDetailRow, AgentListRow, AgentRuntimeRow},
    hyperliquid::queries::BalancePoint,
    memory::MemoryRecord,
    model_catalog::options::ModelPickerOption,
};

use super::*;

pub fn sample_agent_list_row() -> AgentListRow {
    AgentListRow {
        display_name: "Test Agent".to_string(),
        agent_key: "test-agent".to_string(),
        enabled: true,
        wallet_address: "0x1234567890abcdef".to_string(),
        environment: "live".to_string(),
        api_key_last_used_at: None,
    }
}

pub fn sample_agent_detail_row() -> AgentDetailRow {
    AgentDetailRow {
        display_name: "Test Agent".to_string(),
        agent_key: "test-agent".to_string(),
        enabled: true,
        wallet_address: "0x1234567890abcdef".to_string(),
        environment: "live".to_string(),
        api_key: "vt_test_key".to_string(),
        backend_kind: "opencode".to_string(),
        runtime_base_url: Some("http://localhost:14096".to_string()),
        runtime_config: serde_json::json!({}),
    }
}

pub fn sample_account_balance_view() -> AccountBalanceView {
    AccountBalanceView {
        total_balance: Some(rust_decimal::Decimal::new(232_6800, 4)),
        total_u_pnl: AnimatedNumber::for_pnl(rust_decimal::Decimal::new(12_3400, 4)),
    }
}

pub fn sample_runtime_row() -> AgentRuntimeRow {
    let now = Utc::now();
    AgentRuntimeRow {
        id: "opencode-local".to_string(),
        created_at: now,
        name: "OpenCode local".to_string(),
        backend_kind: "opencode".to_string(),
        enabled: true,
        base_url: Some("http://localhost:14096".to_string()),
    }
}

pub fn sample_memory_record(
    symbol: &str,
    timeframe: Option<&str>,
    memory_type: &str,
    summary: &str,
    content: &str,
) -> MemoryRecord {
    MemoryRecord {
        id: Uuid::from_u128(content.len() as u128 + summary.len() as u128),
        created_at: Utc::now(),
        agent_key: "test-agent".to_string(),
        symbol: symbol.to_string(),
        timeframe: timeframe.map(str::to_string),
        memory_type: memory_type.to_string(),
        summary: summary.to_string(),
        content: content.to_string(),
        metadata: serde_json::json!({
            "confidence": "high",
            "source": "test",
        }),
    }
}

pub fn sample_opencode_detail_row() -> AgentDetailRow {
    let mut row = sample_agent_detail_row();
    row.backend_kind = crate::agents::model::BACKEND_KIND_OPENCODE.to_string();
    row.runtime_config = serde_json::json!({
        "workspace_host_path": "workspaces/agents/test-agent",
        "workspace_container_path": "/workspaces/agents/test-agent",
        "profile_source": "agent-runtime/workspace-template"
    });
    row
}

pub fn sample_schedule_row(
    id: i64,
    job_key: &str,
    job_kind: &str,
    enabled: bool,
) -> crate::agentic::model::AgenticJobScheduleRow {
    let now = Utc::now();
    let timeframe = if job_kind == "trading" { "1m" } else { "15m" };
    crate::agentic::model::AgenticJobScheduleRow {
        id,
        agent_key: "test-agent".to_string(),
        job_key: job_key.to_string(),
        job_kind: job_kind.to_string(),
        enabled,
        timeframe: timeframe.to_string(),
        next_run_at: now,
        model_provider_id: Some("anthropic".to_string()),
        model_id: Some("claude-3-5-sonnet".to_string()),
        timeout_seconds: 600,
        operator_prompt: String::new(),
    }
}

pub fn sample_hook_row(id: i64, enabled: bool) -> crate::agentic::model::AgenticJobHookRow {
    let now = Utc::now();
    crate::agentic::model::AgenticJobHookRow {
        id,
        agent_key: "test-agent".to_string(),
        job_key: "market-analysis".to_string(),
        job_kind: crate::agentic::model::JOB_KIND_MARKET_ANALYSIS.to_string(),
        hook_event: crate::agentic::model::HOOK_EVENT_ANALYSIS_BATCH_COMPLETED.to_string(),
        enabled,
        model_provider_id: Some("anthropic".to_string()),
        model_id: Some("claude-3-5-sonnet".to_string()),
        timeout_seconds: 600,
        operator_prompt: String::new(),
        created_at: now,
        updated_at: now,
    }
}

pub fn sample_model_options() -> Vec<ModelPickerOption> {
    vec![ModelPickerOption {
        value: "anthropic/claude-sonnet-4".to_string(),
        provider_id: "anthropic".to_string(),
        provider_name: "Anthropic".to_string(),
        provider_logo_url: "/model-catalog/logos/anthropic.svg".to_string(),
        model_id: "claude-sonnet-4".to_string(),
        model_name: "Claude Sonnet 4".to_string(),
        metadata_text: "1M ctx · tools".to_string(),
    }]
}

pub fn sample_run_row(
    id: i64,
    status: &str,
    job_key: &str,
) -> crate::agentic::model::AgenticRunRow {
    let now = Utc::now();
    crate::agentic::model::AgenticRunRow {
        id,
        schedule_id: Some(1),
        hook_id: None,
        agent_key: "test-agent".to_string(),
        job_key: job_key.to_string(),
        timeframe: Some("15m".to_string()),
        status: status.to_string(),
        backend_run_ref: Some("ses_abc123".to_string()),
        scheduled_for: now,
        started_at: Some(now),
        finished_at: Some(now + chrono::Duration::seconds(42)),
        timeout_seconds: 600,
        error_summary: None,
    }
}

pub fn bp(at: DateTime<Utc>, balance: Decimal) -> BalancePoint {
    BalancePoint {
        bucket: at,
        balance,
    }
}
