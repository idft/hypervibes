//! Tests for agents/live_stream_tests.rs
use super::*;
use crate::web::routes::router;
use crate::web::routes::test_support::*;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::Utc;
use http_body_util::BodyExt as _;
use tower::util::ServiceExt;

use crate::{
    agents::store::replace_agent_instruments,
    hyperliquid::live_state::{AccountKey, AccountLiveState, LiveConnectionStatus},
    hyperliquid::market_data::MarketPrice,
};
use std::{collections::HashMap, sync::Arc};

#[tokio::test]
async fn account_balance_stream_returns_404_for_unknown_agent() {
    let state = test_state().await;

    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/agents/does-not-exist-12345/live/stream")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn account_live_stream_closes_when_shutdown_is_signaled() {
    let state = test_state_with_backend_and_shutdown(Arc::new(NoopHarnessBackend), true).await;
    let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");

    let response = router(state)
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/live/stream"))
                .body(Body::empty())
                .expect("build live stream request"),
        )
        .await
        .expect("serve live stream");
    let mut body = response.into_body();
    let frame = tokio::time::timeout(std::time::Duration::from_millis(250), body.frame())
        .await
        .expect("shutdown should close the SSE stream promptly");

    assert!(frame.is_none(), "shutdown should close the SSE body");
}

