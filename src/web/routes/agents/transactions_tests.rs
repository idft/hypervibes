//! Tests for agents/transactions_tests.rs
use crate::web::routes::router;
use crate::web::routes::test_support::*;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::{DateTime, Duration, TimeZone, Utc};
use rust_decimal::Decimal;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tower::util::ServiceExt;

use crate::hyperliquid::live_state::{AccountKey, AccountLiveState};
use crate::web::AppState;

const TRANSACTIONS_PER_PAGE: usize = 25;

#[tokio::test]
async fn agent_transactions_route_renders_full_timeline() {
    let state = test_state().await;
    let (agent_key, wallet_address) = insert_test_agent(&state).await.expect("insert agent");
    seed_ledger_event(
        &state,
        &wallet_address,
        &format!("tx-route-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0)),
        Utc::now(),
        Decimal::new(42, 0),
    )
    .await;

    let default_response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/trades"))
                .body(Body::empty())
                .expect("default view request"),
        )
        .await
        .expect("default view response");
    assert_eq!(default_response.status(), StatusCode::OK);
    let default_text = response_text(default_response).await;
    assert!(default_text.contains("No trades yet"));
    assert!(!default_text.contains("Showing 1-1 of 1 transactions"));

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/transactions"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert!(text.contains("Transactions"));
    assert!(text.contains("42.0000"));
    assert!(text.contains("Showing 1-1 of 1 transactions"));
    assert!(text.contains("Page 1 of 1"));
}
#[tokio::test]
async fn agent_transactions_route_does_not_anchor_running_balance_to_live_cash_balance() {
    let state = test_state().await;
    let (agent_key, wallet_address) = insert_test_agent(&state).await.expect("insert agent");
    seed_ledger_event(
        &state,
        &wallet_address,
        &format!(
            "tx-anchor-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ),
        Utc::now(),
        Decimal::new(42, 0),
    )
    .await;

    let account_key = AccountKey::new(&wallet_address, "live");
    state.live_accounts.replace(
        account_key,
        AccountLiveState {
            account_address: wallet_address.clone(),
            environment: "live".to_string(),
            spot_balances: vec![crate::hyperliquid::live_state::LiveSpotBalance {
                coin: "USDC".to_string(),
                total: Some(Decimal::new(420017, 4)),
                available: Some(Decimal::new(420017, 4)),
                ..Default::default()
            }],
            ..Default::default()
        },
    );

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/transactions"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert!(text.contains("42.0000"));
    assert!(!text.contains("42.0017"));
}

fn pagination_base_time() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
}

async fn seed_pagination_events(state: &Arc<AppState>, wallet_address: &str, count: usize) {
    let base = pagination_base_time();
    for index in 1..=count {
        let event_time = base + Duration::minutes(index as i64);
        seed_ledger_event(
            state,
            wallet_address,
            &format!("tx-page-{index:03}"),
            event_time,
            Decimal::new(1, 0),
        )
        .await;
    }
}

fn expected_time_text(minutes_offset: i64) -> String {
    let event_time = pagination_base_time() + Duration::minutes(minutes_offset);
    event_time.format("%Y-%m-%d %H:%M UTC").to_string()
}

fn format_money_like_template(value: Decimal) -> String {
    if value.is_sign_negative() {
        format!("({:.4})", value.abs())
    } else {
        format!("{:.4}", value)
    }
}

#[tokio::test]
async fn agent_transactions_route_paginates_timeline() {
    let state = test_state().await;
    let (agent_key, wallet_address) = insert_test_agent(&state).await.expect("insert agent");
    let total = TRANSACTIONS_PER_PAGE + 5;
    seed_pagination_events(&state, &wallet_address, total).await;

    let page_one = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/transactions"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(page_one.status(), StatusCode::OK);
    let page_one_text = response_text(page_one).await;
    assert!(page_one_text.contains(&format!(
        "Showing 1-{TRANSACTIONS_PER_PAGE} of {total} transactions"
    )));
    assert!(page_one_text.contains("Page 1 of 2"));
    assert!(page_one_text.contains(&expected_time_text(total as i64)));
    assert!(page_one_text.contains(&expected_time_text(6)));
    assert!(!page_one_text.contains(&expected_time_text(5)));
    assert!(!page_one_text.contains(&expected_time_text(1)));
    assert!(page_one_text.contains(&format!("/agents/{agent_key}/transactions?page=2")));

    let page_two = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/transactions?page=2"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(page_two.status(), StatusCode::OK);
    let page_two_text = response_text(page_two).await;
    assert!(page_two_text.contains(&format!(
        "Showing {first}-{last} of {total} transactions",
        first = TRANSACTIONS_PER_PAGE + 1,
        last = total
    )));
    assert!(page_two_text.contains("Page 2 of 2"));
    assert!(page_two_text.contains(&expected_time_text(5)));
    assert!(page_two_text.contains(&expected_time_text(1)));
    assert!(!page_two_text.contains(&expected_time_text(6)));
    assert!(page_two_text.contains(&format!("/agents/{agent_key}/transactions?page=1")));
}

