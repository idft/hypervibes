use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::{
    agents::model::{AgentDetailRow, AgentListRow},
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
        trading_account_address: "0x1234567890abcdef".to_string(),
        environment: "live".to_string(),
        api_key_last_used_at: None,
    }
}

pub fn sample_agent_detail_row() -> AgentDetailRow {
    AgentDetailRow {
        display_name: "Test Agent".to_string(),
        agent_key: "test-agent".to_string(),
        enabled: true,
        lifecycle: crate::agents::model::AGENT_LIFECYCLE_ACTIVE.to_string(),
        trading_account_address: Some("0x1234567890abcdef".to_string()),
        environment: "live".to_string(),
        api_key: "vt_test_key".to_string(),
        runtime_config: serde_json::json!({}),
    }
}

pub fn sample_account_balance_view() -> AccountBalanceView {
    AccountBalanceView {
        total_balance: Some(rust_decimal::Decimal::new(232_6800, 4)),
        total_u_pnl: AnimatedNumber::for_pnl(rust_decimal::Decimal::new(12_3400, 4)),
        data_available: true,
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
    row.runtime_config = serde_json::json!({
        "workspace_host_path": "workspaces/agents/test-agent",
        "workspace_container_path": "/workspaces/agents/test-agent",
        "profile_source": "agent-runtime/workspace-template"
    });
    row
}

pub fn sample_candle_job_row(
    id: i64,
    job_key: &str,
    job_kind: &str,
    enabled: bool,
) -> crate::harness::model::HarnessJobRow {
    let now = Utc::now();
    let timeframe = if job_kind == "trading" { "1m" } else { "15m" };
    crate::harness::model::HarnessJobRow {
        id,
        agent_key: "test-agent".to_string(),
        job_key: job_key.to_string(),
        job_kind: job_kind.to_string(),
        trigger_type: crate::harness::model::TRIGGER_TYPE_CANDLE_CLOSED.to_string(),
        enabled,
        timeframe: Some(timeframe.to_string()),
        next_run_at: Some(now),
        model_provider_id: Some("anthropic".to_string()),
        model_id: Some("claude-3-5-sonnet".to_string()),
        model_variant: None,
        timeout_seconds: 600,
        operator_prompt: String::new(),
        created_at: now,
        updated_at: now,
    }
}

pub fn sample_event_job_row(id: i64, enabled: bool) -> crate::harness::model::HarnessJobRow {
    let now = Utc::now();
    crate::harness::model::HarnessJobRow {
        id,
        agent_key: "test-agent".to_string(),
        job_key: "market-analysis".to_string(),
        job_kind: crate::harness::model::JOB_KIND_MARKET_ANALYSIS.to_string(),
        trigger_type: crate::harness::model::TRIGGER_TYPE_ANALYSIS_BATCH_COMPLETED.to_string(),
        enabled,
        timeframe: None,
        next_run_at: None,
        model_provider_id: Some("anthropic".to_string()),
        model_id: Some("claude-3-5-sonnet".to_string()),
        model_variant: None,
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
        thinking_variants: vec!["high".to_string(), "low".to_string()],
    }]
}

pub fn sample_run_row(
    id: i64,
    status: &str,
    job_key: &str,
) -> crate::harness::model::HarnessRunRow {
    let now = Utc::now();
    crate::harness::model::HarnessRunRow {
        id,
        job_id: 1,
        agent_key: "test-agent".to_string(),
        job_key: job_key.to_string(),
        job_kind: "analysis".to_string(),
        trigger_type: "candle_closed".to_string(),
        timeframe: Some("15m".to_string()),
        status: status.to_string(),
        backend_run_ref: Some("ses_abc123".to_string()),
        model_provider_id: Some("anthropic".to_string()),
        model_id: Some("claude-3-5-sonnet".to_string()),
        model_variant: None,
        scheduled_for: now,
        started_at: Some(now),
        finished_at: Some(now + chrono::Duration::seconds(42)),
        timeout_seconds: 600,
        error_summary: None,
        created_at: now,
        updated_at: now,
    }
}

pub fn bp(at: DateTime<Utc>, balance: Decimal) -> BalancePoint {
    BalancePoint {
        bucket: at,
        balance,
    }
}