#[tokio::test]
async fn reconnect_grace_expires_on_the_stream_without_new_exchange_data() {
    use crate::hyperliquid::live_state::{LiveMarginState, LiveOpenOrder, LivePosition};
    use rust_decimal::Decimal;

    let state = test_state().await;
    let (agent_key, wallet_address) = insert_test_agent(&state).await.expect("insert agent");
    let key = AccountKey::new(&wallet_address, "live");
    let now = Utc::now();
    state.live_accounts.replace(
        key.clone(),
        AccountLiveState {
            status: LiveConnectionStatus::Connected,
            clearinghouse_updated_at: Some(now),
            open_orders_updated_at: Some(now),
            spot_updated_at: Some(now),
            margin: Some(LiveMarginState {
                account_value: Some(Decimal::new(1000, 0)),
                ..Default::default()
            }),
            open_positions: vec![LivePosition {
                coin: "BTC".to_string(),
                szi: Some(Decimal::ONE),
                ..Default::default()
            }],
            open_orders: vec![LiveOpenOrder {
                coin: "ETH".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        },
    );
    state
        .live_accounts
        .set_status(&key, LiveConnectionStatus::Reconnecting);
    state.live_accounts.upsert(key, |state| {
        state.connection_interrupted_at = Some(now - chrono::Duration::seconds(28));
    });
    let response = router(state)
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/live/stream"))
                .body(Body::empty())
                .expect("build live stream request"),
        )
        .await
        .expect("serve live stream");
    let mut body = response.into_body();
    tokio::time::timeout(std::time::Duration::from_secs(8), async {
        let mut saw_cached_positions = false;
        let mut saw_cached_orders = false;
        let mut saw_reconnect_warning = false;
        let mut saw_expired_positions = false;
        loop {
            let frame = body
                .frame()
                .await
                .expect("stream remains open")
                .expect("valid SSE frame");
            let Ok(data) = frame.into_data() else {
                continue;
            };
            let text = String::from_utf8(data.to_vec()).expect("UTF-8 SSE event");
            saw_reconnect_warning |= text.contains("Showing last known data");
            if text.contains("event: positions") {
                saw_cached_positions |= text.contains("BTC");
                saw_expired_positions |= text.contains("unavailable");
            }
            if text.contains("event: orders") {
                saw_cached_orders |= text.contains("ETH");
                if text.contains("unavailable") {
                    assert!(saw_cached_positions);
                    assert!(saw_cached_orders);
                    assert!(saw_reconnect_warning);
                    assert!(saw_expired_positions);
                    break;
                }
            }
        }
    })
    .await
    .expect("the five-second freshness tick expires the reconnect display grace");
}

#[tokio::test]
async fn account_balance_stream_emits_initial_loading_placeholder() {
    let state = test_state().await;

    let (agent_key, _wallet_address) = match insert_test_agent(&state).await {
        Some(pair) => pair,
        None => return,
    };

    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{}/live/stream", agent_key))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("text/event-stream")
    );

    // Read just the first event boundary of the body so we capture the
    // initial event without waiting for the keep-alive timer.
    let text = read_sse_chunk(response.into_body(), 250).await;
    assert!(
        text.contains("event: balance"),
        "missing event line in {text}"
    );
    assert!(text.contains("Loading"), "expected placeholder in {text}");
}
#[tokio::test]
async fn account_balance_stream_emits_initial_value_when_state_present() {
    let state = test_state().await;

    let (agent_key, wallet_address) = match insert_test_agent(&state).await {
        Some(pair) => pair,
        None => return,
    };

    let key = AccountKey::new(&wallet_address, "live");
    state.live_accounts.replace(
        key.clone(),
        AccountLiveState {
            account_address: key.account_address.clone(),
            environment: key.environment.clone(),
            status: LiveConnectionStatus::Connected,
            margin: Some(crate::hyperliquid::live_state::LiveMarginState {
                account_value: Some(rust_decimal::Decimal::new(123_4567, 4)),
                ..Default::default()
            }),
            clearinghouse_updated_at: Some(Utc::now()),
            open_orders_updated_at: Some(Utc::now()),
            spot_updated_at: Some(Utc::now()),
            ..Default::default()
        },
    );
    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{}/live/stream", agent_key))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let text = read_sse_chunk(response.into_body(), 250).await;
    assert!(text.contains("event: balance"));
    assert!(text.contains("123.4567"));
    assert!(text.contains("USDC"));
}
#[tokio::test]
async fn account_balance_stream_emits_updates_when_state_changes() {
    let state = test_state().await;

    let (agent_key, wallet_address) = match insert_test_agent(&state).await {
        Some(pair) => pair,
        None => return,
    };

    let key = AccountKey::new(&wallet_address, "live");

    let app = router(state.clone());
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{}/live/stream", agent_key))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Update the live store; the SSE consumer should observe the
    // notification and emit a new event.
    state.live_accounts.replace(
        key.clone(),
        AccountLiveState {
            account_address: key.account_address.clone(),
            environment: key.environment.clone(),
            status: LiveConnectionStatus::Connected,
            margin: Some(crate::hyperliquid::live_state::LiveMarginState {
                account_value: Some(rust_decimal::Decimal::new(99_0000, 4)),
                ..Default::default()
            }),
            clearinghouse_updated_at: Some(Utc::now()),
            open_orders_updated_at: Some(Utc::now()),
            spot_updated_at: Some(Utc::now()),
            ..Default::default()
        },
    );

    // Longer wait: this test expects a second event after we mutate
    // live_accounts, which only happens once a real broadcast fires.
    let text = read_sse_chunk(response.into_body(), 2000).await;
    assert!(text.contains("event: balance"));
    assert!(text.contains("99.0000"));
}
#[tokio::test]
async fn open_positions_stream_returns_404_for_unknown_agent() {
    let state = test_state().await;

    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/agents/does-not-exist-12345/live/stream")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
#[tokio::test]
async fn open_positions_stream_emits_initial_loading_placeholder() {
    let state = test_state().await;

    let (agent_key, _wallet_address) = match insert_test_agent(&state).await {
        Some(pair) => pair,
        None => return,
    };

    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{}/live/stream", agent_key))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("text/event-stream")
    );

    let text = read_sse_chunk(response.into_body(), 250).await;
    assert!(
        text.contains("event: positions"),
        "missing event line in {text}"
    );
    assert!(text.contains("Loading"), "expected placeholder in {text}");
}
#[tokio::test]
async fn open_positions_stream_emits_initial_rows_when_state_present() {
    let state = test_state().await;

    let (agent_key, wallet_address) = match insert_test_agent(&state).await {
        Some(pair) => pair,
        None => return,
    };

    let key = AccountKey::new(&wallet_address, "live");
    state.live_accounts.replace(
        key.clone(),
        AccountLiveState {
            account_address: key.account_address.clone(),
            environment: key.environment.clone(),
            status: LiveConnectionStatus::Connected,
            clearinghouse_updated_at: Some(Utc::now()),
            open_orders_updated_at: Some(Utc::now()),
            spot_updated_at: Some(Utc::now()),
            open_positions: vec![crate::hyperliquid::live_state::LivePosition {
                coin: "BTC".to_string(),
                szi: Some(rust_decimal::Decimal::new(1, 0)),
                entry_px: Some(rust_decimal::Decimal::new(30000, 0)),
                liquidation_px: Some(rust_decimal::Decimal::new(25000, 0)),
                margin_used: Some(rust_decimal::Decimal::new(6000, 0)),
                position_value: Some(rust_decimal::Decimal::new(30000, 0)),
                unrealized_pnl: Some(rust_decimal::Decimal::new(1500, 0)),
                return_on_equity: Some(rust_decimal::Decimal::new(2500, 2)),
                leverage_type: Some("cross".to_string()),
                leverage_value: Some(5),
                max_leverage: Some(50),
            }],
            ..Default::default()
        },
    );
    state.market_data.replace_for_test(HashMap::from([(
        "BTC".to_string(),
        MarketPrice {
            current: Some(rust_decimal::Decimal::new(65_000, 0)),
            prices_24h: vec![
                rust_decimal::Decimal::new(63_000, 0),
                rust_decimal::Decimal::new(64_000, 0),
            ],
        },
    )]));

    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{}/live/stream", agent_key))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let text = read_sse_chunk(response.into_body(), 250).await;
    assert!(text.contains("event: positions"));
    assert!(text.contains("BTC"));
    assert!(text.contains("long"));
    assert!(text.contains("+25.00%"));
    assert!(text.contains("65,000.0000"));
    assert!(text.contains("BTC price over the last 24 hours"));
}

