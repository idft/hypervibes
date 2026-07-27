use askama::Template;

use super::*;
use crate::web::templates::test_support::*;

#[test]
fn hook_detail_page_renders_hook_metadata_and_runs() {
    let agent = sample_opencode_detail_row();
    let hook = AgenticHookDetailView::from_row(&sample_hook_row(3, true));
    let mut hook_run = sample_run_row(1, "succeeded", "market-analysis");
    hook_run.schedule_id = None;
    hook_run.hook_id = Some(3);
    hook_run.timeframe = None;
    let runs = vec![AgenticRunView::from_row(&hook_run)];

    let rendered = AgentHookDetailPageTemplate::render_view(
        agent,
        hook,
        ModelPickerView {
            input_id: "hook-model-selection".to_string(),
            input_name: "model_selection".to_string(),
            variant_input_id: "hook-model-selection-variant".to_string(),
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
        None,
    )
    .expect("render hook detail page");

    assert!(rendered.contains("Agent sections"));
    assert!(
        rendered.contains("href=\"/agents/test-agent/jobs\" aria-label=\"Back\" title=\"Back\"")
    );
    assert!(rendered.contains("d=\"M10.5 19.5 3 12m0 0 7.5-7.5M3 12h18\""));
    assert!(!rendered.contains(">Back<"));
    assert!(rendered.contains("Hook details"));
    assert!(rendered.contains("analysis_batch_completed"));
    assert!(rendered.contains("Created"));
    assert!(rendered.contains("Updated"));
    assert!(rendered.contains("/agents/test-agent/hooks/3/run"));
    assert!(rendered.contains("/agents/test-agent/runs/1"));
    assert!(rendered.contains(
        "action=\"/agents/test-agent/hooks/3/model\" method=\"post\" hx-post=\"/agents/test-agent/hooks/3/model\" hx-swap=\"none\""
    ));
    assert!(rendered.contains("Select model"));
    assert!(rendered.contains("data-model-picker-mode="));
    assert!(!rendered.contains("Save model"));
}

#[test]
fn new_hook_page_renders_form() {
    let agent = sample_opencode_detail_row();
    let template = AgentHookNewPageTemplate {
        tabs: build_agent_show_tabs(&agent, AgentShowTab::Jobs),
        agent_tabs_use_htmx: false,
        agent,
        form: CreateAgentHookFormValues {
            timeout_seconds: "600".to_string(),
            model_selection: "anthropic/claude-sonnet-4".to_string(),
            model_variant: String::new(),
            operator_prompt: "Summarize multi-timeframe agreement".to_string(),
            enabled: true,
        },
        model_picker: ModelPickerView {
            input_id: "hook-model-selection".to_string(),
            input_name: "model_selection".to_string(),
            variant_input_id: "hook-model-selection-variant".to_string(),
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
            show_label: true,
            auto_submit: false,
            use_modal: false,
            lazy_options_url: None,
        },
        errors: Vec::new(),
        current_path: "/agents/test-agent/hooks/new".to_string(),
        navbar: Navbar::default(),
    };

    let rendered = template.render().expect("render new hook page");
    assert!(rendered.contains("Agent sections"));
    assert!(rendered.contains("Create market-analysis hook"));
    assert!(rendered.contains("action=\"/agents/test-agent/hooks\""));
    assert!(rendered.contains("analysis_batch_completed"));
}
