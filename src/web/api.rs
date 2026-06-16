use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::json;
use tracing::error;
use uuid::Uuid;

use crate::{
    agents::{AuthenticatedAgent, store::get_agent},
    hyperliquid::live_state::{AccountKey, AccountLiveState, LiveConnectionStatus},
    memory::{CreateMemory, MemoryListFilter, MemoryRecord, store as memory_store},
    web::AppState,
};

/// Build the `/api/v1` sub-router. Merged into the main router in
/// `src/web/routes.rs`.
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/memories", post(create_memory).get(list_memories))
        .route("/memories/{id}", get(get_memory_by_id))
        .route("/account", get(get_account))
        .with_state(state)
}

/// Wire the `/api/v1` sub-router into a parent router.
pub fn merge(parent: Router, state: Arc<AppState>) -> Router {
    parent.nest("/api/v1", router(state))
}

/// JSON error type for the agent API. Distinct from the operator HTML
/// `AppError` in `src/web/routes.rs` — the agent API must always return
/// `{ "error": "..." }` so trading agents can parse it.
#[derive(Debug)]
pub enum ApiError {
    NotFound(&'static str),
    Validation(String),
    BadUuid,
    Internal(anyhow::Error),
}

impl ApiError {
    fn message(&self) -> String {
        match self {
            ApiError::NotFound(msg) => (*msg).to_string(),
            ApiError::Validation(msg) => msg.clone(),
            ApiError::BadUuid => "invalid memory id".to_string(),
            ApiError::Internal(_) => "internal server error".to_string(),
        }
    }

