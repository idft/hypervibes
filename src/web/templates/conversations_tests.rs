use super::*;

fn conversation_list_row(
    channel: &str,
    external_conversation_key: Option<&str>,
) -> crate::agent_conversations::model::AgentConversationListRow {
    let now = chrono::Utc::now();
    crate::agent_conversations::model::AgentConversationListRow {
        id: uuid::Uuid::new_v4(),
        agent_key: "test-agent".to_string(),
        opencode_session_id: "ses_test".to_string(),
        channel: channel.to_string(),
        external_conversation_key: external_conversation_key.map(str::to_string),
        title: "Test conversation".to_string(),
        model_provider_id: "ollama-cloud".to_string(),
        model_id: "glm-5.2".to_string(),
        model_variant: None,
        created_at: now,
        updated_at: now,
        tool_policies: Vec::new(),
        opencode_status: None,
        opencode_updated_at: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        cache_write_tokens: None,
        reasoning_tokens: None,
        context_tokens: None,
        peak_context_tokens: None,
        estimated_cost: None,
        compaction_count: None,
    }
}

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

#[test]
fn conversation_sidebar_marks_telegram_conversations() {
    let rendered = askama::Template::render(&AgentConversationSidebarPartialTemplate {
        agent_key: "test-agent".to_string(),
        new_conversation_model_selection: "ollama-cloud/glm-5.2".to_string(),
        new_conversation_model_variant: String::new(),
        conversations: vec![AgentConversationListItemView {
            title: "Telegram chat".to_string(),
            is_telegram: true,
            model_text: "ollama-cloud/glm-5.2".to_string(),
            status_text: "idle".to_string(),
            selected: true,
            href: "/agents/test-agent/chat/00000000-0000-0000-0000-000000000000".to_string(),
        }],
    })
    .expect("render Telegram conversation sidebar");

    assert!(rendered.contains("src=\"/static/telegram.svg\""));
    assert!(rendered.contains("alt=\"Telegram\""));
}

#[test]
fn conversation_items_only_marks_the_currently_linked_telegram_chat() {
    let telegram = conversation_list_row(
        crate::agent_conversations::model::CONVERSATION_CHANNEL_TELEGRAM,
        Some("42"),
    );
    let web = conversation_list_row(
        crate::agent_conversations::model::CONVERSATION_CHANNEL_WEB,
        None,
    );

    assert!(AgentConversationListItemView::from_row(&telegram, None, Some("42")).is_telegram);
    assert!(!AgentConversationListItemView::from_row(&telegram, None, None).is_telegram);
    assert!(!AgentConversationListItemView::from_row(&telegram, None, Some("7")).is_telegram);
    assert!(!AgentConversationListItemView::from_row(&web, None, Some("42")).is_telegram);
}
