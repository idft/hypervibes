use axum::{
    Json, Router,
    body::Body,
    extract::{Path, State},
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, patch, post},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Mutex, Notify};
use tower::ServiceExt;
use tracing_subscriber::layer::SubscriberExt;
use uuid::Uuid;

use crate::{
    agent_conversations::{
        model::{AgentConversationRow, CreateAgentConversation},
        service::ConversationService,
        store,
    },
    opencode::workspace_control_client::HttpWorkspaceController,
    web::{
        AppState,
        routes::{
            router,
            test_support::{insert_test_opencode_agent, response_text, test_state},
        },
        run_detail_events::RunDetailDbEvent,
    },
};

use super::chat::{NewConversationForm, opencode_message_id_at, strategy_prompt_chat_message};

#[test]
fn opencode_message_ids_sort_by_timestamp() {
    let first = opencode_message_id_at(
        1_785_042_399_207,
        Uuid::from_u128(0x1111_1111_1111_1111_1111_1111_1111_1111),
    );
    let second = opencode_message_id_at(
        1_785_042_399_208,
        Uuid::from_u128(0x2222_2222_2222_2222_2222_2222_2222_2222),
    );

    assert!(first.starts_with("msg_f9cd16fe7000"));
    assert!(first < second);
}

#[test]
fn prompt_chat_message_includes_the_current_editor_draft() {
    let message = strategy_prompt_chat_message("analysis", "Favor trend continuation.");

    assert!(message.contains("`analysis` strategy prompt"));
    assert!(message.contains("Favor trend continuation."));
    assert!(message.starts_with("We are discussing"));
}

