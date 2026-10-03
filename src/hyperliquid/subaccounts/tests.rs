use std::{collections::VecDeque, sync::Mutex as StdMutex, time::Duration};

use axum::{Json, Router, extract::State, http::StatusCode, response::IntoResponse, routing::post};
use serde_json::{Value, json};

use super::*;

pub(crate) const OWNER: &str = "0x1111111111111111111111111111111111111111";
pub(crate) const ADDRESS: &str = "0xabcdefabcdefabcdefabcdefabcdefabcdefabcd";
pub(crate) const PRIVATE_KEY: &str =
    "4c0883a69102937d6231471b5dbb6204fe5129617082795f9d3d2c7e2f9f3f5b";
const NAME: &str = "vt-BTC Momentum";

pub(crate) enum Reply {
    Json(Value),
    Malformed,
    HttpFailure,
    Timeout,
}

impl Reply {
    pub(crate) fn success() -> Self {
        Self::Json(json!({
            "status": "ok",
            "response": {"type": "createSubAccount", "data": ADDRESS}
        }))
    }
}

#[derive(Default)]
struct MockState {
    info: StdMutex<VecDeque<Reply>>,
    exchange: StdMutex<VecDeque<Reply>>,
    requests: StdMutex<Vec<(String, Value)>>,
}

pub(crate) struct MockExchange {
    pub(crate) creator: Arc<SubaccountCreator>,
    state: Arc<MockState>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for MockExchange {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl MockExchange {
    pub(crate) async fn new(info: Vec<Reply>, exchange: Vec<Reply>) -> Self {
        let state = Arc::new(MockState {
            info: StdMutex::new(info.into()),
            exchange: StdMutex::new(exchange.into()),
            ..MockState::default()
        });
        let app = Router::new()
            .route("/info", post(info_handler))
            .route("/exchange", post(exchange_handler))
            .with_state(Arc::clone(&state));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock exchange");
        let url = format!("http://{}", listener.local_addr().expect("mock address"));
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve mock exchange");
        });
        let client = hypersdk::hypercore::mainnet()
            .with_url(url.parse().expect("mock URL"))
            .with_http_client(
                reqwest::Client::builder()
                    .no_proxy()
                    .build()
                    .expect("mock HTTP client"),
            );
        Self {
            creator: Arc::new(SubaccountCreator::new(client)),
            state,
            task,
        }
    }

    pub(crate) fn requests(&self, path: &str) -> Vec<Value> {
        self.state
            .requests
            .lock()
            .expect("mock requests lock")
            .iter()
            .filter(|(request_path, _)| request_path == path)
            .map(|(_, request)| request.clone())
            .collect()
    }
}

async fn info_handler(
    State(state): State<Arc<MockState>>,
    Json(request): Json<Value>,
) -> axum::response::Response {
    respond(state, "info", request).await
}

async fn exchange_handler(
    State(state): State<Arc<MockState>>,
    Json(request): Json<Value>,
) -> axum::response::Response {
    respond(state, "exchange", request).await
}

async fn respond(state: Arc<MockState>, path: &str, request: Value) -> axum::response::Response {
    state
        .requests
        .lock()
        .expect("mock requests lock")
        .push((path.to_string(), request));
    let queue = if path == "info" {
        &state.info
    } else {
        &state.exchange
    };
    let reply = queue
        .lock()
        .expect("mock replies lock")
        .pop_front()
        .expect("unexpected mock request");
    match reply {
        Reply::Json(value) => Json(value).into_response(),
        Reply::Malformed => "invalid JSON".into_response(),
        Reply::HttpFailure => (StatusCode::BAD_GATEWAY, "exchange unavailable").into_response(),
        Reply::Timeout => {
            tokio::time::sleep(Duration::from_secs(6)).await;
            Json(json!({"status": "ok", "response": {"type": "createSubAccount", "data": ADDRESS}}))
                .into_response()
        }
    }
}

pub(crate) fn accounts(owner: &str, name: &str) -> Reply {
    let margin = json!({
        "accountValue": "0", "totalNtlPos": "0", "totalRawUsd": "0", "totalMarginUsed": "0"
    });
    Reply::Json(json!([{
        "name": name,
        "subAccountUser": ADDRESS,
        "master": owner,
        "clearinghouseState": {
            "marginSummary": margin,
            "crossMarginSummary": margin,
            "crossMaintenanceMarginUsed": "0",
            "withdrawable": "0",
            "assetPositions": [],
            "time": 0
        },
        "spotState": {"balances": []}
    }]))
}

pub(crate) fn empty() -> Reply {
    Reply::Json(json!([]))
}

fn signer() -> PrivateKeySigner {
    PRIVATE_KEY.parse().expect("test signer")
}

fn owner() -> Address {
    OWNER.parse().expect("test owner")
}

#[tokio::test]
async fn sdk_success_returns_address_without_post_creation_discovery() {
    let mock = MockExchange::new(vec![empty()], vec![Reply::success()]).await;
    let result = mock
        .creator
        .create(&signer(), owner(), NAME)
        .await
        .expect("create subaccount");
    assert_eq!(
        result,
        CreatedSubaccount {
            address: ADDRESS.parse().expect("address"),
            created: true
        }
    );
    assert_eq!(
        mock.requests("info"),
        vec![json!({"type": "subAccounts", "user": OWNER})]
    );
    let requests = mock.requests("exchange");
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0]["action"],
        json!({"type": "createSubAccount", "name": NAME})
    );
    assert!(requests[0]["nonce"].as_u64().is_some_and(|nonce| nonce > 0));
    assert!(requests[0]["signature"].is_object());
}

