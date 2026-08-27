use super::*;
use crate::web::templates::test_support::*;

#[test]
fn job_detail_page_renders_job_metadata_and_runs() {
    let agent = sample_opencode_detail_row();
    let job = HarnessSubAgentDetailView::from_row(&sample_candle_job_row(
        1,
        "analysis-15m",
        "analysis",
        true,
    ));
    let runs = vec![
        HarnessSubAgentRunView::from_row(&sample_run_row(1, "succeeded", "analysis-15m")),
        HarnessSubAgentRunView::from_row(&sample_run_row(2, "failed", "analysis-15m")),
    ];

    let rendered = AgentJobDetailPageTemplate::render_view(
        agent,
        job,
        ModelPickerView {
            input_id: "sub-agent-model-selection".to_string(),
            input_name: "model_selection".to_string(),
            variant_input_id: "sub-agent-model-selection-variant".to_string(),
            variant_input_name: "model_variant".to_string(),
            selected_value: "anthropic/claude-sonnet-4".to_string(),
            selected_variant: String::new(),
            selected_label: "Anthropic / Claude Sonnet 4".to_string(),
            empty_label: "None selected".to_string(),
            provider_groups: vec![ModelPickerProviderGroup {
                provider_id: "anthropic".to_string(),
                provider_name: "Anthropic".to_string(),
                provider_logo_url: Some("/model-catalog/logos/anthropic.svg".to_string()),
                options: sample_model_options(),
            }],
            options: sample_model_options(),
            warning: None,
            show_label: false,
            submit_on_save: true,
            lazy_options_url: None,
        },
        runs,
        true,
        HarnessSubAgentRunsPagination {
            page: 1,
            total_pages: 1,
            total_count: 2,
            range_start: 1,
            range_end: 2,
            previous_page_url: None,
            next_page_url: None,
        },
        AgentJobPageNavigation {
            notification_count: 0,
            navbar: Navbar::default(),
        },
    )
    .expect("render sub-agent detail page");

    assert!(rendered.contains("Agent sections"));
    assert!(
        rendered
            .contains("href=\"/agents/test-agent/sub-agents\" aria-label=\"Back\" title=\"Back\"")
    );
    assert!(rendered.contains("d=\"M10.5 19.5 3 12m0 0 7.5-7.5M3 12h18\""));
    assert!(!rendered.contains(">Back<"));
    assert!(rendered.contains("analysis-15m"));
    assert!(rendered.contains("At 15m candle close"));
    assert!(!rendered.contains(">Timeframe</p>"));
    assert!(rendered.contains("Additional Instructions"));
    assert!(rendered.contains("Preview Prompt"));
    assert!(rendered.contains("additional-instructions-modal"));
    assert!(rendered.contains("action=\"/agents/test-agent/sub-agents/1/operator-prompt\""));
    assert!(!rendered.contains("Operator prompt"));
    assert!(rendered.contains("/agents/test-agent/sub-agents/1/run"));
    assert!(rendered.contains("Disable"));
    assert!(rendered.contains("/agents/test-agent/runs/1"));
    assert!(rendered.contains("action=\"/agents/test-agent/sub-agents/1/model\""));
    assert!(rendered.contains("hx-post=\"/agents/test-agent/sub-agents/1/model\""));
    assert!(rendered.contains("hx-swap=\"none\""));
    assert!(rendered.contains("Select model"));
    assert!(rendered.contains("data-model-picker-modal"));
    assert!(rendered.contains("Cancel"));
    assert!(rendered.contains("Save"));
    assert!(!rendered.contains("Trigger delay"));
}

#[test]
fn event_job_view_has_no_next_run() {
    let view = HarnessSubAgentView::from_row(&sample_event_job_row(3, true));

    assert!(view.trigger_text.contains("analysis batch"));
    assert!(view.next_run_at.is_none());
}

#[test]
fn job_detail_highlights_model_picker_when_setup_needs_a_model() {
    let agent = sample_opencode_detail_row();
    let mut job = HarnessSubAgentDetailView::from_row(&sample_candle_job_row(
        1,
        "trading-1m",
        "trading",
        false,
    ));
    job.has_model = false;
    job.highlight_model_selector = true;

    let rendered = AgentJobDetailPageTemplate::render_view(
        agent,
        job,
        ModelPickerView {
            input_id: "sub-agent-model-selection".to_string(),
            input_name: "model_selection".to_string(),
            variant_input_id: "sub-agent-model-selection-variant".to_string(),
            variant_input_name: "model_variant".to_string(),
            selected_value: String::new(),
            selected_variant: String::new(),
            selected_label: "None selected".to_string(),
            empty_label: "None selected".to_string(),
            provider_groups: Vec::new(),
            options: Vec::new(),
            warning: None,
            show_label: false,
            submit_on_save: true,
            lazy_options_url: None,
        },
        Vec::new(),
        true,
        HarnessSubAgentRunsPagination {
            page: 1,
            total_pages: 0,
            total_count: 0,
            range_start: 0,
            range_end: 0,
            previous_page_url: None,
            next_page_url: None,
        },
        AgentJobPageNavigation {
            notification_count: 0,
            navbar: Navbar::default(),
        },
    )
    .expect("render highlighted sub-agent detail page");

    assert!(rendered.contains("border-violet-700/70"));
    assert!(rendered.contains("Select a model before enabling this sub-agent."));
}