#[test]
fn new_conversation_form_accepts_prompt_editor_fields() {
    let form: NewConversationForm =
        serde_json::from_str(r#"{"prompt_kind":"analysis","prompt":"Favor trend continuation."}"#)
            .expect("deserialize prompt editor fields");

    assert_eq!(
        form.strategy_prompt_context(),
        ("analysis", "Favor trend continuation.")
    );
}

#[derive(Default)]
struct CreationBackend {
    pause_workspace: AtomicBool,
    pause_session: AtomicBool,
    fail_session: AtomicBool,
    fail_workspace: AtomicBool,
    workspace_entered: Notify,
    session_entered: Notify,
    release_workspace: Notify,
    release_session: Notify,
    requests: Mutex<Vec<(String, Value)>>,
}

struct FakeBackend {
    base_url: String,
    control: Arc<CreationBackend>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for FakeBackend {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl FakeBackend {
    async fn start() -> Self {
        let control = Arc::new(CreationBackend::default());
        let app = Router::new()
            .route("/v1/conversation-workspaces/{agent}/{id}/materialize", post(fake_materialize))
            .route("/v1/conversation-workspaces/{agent}/{id}", delete(fake_delete_workspace))
            .route("/v1/conversation-workspaces/{agent}/{id}/inspection", get(fake_inspect_workspace))
            .route("/session", post(fake_create_session))
            .route("/session/status", get(fake_session_status))
            .route("/session/{id}/prompt_async", post(fake_session_action))
            .route("/session/{id}", patch(fake_session_action))
            .route("/provider", get(|| async { Json(json!({"all": [{"id": "test", "models": {"model": {"id": "model", "variants": {"high": {}}}}}], "connected": ["test"]})) }))
            .route("/permission", get(fake_permissions))
            .fallback(fake_unexpected_request)
            .with_state(Arc::clone(&control));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind fake backend");
        let base_url = format!("http://{}/", listener.local_addr().expect("fake address"));
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve fake backend");
        });
        Self {
            base_url,
            control,
            task,
        }
    }

    async fn state(&self) -> Arc<AppState> {
        let state = test_state().await;
        let mut inner = (*state).clone();
        inner.opencode_base_url = self.base_url.clone();
        inner.workspace_controller = Arc::new(
            HttpWorkspaceController::new(&self.base_url, "test-key".to_string())
                .expect("workspace controller"),
        );
        Arc::new(inner)
    }
}

async fn fake_materialize(
    State(control): State<Arc<CreationBackend>>,
    Path((agent, id)): Path<(String, Uuid)>,
) -> Response {
    control.workspace_entered.notify_one();
    if control.pause_workspace.load(Ordering::SeqCst) {
        control.release_workspace.notified().await;
    }
    if control.fail_workspace.load(Ordering::SeqCst) {
        return (StatusCode::INTERNAL_SERVER_ERROR, "materialization failed").into_response();
    }
    Json(
        json!({"workspace_container_path": format!("/workspaces/conversations/{agent}/{id}/workspace")}),
    ).into_response()
}

async fn fake_create_session(
    State(control): State<Arc<CreationBackend>>,
    Json(input): Json<Value>,
) -> Response {
    control
        .requests
        .lock()
        .await
        .push(("session".to_string(), input));
    control.session_entered.notify_one();
    if control.pause_session.load(Ordering::SeqCst) {
        control.release_session.notified().await;
    }
    if control.fail_session.load(Ordering::SeqCst) {
        return (StatusCode::INTERNAL_SERVER_ERROR, "creation failed").into_response();
    }
    Json(json!({"id": format!("ses_{}", Uuid::new_v4())})).into_response()
}

async fn fake_delete_workspace(State(control): State<Arc<CreationBackend>>) -> Json<Value> {
    control
        .requests
        .lock()
        .await
        .push(("delete-workspace".to_string(), Value::Null));
    Json(json!({"deleted": true}))
}

async fn fake_permissions(State(control): State<Arc<CreationBackend>>) -> Json<Value> {
    control
        .requests
        .lock()
        .await
        .push(("permission".to_string(), Value::Null));
    Json(json!([]))
}

async fn fake_session_status(State(control): State<Arc<CreationBackend>>) -> Json<Value> {
    control
        .requests
        .lock()
        .await
        .push(("session-status".to_string(), Value::Null));
    Json(json!({}))
}

async fn fake_inspect_workspace(State(control): State<Arc<CreationBackend>>) -> Json<Value> {
    control
        .requests
        .lock()
        .await
        .push(("inspect-workspace".to_string(), Value::Null));
    Json(
        json!({"workspace_exists": true, "size_bytes": 0, "file_count": 0, "runtime_secrets_present": true}),
    )
}

async fn fake_session_action(
    State(control): State<Arc<CreationBackend>>,
    Path(id): Path<String>,
    Json(input): Json<Value>,
) -> StatusCode {
    control.requests.lock().await.push((id, input));
    StatusCode::NO_CONTENT
}

async fn fake_unexpected_request(
    State(control): State<Arc<CreationBackend>>,
    request: Request<Body>,
) -> Response {
    control
        .requests
        .lock()
        .await
        .push((request.uri().path().to_string(), Value::Null));
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "unexpected remote request",
    )
        .into_response()
}

fn conversation_service(state: &AppState) -> ConversationService<'_> {
    ConversationService {
        pool: &state.db_pool,
        client: &state.opencode_client,
        base_url: &state.opencode_base_url,
        agent_api_base_url: &state.hypervibes_agent_api_base_url,
        workspace_controller: Arc::clone(&state.workspace_controller),
        in_flight: &state.in_flight,
        turn_tracker: &state.conversation_turns,
        shutdown_rx: state.shutdown_rx.clone(),
    }
}

async fn seed_pending(state: &Arc<AppState>, agent: &str) -> AgentConversationRow {
    store::create_conversation_with_default_policies(
        &state.db_pool,
        &CreateAgentConversation {
            agent_key: agent.to_string(),
            opencode_session_id: format!("pending_{}", Uuid::new_v4()),
            channel: "web".to_string(),
            external_conversation_key: None,
            title: "New conversation".to_string(),
            model_provider_id: "test".to_string(),
            model_id: "model".to_string(),
            model_variant: Some("high".to_string()),
        },
    )
    .await
    .expect("reserve pending conversation")
}

