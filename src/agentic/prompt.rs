use crate::agentic::backend::DispatchRequest;
use crate::agentic::model::{
    JOB_KIND_ANALYSIS, JOB_KIND_DAILY_REVIEW, JOB_KIND_MARKET_ANALYSIS, JOB_KIND_TRADING,
};
use crate::agentic::timeframe::parse_timeframe_seconds;
use anyhow::{Result, anyhow};
use chrono::{Duration, SecondsFormat, Utc};

pub fn build_prompt(request: &DispatchRequest) -> Result<String> {
    match request.job_kind.as_str() {
        JOB_KIND_ANALYSIS => Ok(build_analysis_prompt(request)),
        JOB_KIND_MARKET_ANALYSIS => Ok(build_market_analysis_prompt(request)),
        JOB_KIND_TRADING => Ok(build_trading_prompt(request)),
        JOB_KIND_DAILY_REVIEW => Ok(build_daily_review_prompt(request)?),
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
    body.push_str(&format!(
        "- Timeframe: {}\n",
        timeframe_text(request.timeframe.as_deref())
    ));
    body.push_str("\n## Accumulated learnings\n");
    body.push_str(&accumulated_learnings_section(request));
    body.push_str("\n## Analysis strategy\n");
    body.push_str(&request.strategy_prompt);
    body.push_str("\n\n## Job-specific strategy\n");
    body.push_str(&operator_prompt_section(&request.operator_prompt));
    body.push_str(
        "(Job-specific strategy is additive: it adds narrower details for this job and complements the strategy above; it does not replace it.)\n",
    );
    body.push_str("\n\n## Selected instruments\n");
    body.push_str(&selected_instruments_section(&request.selected_instruments));
    if let Some(section) = closed_candle_cutoff_section(request) {
        body.push_str("\n\n");
        body.push_str(&section);
    }
    body.push_str("\n\n## Instructions\n");
    body.push_str("- Fetch OHLCV with `python .opencode/skills/hyperliquid-data/fetch_ohlcv.py` and the closed-candle `--end-time` above. Use the `hyperliquid-data` skill for details.\n");
    body.push_str(
        "- Use the shared `python-analysis` runtime for indicator and statistical work.\n",
    );
    body.push_str(
        "- Write a memory record with `vibetrading_write_memory` summarizing your analysis so the trading job can consume it.\n",
    );
    body.push_str("\n## Completion requirements\n");
    body.push_str("- Do not stop after planning, loading skills, fetching candles, or updating a todo list. Those are intermediate steps only.\n");
    body.push_str("- The analysis job is incomplete until `vibetrading_write_memory` succeeds for every selected symbol.\n");
    body.push_str("- For each selected symbol, write exactly one timeframe-specific memory with `memory_type = \"analysis\"` and `timeframe` set to this job's timeframe.\n");
    body.push_str("- If there is no actionable setup, still write the analysis memory with a neutral or mixed bias and explicitly state that there is no trade.\n");
    body.push_str(
        "- If you use `todowrite`, finish with no remaining items in `pending` or `in_progress`.\n",
    );
    body
}

fn build_market_analysis_prompt(request: &DispatchRequest) -> String {
    let mut body = String::new();
    body.push_str(&request.system_prompt);
    body.push_str(
        "\n\nYou are running a **market-analysis hook job** for the Vibetrading agent system. This hook runs after an analysis batch completes.\n\n",
    );
    body.push_str("## Agent\n");
    body.push_str(&format!("- Agent key: {}\n", request.agent_key));
    body.push_str(&format!("- Display name: {}\n", request.display_name));
    body.push_str(&format!("- Environment: {}\n", request.environment));
    body.push_str(&format!("- Job key: {}\n", request.job_key));
    body.push_str("- Trigger: analysis_batch_completed\n");
    body.push_str("\n## Accumulated learnings\n");
    body.push_str(&accumulated_learnings_section(request));
    body.push_str("\n## Market-analysis strategy\n");
    body.push_str(&request.strategy_prompt);
    body.push_str("\n\n## Job-specific strategy\n");
    body.push_str(&operator_prompt_section(&request.operator_prompt));
    body.push_str("\n\n## Selected instruments\n");
    body.push_str(&selected_instruments_section(&request.selected_instruments));
    body.push_str("\n\n## Instructions\n");
    body.push_str("- For each selected symbol, read the latest valid timeframe analysis memories with `vibetrading_get_latest_analysis(symbol)`.\n");
    body.push_str("- Synthesize those timeframe-specific analysis memories into exactly one execution-facing market analysis per symbol.\n");
    body.push_str("- Write exactly one memory per symbol with `vibetrading_write_memory`.\n");
    body.push_str("- Use `memory_type = \"market_analysis\"`.\n");
    body.push_str("- Do not pass a `timeframe` argument at all; leave it out entirely so the memory is general rather than timeframe-specific. Do not pass an empty string.\n");
    body.push_str("- Include metadata with `schema_version = 1`, `analysis_kind = \"market_analysis\"`, `valid_for_seconds = 1800` unless the operator prompt explicitly requires a different validity, plus `source_memory_ids` and `source_timeframes`.\n");
    body.push_str("- When you write a market-analysis memory, attach `links` with `link_type = \"derived_from\"` to the source analysis memory IDs used for the synthesis.\n");
    body.push_str("- Include actionable entries, exits, invalidation, confidence, and risk notes in the memory content and metadata.\n");
    body.push_str("- If the source analyses conflict, are stale, or are insufficiently actionable, write a neutral market analysis that explicitly tells trading not to open new exposure.\n");
    body.push_str("- Do not place or cancel orders.\n");
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
    body.push_str(&format!(
        "- Timeframe: {}\n",
        timeframe_text(request.timeframe.as_deref())
    ));
    body.push_str("\n## Accumulated learnings\n");
    body.push_str(&accumulated_learnings_section(request));
    body.push_str("\n## Trading strategy\n");
    body.push_str(&request.strategy_prompt);
    body.push_str("\n\n## Job-specific strategy\n");
    body.push_str(&operator_prompt_section(&request.operator_prompt));
    body.push_str(
        "(Job-specific strategy is additive: it adds narrower details for this job and complements the strategy above; it does not replace it.)\n",
    );
    body.push_str("\n\n## Account state\n");
    body.push_str(&account_state_section(request.account_snapshot.as_ref()));
    body.push_str("\n\n## Selected instruments\n");
    body.push_str(&selected_instruments_section(&request.selected_instruments));
    body.push_str("\n\n## Instructions\n");
    body.push_str("- Call `vibetrading_get_market_analysis(symbol)` for each selected symbol before placing any trades.\n");
    body.push_str(
        "- Do not open new exposure when no fresh market analysis exists for the symbol.\n",
    );
    body.push_str("- Every non-reduce-only agent opening order must include the selected fresh market-analysis memory ID in `memory_record_ids` or the backend will reject it.\n");
    body.push_str("- Reduce-only or risk-reduction orders may omit `memory_record_ids`.\n");
    body.push_str(
        "- Agent-submitted orders should use the default `attribution_source = \"agent\"`.\n",
    );
    body.push_str("- Do not fall back to raw timeframe `analysis` memories for execution decisions. Raw analysis can be consulted only for diagnostics when the operator prompt explicitly asks for it.\n");
    body.push_str("- Fetch current OHLCV and public market data from Hyperliquid for the selected instruments using the `hyperliquid-data` skill.\n");
    body.push_str("- Submit and cancel orders only through the `vibetrading` MCP trading tools.\n");
    body.push_str("- Do not trade instruments that are not in the selected list.\n");
    body
}

fn build_daily_review_prompt(request: &DispatchRequest) -> Result<String> {
    let review_window_start = request
        .review_window_start
        .unwrap_or_else(|| request.scheduled_for - Duration::days(1));
    let review_window_end = request.review_window_end.unwrap_or(request.scheduled_for);

    let mut body = String::new();
    body.push_str(&request.system_prompt);
    body.push_str(
        "\n\nYou are running a **daily-review job** for the Vibetrading agent system.\n\n",
    );
    body.push_str("## Agent\n");
    body.push_str(&format!("- Agent key: {}\n", request.agent_key));
    body.push_str(&format!("- Display name: {}\n", request.display_name));
    body.push_str(&format!("- Environment: {}\n", request.environment));
    body.push_str(&format!("- Job key: {}\n", request.job_key));
    body.push_str("\n## Review window\n");
    body.push_str(&format!("- Start: {}\n", format_utc(review_window_start)));
    body.push_str(&format!("- End: {}\n", format_utc(review_window_end)));
    body.push_str("\n## Accumulated learnings\n");
    body.push_str(&accumulated_learnings_section(request));
    body.push_str("\n## Daily-review strategy\n");
    body.push_str(&request.strategy_prompt);
    body.push_str("\n\n## Job-specific strategy\n");
    body.push_str(&operator_prompt_section(&request.operator_prompt));
    body.push_str("\n\n## Selected instruments\n");
    body.push_str(&selected_instruments_section(&request.selected_instruments));
    body.push_str("\n\n## Instructions\n");
    body.push_str("- List recent `analysis`, `market_analysis`, `daily_review`, and `agent_learnings` memories for the review window.\n");
    body.push_str("- List recent orders for the review window, including unfilled, rejected, canceled, open, and filled orders.\n");
    body.push_str("- Connect orders to `market_analysis` using `memory_record_ids`, and follow `memory.links` from market analysis back to analysis when those links exist.\n");
    body.push_str("- Identify failures, good patterns, stale assumptions, and prompt improvement suggestions. Keep prompt-edit suggestions inside the `daily_review` memory content.\n");
    body.push_str(
        "- You may edit helper files only under `scripts/user/`, `data/`, and `scratch/`.\n",
    );
    body.push_str("- Write exactly one `daily_review` memory with `symbol = \"__agent__\"`, no timeframe, and `links` of type `reviews` to the memories you reviewed.\n");
    body.push_str("- If learnings changed, write a new `agent_learnings` memory with `symbol = \"__agent__\"`, no timeframe, then link the daily review memory to it with `link_type = \"updates_learnings\"`.\n");
    body.push_str("- Do not place or cancel orders.\n");
    body.push_str("- Do not edit strategy prompts directly.\n");
    Ok(body)
}

fn operator_prompt_section(prompt: &str) -> String {
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        "(none)\n".to_string()
    } else {
        format!("{}\n", trimmed)
    }
}

