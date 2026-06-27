use crate::agentic::backend::DispatchRequest;
use crate::agentic::model::{JOB_KIND_ANALYSIS, JOB_KIND_TRADING};
use anyhow::{Result, anyhow};

pub fn build_prompt(request: &DispatchRequest) -> Result<String> {
    match request.job_kind.as_str() {
        JOB_KIND_ANALYSIS => Ok(build_analysis_prompt(request)),
        JOB_KIND_TRADING => Ok(build_trading_prompt(request)),
        other => Err(anyhow!("unknown job kind for prompt building: {other}")),
    }
}

fn build_analysis_prompt(request: &DispatchRequest) -> String {
    let mut body = String::new();
    body.push_str(&request.system_prompt);
    body.push_str("\n\nYou are running an **analysis job** for the Vibetrading agent system.\n\n");
    body.push_str("## Agent\n");
    body.push_str(&format!("- Agent key: {}\n", request.agent_key));
    body.push_str(&format!("- Display name: {}\n", request.display_name));
    body.push_str(&format!("- Environment: {}\n", request.environment));
    body.push_str(&format!("- Job key: {}\n", request.job_key));
    body.push_str(&format!("- Timeframe: {}\n", request.timeframe));
    body.push_str("\n## Analysis strategy\n");
    body.push_str(&request.analysis_prompt);
    body.push_str("\n\n## Job-specific strategy\n");
    body.push_str(&operator_prompt_section(&request.operator_prompt));
    body.push_str("\n\n## Selected instruments\n");
    body.push_str(&selected_instruments_section(&request.selected_instruments));
    body.push_str("\n\n## Instructions\n");
    body.push_str("- Fetch OHLCV and relevant market data from Hyperliquid for the selected instruments.\n");
    body.push_str("- Analyze market structure, trend, volatility, support/resistance, liquidity zones, and risk/reward.\n");
    body.push_str("- Only produce actionable setups when confidence is at least the threshold defined in the strategy.\n");
    body.push_str("- If there is no clear edge, mark the bias neutral or mixed and provide no actionable setup.\n");
    body.push_str("- Write a memory record summarizing your analysis so the trading job can consume it.\n");
    body
}

fn build_trading_prompt(request: &DispatchRequest) -> String {
    let mut body = String::new();
    body.push_str(&request.system_prompt);
    body.push_str("\n\nYou are running a **trading job** for the Vibetrading agent system.\n\n");
    body.push_str("## Agent\n");
    body.push_str(&format!("- Agent key: {}\n", request.agent_key));
    body.push_str(&format!("- Display name: {}\n", request.display_name));
    body.push_str(&format!("- Environment: {}\n", request.environment));
    body.push_str(&format!("- Job key: {}\n", request.job_key));
    body.push_str(&format!("- Timeframe: {}\n", request.timeframe));
    body.push_str("\n## Trading strategy\n");
    body.push_str(&request.trading_prompt);
    body.push_str("\n\n## Job-specific strategy\n");
    body.push_str(&operator_prompt_section(&request.operator_prompt));
    body.push_str("\n\n## Account state\n");
    body.push_str(&account_state_section(request.account_snapshot.as_ref()));
    body.push_str("\n\n## Selected instruments\n");
    body.push_str(&selected_instruments_section(&request.selected_instruments));
    body.push_str("\n\n## Instructions\n");
    body.push_str("- Read the latest analysis memory(ies) before placing any trades.\n");
    body.push_str("- Fetch current OHLCV and market data from Hyperliquid for the selected instruments.\n");
    body.push_str("- Use limit orders for new entries. Avoid full-size entries on first fill.\n");
    body.push_str("- Only open new exposure when analysis is fresh, non-neutral, and confidence meets the strategy threshold.\n");
    body.push_str("- Cancel unfilled entry orders when the source analysis expires or is invalidated.\n");
    body.push_str("- Avoid duplicate resting orders at similar prices.\n");
    body.push_str("- Do not trade instruments that are not in the selected list.\n");
    body
}

fn operator_prompt_section(prompt: &str) -> String {
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        "(none)\n".to_string()
    } else {
        format!("{}\n", trimmed)
    }
}

fn selected_instruments_section(instruments: &[String]) -> String {
    if instruments.is_empty() {
        "None. Do not analyze markets or place trades.\n".to_string()
    } else {
        let joined = instruments.join(", ");
        format!("{}\n", joined)
    }
}

fn account_state_section(
    snapshot: Option<&crate::hyperliquid::live_state::LiveAgentSnapshot>,
) -> String {
    match snapshot {
        Some(s) => format!("{}\n", s.to_markdown()),
        None => "Account state unavailable. Do not place new opening orders.\n".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn sample_request(job_kind: &str) -> DispatchRequest {
        DispatchRequest {
            run_id: 1,
            schedule_id: 2,
            agent_key: "btc-2".to_string(),
            display_name: "BTC 2".to_string(),
            job_key: "analysis-15m".to_string(),
            job_kind: job_kind.to_string(),
            timeframe: "15m".to_string(),
            operator_prompt: "Focus on BTC.".to_string(),
            analysis_prompt: "Analyze trends.".to_string(),
            trading_prompt: "Trade breakouts.".to_string(),
            system_prompt: "You are a crypto trading assistant.".to_string(),
            environment: "live".to_string(),
            selected_instruments: vec!["BTC".to_string(), "ETH".to_string()],
            account_snapshot: None,
            model_provider_id: None,
            model_id: None,
            timeout_seconds: 10,
            runtime_base_url: "http://localhost:14096".to_string(),
            runtime_config: serde_json::json!({}),
            scheduled_for: Utc::now(),
        }
    }

    #[test]
    fn analysis_prompt_contains_expected_sections() {
        let request = sample_request(JOB_KIND_ANALYSIS);
        let prompt = build_prompt(&request).expect("build analysis prompt");
        assert!(prompt.contains("## Analysis strategy"));
        assert!(prompt.contains("## Job-specific strategy"));
        assert!(prompt.contains("## Instructions"));
        assert!(prompt.contains("Analyze trends."));
        assert!(prompt.contains("You are a crypto trading assistant."));
    }

    #[test]
    fn trading_prompt_contains_expected_sections() {
        let request = sample_request(JOB_KIND_TRADING);
        let prompt = build_prompt(&request).expect("build trading prompt");
        assert!(prompt.contains("## Trading strategy"));
        assert!(prompt.contains("## Account state"));
        assert!(prompt.contains("## Instructions"));
        assert!(prompt.contains("Trade breakouts."));
        assert!(prompt.contains("Account state unavailable. Do not place new opening orders."));
    }

    #[test]
    fn unknown_job_kind_returns_error() {
        let request = sample_request("unknown");
        let result = build_prompt(&request);
        assert!(result.is_err());
        let message = format!("{}", result.unwrap_err());
        assert!(message.contains("unknown job kind"));
    }
}
