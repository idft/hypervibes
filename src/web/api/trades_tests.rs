use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::Value;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

use super::test_support::*;
use crate::agents::store::get_agent;

#[tokio::test]
async fn journal_notes_require_scoped_writer_and_account_target() {
    let state = test_state().await;
    let (first, key) = seed_agent(&state, "journal-first").await;
    let (second, other_key) = seed_agent(&state, "journal-second").await;
    let first = get_agent(&state.db_pool, &first)
        .await
        .expect("agent")
        .expect("first");
    let second = get_agent(&state.db_pool, &second)
        .await
        .expect("agent")
        .expect("second");
    for (hash, agent) in [("first-event", &first), ("second-event", &second)] {
        sqlx::query(
            "INSERT INTO hyperliquid.ledger_events(hash,account_address,environment,event_time,
            event_type,source_stream,ledger_type,usdc,ingest_source,inserted_at)
            VALUES ($1,$2,$3,now(),'deposit','test','deposit',10,'test',now())",
        )
        .bind(hash)
        .bind(agent.trading_account_address.as_deref().expect("account"))
        .bind(&agent.environment)
        .execute(&state.db_pool)
        .await
        .expect("ledger");
    }
    for (path, expected) in [
        ("/account/journal/ledger/first-event/notes", StatusCode::OK),
        (
            "/account/journal/ledger/second-event/notes",
            StatusCode::NOT_FOUND,
        ),
    ] {
        let response = app(Arc::clone(&state))
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header("authorization", format!("Bearer {key}"))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), expected);
    }
    let response = app(Arc::clone(&state))
        .oneshot(
            Request::builder()
                .uri("/account/journal/ledger/first-event/notes")
                .method("POST")
                .header("authorization", format!("Bearer {key}"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"body":"unscoped writer"}"#))
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let conversation = Uuid::new_v4();
    sqlx::query("INSERT INTO agent_conversations(id,agent_key,opencode_session_id,title,model_provider_id,model_id)
        VALUES ($1,$2,$3,'Test chat','test','test')")
        .bind(conversation).bind(&first.agent_key).bind(format!("session-{conversation}"))
        .execute(&state.db_pool).await.expect("conversation");
    sqlx::query(
        "INSERT INTO agent_conversation_tool_policies(conversation_id,tool_group,policy)
        VALUES ($1,'journal_writes','allow')",
    )
    .bind(conversation)
    .execute(&state.db_pool)
    .await
    .expect("permission");
    let response = app(Arc::clone(&state))
        .oneshot(
            Request::builder()
                .uri("/account/journal/ledger/first-event/notes")
                .method("POST")
                .header("authorization", format!("Bearer {key}"))
                .header("x-hypervibes-conversation-id", conversation.to_string())
                .header("content-type", "application/json")
                .body(Body::from(r#"{"body":"reviewed by chat"}"#))
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let (count,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM hyperliquid.journal_notes
        WHERE event_id='first-event' AND source_conversation_id=$1 AND author_id=$2",
    )
    .bind(conversation)
    .bind(&first.agent_key)
    .fetch_one(&state.db_pool)
    .await
    .expect("notes");
    assert_eq!(count, 1);
    let response = app(Arc::clone(&state))
        .oneshot(
            Request::builder()
                .uri("/account/journal/ledger/first-event/notes")
                .method("POST")
                .header("authorization", format!("Bearer {other_key}"))
                .header("x-hypervibes-conversation-id", conversation.to_string())
                .header("content-type", "application/json")
                .body(Body::from(r#"{"body":"wrong agent"}"#))
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let response = app(Arc::clone(&state))
        .oneshot(
            Request::builder()
                .uri("/account/trades?limit=101")
                .header("authorization", format!("Bearer {other_key}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let response = app(state)
        .oneshot(
            Request::builder()
                .uri("/account/trades")
                .header("authorization", format!("Bearer {key}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.expect("body"))
            .expect("JSON");
    assert_eq!(body, serde_json::json!([]));
}
