use super::*;

#[test]
fn conversation_transcript_shows_thinking_bubble_while_busy() {
    let busy = askama::Template::render(&AgentConversationTranscriptPartialTemplate {
        session: None,
        busy: true,
    })
    .expect("render busy conversation transcript");
    let idle = askama::Template::render(&AgentConversationTranscriptPartialTemplate {
        session: None,
        busy: false,
    })
    .expect("render idle conversation transcript");

    assert!(busy.contains("Assistant is thinking"));
    assert!(busy.contains("animate-spin"));
    assert!(!idle.contains("Assistant is thinking"));
}

#[test]
fn conversation_composer_autofocuses_the_message_input() {
    let rendered = askama::Template::render(&AgentConversationComposerPartialTemplate {
        agent_key: "test-agent".to_string(),
        conversation_id: uuid::Uuid::nil(),
        message_id: "msg_test".to_string(),
        busy: false,
        message: String::new(),
        error: None,
    })
    .expect("render conversation composer");

    assert!(rendered.contains("autofocus"));
    assert!(rendered.contains("hx-post"));
    assert!(rendered.contains("hx-target=\"#conversation-composer\""));
    assert!(rendered.contains("hx-swap=\"innerHTML\""));
}

#[test]
fn conversation_sidebar_posts_the_selected_conversation_model() {
    let rendered = askama::Template::render(&AgentConversationSidebarPartialTemplate {
        agent_key: "test-agent".to_string(),
        new_conversation_model_selection: "ollama-cloud/glm-5.2".to_string(),
        new_conversation_model_variant: "high".to_string(),
        conversations: Vec::new(),
    })
    .expect("render conversation sidebar");

    assert!(rendered.contains("action=\"/agents/test-agent/chat/conversations\" method=\"post\""));
    assert!(rendered.contains("name=\"model_selection\" value=\"ollama-cloud/glm-5.2\""));
}