async fn wait_for(notification: &Notify) {
    tokio::time::timeout(Duration::from_secs(5), notification.notified())
        .await
        .expect("operation should reach pause point");
}

async fn drain(state: &AppState) {
    assert!(
        state
            .in_flight
            .wait_idle_with_timeout(Duration::from_secs(5))
            .await,
        "creation should drain"
    );
    assert_eq!(state.in_flight.in_flight(), 0);
}

#[derive(Clone, Default)]
struct CapturedCreationLogs(Arc<std::sync::Mutex<Vec<String>>>);

impl tracing_subscriber::Layer<tracing_subscriber::Registry> for CapturedCreationLogs {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _context: tracing_subscriber::layer::Context<'_, tracing_subscriber::Registry>,
    ) {
        struct Fields(String);
        impl tracing::field::Visit for Fields {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                use std::fmt::Write;
                write!(&mut self.0, "{}={value:?} ", field.name()).expect("write to log string");
            }
        }
        let mut fields = Fields(String::new());
        event.record(&mut fields);
        self.0.lock().expect("log capture lock").push(fields.0);
    }
}

#[tokio::test]
async fn conversation_creation_survives_caller_cancellation_at_each_remote_stage() {
    let logs = CapturedCreationLogs::default();
    let _subscriber = tracing::subscriber::set_default(
        tracing_subscriber::Registry::default().with(logs.clone()),
    );
    for (pause_workspace, fail_creation) in
        [(true, false), (false, false), (true, true), (false, true)]
    {
        let backend = FakeBackend::start().await;
        backend
            .control
            .pause_workspace
            .store(pause_workspace, Ordering::SeqCst);
        backend
            .control
            .pause_session
            .store(!pause_workspace, Ordering::SeqCst);
        backend
            .control
            .fail_session
            .store(fail_creation && !pause_workspace, Ordering::SeqCst);
        backend
            .control
            .fail_workspace
            .store(fail_creation && pause_workspace, Ordering::SeqCst);
        let state = backend.state().await;
        let (agent, _) = insert_test_opencode_agent(&state)
            .await
            .expect("insert agent");
        let caller_state = Arc::clone(&state);
        let caller_agent = agent.clone();
        let caller = tokio::spawn(async move {
            conversation_service(&caller_state)
                .create_web_conversation(&caller_agent, "test", "model", Some("high"))
                .await
        });
        wait_for(if pause_workspace {
            &backend.control.workspace_entered
        } else {
            &backend.control.session_entered
        })
        .await;
        let rows = store::list_agent_conversations(&state.db_pool, &agent)
            .await
            .expect("list reservations");
        assert_eq!(rows.len(), 1);
        let id = rows[0].id;
        assert!(crate::agent_conversations::model::session_is_pending(
            &rows[0].opencode_session_id
        ));
        assert_eq!(state.in_flight.in_flight(), 1);
        caller.abort();
        assert!(caller.await.expect_err("caller cancelled").is_cancelled());
        backend.control.release_workspace.notify_one();
        backend.control.release_session.notify_one();
        drain(&state).await;
        let row = store::get_agent_conversation(&state.db_pool, &agent, id)
            .await
            .expect("load finalized mapping");
        if fail_creation {
            assert!(
                row.is_none(),
                "ordinary failure must clean up the reservation"
            );
            if !pause_workspace {
                assert!(
                    backend
                        .control
                        .requests
                        .lock()
                        .await
                        .iter()
                        .any(|(path, _)| path == "delete-workspace")
                );
            }
            let logs = logs.0.lock().expect("read captured logs");
            let failure = logs
                .iter()
                .find(|line| line.contains("conversation creation failed") && line.contains(&agent))
                .expect("detached task logs failure");
            assert!(failure.contains(&id.to_string()));
            assert!(failure.contains(if pause_workspace {
                "workspace materialization"
            } else {
                "session creation"
            }));
            assert!(failure.contains("elapsed_ms="));
        } else {
            let row = row.expect("detached creation persists");
            assert!(row.opencode_session_id.starts_with("ses_"));
            assert_eq!(row.model_variant.as_deref(), Some("high"));
            assert_eq!(row.channel, "web");
            assert_eq!(row.tool_policies.len(), 4);
        }
    }
}