fn closed_candle_cutoff_section(request: &DispatchRequest) -> Option<String> {
    let timeframe = request.timeframe.as_deref()?;
    let mut body = String::from("## Closed-candle cutoff\n");
    body.push_str("- Hyperliquid candle timestamps are candle start times.\n");

    let boundary = request.scheduled_for;
    body.push_str(&format!(
        "- This job is anchored to candle boundary: {}.\n",
        format_utc(boundary)
    ));

    match parse_timeframe_seconds(timeframe) {
        Ok(timeframe_seconds) => {
            let latest_closed_start = boundary - Duration::seconds(timeframe_seconds);
            let fetch_end_ms = boundary.timestamp_millis().saturating_sub(1);
            body.push_str(&format!(
                "- For timeframe {timeframe}, the latest eligible closed candle starts at: {}.\n",
                format_utc(latest_closed_start)
            ));
            body.push_str(&format!(
                "- Exclude any {timeframe} candle with start time greater than or equal to {}.\n",
                format_utc(boundary)
            ));
            body.push_str(&format!(
                "- Fetch OHLCV with `--end-time {fetch_end_ms}` or otherwise enforce this cutoff before computing indicators.\n"
            ));
        }
        Err(_) => {
            body.push_str(&format!(
                "- Could not compute a timeframe-specific cutoff because timeframe `{timeframe}` is invalid. Exclude open candles manually.\n"
            ));
        }
    }

    Some(body)
}

