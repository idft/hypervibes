use super::*;
use crate::web::templates::test_support::*;

#[test]
fn job_detail_page_renders_job_metadata_and_runs() {
    let agent = sample_opencode_detail_row();
    let job =
        HarnessJobDetailView::from_row(&sample_candle_job_row(1, "analysis-15m", "analysis", true));
    let runs = vec![
        HarnessRunView::from_row(&sample_run_row(1, "succeeded", "analysis-15m")),
        HarnessRunView::from_row(&sample_run_row(2, "failed", "analysis-15m")),
    ];

    let rendered = AgentJobDetailPageTemplate::render_view(
        agent,
        job,
        ModelPickerView {
            input_id: "job-model-selection".to_string(),
            input_name: "model_selection".to_string(),
            variant_input_id: "job-model-selection-variant".to_string(),
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
            auto_submit: false,
            use_modal: true,
            lazy_options_url: None,
        },
        runs,
        true,
        Navbar::default(),
    )
    .expect("render job detail page");

    assert!(rendered.contains("Agent sections"));
    assert!(
        rendered.contains("href=\"/agents/test-agent/jobs\" aria-label=\"Back\" title=\"Back\"")
    );
    assert!(rendered.contains("d=\"M10.5 19.5 3 12m0 0 7.5-7.5M3 12h18\""));
    assert!(!rendered.contains(">Back<"));
    assert!(rendered.contains("analysis-15m"));
    assert!(rendered.contains("Operator prompt"));
    assert!(rendered.contains("/agents/test-agent/jobs/1/run"));
    assert!(rendered.contains("Disable"));
    assert!(rendered.contains("/agents/test-agent/runs/1"));
    assert!(rendered.contains(
        "action=\"/agents/test-agent/jobs/1/model\" method=\"post\" hx-post=\"/agents/test-agent/jobs/1/model\" hx-swap=\"none\""
    ));
    assert!(rendered.contains("Select model"));
    assert!(rendered.contains("data-model-picker-mode="));
    assert!(rendered.contains("Cancel"));
    assert!(rendered.contains("Save"));
    assert!(!rendered.contains("Trigger delay"));
}

#[test]
fn event_job_view_has_no_next_run() {
    let view = HarnessJobView::from_row(&sample_event_job_row(3, true));

    assert_eq!(view.trigger_text, "Analysis batch completed");
    assert!(view.next_run_at.is_none());
}
