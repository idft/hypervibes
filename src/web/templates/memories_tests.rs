use super::*;
use crate::web::templates::test_support::*;

#[test]
fn memory_detail_page_renders_agent_navbar_with_memories_active() {
    let agent = sample_agent_detail_row();
    let memory = MemoryView::from_record(sample_memory_record(
        "BTC",
        Some("15m"),
        "analysis",
        "Momentum remains constructive",
        "### Readout\n\n- Wait for confirmation.",
    ));
    let memory_detail_html = AgentMemoryDetailPartialTemplate::render_page_view(
        memory.clone(),
        "/agents/test-agent/memories".to_string(),
    )
    .expect("render memory detail partial");

    let rendered = AgentMemoryDetailPageTemplate::render_view(
        agent,
        memory,
        memory_detail_html,
        0,
        Navbar::default(),
    )
    .expect("render memory detail page");

    assert!(rendered.contains("Agent sections"));
    assert!(
        rendered
            .contains("href=\"/agents/test-agent/memories\" aria-label=\"Back\" title=\"Back\"")
    );
    assert!(rendered.contains("Momentum remains constructive"));
    assert_eq!(rendered.matches("<h2 ").count(), 0);
}
