use askama::Template;

use crate::agents::model::CreateAgentRuntimeForm;

use super::*;
use crate::web::templates::test_support::*;

#[test]
fn backends_page_renders_runtime_row() {
    let template = BackendsPageTemplate {
        runtimes: vec![AgentRuntimeView::from_row(sample_runtime_row())],
        current_path: "/backends".to_string(),
        navbar: Navbar::default(),
    };

    let rendered = template.render().unwrap();
    assert!(rendered.contains("Backends"));
    assert!(rendered.contains("opencode-local"));
    assert!(rendered.contains("OpenCode local"));
    assert!(rendered.contains("http://localhost:14096"));
}

#[test]
fn backends_new_page_renders_create_form() {
    let template = BackendsNewPageTemplate {
        form: CreateAgentRuntimeForm {
            backend_kind: "opencode".to_string(),
            enabled: Some("on".to_string()),
            ..Default::default()
        },
        errors: Vec::new(),
        current_path: "/backends/new".to_string(),
        navbar: Navbar::default(),
    };

    let rendered = template.render().unwrap();
    assert!(rendered.contains("Create backend"));
    assert!(rendered.contains("name=\"id\""));
    assert!(rendered.contains("name=\"backend_kind\""));
    assert!(rendered.contains("name=\"base_url\""));
}
