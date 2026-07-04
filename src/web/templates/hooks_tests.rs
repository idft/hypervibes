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
    hook_run.job_kind = crate::agentic::model::JOB_KIND_MARKET_ANALYSIS.to_string();
    hook_run.timeframe = None;
    let runs = vec![AgenticRunView::from_row(&hook_run)];

    let rendered = AgentHookDetailPageTemplate::render_view(
        agent,
        hook,
        ModelPickerView {
            input_id: "hook-model-selection".to_string(),
            input_name: "model_selection".to_string(),
            selected_value: "anthropic/claude-sonnet-4".to_string(),
            selected_label: "Anthropic / Claude Sonnet 4".to_string(),
            options: sample_model_options(),
            warning: None,
        },
        runs,
        true,
    )
    .expect("render hook detail page");

    assert!(rendered.contains("Back to jobs"));
    assert!(rendered.contains("Hook details"));
    assert!(rendered.contains("analysis_batch_completed"));
    assert!(rendered.contains("Created"));
    assert!(rendered.contains("Updated"));
    assert!(rendered.contains("/agents/test-agent/hooks/3/run"));
    assert!(rendered.contains("/agents/test-agent/runs/1"));
}

#[test]
fn new_hook_page_renders_form() {
    let template = AgentHookNewPageTemplate {
        agent: sample_opencode_detail_row(),
        form: CreateAgentHookFormValues {
            timeout_seconds: "600".to_string(),
            model_selection: "anthropic/claude-sonnet-4".to_string(),
            operator_prompt: "Summarize multi-timeframe agreement".to_string(),
            enabled: true,
        },
        model_picker: ModelPickerView {
            input_id: "hook-model-selection".to_string(),
            input_name: "model_selection".to_string(),
            selected_value: "anthropic/claude-sonnet-4".to_string(),
            selected_label: "Anthropic / Claude Sonnet 4".to_string(),
            options: sample_model_options(),
            warning: None,
        },
        errors: Vec::new(),
        current_path: "/agents/test-agent/hooks/new".to_string(),
    };

    let rendered = template.render().expect("render new hook page");
    assert!(rendered.contains("Create market-analysis hook"));
    assert!(rendered.contains("action=\"/agents/test-agent/hooks\""));
    assert!(rendered.contains("analysis_batch_completed"));
}
