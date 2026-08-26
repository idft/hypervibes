use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::util::ServiceExt;

use crate::web::routes::router;
use crate::web::routes::test_support::*;

#[tokio::test]
async fn telegram_gateway_settings_page_renders_disabled_section() {
    let state = test_state().await;
    let (agent_key, _) = insert_test_agent(&state).await.expect("insert agent");

    let response = router(Arc::clone(&state))
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/settings"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert!(text.contains("Telegram gateway"));
}

#[tokio::test]
async fn toggle_telegram_gateway_creates_disabled_row() {
    let state = test_state().await;
    let (agent_key, _) = insert_test_agent(&state).await.expect("insert agent");

    let response = router(Arc::clone(&state))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/agents/{agent_key}/settings/gateway/telegram/toggle"
                ))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(""))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let stored = crate::gateway::store::get_gateway(&state.db_pool, &agent_key, "telegram")
        .await
        .expect("get gateway")
        .expect("gateway exists");
    assert!(!stored.enabled);
}

#[tokio::test]
async fn toggle_telegram_gateway_with_enabled_flag_persists_enabled() {
    let state = test_state().await;
    let (agent_key, _) = insert_test_agent(&state).await.expect("insert agent");

    let response = router(Arc::clone(&state))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/agents/{agent_key}/settings/gateway/telegram/toggle"
                ))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("enabled=on"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let stored = crate::gateway::store::get_gateway(&state.db_pool, &agent_key, "telegram")
        .await
        .expect("get gateway")
        .expect("gateway exists");
    assert!(stored.enabled);
}

#[tokio::test]
async fn unlink_telegram_chat_clears_binding() {
    let state = test_state().await;
    let (agent_key, _) = insert_test_agent(&state).await.expect("insert agent");
    let config = crate::gateway::model::TelegramGatewayConfig {
        bot_token_ciphertext: Some(vec![1, 2, 3]),
        chat_id: Some(42),
        chat_username: Some("user".to_string()),
        ..Default::default()
    };
    crate::gateway::store::upsert_telegram_config(&state.db_pool, &agent_key, &config)
        .await
        .expect("seed gateway");

    let response = router(Arc::clone(&state))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/agents/{agent_key}/settings/gateway/telegram/unlink"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let stored = crate::gateway::store::get_gateway(&state.db_pool, &agent_key, "telegram")
        .await
        .expect("get gateway")
        .expect("gateway exists");
    let config = crate::gateway::model::TelegramGatewayConfig::from_value(&stored.config);
    assert_eq!(config.chat_id, None);
    assert_eq!(config.chat_username, None);
    assert_eq!(config.bot_token_ciphertext, Some(vec![1, 2, 3]));
}

