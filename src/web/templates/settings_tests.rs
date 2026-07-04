use askama::Template;

use super::*;

#[test]
fn server_error_page_renders_base_layout() {
    let template = ServerErrorPageTemplate {
        message: "Internal server error: boom".to_string(),
        current_path: String::new(),
    };
    let rendered = template.render().unwrap();
    assert!(rendered.contains("<!DOCTYPE html>"));
    assert!(rendered.contains("500 Server Error"));
    assert!(rendered.contains("Internal server error: boom"));
}