#[tokio::test]
async fn open_positions_stream_emits_configured_placeholder_rows_without_live_position() {
    let state = test_state().await;

    let (agent_key, wallet_address) = match insert_test_agent(&state).await {
        Some(pair) => pair,
        None => return,
    };

    seed_instrument(&state, "BTC", true).await;
    replace_agent_instruments(&state.db_pool, &agent_key, &["BTC".to_string()])
        .await
        .expect("select BTC instrument");

    let key = AccountKey::new(&wallet_address, "live");
    state.live_accounts.replace(
        key.clone(),
        AccountLiveState {
            account_address: key.account_address.clone(),
            environment: key.environment.clone(),
            status: LiveConnectionStatus::Connected,
            clearinghouse_updated_at: Some(Utc::now()),
            open_orders_updated_at: Some(Utc::now()),
            spot_updated_at: Some(Utc::now()),
            ..Default::default()
        },
    );

    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{}/live/stream", agent_key))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let text = read_sse_chunk(response.into_body(), 250).await;
    assert!(text.contains("event: positions"));
    assert!(text.contains("BTC"));
    assert!(text.contains("No position"));
    assert!(text.contains("https://app.hyperliquid.xyz/trade/BTC"));
    assert!(!text.contains("No open positions"));
}
#[tokio::test]
async fn open_orders_stream_returns_404_for_unknown_agent() {
    let state = test_state().await;

    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/agents/does-not-exist-12345/live/stream")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
#[tokio::test]
async fn open_orders_stream_emits_initial_loading_placeholder() {
    let state = test_state().await;

    let (agent_key, _wallet_address) = match insert_test_agent(&state).await {
        Some(pair) => pair,
        None => return,
    };

    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{}/live/stream", agent_key))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("text/event-stream")
    );

    let text = read_sse_chunk(response.into_body(), 250).await;
    assert!(
        text.contains("event: orders"),
        "missing event line in {text}"
    );
    assert!(text.contains("Loading"), "expected placeholder in {text}");
}
#[tokio::test]
async fn open_orders_stream_emits_initial_rows_when_state_present() {
    let state = test_state().await;

    let (agent_key, wallet_address) = match insert_test_agent(&state).await {
        Some(pair) => pair,
        None => return,
    };

    let key = AccountKey::new(&wallet_address, "live");
    state.live_accounts.replace(
        key.clone(),
        AccountLiveState {
            account_address: key.account_address.clone(),
            environment: key.environment.clone(),
            status: LiveConnectionStatus::Connected,
            clearinghouse_updated_at: Some(Utc::now()),
            open_orders_updated_at: Some(Utc::now()),
            spot_updated_at: Some(Utc::now()),
            open_orders: vec![crate::hyperliquid::live_state::LiveOpenOrder {
                coin: "ETH".to_string(),
                side: Some("buy".to_string()),
                limit_px: Some(rust_decimal::Decimal::new(1900, 0)),
                sz: Some(rust_decimal::Decimal::new(1, 0)),
                orig_sz: Some(rust_decimal::Decimal::new(1, 0)),
                oid: Some("123".to_string()),
                timestamp: Some(chrono::Utc::now().timestamp_millis() as u64),
                cloid: None,
                order_type: Some("limit".to_string()),
                tif: Some("Gtc".to_string()),
                reduce_only: Some(false),
                is_trigger: Some(false),
                trigger_px: None,
                trigger_condition: None,
                is_position_tpsl: Some(false),
            }],
            ..Default::default()
        },
    );

    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{}/live/stream", agent_key))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let text = read_sse_chunk(response.into_body(), 250).await;
    assert!(text.contains("event: orders"));
    assert!(text.contains("ETH"));
    assert!(text.contains("buy"));
    assert!(text.contains("limit"));
}
#[tokio::test]
async fn activity_event_renders_empty_without_memories() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");

    let event = render_agent_activity_event(&state.db_pool, &agent_key)
        .await
        .expect("render activity event");
    let text = format!("{event:?}");

    assert!(text.contains("activity"));
    assert!(text.contains("No activity yet."));
}

