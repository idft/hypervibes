use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use tower::util::ServiceExt;

use crate::{
    gateway::model::NotificationSeverity,
    notifications::store::{count_notifications, create_notification, mark_failed, mark_sent},
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
        "href=\"/agents/{agent_key}/notifications/{}\"",
        sent.id
    )));
    assert!(text.contains("Showing 1-2 of 2 notifications"));
    assert!(text.contains(&format!(
        "hx-get=\"/agents/{agent_key}/notifications/count\""
    )));
    assert!(text.contains("id=\"agent-notification-count\""));
    assert!(text.contains("hx-trigger=\"every 15s\""));
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
async fn agent_notifications_route_paginates_fifty_per_page() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
    for index in 0..51 {
        create_notification(
            &state.db_pool,
            &agent_key,
            &format!("Notification {index}"),
            "Body",
            NotificationSeverity::Info,
        )
        .await
        .expect("create notification");
    }

    let first_page = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/notifications"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(first_page.status(), StatusCode::OK);
    let first_page_text = response_text(first_page).await;
    assert!(first_page_text.contains("Showing 1-50 of 51 notifications"));
    assert!(first_page_text.contains(&format!(
        "href=\"/agents/{agent_key}/notifications?page=2\""
    )));

    let second_page = router(state)
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/notifications?page=2"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(second_page.status(), StatusCode::OK);
    let second_page_text = response_text(second_page).await;
    assert!(second_page_text.contains("Showing 51-51 of 51 notifications"));
    assert!(second_page_text.contains(&format!(
        "href=\"/agents/{agent_key}/notifications?page=1\""
    )));
}

#[tokio::test]
async fn notification_detail_route_renders_full_content_and_is_scoped_to_its_agent() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
    let (other_agent_key, _wallet_address) =
        insert_test_agent(&state).await.expect("insert other agent");
    let notification = create_notification(
        &state.db_pool,
        &agent_key,
        "A detailed notification title that does not fit in the compact list",
        "A detailed notification body that remains fully visible on the detail page.",
        NotificationSeverity::Error,
    )
    .await
    .expect("create notification");
    mark_failed(
        &state.db_pool,
        notification.id,
        "telegram send_message failed",
    )
    .await
    .expect("mark notification failed");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/agents/{agent_key}/notifications/{}",
                    notification.id
                ))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert!(text.contains("<!DOCTYPE html>"));
    assert!(text.contains("A detailed notification title that does not fit in the compact list"));
    assert!(
        text.contains(
            "A detailed notification body that remains fully visible on the detail page."
        )
    );
    assert!(text.contains("telegram send_message failed"));
    assert!(text.contains(&format!("href=\"/agents/{agent_key}/notifications\"")));

    let other_agent_response = router(state)
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/agents/{other_agent_key}/notifications/{}",
                    notification.id
                ))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(other_agent_response.status(), StatusCode::NOT_FOUND);
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
    assert_eq!(
        count_notifications(&state.db_pool, &agent_key)
            .await
            .expect("count notification history"),
        0
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
    assert_eq!(
        count_notifications(&state.db_pool, &agent_key)
            .await
            .expect("count notification history"),
        0
    );
}

#[tokio::test]
async fn notification_count_route_renders_current_count() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
    create_notification(
        &state.db_pool,
        &agent_key,
        "Orders submitted",
        "BTC limit entry submitted.",
        NotificationSeverity::Info,
    )
    .await
    .expect("create notification");

    let response = router(state)
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/notifications/count"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_text(response).await;
    assert!(body.contains("id=\"agent-notification-count\""));
    assert!(body.contains(">1</span>"));
}