#[tokio::test]
async fn conversation_creation_drains_during_shutdown_and_refuses_new_admissions() {
    let backend = FakeBackend::start().await;
    backend.control.pause_session.store(true, Ordering::SeqCst);
    let initial_state = backend.state().await;
    let mut inner = (*initial_state).clone();
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    inner.shutdown_rx = shutdown_rx;
    let state = Arc::new(inner);
    let (agent, _) = insert_test_opencode_agent(&state)
        .await
        .expect("insert agent");
    let caller_state = Arc::clone(&state);
    let caller_agent = agent.clone();
    let caller = tokio::spawn(async move {
        conversation_service(&caller_state)
            .create_web_conversation(&caller_agent, "test", "model", None)
            .await
    });
    wait_for(&backend.control.session_entered).await;
    shutdown_tx.send(true).expect("signal shutdown");
    let error = conversation_service(&state)
        .create_web_conversation(&agent, "test", "model", None)
        .await
        .expect_err("reject admission");
    assert!(error.to_string().contains("shutting down"));
    assert_eq!(state.in_flight.in_flight(), 1);
    let drained = Arc::new(AtomicBool::new(false));
    let drain_state = Arc::clone(&state);
    let flag = Arc::clone(&drained);
    let waiter = tokio::spawn(async move {
        drain_state.in_flight.wait_idle().await;
        flag.store(true, Ordering::SeqCst);
    });
    tokio::task::yield_now().await;
    assert!(!drained.load(Ordering::SeqCst));
    backend.control.release_session.notify_one();
    let row = caller
        .await
        .expect("caller joins")
        .expect("admitted creation completes");
    waiter.await.expect("shutdown drain completes");
    assert!(row.opencode_session_id.starts_with("ses_"));
    drain(&state).await;
}

#[tokio::test]
async fn independently_admitted_conversations_keep_their_models_channels_and_policies() {
    let backend = FakeBackend::start().await;
    let state = backend.state().await;
    let (agent, _) = insert_test_opencode_agent(&state)
        .await
        .expect("insert agent");
    let service = conversation_service(&state);
    let (web, telegram) = tokio::join!(
        service.create_web_conversation(&agent, "test", "model", Some("high")),
        service.create_gateway_conversation(&agent, "telegram", "42", "test", "model", None),
    );
    let web = web.expect("web creation");
    let telegram = telegram.expect("gateway creation");
    assert_ne!(web.id, telegram.id);
    assert_ne!(web.opencode_session_id, telegram.opencode_session_id);
    assert_eq!(web.channel, "web");
    assert_eq!(web.external_conversation_key, None);
    assert_eq!(telegram.channel, "telegram");
    assert_eq!(telegram.external_conversation_key.as_deref(), Some("42"));
    for row in [web, telegram] {
        let stored = store::get_agent_conversation(&state.db_pool, &agent, row.id)
            .await
            .expect("load conversation")
            .expect("persisted");
        assert_eq!(stored.opencode_session_id, row.opencode_session_id);
        assert_eq!(stored.model_provider_id, "test");
        assert_eq!(stored.model_id, "model");
        assert_eq!(stored.model_variant, row.model_variant);
        assert_eq!(stored.tool_policies.len(), 4);
    }
    let requests = backend.control.requests.lock().await;
    assert_eq!(requests.len(), 2);
    assert!(
        requests
            .iter()
            .any(|(_, input)| input["model"]["variant"] == "high")
    );
    for (_, input) in requests.iter() {
        assert_eq!(input["agent"], "agent-conversations");
        let rules = input["permission"].as_array().expect("permission rules");
        for (permission, action) in [
            ("hypervibes_submit_orders", "ask"),
            ("hypervibes_write_memory", "ask"),
            ("hypervibes_send_notification", "deny"),
            ("hypervibes_add_journal_note", "deny"),
            ("hypervibes_list_account_trades", "allow"),
            ("hypervibes_get_account_trade", "allow"),
        ] {
            assert!(
                rules
                    .iter()
                    .any(|rule| rule["permission"] == permission && rule["action"] == action)
            );
        }
    }
    drain(&state).await;
}

