//! Tests for emergency agent operations.

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use tower::util::ServiceExt;

use crate::{
    agents::store::get_agent,
    web::routes::{router, test_support::*},
};

#[tokio::test]
async fn emergency_stop_stays_disabled_when_the_trading_signer_is_unavailable() {
    let state = test_state().await;
    let (agent_key, _) = insert_test_agent(&state).await.expect("insert agent");

    let response = router(Arc::clone(&state))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/emergency-stop"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let location = response
        .headers()
        .get("location")
        .and_then(|value| value.to_str().ok())
        .expect("redirect location");
    assert!(location.starts_with(&format!("/agents/{agent_key}?notice=")));

    let agent = get_agent(&state.db_pool, &agent_key)
        .await
        .expect("load agent")
        .expect("agent exists");
    assert!(
        !agent.enabled,
        "emergency stop must be durable before cleanup"
    );
}
