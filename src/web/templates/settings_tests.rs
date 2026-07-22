use askama::Template;

use super::*;

#[test]
fn server_error_page_renders_base_layout() {
    let template = ServerErrorPageTemplate {
        message: "Internal server error: boom".to_string(),
        current_path: String::new(),
        navbar: Navbar::default(),
    };
    let rendered = template.render().unwrap();
    assert!(rendered.contains("<!DOCTYPE html>"));
    assert!(rendered.contains("500 Server Error"));
    assert!(rendered.contains("Internal server error: boom"));
}

#[test]
fn not_found_page_renders_base_layout() {
    let template = NotFoundPageTemplate {
        current_path: "/agents/old-agent/chat".to_string(),
        navbar: Navbar::default(),
    };
    let rendered = template.render().unwrap();
    assert!(rendered.contains("<!DOCTYPE html>"));
    assert!(rendered.contains("404 Page Not Found"));
    assert!(rendered.contains("/agents/old-agent/chat"));
}
