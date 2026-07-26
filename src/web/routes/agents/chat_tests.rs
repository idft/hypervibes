use uuid::Uuid;

use super::chat::{chat_message_redirect, opencode_message_id_at};

#[test]
fn opencode_message_ids_sort_by_timestamp() {
    let first = opencode_message_id_at(
        1_785_042_399_207,
        Uuid::from_u128(0x1111_1111_1111_1111_1111_1111_1111_1111),
    );
    let second = opencode_message_id_at(
        1_785_042_399_208,
        Uuid::from_u128(0x2222_2222_2222_2222_2222_2222_2222_2222),
    );

    assert!(first.starts_with("msg_f9cd16fe7000"));
    assert!(first < second);
}

#[test]
fn htmx_message_submission_returns_an_hx_redirect() {
    let response =
        chat_message_redirect("test-agent", Uuid::nil(), true).expect("create HTMX chat redirect");

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("HX-Redirect")
            .and_then(|value| value.to_str().ok()),
        Some("/agents/test-agent/chat/00000000-0000-0000-0000-000000000000")
    );
}