#[tokio::test]
async fn pending_conversation_actions_and_agent_deletion_preserve_local_state() {
    let backend = FakeBackend::start().await;
    let state = backend.state().await;
    let (agent, _) = insert_test_opencode_agent(&state)
        .await
        .expect("insert agent");
    let row = seed_pending(&state, &agent).await;
    crate::agent_conversations::workspace::prepare_conversation_workspace(
        &state.db_pool,
        &agent,
        row.id,
        crate::harness::model::CAPABILITY_SCHEMA_VERSION,
    )
    .await
    .expect("prepare workspace");
    let app = router(Arc::clone(&state));
    for (action, form) in [
        ("messages", "message=hello&message_id=msg_test"),
        ("stop", ""),
        ("compact", ""),
        ("settings", ""),
        ("permissions/request/reply", "reply=once"),
        ("delete", ""),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/agents/{agent}/chat/{}/{action}", row.id))
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(form))
                    .expect("action request"),
            )
            .await
            .expect("action response");
        assert_eq!(
            response.status(),
            if action == "messages" {
                StatusCode::OK
            } else {
                StatusCode::CONFLICT
            },
            "{action}"
        );
        assert!(
            response_text(response)
                .await
                .contains("Conversation is still initializing. Please wait for it to finish."),
            "{action}"
        );
    }
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent}/delete"))
                .body(Body::empty())
                .expect("delete agent request"),
        )
        .await
        .expect("delete agent response");
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert!(
        response_text(response)
            .await
            .contains("Conversation is still initializing")
    );
    let preserved = crate::agents::store::get_agent(&state.db_pool, &agent)
        .await
        .expect("load agent")
        .expect("preserved agent");
    assert!(!preserved.enabled);
    assert!(
        store::get_agent_conversation(&state.db_pool, &agent, row.id)
            .await
            .expect("load conversation")
            .is_some()
    );
    assert!(
        crate::agent_conversations::workspace::get_conversation_workspace(
            &state.db_pool,
            &agent,
            row.id
        )
        .await
        .expect("load workspace")
        .is_some()
    );
    assert!(
        backend.control.requests.lock().await.is_empty(),
        "no session-specific requests or workspace deletion while pending"
    );
}

#[tokio::test]
async fn ordinary_web_creation_stores_the_session_and_redirects() {
    let backend = FakeBackend::start().await;
    let state = backend.state().await;
    let (agent, _) = insert_test_opencode_agent(&state)
        .await
        .expect("insert agent");
    let response = router(Arc::clone(&state))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent}/chat/conversations"))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("model_selection=test/model&model_variant=high"))
                .expect("create request"),
        )
        .await
        .expect("creation response");
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let rows = store::list_agent_conversations(&state.db_pool, &agent)
        .await
        .expect("list conversations");
    assert_eq!(rows.len(), 1);
    assert_eq!(
        response.headers()["location"],
        format!("/agents/{agent}/chat/{}", rows[0].id)
    );
    assert!(rows[0].opencode_session_id.starts_with("ses_"));
    assert_eq!(rows[0].model_variant.as_deref(), Some("high"));
    drain(&state).await;
}

