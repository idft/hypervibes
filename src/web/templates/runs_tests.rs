use super::*;
use crate::web::templates::test_support::*;
use serde_json::json;

#[test]
fn run_detail_page_renders_opencode_session_sections() {
    let agent = sample_opencode_detail_row();
    let run = AgenticRunDetailView::from_row(&sample_run_row(7, "succeeded", "analysis-15m"));
    let session = OpenCodeSessionView {
        model_text: "anthropic/claude-3-5-sonnet".to_string(),
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
        transcript: vec![TranscriptItem::Message(OpenCodeMessageView {
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
        })],
        session_errors: Vec::new(),
    };

    let rendered = AgentRunDetailPageTemplate::render_view(agent, run, Some(session), true)
        .expect("render run detail page");

    assert!(rendered.contains("Agent sections"));
    assert!(
        rendered
            .contains("href=\"/agents/test-agent/jobs\" data-agent-tab-link aria-current=\"page\"")
    );
    assert!(!rendered.contains("hx-target=\"#agent-show-tab-content\""));
    assert!(rendered.contains("Scheduled for"));
    assert!(rendered.contains("Timeframe"));
    assert!(rendered.contains("Duration"));
    assert!(rendered.contains("Timeout"));
    assert!(rendered.contains("Started"));
    assert!(rendered.contains("Finished"));
    assert!(rendered.contains("Model"));
    assert!(rendered.contains("Estimated cost"));
    assert!(rendered.contains("anthropic/claude-3-5-sonnet"));
    assert!(rendered.contains("1,200"));
    assert!(!rendered.contains(">Transcript<"));
    assert!(!rendered.contains("mirrored messages from the OpenCode conversation"));
    assert!(!rendered.contains(">Commands<"));
    assert!(!rendered.contains(">Tool executions<"));
    assert!(rendered.contains("Analysis complete"));
}

#[test]
fn hook_run_detail_view_uses_dash_timeframe_and_hook_job_url() {
    let mut row = sample_run_row(8, "succeeded", "market-analysis");
    row.schedule_id = None;
    row.hook_id = Some(3);
    row.timeframe = None;

    let run = AgenticRunDetailView::from_row(&row);
    assert_eq!(run.timeframe_text, "—");
    assert_eq!(run.job_url, Some("/agents/test-agent/hooks/3".to_string()));
    assert_eq!(run.job_label, "hook");
}

#[test]
fn pick_json_preview_prefers_priority_field_and_truncates() {
    let value = json!({
        "command": "echo hi",
        "extra": "noise",
    });
    assert_eq!(pick_json_preview(Some(&value)), "echo hi");
}

#[test]
fn pick_json_preview_falls_back_to_first_field() {
    let value = json!({
        "alpha_key": "alpha value",
        "beta_key": "beta value",
    });
    let preview = pick_json_preview(Some(&value));
    assert!(
        preview == "alpha value" || preview == "beta value",
        "expected one of the field values, got: {preview}"
    );
}

#[test]
fn pick_json_preview_handles_arrays_and_scalars() {
    assert_eq!(
        pick_json_preview(Some(&json!(["first", "second"]))),
        "first"
    );
    assert_eq!(pick_json_preview(Some(&json!("just a string"))), "just a string");
    assert_eq!(pick_json_preview(Some(&json!(42))), "42");
    assert_eq!(pick_json_preview(None), "");
    assert_eq!(pick_json_preview(Some(&json!({}))), "");
}

#[test]
fn pick_json_preview_truncates_long_strings() {
    let long = "a".repeat(500);
    let value = json!({ "content": long });
    let preview = pick_json_preview(Some(&value));
    assert!(preview.ends_with('…'));
    assert_eq!(preview.chars().count(), 201);
}
