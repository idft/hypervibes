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
    memory::CreateMemory,
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
async fn latest_trade_execution_summary_event_renders_latest_summary() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
    seed_memory_with_type(
        &state,
        &agent_key,
        "trade_execution",
        "Scaled out into strength",
        "Took profit on the upper band.",
    )
    .await;

    let event = render_latest_trade_execution_summary_event(&state.db_pool, &agent_key)
        .await
        .expect("render latest trade execution summary event");
    let text = format!("{event:?}");

    assert!(text.contains("latest-trade-execution-summary"));
    assert!(text.contains("Scaled out into strength"));
    assert!(text.contains("timeago"));
    assert!(text.contains("datetime="));
    // Trade execution summaries never carry the warning treatment —
    // only the analysis section can turn amber.
    assert!(!text.contains("text-amber-400"));
}
#[tokio::test]
async fn latest_analysis_summary_event_renders_latest_summary() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
    let analysis = seed_memory_with_type(
        &state,
        &agent_key,
        "market_analysis",
        "BTC bullish continuation above 67k",
        "## Thesis\nReclaimed intraday support.",
    )
    .await;

    let event = render_latest_analysis_summary_event(&state.db_pool, &agent_key)
        .await
        .expect("render latest analysis summary event");
    let text = format!("{event:?}");

    assert!(text.contains("latest-analysis-summary"));
    assert!(text.contains("BTC bullish continuation above 67k"));
    assert!(text.contains(&format!("/agents/{agent_key}/memories/{}", analysis.id)));
    assert!(text.contains("timeago"));
    assert!(text.contains("datetime="));
    // The row we just inserted is brand new, so it should not be flagged
    // as expired or carry a warning icon.
    assert!(!text.contains("text-amber-400"));
    assert!(!text.contains("(expired"));
}
#[tokio::test]
async fn latest_analysis_summary_event_marks_expired_memory_with_warning() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");
    // `valid_for_seconds: 1` + no sleep on a slow CI machine still
    // lands the row in the past by the time we read it back.
    let _analysis = crate::memory::insert_memory(
        &state.db_pool,
        &agent_key,
        &CreateMemory {
            symbol: "BTC".to_string(),
            timeframe: None,
            memory_type: "market_analysis".to_string(),
            summary: "Stale breakout call".to_string(),
            content: "## Thesis\nBid got pulled.".to_string(),
            metadata: Some(serde_json::json!({ "valid_for_seconds": 1 })),
            links: None,
        },
    )
    .await
    .expect("insert memory");
    // Make sure the row's created_at is comfortably in the past so
    // `expires_at <= now` is unambiguous regardless of clock skew.
    sqlx::query("UPDATE memory.records SET created_at = NOW() - INTERVAL '5 minutes'")
        .execute(&state.db_pool)
        .await
        .expect("backdate memory");

    let event = render_latest_analysis_summary_event(&state.db_pool, &agent_key)
        .await
        .expect("render latest analysis summary event");
    let text = format!("{event:?}");

    assert!(text.contains("Stale breakout call"));
    assert!(
        text.contains("text-amber-400"),
        "expired market analysis should turn the timestamp amber"
    );
    assert!(
        text.contains("(expired"),
        "expired market analysis should annotate the title"
    );
    // The inline warning triangle is the visual signal that the market
    // analysis has aged past `valid_for_seconds` / `stale_after`.
    assert!(
        text.contains("viewBox=\\\"0 0 20 20\\\""),
        "expired market analysis should render a warning icon"
    );
}
#[tokio::test]
async fn latest_analysis_summary_event_renders_empty_when_no_market_analysis_memory() {
    let state = test_state().await;
    let (agent_key, _wallet_address) = insert_test_agent(&state).await.expect("insert agent");

    let event = render_latest_analysis_summary_event(&state.db_pool, &agent_key)
        .await
        .expect("render latest analysis summary event");
    let text = format!("{event:?}");

    assert!(text.contains("latest-analysis-summary"));
    // Empty placeholder — no summary text leaks through.
    assert!(!text.contains("Reclaimed intraday support"));
}