#[tokio::test]
async fn existing_exact_owned_name_is_reused_without_submission() {
    let mock = MockExchange::new(vec![accounts(OWNER, NAME)], vec![]).await;
    let result = mock
        .creator
        .create(&signer(), owner(), NAME)
        .await
        .expect("existing subaccount");
    assert!(!result.created);
    assert_eq!(result.address, ADDRESS.parse::<Address>().expect("address"));
    assert!(mock.requests("exchange").is_empty());
}

#[tokio::test]
async fn discovery_requires_exact_name_and_owner() {
    for response in [accounts(OWNER, "vt-BTC"), accounts(ADDRESS, NAME)] {
        let mock = MockExchange::new(vec![response], vec![Reply::success()]).await;
        assert!(
            mock.creator
                .create(&signer(), owner(), NAME)
                .await
                .expect("create")
                .created
        );
        assert_eq!(mock.requests("exchange").len(), 1);
    }
}

#[tokio::test]
async fn exchange_rejection_preserves_message_and_allows_a_later_attempt() {
    let mock = MockExchange::new(
        vec![empty(), empty()],
        vec![
            Reply::Json(json!({"status": "err", "response": "Need to deposit"})),
            Reply::success(),
        ],
    )
    .await;
    assert!(
        matches!(mock.creator.create(&signer(), owner(), NAME).await,
        Err(CreateSubaccountError::Rejected(message)) if message == "Need to deposit")
    );
    assert!(
        mock.creator
            .create(&signer(), owner(), NAME)
            .await
            .expect("retry rejected submission")
            .created
    );
    assert_eq!(mock.requests("exchange").len(), 2);
}

#[tokio::test]
async fn uncertain_results_reconcile_with_sdk_queries_without_resubmission() {
    for reply in [
        Reply::Malformed,
        Reply::HttpFailure,
        Reply::Timeout,
        Reply::Json(json!({"status": "ok", "response": {"type": "default"}})),
    ] {
        let mock = MockExchange::new(vec![empty(), accounts(OWNER, NAME)], vec![reply]).await;
        let result = mock
            .creator
            .create(&signer(), owner(), NAME)
            .await
            .expect("recover subaccount");
        assert!(result.created);
        assert_eq!(result.address, ADDRESS.parse::<Address>().expect("address"));
        assert_eq!(mock.requests("exchange").len(), 1);
        assert_eq!(mock.requests("info").len(), 2);
    }
}

#[tokio::test]
async fn unresolved_outcome_blocks_retries_until_the_account_appears() {
    let mock = MockExchange::new(
        vec![
            empty(),
            empty(),
            empty(),
            Reply::Malformed,
            accounts(OWNER, NAME),
        ],
        vec![Reply::Malformed],
    )
    .await;
    for _ in 0..3 {
        assert!(matches!(
            mock.creator.create(&signer(), owner(), NAME).await,
            Err(CreateSubaccountError::Unconfirmed)
        ));
    }
    let result = mock
        .creator
        .create(&signer(), owner(), NAME)
        .await
        .expect("eventual discovery");
    assert!(!result.created);
    assert_eq!(mock.requests("exchange").len(), 1);
}

#[tokio::test]
async fn failed_preflight_lookup_never_submits_creation() {
    let mock = MockExchange::new(vec![Reply::Malformed, Reply::HttpFailure], vec![]).await;
    for _ in 0..2 {
        assert!(matches!(
            mock.creator.create(&signer(), owner(), NAME).await,
            Err(CreateSubaccountError::LookupUnavailable)
        ));
    }
    assert!(mock.requests("exchange").is_empty());
}

#[tokio::test]
async fn concurrent_requests_do_not_duplicate_an_uncertain_submission() {
    let mock = MockExchange::new(vec![empty(), empty(), empty()], vec![Reply::Malformed]).await;
    let signer = signer();
    let (first, second) = tokio::join!(
        mock.creator.create(&signer, owner(), NAME),
        mock.creator.create(&signer, owner(), NAME),
    );
    assert!(matches!(first, Err(CreateSubaccountError::Unconfirmed)));
    assert!(matches!(second, Err(CreateSubaccountError::Unconfirmed)));
    assert_eq!(mock.requests("exchange").len(), 1);
}

#[tokio::test]
async fn cancelled_submission_is_reconciled_before_any_retry() {
    let mock = MockExchange::new(vec![empty(), empty()], vec![Reply::Timeout]).await;
    let creator = Arc::clone(&mock.creator);
    let task = tokio::spawn(async move { creator.create(&signer(), owner(), NAME).await });
    tokio::time::timeout(Duration::from_secs(1), async {
        while mock.requests("exchange").is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("submission started");
    task.abort();
    assert!(task.await.expect_err("cancelled request").is_cancelled());
    assert!(matches!(
        mock.creator.create(&signer(), owner(), NAME).await,
        Err(CreateSubaccountError::Unconfirmed)
    ));
    assert_eq!(mock.requests("exchange").len(), 1);
}