    fn status(&self) -> StatusCode {
        match self {
            ApiError::NotFound(_) => StatusCode::NOT_FOUND,
            ApiError::Validation(_) => StatusCode::UNPROCESSABLE_ENTITY,
            ApiError::BadUuid => StatusCode::NOT_FOUND,
            ApiError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl<E> From<E> for ApiError
where
    E: Into<anyhow::Error>,
{
    fn from(error: E) -> Self {
        ApiError::Internal(error.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        if let ApiError::Internal(ref e) = self {
            error!(error = ?e, "agent API request failed");
        }
        let body = Json(json!({ "error": self.message() }));
        (self.status(), body).into_response()
    }
}

/// `POST /api/v1/memories`
async fn create_memory(
    State(state): State<Arc<AppState>>,
    agent: AuthenticatedAgent,
    Json(input): Json<CreateMemory>,
) -> Result<Response, ApiError> {
    if let Err(errors) = input.validate() {
        return Err(ApiError::Validation(errors.join(" ")));
    }

    let record = memory_store::insert_memory(&state.db_pool, &agent.agent_key, &input)
        .await
        .map_err(ApiError::Internal)?;

    Ok((
        StatusCode::CREATED,
        Json(MemoryRecordResponse::from(record)),
    )
        .into_response())
}

/// `GET /api/v1/memories`
async fn list_memories(
    State(state): State<Arc<AppState>>,
    agent: AuthenticatedAgent,
    Query(filter): Query<MemoryListFilter>,
) -> Result<Response, ApiError> {
    // V1: empty `timeframe=` is treated as invalid (a blank-string match is
    //   never useful — callers should omit the param entirely).
    if let Some(tf) = &filter.timeframe
        && tf.trim().is_empty()
    {
        return Err(ApiError::Validation("timeframe must not be empty".into()));
    }

    let rows = memory_store::list_memories(&state.db_pool, &agent.agent_key, &filter)
        .await
        .map_err(ApiError::Internal)?;

    let bodies: Vec<MemoryRecordResponse> =
        rows.into_iter().map(MemoryRecordResponse::from).collect();
    Ok(Json(bodies).into_response())
}

/// `GET /api/v1/memories/{id}`
async fn get_memory_by_id(
    State(state): State<Arc<AppState>>,
    agent: AuthenticatedAgent,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let id = Uuid::parse_str(&id).map_err(|_| ApiError::BadUuid)?;
    let record = memory_store::get_memory(&state.db_pool, &agent.agent_key, id)
        .await
        .map_err(ApiError::Internal)?;

    match record {
        Some(r) => Ok(Json(MemoryRecordResponse::from(r)).into_response()),
        None => Err(ApiError::NotFound("memory not found")),
    }
}

/// `GET /api/v1/account`
///
/// Snapshot of the calling agent's Hyperliquid live account state. The
/// operator UI uses the SSE stream at `/agents/{agent_key}/live/stream`
/// instead; this is the agent-facing JSON poll.
async fn get_account(
    State(state): State<Arc<AppState>>,
    agent: AuthenticatedAgent,
) -> Result<Response, ApiError> {
    let row = get_agent(&state.db_pool, &agent.agent_key)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound("agent not found"))?;
    let key = AccountKey::new(&row.wallet_address, &row.environment);
    let snapshot = state.live_accounts.get(&key);
    let body = LiveAgentSnapshot {
        agent_key: row.agent_key.clone(),
        account_address: row.wallet_address.clone(),
        environment: row.environment.clone(),
        connected: snapshot
            .as_ref()
            .is_some_and(|s| s.status == LiveConnectionStatus::Connected),
        state: snapshot,
    };
    Ok(Json(body).into_response())
}

/// Snapshot of the calling agent's live account state. Returned to the
/// agent as JSON; the operator UI's SSE stream emits the rendered view
/// types from `src/web/templates.rs` directly.
#[derive(Debug, serde::Serialize)]
struct LiveAgentSnapshot {
    agent_key: String,
    account_address: String,
    environment: String,
    connected: bool,
    state: Option<AccountLiveState>,
}

/// Response shape returned to agents. Today it mirrors [`MemoryRecord`]
/// 1:1, but keeping a dedicated type lets us evolve the wire format
/// without breaking the DB row struct.
#[derive(Debug, serde::Serialize)]
pub struct MemoryRecordResponse {
    pub id: Uuid,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub agent_key: String,
    pub symbol: String,
    pub timeframe: Option<String>,
    pub memory_type: String,
    pub summary: String,
    pub content: String,
    pub metadata: serde_json::Value,
}

impl From<MemoryRecord> for MemoryRecordResponse {
    fn from(r: MemoryRecord) -> Self {
        Self {
            id: r.id,
            created_at: r.created_at,
            agent_key: r.agent_key,
            symbol: r.symbol,
            timeframe: r.timeframe,
            memory_type: r.memory_type,
            summary: r.summary,
            content: r.content,
            metadata: r.metadata,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use chrono::Utc;
    use tower::util::ServiceExt;

    use crate::{
        agents::{
            crypto::EncryptionKey,
            keys::derive_wallet_address,
            model::AgentRegistryRow,
            store::{get_agent, insert_agent},
        },
        db::{connect, migrate},
        hyperliquid::live_state::{AccountKey, LiveConnectionStatus},
        web::{AppState, api},
    };

    fn db_url() -> Option<String> {
        std::env::var("DATABASE_URL").ok()
    }

    async fn test_state() -> Option<Arc<AppState>> {
        let database_url = db_url()?;
        let pool = connect(&database_url).await.ok()?;
        migrate(&pool).await.ok()?;
        Some(Arc::new(AppState {
            db_pool: pool,
            encryption_key: EncryptionKey::new(
                "test",
                [
                    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21,
                    22, 23, 24, 25, 26, 27, 28, 29, 30, 31,
                ],
            ),
            live_accounts: Arc::new(crate::hyperliquid::live_state::LiveAccountStore::new()),
        }))
    }

    fn random_private_key() -> String {
        use rand::Rng;
        format!("0x{}", hex::encode(rand::thread_rng().r#gen::<[u8; 32]>()))
    }

    async fn seed_agent(state: &Arc<AppState>, suffix: &str) -> (String, String) {
        let timestamp = Utc::now().timestamp_millis();
        let display_name = format!("MemApiTest{}{}", suffix, timestamp);
        let agent_key = crate::agents::model::slugify_agent_key(&display_name);
        let private_key = random_private_key();
        let wallet_address = derive_wallet_address(&private_key).expect("derives");
        let api_key = format!("vta_memapi-{suffix}-{timestamp}");
        let now = Utc::now();
        let row = AgentRegistryRow {
            agent_key: agent_key.clone(),
            created_at: now,
            updated_at: now,
            enabled: true,
            display_name,
            prompt: String::new(),
            wallet_address,
            environment: "live".to_string(),
            api_key: api_key.clone(),
            api_key_last_used_at: None,
            hyperliquid_private_key_ciphertext: Vec::new(),
            hyperliquid_private_key_key_id: "test".to_string(),
        };
        insert_agent(&state.db_pool, &row)
            .await
            .expect("insert agent");
        (agent_key, api_key)
    }

    fn app(state: Arc<AppState>) -> Router {
        api::router(state)
    }

    fn json_body<T: serde::Serialize>(value: &T) -> (Option<(&'static str, String)>, Body) {
        let body = serde_json::to_vec(value).expect("serialize");
        (
            Some(("content-type", "application/json".to_string())),
            Body::from(body),
        )
    }

    #[tokio::test]
    async fn post_memories_creates_record() {
        let Some(state) = test_state().await else {
            eprintln!("DATABASE_URL not set; skipping route test");
            return;
        };

        let (_agent_key, api_key) = seed_agent(&state, "create").await;

        let body = serde_json::json!({
            "symbol": "BTC",
            "timeframe": "1h",
            "memory_type": "plan",
            "summary": "buy pullback",
            "content": "BTC reclaimed the prior breakout level",
            "metadata": { "confidence": 0.72 }
        });
        let (headers, body) = json_body(&body);
        let mut builder = Request::builder()
            .method("POST")
            .uri("/memories")
            .header("authorization", format!("Bearer {api_key}"));
        if let Some((k, v)) = headers {
            builder = builder.header(k, v);
        }
        let response = app(Arc::clone(&state))
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let ct = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(
            ct.starts_with("application/json"),
            "expected application/json content-type, got {ct}"
        );

        let body_bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(body["symbol"], "BTC");
        assert_eq!(body["timeframe"], "1h");
        assert_eq!(body["summary"], "buy pullback");
        assert_eq!(body["metadata"]["confidence"], 0.72);
        assert!(body["id"].is_string());
    }

    #[tokio::test]
    async fn post_memories_without_auth_returns_401_json() {
        let Some(state) = test_state().await else {
            eprintln!("DATABASE_URL not set; skipping route test");
            return;
        };

        let body = serde_json::json!({
            "symbol": "BTC",
            "memory_type": "plan",
            "summary": "x",
            "content": "y"
        });
        let (headers, body) = json_body(&body);
        let mut builder = Request::builder().method("POST").uri("/memories");
        if let Some((k, v)) = headers {
            builder = builder.header(k, v);
        }
        let response = app(state)
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
    async fn post_memories_with_invalid_bearer_returns_401_json() {
        let Some(state) = test_state().await else {
            eprintln!("DATABASE_URL not set; skipping route test");
            return;
        };

        let body = serde_json::json!({
            "symbol": "BTC",
            "memory_type": "plan",
            "summary": "x",
            "content": "y"
        });
        let (headers, body) = json_body(&body);
        let mut builder = Request::builder()
            .method("POST")
            .uri("/memories")
            .header("authorization", "Bearer vta_does-not-exist");
        if let Some((k, v)) = headers {
            builder = builder.header(k, v);
        }
        let response = app(state)
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn post_memories_empty_field_returns_422_json() {
        let Some(state) = test_state().await else {
            eprintln!("DATABASE_URL not set; skipping route test");
            return;
        };

        let (_agent_key, api_key) = seed_agent(&state, "val").await;

        let body = serde_json::json!({
            "symbol": "BTC",
            "memory_type": "plan",
            "summary": "  ",
            "content": "y"
        });
        let (headers, body) = json_body(&body);
        let mut builder = Request::builder()
            .method("POST")
            .uri("/memories")
            .header("authorization", format!("Bearer {api_key}"));
        if let Some((k, v)) = headers {
            builder = builder.header(k, v);
        }
        let response = app(state)
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body_bytes = axum::body::to_bytes(response.into_body(), 16 * 1024)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert!(body["error"].as_str().unwrap().contains("summary"));
    }

    #[tokio::test]
    async fn post_memories_non_object_metadata_returns_422() {
        let Some(state) = test_state().await else {
            eprintln!("DATABASE_URL not set; skipping route test");
            return;
        };

        let (_agent_key, api_key) = seed_agent(&state, "meta").await;

        let body = serde_json::json!({
            "symbol": "BTC",
            "memory_type": "plan",
            "summary": "x",
            "content": "y",
            "metadata": [1, 2, 3]
        });
        let (headers, body) = json_body(&body);
        let mut builder = Request::builder()
            .method("POST")
            .uri("/memories")
            .header("authorization", format!("Bearer {api_key}"));
        if let Some((k, v)) = headers {
            builder = builder.header(k, v);
        }
        let response = app(state)
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn auth_touches_api_key_last_used_at() {
        let Some(state) = test_state().await else {
            eprintln!("DATABASE_URL not set; skipping route test");
            return;
        };

        let (agent_key, api_key) = seed_agent(&state, "touch").await;

        let body = serde_json::json!({
            "symbol": "BTC",
            "memory_type": "plan",
            "summary": "x",
            "content": "y"
        });
        let (headers, body) = json_body(&body);
        let mut builder = Request::builder()
            .method("POST")
            .uri("/memories")
            .header("authorization", format!("Bearer {api_key}"));
        if let Some((k, v)) = headers {
            builder = builder.header(k, v);
        }
        let response = app(Arc::clone(&state))
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);

        let agent = get_agent(&state.db_pool, &agent_key)
            .await
            .unwrap()
            .expect("present");
        assert!(agent.api_key_last_used_at.is_some());
    }

    #[tokio::test]
    async fn list_memories_filters_and_orders_desc() {
        let Some(state) = test_state().await else {
            eprintln!("DATABASE_URL not set; skipping route test");
            return;
        };

        let (_agent_key, api_key) = seed_agent(&state, "list").await;

        for summary in ["a", "b", "c"] {
            let body = serde_json::json!({
                "symbol": "BTC",
                "timeframe": "1h",
                "memory_type": "plan",
                "summary": summary,
                "content": format!("body {summary}"),
            });
            let (headers, body) = json_body(&body);
            let mut builder = Request::builder()
                .method("POST")
                .uri("/memories")
                .header("authorization", format!("Bearer {api_key}"));
            if let Some((k, v)) = headers {
                builder = builder.header(k, v);
            }
            let response = app(Arc::clone(&state))
                .oneshot(builder.body(body).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::CREATED);
        }

        // Also insert a general (NULL timeframe) memory.
        let body = serde_json::json!({
            "symbol": "BTC",
            "memory_type": "plan",
            "summary": "general",
            "content": "general"
        });
        let (headers, body) = json_body(&body);
        let mut builder = Request::builder()
            .method("POST")
            .uri("/memories")
            .header("authorization", format!("Bearer {api_key}"));
        if let Some((k, v)) = headers {
            builder = builder.header(k, v);
        }
        let response = app(Arc::clone(&state))
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);

        // List the 1h timeframe: should return the 3 plan memories in DESC order.
        let request = Request::builder()
            .uri("/memories?symbol=BTC&timeframe=1h&limit=10")
            .header("authorization", format!("Bearer {api_key}"))
            .body(Body::empty())
            .unwrap();
        let response = app(Arc::clone(&state)).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body_bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let rows: Vec<serde_json::Value> = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(rows.len(), 3);
        let summaries: Vec<&str> = rows
            .iter()
            .map(|r| r["summary"].as_str().unwrap())
            .collect();
        assert_eq!(summaries, vec!["c", "b", "a"]);

        // No timeframe => NULL-timeframe memories only.
        let request = Request::builder()
            .uri("/memories?symbol=BTC")
            .header("authorization", format!("Bearer {api_key}"))
            .body(Body::empty())
            .unwrap();
        let response = app(Arc::clone(&state)).oneshot(request).await.unwrap();
        let body_bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let rows: Vec<serde_json::Value> = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["summary"], "general");
        assert!(rows[0]["timeframe"].is_null());

        // Scope check: another agent must not see any of these rows.
        let (_other_key, other_api_key) = seed_agent(&state, "other").await;
        let request = Request::builder()
            .uri("/memories?symbol=BTC&timeframe=1h")
            .header("authorization", format!("Bearer {other_api_key}"))
            .body(Body::empty())
            .unwrap();
        let response = app(Arc::clone(&state)).oneshot(request).await.unwrap();
        let body_bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let rows: Vec<serde_json::Value> = serde_json::from_slice(&body_bytes).unwrap();
        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn get_memory_by_id_returns_200_or_404() {
        let Some(state) = test_state().await else {
            eprintln!("DATABASE_URL not set; skipping route test");
            return;
        };

        let (_agent_key, api_key) = seed_agent(&state, "get").await;
        let (_other_key, other_api_key) = seed_agent(&state, "get-other").await;

        let body = serde_json::json!({
            "symbol": "BTC",
            "timeframe": "1h",
            "memory_type": "plan",
            "summary": "x",
            "content": "y"
        });
        let (headers, body) = json_body(&body);
        let mut builder = Request::builder()
            .method("POST")
            .uri("/memories")
            .header("authorization", format!("Bearer {api_key}"));
        if let Some((k, v)) = headers {
            builder = builder.header(k, v);
        }
        let response = app(Arc::clone(&state))
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let body_bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let created: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        let id = created["id"].as_str().unwrap().to_string();

        // Owner can fetch.
        let request = Request::builder()
            .uri(format!("/memories/{id}"))
            .header("authorization", format!("Bearer {api_key}"))
            .body(Body::empty())
            .unwrap();
        let response = app(Arc::clone(&state)).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body_bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let fetched: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(fetched["id"], created["id"]);

        // Another agent gets 404 (do not leak existence).
        let request = Request::builder()
            .uri(format!("/memories/{id}"))
            .header("authorization", format!("Bearer {other_api_key}"))
            .body(Body::empty())
            .unwrap();
        let response = app(Arc::clone(&state)).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        // Invalid UUID -> 404.
        let request = Request::builder()
            .uri("/memories/not-a-uuid")
            .header("authorization", format!("Bearer {api_key}"))
            .body(Body::empty())
            .unwrap();
        let response = app(Arc::clone(&state)).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn get_account_returns_200_with_null_state_when_orchestrator_has_no_snapshot() {
        let Some(state) = test_state().await else {
            eprintln!("DATABASE_URL not set; skipping route test");
            return;
        };

        let (agent_key, api_key) = seed_agent(&state, "acct-empty").await;

        let request = Request::builder()
            .uri("/account")
            .header("authorization", format!("Bearer {api_key}"))
            .body(Body::empty())
            .unwrap();
        let response = app(Arc::clone(&state)).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body_bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(body["agent_key"], serde_json::Value::from(agent_key));
        assert_eq!(body["connected"], serde_json::Value::from(false));
        assert!(body["state"].is_null());
    }

    #[tokio::test]
    async fn get_account_returns_connected_true_after_live_state_seeded() {
        let Some(state) = test_state().await else {
            eprintln!("DATABASE_URL not set; skipping route test");
            return;
        };

        let (agent_key, api_key) = seed_agent(&state, "acct-live").await;
        let row = get_agent(&state.db_pool, &agent_key)
            .await
            .unwrap()
            .expect("present");
        let key = AccountKey::new(&row.wallet_address, &row.environment);
        state.live_accounts.set_status(&key, LiveConnectionStatus::Connected);

        let request = Request::builder()
            .uri("/account")
            .header("authorization", format!("Bearer {api_key}"))
            .body(Body::empty())
            .unwrap();
        let response = app(Arc::clone(&state)).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body_bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(body["agent_key"], serde_json::Value::from(agent_key));
        assert_eq!(
            body["account_address"],
            serde_json::Value::from(row.wallet_address)
        );
        assert_eq!(body["environment"], serde_json::Value::from(row.environment));
        assert_eq!(body["connected"], serde_json::Value::from(true));
        assert_eq!(
            body["state"]["status"],
            serde_json::Value::from("connected")
        );
    }

    #[tokio::test]
    async fn get_account_without_auth_returns_401_json() {
        let Some(state) = test_state().await else {
            eprintln!("DATABASE_URL not set; skipping route test");
            return;
        };

        let request = Request::builder()
            .uri("/account")
            .body(Body::empty())
            .unwrap();
        let response = app(state).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let body_bytes = axum::body::to_bytes(response.into_body(), 16 * 1024)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert!(body["error"].is_string());
    }

    #[tokio::test]
    async fn get_account_with_invalid_bearer_returns_401_json() {
        let Some(state) = test_state().await else {
            eprintln!("DATABASE_URL not set; skipping route test");
            return;
        };

        let request = Request::builder()
            .uri("/account")
            .header("authorization", "Bearer vta_does-not-exist")
            .body(Body::empty())
            .unwrap();
        let response = app(state).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
