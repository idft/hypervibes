use super::*;
use crate::web::templates::test_support::*;

#[test]
fn job_detail_page_renders_job_metadata_and_runs() {
    let agent = sample_opencode_detail_row();
    let job = AgenticJobDetailView::from_row(&sample_schedule_row(
        1,
        "analysis-15m",
        "analysis",
        true,
    ));
    let runs = vec![
        AgenticRunView::from_row(&sample_run_row(1, "succeeded", "analysis-15m")),
        AgenticRunView::from_row(&sample_run_row(2, "failed", "analysis-15m")),
    ];

    let rendered = AgentJobDetailPageTemplate::render_view(
        agent,
        job,
        ModelPickerView {
            input_id: "job-model-selection".to_string(),
            input_name: "model_selection".to_string(),
            selected_value: "anthropic/claude-sonnet-4".to_string(),
            selected_label: "Anthropic / Claude Sonnet 4".to_string(),
            options: sample_model_options(),
            warning: None,
        },
        runs,
        true,
    )
    .expect("render job detail page");

    assert!(rendered.contains("Back to jobs"));
    assert!(rendered.contains("Job details"));
    assert!(rendered.contains("Operator prompt"));
    assert!(rendered.contains("/agents/test-agent/jobs/1/run"));
    assert!(rendered.contains("/agents/test-agent/runs/1"));
}