#[tokio::test]
async fn start_telegram_link_without_gateway_returns_bad_request() {
    let state = test_state().await;
    let (agent_key, _) = insert_test_agent(&state).await.expect("insert agent");

    let response = router(Arc::clone(&state))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/agents/{agent_key}/settings/gateway/telegram/link"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn start_telegram_link_reports_unavailable_without_pending_links_state() {
    let state = test_state().await;
    let (agent_key, _) = insert_test_agent(&state).await.expect("insert agent");
    let config = crate::gateway::model::TelegramGatewayConfig {
        bot_token_ciphertext: Some(vec![1, 2, 3]),
        bot_username: Some("testbot".to_string()),
        ..Default::default()
    };
    crate::gateway::store::upsert_telegram_config(&state.db_pool, &agent_key, &config)
        .await
        .expect("seed gateway");
    // The default test state has no gateway_pending_links; this test
    // verifies that the handler correctly reports the service as
    // unavailable when no shared map is present.
    let response = router(Arc::clone(&state))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/agents/{agent_key}/settings/gateway/telegram/link"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn confirm_telegram_link_without_pending_links_returns_service_unavailable() {
    let state = test_state().await;
    let (agent_key, _) = insert_test_agent(&state).await.expect("insert agent");
    let token = uuid::Uuid::new_v4();

    let response = router(Arc::clone(&state))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/agents/{agent_key}/settings/gateway/telegram/link/{token}/confirm"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

async fn seed_telegram_gateway(
    state: &std::sync::Arc<crate::web::AppState>,
    agent_key: &str,
    enabled: bool,
) {
    let config = crate::gateway::model::TelegramGatewayConfig {
        bot_token_ciphertext: Some(vec![1, 2, 3]),
        bot_token_key_id: Some("test".to_string()),
        bot_username: Some("testbot".to_string()),
        ..Default::default()
    };
    crate::gateway::store::upsert_telegram_config(&state.db_pool, agent_key, &config)
        .await
        .expect("seed gateway");
    if enabled {
        crate::gateway::store::set_gateway_enabled(&state.db_pool, agent_key, "telegram", true)
            .await
            .expect("enable gateway");
    }
}

#[tokio::test]
async fn start_telegram_link_creates_pending_token_and_redirects() {
    let (state, pending_links) = test_state_with_gateway_pending_links().await;
    let (agent_key, _) = insert_test_agent(&state).await.expect("insert agent");
    seed_telegram_gateway(&state, &agent_key, true).await;

    let response = router(Arc::clone(&state))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/agents/{agent_key}/settings/gateway/telegram/link"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let location = response
        .headers()
        .get("location")
        .and_then(|value| value.to_str().ok())
        .expect("redirect location");
    assert!(
        pending_links.len() == 1,
        "exactly one pending link expected"
    );
    let entry = pending_links.iter().next().expect("pending link entry");
    let token = *entry.key();
    assert_eq!(
        location,
        format!(
            "/agents/{agent_key}/settings?gateway_link={}#telegram-gateway",
            token
        )
    );
    assert_eq!(entry.agent_key, agent_key);
    assert_eq!(entry.user_id, crate::test_db::test_user_id());
    assert!(entry.chat_id.is_none());
    drop(entry);

    let response = router(Arc::clone(&state))
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/settings?gateway_link={token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert!(
        text.contains(&format!("?start={token}")),
        "settings page should render the requested deep link token"
    );
}

#[tokio::test]
async fn start_telegram_link_without_bot_username_returns_bad_request() {
    let (state, _pending_links) = test_state_with_gateway_pending_links().await;
    let (agent_key, _) = insert_test_agent(&state).await.expect("insert agent");
    // Seed a gateway row without a bot token (and thus no bot username).
    let config = crate::gateway::model::TelegramGatewayConfig::default();
    crate::gateway::store::upsert_telegram_config(&state.db_pool, &agent_key, &config)
        .await
        .expect("seed gateway");

    let response = router(Arc::clone(&state))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/agents/{agent_key}/settings/gateway/telegram/link"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn confirm_telegram_link_persists_chat_binding_and_consumes_token() {
    let (state, pending_links) = test_state_with_gateway_pending_links().await;
    let (agent_key, _) = insert_test_agent(&state).await.expect("insert agent");
    seed_telegram_gateway(&state, &agent_key, false).await;

    // Simulate the operator opening the deep link: the gateway service
    // would have filled in the chat identity from the Telegram /start.
    let mut link =
        crate::gateway::model::PendingLink::new(agent_key.clone(), crate::test_db::test_user_id());
    link.chat_id = Some(-100_456);
    link.chat_username = Some("operator".to_string());
    let token = link.token;
    pending_links.insert(token, link);

    let response = router(Arc::clone(&state))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/agents/{agent_key}/settings/gateway/telegram/link/{token}/confirm"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert!(pending_links.is_empty(), "confirm must consume the token");

    let stored = crate::gateway::store::get_gateway(&state.db_pool, &agent_key, "telegram")
        .await
        .expect("get gateway")
        .expect("gateway exists");
    let config = crate::gateway::model::TelegramGatewayConfig::from_value(&stored.config);
    assert_eq!(config.chat_id, Some(-100_456));
    assert_eq!(config.chat_username.as_deref(), Some("operator"));
    assert_eq!(config.bot_token_ciphertext, Some(vec![1, 2, 3]));
}

#[tokio::test]
async fn confirm_telegram_link_before_start_returns_conflict() {
    let (state, pending_links) = test_state_with_gateway_pending_links().await;
    let (agent_key, _) = insert_test_agent(&state).await.expect("insert agent");
    let link =
        crate::gateway::model::PendingLink::new(agent_key.clone(), crate::test_db::test_user_id());
    let token = link.token;
    pending_links.insert(token, link);

    let response = router(Arc::clone(&state))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/agents/{agent_key}/settings/gateway/telegram/link/{token}/confirm"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn confirm_telegram_link_for_another_agent_returns_not_found() {
    let (state, pending_links) = test_state_with_gateway_pending_links().await;
    let (linked_agent_key, _) = insert_test_agent(&state)
        .await
        .expect("insert linked agent");
    let (other_agent_key, _) = insert_test_agent(&state).await.expect("insert other agent");
    let mut link =
        crate::gateway::model::PendingLink::new(linked_agent_key, crate::test_db::test_user_id());
    link.chat_id = Some(-100_456);
    let token = link.token;
    pending_links.insert(token, link);

    let response = router(Arc::clone(&state))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/agents/{other_agent_key}/settings/gateway/telegram/link/{token}/confirm"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(pending_links.contains_key(&token));
}

#[tokio::test]
async fn confirm_telegram_link_with_unknown_token_returns_not_found() {
    let (state, _pending_links) = test_state_with_gateway_pending_links().await;
    let (agent_key, _) = insert_test_agent(&state).await.expect("insert agent");
    let token = uuid::Uuid::new_v4();

    let response = router(Arc::clone(&state))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/agents/{agent_key}/settings/gateway/telegram/link/{token}/confirm"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn reject_telegram_link_removes_pending_entry() {
    let (state, pending_links) = test_state_with_gateway_pending_links().await;
    let (agent_key, _) = insert_test_agent(&state).await.expect("insert agent");
    let link =
        crate::gateway::model::PendingLink::new(agent_key.clone(), crate::test_db::test_user_id());
    let token = link.token;
    pending_links.insert(token, link);

    let response = router(Arc::clone(&state))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/agents/{agent_key}/settings/gateway/telegram/link/{token}/reject"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert!(pending_links.is_empty(), "reject must remove the token");
}

#[tokio::test]
async fn settings_page_renders_pending_link_token_when_flow_is_active() {
    let (state, pending_links) = test_state_with_gateway_pending_links().await;
    let (agent_key, _) = insert_test_agent(&state).await.expect("insert agent");
    seed_telegram_gateway(&state, &agent_key, true).await;

    let link =
        crate::gateway::model::PendingLink::new(agent_key.clone(), crate::test_db::test_user_id());
    let token = link.token;
    pending_links.insert(token, link);

    let response = router(Arc::clone(&state))
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/settings"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert!(
        text.contains(&format!("?start={token}")),
        "settings page should render the pending deep link token"
    );
}