#[tokio::test]
async fn activity_shows_five_latest_agent_memories_and_updates_live() {
    use crate::memory::{CreateMemory, store::insert_memory};
    use crate::web::ui_events::UiEvent;

    let state = test_state().await;
    let (agent_key, _) = insert_test_agent(&state).await.expect("insert agent");
    let (other_agent_key, _) = insert_test_agent(&state).await.expect("insert other agent");
    let input = CreateMemory {
        scope_kind: "agent".to_string(),
        instrument_ids: Vec::new(),
        timeframe: None,
        memory_type: "review".to_string(),
        summary: "other-agent-activity".to_string(),
        content: "Activity details".to_string(),
        metadata: None,
        links: None,
    };
    insert_memory(&state.db_pool, &other_agent_key, &input, None)
        .await
        .expect("insert other agent memory");
    for index in 0..6 {
        let input = CreateMemory {
            memory_type: if index % 2 == 0 {
                "trading_decision"
            } else {
                "review"
            }
            .to_string(),
            summary: format!("activity-entry-{index}"),
            ..input.clone()
        };
        insert_memory(&state.db_pool, &agent_key, &input, None)
            .await
            .expect("insert activity memory");
    }

    let html = render_agent_activity_html(&state.db_pool, &agent_key)
        .await
        .expect("render activity");
    assert!(!html.contains("activity-entry-0"));
    assert!(!html.contains("other-agent-activity"));
    for index in 1..6 {
        assert!(html.contains(&format!("activity-entry-{index}")));
    }
    assert!(html.find("activity-entry-5") < html.find("activity-entry-4"));
    assert!(html.contains("trading decision"));
    assert!(html.contains("review"));

    let response = router(Arc::clone(&state))
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/live/stream"))
                .body(Body::empty())
                .expect("build live stream request"),
        )
        .await
        .expect("serve live stream");
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body();
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let frame = body
                .frame()
                .await
                .expect("initial SSE frame")
                .expect("read frame");
            if let Ok(data) = frame.into_data()
                && String::from_utf8_lossy(&data).contains("event: activity")
            {
                break;
            }
        }
    })
    .await
    .expect("receive initial activity snapshot");

    let memory = insert_memory(
        &state.db_pool,
        &agent_key,
        &CreateMemory {
            summary: "activity-entry-new".to_string(),
            ..input
        },
        None,
    )
    .await
    .expect("insert new review memory");
    state.ui_events.publish(UiEvent::MemoryCreated {
        agent_key: agent_key.clone(),
        memory_id: memory.id,
    });
    let text = read_sse_chunk(body, 1000).await;
    assert!(text.contains("event: activity"));
    assert!(text.contains("activity-entry-new"));
    assert!(text.contains(&format!("/agents/{agent_key}/memories/{}", memory.id)));
    assert!(!text.contains("activity-entry-1"));
}
