use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use tower::util::ServiceExt;

use crate::{
    gateway::model::NotificationSeverity,
    notifications::store::{
        create_notification, delete_notification, list_notification_history, mark_failed, mark_sent,
    },
    web::{
        routes::{router, test_support::*},
        ui_events::UiEvent,
    },
};

#[tokio::test]
async fn agent_notifications_route_renders_notification_history() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
    let sent = create_notification(
        &state.db_pool,
        &agent_key,
        "Orders submitted",
        "BTC limit entry and stop-loss submitted.",
        NotificationSeverity::Info,
    )
    .await
    .expect("create sent notification");
    mark_sent(&state.db_pool, sent.id)
        .await
        .expect("mark notification sent");
    let failed = create_notification(
        &state.db_pool,
        &agent_key,
        "Position changed",
        "BTC position increased from 0.1 to 0.2.",
        NotificationSeverity::Warning,
    )
    .await
    .expect("create failed notification");
    mark_failed(&state.db_pool, failed.id, "telegram send_message failed")
        .await
        .expect("mark notification failed");

    let response = router(state)
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/notifications"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert!(text.contains("id=\"agent-notifications\""));
    assert!(text.contains("Orders submitted"));
    assert!(text.contains("Position changed"));
    assert!(text.contains("Sent"));
    assert!(text.contains("Failed"));
    assert!(text.contains("telegram send_message failed"));
    assert!(text.contains(&format!("href=\"/agents/{agent_key}/notifications\"")));
    assert!(text.contains("data-notification-select"));
    assert!(text.contains("data-notifications-select-all"));
    assert!(text.contains("data-notifications-delete-selected"));
    assert!(text.contains("name=\"delete_id\""));
    assert!(text.contains(&format!(
        "sse-connect=\"/agents/{agent_key}/notifications/count/stream\""
    )));
    assert!(text.contains("id=\"agent-notification-count\""));
    assert!(text.contains("sse-swap=\"notification-count\" hx-target=\"this\""));
    assert!(text.contains(">2</span>"));
}

#[tokio::test]
async fn agent_notifications_route_renders_empty_state() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");

    let response = router(state)
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/notifications"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response_text(response)
            .await
            .contains("No notifications have been queued yet.")
    );
}

#[tokio::test]
async fn agent_notifications_delete_route_removes_one_notification() {
    let state = test_state().await;
    let mut ui_events = state.ui_events.subscribe();
    let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
    let notification = create_notification(
        &state.db_pool,
        &agent_key,
        "Orders submitted",
        "BTC limit entry submitted.",
        NotificationSeverity::Info,
    )
    .await
    .expect("create notification");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/notifications"))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(format!("delete_id={}", notification.id)))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response
            .headers()
            .get("location")
            .and_then(|value| value.to_str().ok()),
        Some(format!("/agents/{agent_key}/notifications").as_str())
    );
    assert!(
        list_notification_history(&state.db_pool, &agent_key)
            .await
            .expect("list notification history")
            .is_empty()
    );
    assert_eq!(
        ui_events.recv().await.expect("notification deletion event"),
        UiEvent::NotificationsDeleted {
            agent_key: agent_key.clone(),
        }
    );
}

#[tokio::test]
async fn agent_notifications_delete_route_removes_selected_notifications() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
    let first = create_notification(
        &state.db_pool,
        &agent_key,
        "Orders submitted",
        "BTC limit entry submitted.",
        NotificationSeverity::Info,
    )
    .await
    .expect("create first notification");
    let second = create_notification(
        &state.db_pool,
        &agent_key,
        "Position changed",
        "BTC position increased from 0.1 to 0.2.",
        NotificationSeverity::Warning,
    )
    .await
    .expect("create second notification");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/notifications"))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(format!(
                    "notification_id={}&notification_id={}",
                    first.id, second.id
                )))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert!(
        list_notification_history(&state.db_pool, &agent_key)
            .await
            .expect("list notification history")
            .is_empty()
    );
}

#[tokio::test]
async fn notification_count_stream_refreshes_when_notification_is_queued() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
    let response = router(Arc::clone(&state))
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/notifications/count/stream"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let reader = tokio::spawn(read_sse_chunk(response.into_body(), 1_000));
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let notification = create_notification(
        &state.db_pool,
        &agent_key,
        "Orders submitted",
        "BTC limit entry submitted.",
        NotificationSeverity::Info,
    )
    .await
    .expect("create notification");
    state.ui_events.publish(UiEvent::NotificationQueued {
        agent_key: agent_key.clone(),
        notification_id: notification.id,
    });

    let body = reader.await.expect("read notification-count stream");
    assert!(body.matches("event: notification-count").count() >= 2);
    assert!(body.contains("id=\"agent-notification-count\""));
    assert!(body.contains(">0</span>"));
    assert!(body.contains(">1</span>"));
}

#[tokio::test]
async fn notification_count_stream_refreshes_when_notifications_are_deleted() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
    let notification = create_notification(
        &state.db_pool,
        &agent_key,
        "Orders submitted",
        "BTC limit entry submitted.",
        NotificationSeverity::Info,
    )
    .await
    .expect("create notification");
    let response = router(Arc::clone(&state))
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/notifications/count/stream"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let reader = tokio::spawn(read_sse_chunk(response.into_body(), 1_000));
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(
        delete_notification(&state.db_pool, &agent_key, notification.id)
            .await
            .expect("delete notification")
    );
    state.ui_events.publish(UiEvent::NotificationsDeleted {
        agent_key: agent_key.clone(),
    });

    let body = reader.await.expect("read notification-count stream");
    assert!(body.matches("event: notification-count").count() >= 2);
    assert!(body.contains(">1</span>"));
    assert!(body.contains(">0</span>"));
}