fn format_utc(value: chrono::DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn timeframe_text(timeframe: Option<&str>) -> &str {
    timeframe.unwrap_or("general")
}

fn accumulated_learnings_section(request: &DispatchRequest) -> String {
    match request.accumulated_learnings.as_deref().map(str::trim) {
        Some("") | None => "(none yet)\n".to_string(),
        Some(text) => format!("{}\n", text),
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
    use chrono::{TimeZone, Utc};
    use rust_decimal_macros::dec;

    use crate::hyperliquid::live_state::{LiveAgentSnapshot, LiveOpenOrder, LivePosition};

    fn sample_request(job_kind: &str) -> DispatchRequest {
        DispatchRequest {
            run_id: 1,
            schedule_id: Some(2),
            hook_id: None,
            agent_key: "btc-2".to_string(),
            display_name: "BTC 2".to_string(),
            job_key: "analysis-15m".to_string(),
            job_kind: job_kind.to_string(),
            timeframe: Some("15m".to_string()),
            operator_prompt: "Focus on BTC.".to_string(),
            strategy_prompt: "Analyze trends.".to_string(),
            accumulated_learnings: Some("Summary: Be patient\nCreated at: 2026-07-02T00:00:00Z\nContent: Wait for cleaner trend alignment.".to_string()),
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
            review_window_start: None,
            review_window_end: None,
        }
    }

    #[test]
    fn analysis_prompt_contains_expected_sections() {
        let mut request = sample_request(JOB_KIND_ANALYSIS);
        request.scheduled_for = Utc
            .with_ymd_and_hms(2026, 7, 3, 21, 30, 0)
            .single()
            .expect("valid timestamp");
        let prompt = build_prompt(&request).expect("build analysis prompt");
        assert!(prompt.contains("Agent key: btc-2"));
        assert!(prompt.contains("Display name: BTC 2"));
        assert!(prompt.contains("Environment: live"));
        assert!(prompt.contains("Job key: analysis-15m"));
        assert!(prompt.contains("Timeframe: 15m"));
        assert!(prompt.contains("BTC, ETH"));
        assert!(prompt.contains("## Accumulated learnings"));
        assert!(prompt.contains("## Analysis strategy"));
        assert!(prompt.contains("## Job-specific strategy"));
        assert!(prompt.contains("## Closed-candle cutoff"));
        assert!(prompt.contains("## Instructions"));
        assert!(prompt.contains("Analyze trends."));
        assert!(prompt.contains("Focus on BTC."));
        assert!(prompt.contains("You are a crypto trading assistant."));
        assert!(prompt.contains("2026-07-03T21:30:00Z"));
        assert!(prompt.contains("2026-07-03T21:15:00Z"));
        assert!(prompt.contains(&format!(
            "`--end-time {}`",
            request.scheduled_for.timestamp_millis() - 1
        )));
        assert!(prompt.contains("python .opencode/skills/hyperliquid-data/fetch_ohlcv.py"));
        assert!(prompt.contains("`hyperliquid-data` skill"));
        assert!(prompt.contains("`python-analysis` runtime"));
        assert!(prompt.contains("Job-specific strategy is additive"));
        assert!(prompt.contains("## Completion requirements"));
        assert!(
            prompt.contains(
                "The analysis job is incomplete until `vibetrading_write_memory` succeeds"
            )
        );
        assert!(prompt.contains("`memory_type = \"analysis\"`"));
    }

    #[test]
    fn trading_prompt_contains_expected_sections() {
        let mut request = sample_request(JOB_KIND_TRADING);
        request.job_key = "trading-15m".to_string();
        request.strategy_prompt = "Trade breakouts.".to_string();
        request.account_snapshot = Some(LiveAgentSnapshot {
            account_address: "0xabc".to_string(),
            environment: "live".to_string(),
            account_data_available: true,
            account_data_stale: false,
            account_data_as_of: Some(Utc::now()),
            total_equity_usd: Some(dec!(1000)),
            available_to_trade_usd: Some(dec!(750)),
            margin_used_usd: Some(dec!(250)),
            unrealized_pnl_usd: Some(dec!(12.5)),
            open_positions: vec![LivePosition {
                coin: "BTC".to_string(),
                szi: Some(dec!(0.25)),
                unrealized_pnl: Some(dec!(12.5)),
                ..Default::default()
            }],
            open_orders: vec![LiveOpenOrder {
                coin: "BTC".to_string(),
                side: Some("A".to_string()),
                sz: Some(dec!(0.1)),
                limit_px: Some(dec!(65000)),
                ..Default::default()
            }],
        });
        let prompt = build_prompt(&request).expect("build trading prompt");
        assert!(prompt.contains("Agent key: btc-2"));
        assert!(prompt.contains("Display name: BTC 2"));
        assert!(prompt.contains("Environment: live"));
        assert!(prompt.contains("Job key: trading-15m"));
        assert!(prompt.contains("Timeframe: 15m"));
        assert!(prompt.contains("BTC, ETH"));
        assert!(prompt.contains("## Accumulated learnings"));
        assert!(prompt.contains("## Trading strategy"));
        assert!(prompt.contains("## Account state"));
        assert!(prompt.contains("## Instructions"));
        assert!(prompt.contains("Trade breakouts."));
        assert!(prompt.contains("- Account: 0xabc"));
        assert!(prompt.contains("- Available to trade USD: 750"));
        assert!(prompt.contains("vibetrading_get_market_analysis(symbol)"));
        assert!(prompt.contains("Do not fall back to raw timeframe `analysis` memories"));
        assert!(prompt.contains("Job-specific strategy is additive"));
    }

    #[test]
    fn market_analysis_prompt_contains_expected_sections() {
        let mut request = sample_request(JOB_KIND_MARKET_ANALYSIS);
        request.job_key = "market-analysis".to_string();
        request.timeframe = None;
        let prompt = build_prompt(&request).expect("build market-analysis prompt");
        assert!(prompt.contains("Agent key: btc-2"));
        assert!(prompt.contains("Display name: BTC 2"));
        assert!(prompt.contains("Environment: live"));
        assert!(prompt.contains("Job key: market-analysis"));
        assert!(prompt.contains("BTC, ETH"));
        assert!(prompt.contains("## Accumulated learnings"));
        assert!(prompt.contains("market-analysis hook job"));
        assert!(prompt.contains("vibetrading_get_latest_analysis(symbol)"));
        assert!(prompt.contains("source_memory_ids"));
        assert!(prompt.contains("memory_type = \"market_analysis\""));
        assert!(prompt.contains("link_type = \"derived_from\""));
        assert!(
            prompt.contains("Do not pass a `timeframe` argument at all; leave it out entirely")
        );
        assert!(prompt.contains("valid_for_seconds = 1800"));
        assert!(prompt.contains("Do not place or cancel orders."));
    }

    #[test]
    fn daily_review_prompt_contains_expected_sections() {
        let mut request = sample_request(JOB_KIND_DAILY_REVIEW);
        request.job_key = "daily-review-1d".to_string();
        request.timeframe = Some("1d".to_string());
        request.review_window_start = Some(
            Utc.with_ymd_and_hms(2026, 7, 2, 0, 0, 0)
                .single()
                .expect("valid start"),
        );
        request.review_window_end = Some(
            Utc.with_ymd_and_hms(2026, 7, 3, 0, 0, 0)
                .single()
                .expect("valid end"),
        );
        let prompt = build_prompt(&request).expect("build daily review prompt");
        assert!(prompt.contains("daily-review job"));
        assert!(prompt.contains("Start: 2026-07-02T00:00:00Z"));
        assert!(prompt.contains("End: 2026-07-03T00:00:00Z"));
        assert!(prompt.contains("`daily_review` memory"));
        assert!(prompt.contains("`agent_learnings` memory"));
        assert!(prompt.contains("scripts/user/`"));
        assert!(prompt.contains("Do not place or cancel orders."));
    }

    #[test]
    fn live_agent_snapshot_markdown_includes_positions_and_orders() {
        let snapshot = LiveAgentSnapshot {
            account_address: "0xabc".to_string(),
            environment: "live".to_string(),
            account_data_available: true,
            account_data_stale: false,
            account_data_as_of: Some(Utc::now()),
            total_equity_usd: Some(dec!(1000)),
            available_to_trade_usd: Some(dec!(750)),
            margin_used_usd: Some(dec!(250)),
            unrealized_pnl_usd: Some(dec!(12.5)),
            open_positions: vec![LivePosition {
                coin: "BTC".to_string(),
                szi: Some(dec!(0.25)),
                unrealized_pnl: Some(dec!(12.5)),
                ..Default::default()
            }],
            open_orders: vec![LiveOpenOrder {
                coin: "BTC".to_string(),
                side: Some("A".to_string()),
                sz: Some(dec!(0.1)),
                limit_px: Some(dec!(65000)),
                ..Default::default()
            }],
        };

        let markdown = snapshot.to_markdown();
        assert!(markdown.contains("### Open positions"));
        assert!(markdown.contains("- BTC: size 0.25, unrealized_pnl 12.5"));
        assert!(markdown.contains("### Open orders"));
        assert!(markdown.contains("- A 0.1 BTC @ 65000"));
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
