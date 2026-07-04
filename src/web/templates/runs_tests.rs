use super::*;
use crate::web::templates::test_support::*;

#[test]
fn run_detail_page_renders_opencode_session_sections() {
    let agent = sample_opencode_detail_row();
    let run = AgenticRunDetailView::from_row(&sample_run_row(7, "succeeded", "analysis-15m"));
    let session = OpenCodeSessionView {
        id: "ses_abc123".to_string(),
        title: "btc-2 analysis run".to_string(),
        status: "idle".to_string(),
        directory: "/workspaces/agents/test-agent".to_string(),
        model_text: "anthropic/claude-3-5-sonnet".to_string(),
        created_at: LocalTimestampView {
            iso: "2026-06-27T00:00:00Z".to_string(),
            fallback_text: "2026-06-27 00:00 UTC".to_string(),
        },
        updated_at: LocalTimestampView {
            iso: "2026-06-27T00:02:00Z".to_string(),
            fallback_text: "2026-06-27 00:02 UTC".to_string(),
        },
        input_tokens_text: "1,200".to_string(),
        output_tokens_text: "800".to_string(),
        cache_read_tokens_text: "0".to_string(),
        cache_write_tokens_text: "0".to_string(),
        reasoning_tokens_text: "50".to_string(),
        context_tokens_text: "8,000".to_string(),
        peak_context_tokens_text: "8,500".to_string(),
        estimated_cost_text: "0.123456".to_string(),
        compaction_count_text: "1".to_string(),
        share_url: String::new(),
        commands: vec![OpenCodeCommandView {
            created_at: LocalTimestampView {
                iso: "2026-06-27T00:00:00Z".to_string(),
                fallback_text: "2026-06-27 00:00 UTC".to_string(),
            },
            command_name: "vibetrading-analysis".to_string(),
            command_args: "Agent key: test-agent".to_string(),
        }],
        messages: vec![OpenCodeMessageView {
            created_at: LocalTimestampView {
                iso: "2026-06-27T00:01:00Z".to_string(),
                fallback_text: "2026-06-27 00:01 UTC".to_string(),
            },
            role_label: "assistant".to_string(),
            role_class: "border-sky-900/60 bg-sky-950/30 text-sky-300".to_string(),
            model_text: "anthropic/claude-3-5-sonnet".to_string(),
            text: "Analysis complete".to_string(),
            summary: "Trend remains constructive".to_string(),
            system_prompt: String::new(),
        }],
        tool_executions: vec![OpenCodeToolExecutionView {
            started_at: Some(LocalTimestampView {
                iso: "2026-06-27T00:01:00Z".to_string(),
                fallback_text: "2026-06-27 00:01 UTC".to_string(),
            }),
            completed_at: Some(LocalTimestampView {
                iso: "2026-06-27T00:01:45Z".to_string(),
                fallback_text: "2026-06-27 00:01 UTC".to_string(),
            }),
            tool_name: "vibetrading.get_positions".to_string(),
            success_label: "success".to_string(),
            success_class: "border-emerald-900/60 bg-emerald-950/30 text-emerald-300".to_string(),
            duration_text: "45ms".to_string(),
            args_json: "{}".to_string(),
            result_json: "{}".to_string(),
            error_text: String::new(),
        }],
        session_errors: Vec::new(),
    };

    let rendered = AgentRunDetailPageTemplate::render_view(agent, run, Some(session), true)
        .expect("render run detail page");

    assert!(rendered.contains("OpenCode session"));
    assert!(rendered.contains("ses_abc123"));
    assert!(rendered.contains("Transcript"));
    assert!(rendered.contains("Tool executions"));
    assert!(rendered.contains("Analysis complete"));
}

#[test]
fn hook_run_detail_view_uses_dash_timeframe_and_hook_job_url() {
    let mut row = sample_run_row(8, "succeeded", "market-analysis");
    row.schedule_id = None;
    row.hook_id = Some(3);
    row.job_kind = crate::agentic::model::JOB_KIND_MARKET_ANALYSIS.to_string();
    row.timeframe = None;

    let run = AgenticRunDetailView::from_row(&row);
    assert_eq!(run.timeframe_text, "—");
    assert_eq!(run.job_url, Some("/agents/test-agent/hooks/3".to_string()));
    assert_eq!(run.job_label, "hook");
}
