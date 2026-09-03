use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::{
    agents::model::{AgentDetailRow, AgentListRow, AgentReadiness},
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

pub fn sample_agent_readiness() -> AgentReadiness {
    AgentReadiness {
        agent_key: "test-agent".to_string(),
        active: true,
        enabled: true,
        has_selected_instruments: true,
        has_enabled_analysis_job: true,
        has_enabled_market_analysis_job: true,
        has_enabled_trading_job: true,
        market_analysis_sub_agent_id: Some(2),
        trading_sub_agent_id: Some(3),
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
    sample_agent_detail_row()
}

pub fn sample_candle_job_row(
    id: i64,
    sub_agent_key: &str,
    sub_agent_kind: &str,
    enabled: bool,
) -> crate::harness::model::HarnessSubAgentRow {
    let now = Utc::now();
    let timeframe = if sub_agent_kind == "trading" {
        "5m"
    } else {
        "15m"
    };
    crate::harness::model::HarnessSubAgentRow {
        id,
        agent_key: "test-agent".to_string(),
        sub_agent_key: sub_agent_key.to_string(),
        sub_agent_kind: sub_agent_kind.to_string(),
        enabled,
        timeframe: Some(timeframe.to_string()),
        next_run_at: Some(now),
        model_provider_id: Some("anthropic".to_string()),
        model_id: Some("claude-3-5-sonnet".to_string()),
        model_variant: None,
        timeout_seconds: 600,
        operator_prompt: String::new(),
        enabled_capabilities: if sub_agent_kind == "trading" {
            vec!["hypervibes:notification_send".to_string()]
        } else {
            Vec::new()
        },
        created_at: now,
        updated_at: now,
    }
}

pub fn sample_event_job_row(id: i64, enabled: bool) -> crate::harness::model::HarnessSubAgentRow {
    let now = Utc::now();
    crate::harness::model::HarnessSubAgentRow {
        id,
        agent_key: "test-agent".to_string(),
        sub_agent_key: "market-analysis".to_string(),
        sub_agent_kind: crate::harness::model::SUB_AGENT_KIND_MARKET_ANALYSIS.to_string(),
        enabled,
        timeframe: None,
        next_run_at: None,
        model_provider_id: Some("anthropic".to_string()),
        model_id: Some("claude-3-5-sonnet".to_string()),
        model_variant: None,
        timeout_seconds: 600,
        operator_prompt: String::new(),
        enabled_capabilities: Vec::new(),
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
    sub_agent_key: &str,
) -> crate::harness::model::HarnessSubAgentRunRow {
    let now = Utc::now();
    crate::harness::model::HarnessSubAgentRunRow {
        id,
        sub_agent_id: 1,
        agent_key: "test-agent".to_string(),
        sub_agent_key: sub_agent_key.to_string(),
        sub_agent_kind: "analysis".to_string(),
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