#[tokio::test]
async fn agent_transactions_route_clamps_out_of_range_page_to_last_page() {
    let state = test_state().await;
    let (agent_key, wallet_address) = insert_test_agent(&state).await.expect("insert agent");
    let total = TRANSACTIONS_PER_PAGE + 3;
    seed_pagination_events(&state, &wallet_address, total).await;

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/transactions?page=42"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert!(text.contains("Page 2 of 2"));
    assert!(text.contains(&expected_time_text(3)));
    assert!(text.contains(&expected_time_text(1)));
    assert!(!text.contains(&expected_time_text(4)));
}

#[tokio::test]
async fn agent_transactions_route_uses_journal_balance_on_page_two() {
    let state = test_state().await;
    let (agent_key, wallet_address) = insert_test_agent(&state).await.expect("insert agent");
    let total = TRANSACTIONS_PER_PAGE + 5;
    seed_pagination_events(&state, &wallet_address, total).await;

    let account_key = AccountKey::new(&wallet_address, "live");
    state.live_accounts.replace(
        account_key,
        AccountLiveState {
            account_address: wallet_address.clone(),
            environment: "live".to_string(),
            spot_balances: vec![crate::hyperliquid::live_state::LiveSpotBalance {
                coin: "USDC".to_string(),
                total: Some(Decimal::new(100000, 4)),
                available: Some(Decimal::new(100000, 4)),
                ..Default::default()
            }],
            ..Default::default()
        },
    );

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{agent_key}/transactions?page=2"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert!(text.contains(&expected_time_text(5)));

    let formatted_5 = format_money_like_template(Decimal::new(5, 0));
    let formatted_1 = format_money_like_template(Decimal::new(1, 0));
    assert!(
        text.contains(&formatted_5),
        "expected journal balance {formatted_5} for event 5 in {text}"
    );
    assert!(
        text.contains(&formatted_1),
        "expected journal balance {formatted_1} for event 1 in {text}"
    );
}

#[tokio::test]
async fn activity_note_form_requires_csrf_and_preserves_page() {
    let state = test_state().await;
    let (agent_key, wallet) = insert_test_agent(&state).await.expect("insert agent");
    seed_ledger_event(
        &state,
        &wallet,
        "journal-activity",
        Utc::now(),
        Decimal::new(10, 0),
    )
    .await;
    let session = "journal-session";
    let csrf = "journal-csrf";
    sqlx::query(
        "INSERT INTO user_sessions(token_hash,csrf_hash,user_id,issued_at,expires_at)
        VALUES ($1,$2,$3,now(),now()+interval '1 day')",
    )
    .bind(Sha256::digest(session.as_bytes()).to_vec())
    .bind(Sha256::digest(csrf.as_bytes()).to_vec())
    .bind(crate::test_db::test_user_id())
    .execute(&state.db_pool)
    .await
    .expect("session");
    let cookie = format!("vt_session={session}; vt_csrf={csrf}");
    let url = format!("/agents/{agent_key}/transactions?kind=ledger&target=journal-activity");
    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(&url)
                .header("cookie", &cookie)
                .body(Body::empty())
                .expect("GET"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert!(text.contains(&format!("name=\"csrf_token\" value=\"{csrf}\"")));
    assert!(text.contains("data-notes-icon=\"outline\""));
    let path = format!("/agents/{agent_key}/transactions/ledger/journal-activity/notes");
    let bad = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(&path)
                .method("POST")
                .header("cookie", &cookie)
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("body=Not+authorized"))
                .expect("POST"),
        )
        .await
        .expect("response");
    assert_ne!(bad.status(), StatusCode::SEE_OTHER);
    let good = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(&path)
                .method("POST")
                .header("cookie", &cookie)
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(format!(
                    "csrf_token={csrf}&body=This+worked&page=2"
                )))
                .expect("POST"),
        )
        .await
        .expect("response");
    assert_eq!(good.status(), StatusCode::SEE_OTHER);
    assert!(
        good.headers()["location"]
            .to_str()
            .expect("location")
            .contains("page=2")
    );
    let (count,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM hyperliquid.journal_notes WHERE event_id='journal-activity'",
    )
    .fetch_one(&state.db_pool)
    .await
    .expect("notes");
    assert_eq!(count, 1);
    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(&url)
                .header("cookie", &cookie)
                .body(Body::empty())
                .expect("GET saved note"),
        )
        .await
        .expect("saved note response");
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response_text(response)
            .await
            .contains("data-notes-icon=\"filled\"")
    );
}