#[tokio::test]
async fn prompt_context_creation_submits_its_opening_message_after_initialization() {
    let backend = FakeBackend::start().await;
    let state = backend.state().await;
    let (agent, _) = insert_test_opencode_agent(&state)
        .await
        .expect("insert agent");
    let prompt_kind: String = sqlx::query_scalar(
        "SELECT sub_agent_key FROM harness_sub_agents WHERE agent_key = $1 ORDER BY id LIMIT 1",
    )
    .bind(&agent)
    .fetch_one(&state.db_pool)
    .await
    .expect("prompt target");
    let response = router(Arc::clone(&state)).oneshot(Request::builder().method("POST")
        .uri(format!("/agents/{agent}/chat/conversations"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(format!("model_selection=test/model&model_variant=high&strategy_prompt_kind={prompt_kind}&strategy_prompt=Current+draft")))
        .expect("prompt-context request")).await.expect("prompt-context response");
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    drain(&state).await;
    let requests = backend.control.requests.lock().await;
    let (session_id, prompt) = requests
        .iter()
        .find(|(_, input)| input["parts"].is_array())
        .expect("opening message request");
    assert!(session_id.starts_with("ses_"));
    assert_eq!(prompt["variant"], "high");
    assert_eq!(
        prompt["parts"][0]["text"],
        strategy_prompt_chat_message(&prompt_kind, "Current draft")
    );
}

async fn read_snapshot(body: &mut Body) -> String {
    let mut snapshot = String::new();
    for _ in 0..5 {
        let frame = tokio::time::timeout(Duration::from_secs(5), body.frame())
            .await
            .expect("snapshot event arrives")
            .expect("stream stays open")
            .expect("valid body frame");
        snapshot.push_str(&String::from_utf8_lossy(
            frame.data_ref().expect("SSE data"),
        ));
    }
    snapshot
}

#[tokio::test]
async fn pending_conversation_sse_refreshes_and_tracks_the_initialized_session() {
    // Exercise both the conversation notification and the interval fallback
    // (its first tick is immediate, so no sleep-based race is needed).
    for notify in [true, false] {
        let backend = FakeBackend::start().await;
        let state = backend.state().await;
        let (agent, _) = insert_test_opencode_agent(&state)
            .await
            .expect("insert agent");
        let row = seed_pending(&state, &agent).await;
        let response = router(Arc::clone(&state))
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{agent}/chat/{}/stream", row.id))
                    .body(Body::empty())
                    .expect("stream request"),
            )
            .await
            .expect("stream response");
        assert_eq!(response.status(), StatusCode::OK);
        let mut body = response.into_body();
        let pending = read_snapshot(&mut body).await;
        assert!(pending.contains("Creating conversation…"));
        assert!(!pending.contains("Waiting for the OpenCode session mirror"));
        assert!(pending.contains("<fieldset disabled"));
        assert!(pending.contains("autofocus disabled"));
        store::set_conversation_session_id(&state.db_pool, &agent, row.id, "ses_initialized")
            .await
            .expect("finalize mapping");
        if notify {
            state
                .run_detail_events
                .publish(RunDetailDbEvent::ConversationChanged {
                    conversation_id: row.id,
                });
        }
        let initialized = read_snapshot(&mut body).await;
        assert!(initialized.contains("Waiting for the OpenCode session mirror"));
        assert!(!initialized.contains("Creating conversation…"));
        assert!(!initialized.contains("<fieldset disabled"));
        assert!(!initialized.contains("autofocus disabled"));
        // The stream must now recognize the real session's events, including
        // when the mapping update arrived only through the interval fallback.
        store::update_conversation_title(&state.db_pool, &agent, row.id, "Initialized title")
            .await
            .expect("update title");
        state
            .run_detail_events
            .publish(RunDetailDbEvent::SessionChanged {
                session_id: "ses_initialized".to_string(),
            });
        let next = read_snapshot(&mut body).await;
        assert!(next.contains("Initialized title"));
    }
}
