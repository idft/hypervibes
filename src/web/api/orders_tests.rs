use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::json;
use tower::util::ServiceExt;

use super::test_support::*;
// ---- orders endpoint tests -----------------------------------------

#[tokio::test]
async fn post_orders_without_auth_returns_401_json() {
    let state = test_state().await;

    let body = serde_json::json!({
        "orders": [{
            "symbol": "BTC",
            "side": "buy",
            "order_type": "limit",
            "size": "0.1",
            "price": "50000"
        }]
    });
    let (headers, body) = json_body(&body);
    let mut builder = Request::builder().method("POST").uri("/orders");
    if let Some((k, v)) = headers {
        builder = builder.header(k, v);
    }
    let response = app(Arc::clone(&state))
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body_bytes = axum::body::to_bytes(response.into_body(), 16 * 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert!(body["error"].is_string());
}

#[tokio::test]
async fn post_orders_validation_error_returns_422() {
    let state = test_state().await;

    let (_agent_key, api_key) = seed_agent(&state, "ord-val").await;

    // Missing price on a limit order.
    let body = serde_json::json!({
        "orders": [{
            "symbol": "BTC",
            "side": "buy",
            "order_type": "limit",
            "size": "0.1"
        }]
    });
    let (headers, body) = json_body(&body);
    let mut builder = Request::builder()
        .method("POST")
        .uri("/orders")
        .header("authorization", format!("Bearer {api_key}"));
    if let Some((k, v)) = headers {
        builder = builder.header(k, v);
    }
    let response = app(Arc::clone(&state))
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body_bytes = axum::body::to_bytes(response.into_body(), 16 * 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert!(
        body["error"].as_str().unwrap().contains("price"),
        "got: {body}"
    );
}

#[tokio::test]
async fn post_orders_empty_list_returns_422() {
    let state = test_state().await;

    let (_agent_key, api_key) = seed_agent(&state, "ord-empty").await;

    let body = serde_json::json!({ "orders": [] });
    let (headers, body) = json_body(&body);
    let mut builder = Request::builder()
        .method("POST")
        .uri("/orders")
        .header("authorization", format!("Bearer {api_key}"));
    if let Some((k, v)) = headers {
        builder = builder.header(k, v);
    }
    let response = app(Arc::clone(&state))
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn post_orders_rejects_when_no_currencies_are_selected() {
    let state = test_state().await;
    let (_agent_key, api_key) = seed_agent(&state, "ord-no-currencies").await;

    let body = serde_json::json!({
        "orders": [{
            "symbol": "BTC",
            "side": "buy",
            "order_type": "limit",
            "size": "0.1",
            "price": "50000"
        }]
    });
    let (headers, body) = json_body(&body);
    let mut builder = Request::builder()
        .method("POST")
        .uri("/orders")
        .header("authorization", format!("Bearer {api_key}"));
    if let Some((k, v)) = headers {
        builder = builder.header(k, v);
    }
    let response = app(Arc::clone(&state))
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body_bytes = axum::body::to_bytes(response.into_body(), 16 * 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(
        body["error"],
        json!("no currencies are selected for this agent; order placement is disabled")
    );
}

#[tokio::test]
async fn post_orders_rejects_symbol_not_selected_for_agent() {
    let state = test_state().await;
    let (agent_key, api_key) = seed_agent(&state, "ord-symbol-disabled").await;
    seed_instrument(&state, "BTC", true).await;
    select_instruments(&state, &agent_key, &["BTC"]).await;

    let body = serde_json::json!({
        "orders": [{
            "symbol": "ETH",
            "side": "buy",
            "order_type": "limit",
            "size": "0.1",
            "price": "50000"
        }]
    });
    let (headers, body) = json_body(&body);
    let mut builder = Request::builder()
        .method("POST")
        .uri("/orders")
        .header("authorization", format!("Bearer {api_key}"));
    if let Some((k, v)) = headers {
        builder = builder.header(k, v);
    }
    let response = app(Arc::clone(&state))
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body_bytes = axum::body::to_bytes(response.into_body(), 16 * 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(
        body["error"],
        json!("orders[0]: symbol 'ETH' is not enabled for this agent")
    );
}

#[tokio::test]
async fn post_orders_selected_symbol_proceeds_past_selection_validation() {
    let state = test_state().await;
    let (agent_key, api_key) = seed_agent(&state, "ord-symbol-enabled").await;
    seed_instrument(&state, "BTC", true).await;
    select_instruments(&state, &agent_key, &["BTC"]).await;

    let body = serde_json::json!({
        "orders": [{
            "symbol": "BTC",
            "side": "buy",
            "order_type": "limit",
            "size": "0.1",
            "price": "50000"
        }]
    });
    let (headers, body) = json_body(&body);
    let mut builder = Request::builder()
        .method("POST")
        .uri("/orders")
        .header("authorization", format!("Bearer {api_key}"));
    if let Some((k, v)) = headers {
        builder = builder.header(k, v);
    }
    let response = app(Arc::clone(&state))
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body_bytes = axum::body::to_bytes(response.into_body(), 16 * 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(body["error"], json!("internal server error"));
}

#[tokio::test]
async fn get_order_by_id_returns_404_for_other_agent() {
    let state = test_state().await;

    let (owner_key, owner_api) = seed_agent(&state, "ord-own").await;
    let (_other_key, other_api) = seed_agent(&state, "ord-other").await;

    // Insert a fake order directly via the store.
    use crate::hyperliquid::orders::store as orders_store;
    use rust_decimal_macros::dec;
    use serde_json::json;
    let id = uuid::Uuid::new_v4();
    orders_store::insert_order(
        &state.db_pool,
        &orders_store::NewOrder {
            id,
            agent_key: owner_key.clone(),
            account_address: "0xtest".to_string(),
            environment: "live".to_string(),
            group_id: None,
            parent_cloid: None,
            memory_record_ids: json!([]),
            symbol: "BTC".to_string(),
            instrument_id: None,
            side: "buy".to_string(),
            order_kind: "limit".to_string(),
            reduce_only: false,
            requested_price: Some(dec!(50000)),
            rounded_price: Some(dec!(50000)),
            requested_size: dec!(0.1),
            rounded_size: Some(dec!(0.1)),
            trigger_price: None,
            time_in_force: Some("gtc".to_string()),
            cloid: "0xcloid_owner".to_string(),
            status: "resting".to_string(),
            status_detail: None,
            request_payload: json!({}),
        },
    )
    .await
    .expect("insert order");

    // Owner can fetch.
    let request = Request::builder()
        .uri(format!("/orders/{id}"))
        .header("authorization", format!("Bearer {owner_api}"))
        .body(Body::empty())
        .unwrap();
    let response = app(Arc::clone(&state)).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Other agent gets 404.
    let request = Request::builder()
        .uri(format!("/orders/{id}"))
        .header("authorization", format!("Bearer {other_api}"))
        .body(Body::empty())
        .unwrap();
    let response = app(Arc::clone(&state)).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn list_orders_returns_only_callers_orders() {
    let state = test_state().await;

    let (a_key, a_api) = seed_agent(&state, "lst-a").await;
    let (_b_key, _b_api) = seed_agent(&state, "lst-b").await;

    use crate::hyperliquid::orders::store as orders_store;
    use rust_decimal_macros::dec;
    use serde_json::json;
    for (i, cloid) in ["cloid-a-1", "cloid-a-2"].iter().enumerate() {
        orders_store::insert_order(
            &state.db_pool,
            &orders_store::NewOrder {
                id: uuid::Uuid::new_v4(),
                agent_key: a_key.clone(),
                account_address: "0xa".to_string(),
                environment: "live".to_string(),
                group_id: None,
                parent_cloid: None,
                memory_record_ids: json!([]),
                symbol: "BTC".to_string(),
                instrument_id: None,
                side: "buy".to_string(),
                order_kind: "limit".to_string(),
                reduce_only: false,
                requested_price: Some(dec!(50000)),
                rounded_price: Some(dec!(50000)),
                requested_size: dec!(0.1),
                rounded_size: Some(dec!(0.1)),
                trigger_price: None,
                time_in_force: Some("gtc".to_string()),
                cloid: cloid.to_string(),
                status: if i == 0 {
                    "resting".to_string()
                } else {
                    "filled".to_string()
                },
                status_detail: None,
                request_payload: json!({}),
            },
        )
        .await
        .expect("insert order");
    }

    let request = Request::builder()
        .uri("/orders")
        .header("authorization", format!("Bearer {a_api}"))
        .body(Body::empty())
        .unwrap();
    let response = app(Arc::clone(&state)).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    let rows: Vec<serde_json::Value> = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| r["symbol"] == "BTC"));
}
