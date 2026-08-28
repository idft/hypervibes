use std::{
    collections::BTreeMap,
    env, fs,
    io::ErrorKind,
    path::PathBuf,
    sync::{Arc, Weak},
};

use anyhow::{Context, Result, bail};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::task::spawn_blocking;
use tracing::{info, warn};
use workspace_store::{
    coding_workspace::{
        CandidateInspection, PromotionJournalPhase, PromotionResult, delete_coding_candidate,
        inspect_coding_candidate, list_promotion_journals, prepare_coding_candidate,
        promote_coding_candidate, recover_promotion_journal, store_coding_report,
    },
    isolated_workspace::{
        ConversationWorkspacePath, IsolatedWorkspaceCreated, IsolatedWorkspaceInspection,
        RunWorkspacePath, RuntimeSecretsScrubbed, create_conversation_workspace,
        create_run_workspace, delete_conversation_workspace, delete_run_workspace,
        inspect_conversation_workspace, inspect_run_workspace,
        scrub_conversation_workspace_runtime_secrets, scrub_run_workspace_runtime_secrets,
    },
    workspace::{
        OpenCodeWorkspaceAgent, OpenCodeWorkspaceConfig, WorkspaceBrowserListing,
        WorkspaceFilePreview, WorkspaceGenerationMode, WorkspaceTemplateDrift,
        delete_agent_workspace, diff_agent_workspace_from_template, generate_agent_workspace,
        list_workspace_browser_entries, read_workspace_browser_file,
    },
};

type IdempotencyLockMap = Arc<tokio::sync::Mutex<BTreeMap<String, Weak<tokio::sync::Mutex<()>>>>>;

#[derive(Clone)]
struct App {
    store: OpenCodeWorkspaceConfig,
    api_key: Arc<str>,
    idempotency_root: PathBuf,
    idempotency_locks: IdempotencyLockMap,
}

#[derive(Deserialize)]
struct WorkspaceRequest {
    mode: WorkspaceMode,
    display_name: String,
    agent_api_key: String,
    api_base_url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum WorkspaceMode {
    CreateNew,
    Regenerate,
}

#[derive(Serialize, Deserialize)]
struct WorkspaceResponse {
    workspace_container_path: String,
    profile_source: String,
}

#[derive(Deserialize)]
struct CandidateRequest {
    display_name: String,
    agent_api_key: String,
    api_base_url: String,
}

#[derive(Deserialize)]
struct WorkspaceBrowserFileQuery {
    path: String,
}

#[derive(Serialize, Deserialize)]
struct CandidateResponse {
    workspace_container_path: String,
    base_manifest: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct ReportRequest {
    report: Value,
}

#[derive(Deserialize)]
struct PromoteRequest {
    expected_base_manifest: BTreeMap<String, String>,
    candidate_manifest_hash: String,
}

#[derive(Serialize, Deserialize)]
struct DeleteResponse {
    deleted: bool,
}

#[derive(Serialize, Deserialize)]
struct RecoveryResponse {
    recovered: Vec<RecoveryTask>,
}

#[derive(Serialize, Deserialize)]
struct RecoveryTask {
    agent_key: String,
    task_id: i64,
    phase: PromotionJournalPhase,
}

#[derive(Serialize)]
struct ErrorBody {
    code: &'static str,
    message: &'static str,
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: &'static str,
}

impl ApiError {
    const fn bad_request() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "bad_request",
            message: "invalid request",
        }
    }

    const fn unauthorized() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            code: "unauthorized",
            message: "unauthorized",
        }
    }

    const fn invalid() -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            code: "invalid_workspace",
            message: "invalid workspace input",
        }
    }

    const fn conflict() -> Self {
        Self {
            status: StatusCode::CONFLICT,
            code: "workspace_conflict",
            message: "workspace state conflicts with this operation",
        }
    }

    const fn internal() -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "workspace_operation_failed",
            message: "workspace operation failed",
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorBody {
                code: self.code,
                message: self.message,
            }),
        )
            .into_response()
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let app = Arc::new(config_from_env()?);
    recover_at_startup(&app).await;
    let listener = tokio::net::TcpListener::bind(
        env::var("WORKSPACE_CONTROL_BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:14097".into()),
    )
    .await
    .context("failed to bind workspace controller")?;
    info!("workspace controller listening");
    axum::serve(listener, router(app))
        .await
        .context("workspace controller stopped")
}

fn router(app: Arc<App>) -> Router {
    Router::new()
        .route("/health", get(|| async { StatusCode::NO_CONTENT }))
        .route(
            "/v1/agent-workspaces/{agent_key}",
            post(create_workspace).delete(delete_workspace),
        )
        .route(
            "/v1/agent-workspaces/{agent_key}/template-drift",
            post(template_drift),
        )
        .route(
            "/v1/agent-workspaces/{agent_key}/browser",
            get(list_workspace_browser_entries_handler),
        )
        .route(
            "/v1/agent-workspaces/{agent_key}/browser/file",
            get(read_workspace_browser_file_handler),
        )
        .route(
            "/v1/coding-candidates/{agent_key}/{task_id}",
            post(create_candidate).delete(delete_candidate),
        )
        .route(
            "/v1/coding-candidates/{agent_key}/{task_id}/report",
            put(store_report),
        )
        .route(
            "/v1/coding-candidates/{agent_key}/{task_id}/inspection",
            get(inspect_candidate),
        )
        .route(
            "/v1/coding-candidates/{agent_key}/{task_id}/promote",
            post(promote_candidate),
        )
        .route(
            "/v1/run-workspaces/{agent_key}/{run_id}",
            post(create_run_workspace_handler).delete(delete_run_workspace_handler),
        )
        .route(
            "/v1/run-workspaces/{agent_key}/{run_id}/inspection",
            get(inspect_run_workspace_handler),
        )
        .route(
            "/v1/run-workspaces/{agent_key}/{run_id}/runtime-secrets",
            post(scrub_run_workspace_runtime_secrets_handler),
        )
        .route(
            "/v1/conversation-workspaces/{agent_key}/{conversation_id}",
            post(create_conversation_workspace_handler)
                .delete(delete_conversation_workspace_handler),
        )
        .route(
            "/v1/conversation-workspaces/{agent_key}/{conversation_id}/inspection",
            get(inspect_conversation_workspace_handler),
        )
        .route(
            "/v1/conversation-workspaces/{agent_key}/{conversation_id}/runtime-secrets",
            post(scrub_conversation_workspace_runtime_secrets_handler),
        )
        .route("/v1/promotion-recovery", post(recover_promotions))
        .with_state(app)
}

fn config_from_env() -> Result<App> {
    let api_key = required_env("WORKSPACE_CONTROL_API_KEY")?;
    let root = absolute_env("WORKSPACE_CONTROL_WORKSPACES_ROOT", "/workspaces")?;
    let template = absolute_env(
        "WORKSPACE_CONTROL_TEMPLATE_ROOT",
        "/opt/hypervibes/workspace-template",
    )?;
    if !template.is_dir() {
        bail!("WORKSPACE_CONTROL_TEMPLATE_ROOT must be a usable directory");
    }
    fs::create_dir_all(&root).context("failed to create workspace volume root")?;
    let container_root = env::var("WORKSPACE_CONTROL_CONTAINER_WORKSPACES_ROOT")
        .unwrap_or_else(|_| "/workspaces".to_string())
        .trim()
        .to_string();
    if container_root.is_empty() {
        bail!("WORKSPACE_CONTROL_CONTAINER_WORKSPACES_ROOT must not be empty");
    }
    if !container_root.starts_with('/') {
        bail!("WORKSPACE_CONTROL_CONTAINER_WORKSPACES_ROOT must be absolute");
    }
    let idempotency_root = root.join(".workspace-controller/idempotency");
    fs::create_dir_all(&idempotency_root)?;
    Ok(App {
        store: OpenCodeWorkspaceConfig {
            source_root: template,
            host_workspaces_root: root,
            container_workspaces_root: container_root,
            api_base_url: String::new(),
        },
        api_key: Arc::from(api_key),
        idempotency_root,
        idempotency_locks: idempotency_locks(),
    })
}

fn required_env(name: &str) -> Result<String> {
    let value = env::var(name).with_context(|| format!("missing {name}"))?;
    let value = value.trim();
    if value.is_empty() {
        bail!("{name} must not be empty");
    }
    Ok(value.to_owned())
}

fn absolute_env(name: &str, default: &str) -> Result<PathBuf> {
    let value = env::var(name).unwrap_or_else(|_| default.to_string());
    let path = PathBuf::from(value.trim());
    if !path.is_absolute() {
        bail!("{name} must be absolute");
    }
    Ok(path)
}

fn authorize(headers: &HeaderMap, app: &App) -> Result<(), ApiError> {
    let Some(value) = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
    else {
        return Err(ApiError::unauthorized());
    };
    let Some(supplied) = value.strip_prefix("Bearer ") else {
        return Err(ApiError::unauthorized());
    };
    if !constant_time_eq(supplied.as_bytes(), app.api_key.as_bytes()) {
        return Err(ApiError::unauthorized());
    }
    Ok(())
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut difference = left.len() ^ right.len();
    for index in 0..left.len().max(right.len()) {
        difference |= usize::from(*left.get(index).unwrap_or(&0) ^ *right.get(index).unwrap_or(&0));
    }
    difference == 0
}

async fn create_workspace(
    State(app): State<Arc<App>>,
    Path(agent_key): Path<String>,
    headers: HeaderMap,
    Json(request): Json<WorkspaceRequest>,
) -> Result<Json<WorkspaceResponse>, ApiError> {
    authorize(&headers, &app)?;
    let fingerprint = format!(
        "workspace:{agent_key}:{}:{}:{}:{}",
        request.display_name,
        request.agent_api_key,
        request.api_base_url,
        matches!(request.mode, WorkspaceMode::CreateNew)
    );
    idempotent(&app.clone(), &headers, fingerprint, move || {
        let mut config = app.store.clone();
        config.api_base_url = request.api_base_url;
        let generated = generate_agent_workspace(
            &config,
            &OpenCodeWorkspaceAgent {
                agent_key,
                display_name: request.display_name,
                api_key: request.agent_api_key,
            },
            match request.mode {
                WorkspaceMode::CreateNew => WorkspaceGenerationMode::CreateNew,
                WorkspaceMode::Regenerate => WorkspaceGenerationMode::Regenerate,
            },
        )
        .map_err(classify)?;
        Ok(WorkspaceResponse {
            workspace_container_path: generated.workspace_container_path,
            profile_source: "agent-runtime/workspace-template".into(),
        })
    })
    .await
    .map(Json)
}

async fn delete_workspace(
    State(app): State<Arc<App>>,
    Path(agent_key): Path<String>,
    headers: HeaderMap,
) -> Result<Json<DeleteResponse>, ApiError> {
    authorize(&headers, &app)?;
    idempotent(
        &app.clone(),
        &headers,
        format!("delete-workspace:{agent_key}"),
        move || {
            Ok(DeleteResponse {
                deleted: delete_agent_workspace(&app.store, &agent_key).map_err(classify)?,
            })
        },
    )
    .await
    .map(Json)
}

async fn create_run_workspace_handler(
    State(app): State<Arc<App>>,
    Path((agent_key, run_id)): Path<(String, i64)>,
    headers: HeaderMap,
) -> Result<Json<IsolatedWorkspaceCreated>, ApiError> {
    authorize(&headers, &app)?;
    let path = RunWorkspacePath::new(agent_key, run_id).map_err(classify)?;
    let fingerprint = format!("run-workspace:{}:{}", path.agent_key(), path.run_id());
    idempotent(&app.clone(), &headers, fingerprint, move || {
        create_run_workspace(&app.store, &path).map_err(classify)
    })
    .await
    .map(Json)
}

async fn delete_run_workspace_handler(
    State(app): State<Arc<App>>,
    Path((agent_key, run_id)): Path<(String, i64)>,
    headers: HeaderMap,
) -> Result<Json<DeleteResponse>, ApiError> {
    authorize(&headers, &app)?;
    let path = RunWorkspacePath::new(agent_key, run_id).map_err(classify)?;
    let fingerprint = format!(
        "delete-run-workspace:{}:{}",
        path.agent_key(),
        path.run_id()
    );
    idempotent(&app.clone(), &headers, fingerprint, move || {
        Ok(DeleteResponse {
            deleted: delete_run_workspace(&app.store, &path).map_err(classify)?,
        })
    })
    .await
    .map(Json)
}

async fn inspect_run_workspace_handler(
    State(app): State<Arc<App>>,
    Path((agent_key, run_id)): Path<(String, i64)>,
    headers: HeaderMap,
) -> Result<Json<IsolatedWorkspaceInspection>, ApiError> {
    authorize(&headers, &app)?;
    let path = RunWorkspacePath::new(agent_key, run_id).map_err(classify)?;
    let config = app.store.clone();
    let result = spawn_blocking(move || inspect_run_workspace(&config, &path))
        .await
        .map_err(|_| ApiError::internal())?
        .map_err(classify)?;
    Ok(Json(result))
}

async fn scrub_run_workspace_runtime_secrets_handler(
    State(app): State<Arc<App>>,
    Path((agent_key, run_id)): Path<(String, i64)>,
    headers: HeaderMap,
) -> Result<Json<RuntimeSecretsScrubbed>, ApiError> {
    authorize(&headers, &app)?;
    let path = RunWorkspacePath::new(agent_key, run_id).map_err(classify)?;
    let fingerprint = format!("scrub-run-workspace:{}:{}", path.agent_key(), path.run_id());
    idempotent(&app.clone(), &headers, fingerprint, move || {
        scrub_run_workspace_runtime_secrets(&app.store, &path).map_err(classify)
    })
    .await
    .map(Json)
}

async fn create_conversation_workspace_handler(
    State(app): State<Arc<App>>,
    Path((agent_key, conversation_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<IsolatedWorkspaceCreated>, ApiError> {
    authorize(&headers, &app)?;
    let path = ConversationWorkspacePath::parse(agent_key, &conversation_id).map_err(classify)?;
    let fingerprint = format!(
        "conversation-workspace:{}:{}",
        path.agent_key(),
        path.conversation_id()
    );
    idempotent(&app.clone(), &headers, fingerprint, move || {
        create_conversation_workspace(&app.store, &path).map_err(classify)
    })
    .await
    .map(Json)
}

async fn delete_conversation_workspace_handler(
    State(app): State<Arc<App>>,
    Path((agent_key, conversation_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<DeleteResponse>, ApiError> {
    authorize(&headers, &app)?;
    let path = ConversationWorkspacePath::parse(agent_key, &conversation_id).map_err(classify)?;
    let fingerprint = format!(
        "delete-conversation-workspace:{}:{}",
        path.agent_key(),
        path.conversation_id()
    );
    idempotent(&app.clone(), &headers, fingerprint, move || {
        Ok(DeleteResponse {
            deleted: delete_conversation_workspace(&app.store, &path).map_err(classify)?,
        })
    })
    .await
    .map(Json)
}

async fn inspect_conversation_workspace_handler(
    State(app): State<Arc<App>>,
    Path((agent_key, conversation_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<IsolatedWorkspaceInspection>, ApiError> {
    authorize(&headers, &app)?;
    let path = ConversationWorkspacePath::parse(agent_key, &conversation_id).map_err(classify)?;
    let config = app.store.clone();
    let result = spawn_blocking(move || inspect_conversation_workspace(&config, &path))
        .await
        .map_err(|_| ApiError::internal())?
        .map_err(classify)?;
    Ok(Json(result))
}

async fn scrub_conversation_workspace_runtime_secrets_handler(
    State(app): State<Arc<App>>,
    Path((agent_key, conversation_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<RuntimeSecretsScrubbed>, ApiError> {
    authorize(&headers, &app)?;
    let path = ConversationWorkspacePath::parse(agent_key, &conversation_id).map_err(classify)?;
    let fingerprint = format!(
        "scrub-conversation-workspace:{}:{}",
        path.agent_key(),
        path.conversation_id()
    );
    idempotent(&app.clone(), &headers, fingerprint, move || {
        scrub_conversation_workspace_runtime_secrets(&app.store, &path).map_err(classify)
    })
    .await
    .map(Json)
}

async fn template_drift(
    State(app): State<Arc<App>>,
    Path(agent_key): Path<String>,
    headers: HeaderMap,
    Json(request): Json<CandidateRequest>,
) -> Result<Json<WorkspaceTemplateDrift>, ApiError> {
    authorize(&headers, &app)?;
    let mut config = app.store.clone();
    config.api_base_url = request.api_base_url;
    let result = spawn_blocking(move || {
        diff_agent_workspace_from_template(
            &config,
            &OpenCodeWorkspaceAgent {
                agent_key,
                display_name: request.display_name,
                api_key: request.agent_api_key,
            },
        )
    })
    .await
    .map_err(|_| ApiError::internal())?
    .map_err(classify)?;
    Ok(Json(result))
}

async fn list_workspace_browser_entries_handler(
    State(app): State<Arc<App>>,
    Path(agent_key): Path<String>,
    headers: HeaderMap,
) -> Result<Json<WorkspaceBrowserListing>, ApiError> {
    authorize(&headers, &app)?;
    let config = app.store.clone();
    let result = spawn_blocking(move || list_workspace_browser_entries(&config, &agent_key))
        .await
        .map_err(|_| ApiError::internal())?
        .map_err(classify)?;
    Ok(Json(result))
}

async fn read_workspace_browser_file_handler(
    State(app): State<Arc<App>>,
    Path(agent_key): Path<String>,
    headers: HeaderMap,
    Query(query): Query<WorkspaceBrowserFileQuery>,
) -> Result<Json<WorkspaceFilePreview>, ApiError> {
    authorize(&headers, &app)?;
    let config = app.store.clone();
    let result =
        spawn_blocking(move || read_workspace_browser_file(&config, &agent_key, &query.path))
            .await
            .map_err(|_| ApiError::internal())?
            .map_err(classify)?;
    Ok(Json(result))
}

async fn create_candidate(
    State(app): State<Arc<App>>,
    Path((agent_key, task_id)): Path<(String, i64)>,
    headers: HeaderMap,
    Json(request): Json<CandidateRequest>,
) -> Result<Json<CandidateResponse>, ApiError> {
    authorize(&headers, &app)?;
    idempotent(
        &app.clone(),
        &headers,
        format!(
            "candidate:{agent_key}:{task_id}:{}:{}",
            request.display_name, request.agent_api_key
        ),
        move || {
            let mut config = app.store.clone();
            config.api_base_url = request.api_base_url;
            let candidate = prepare_coding_candidate(
                &config,
                &agent_key,
                task_id,
                &request.display_name,
                &request.agent_api_key,
            )
            .map_err(classify)?;
            let workspace_container_path =
                workspace_store::coding_workspace::candidate_container_root(
                    &config, &agent_key, task_id,
                )
                .map_err(classify)?;
            Ok(CandidateResponse {
                workspace_container_path,
                base_manifest: candidate.base_manifest,
            })
        },
    )
    .await
    .map(Json)
}

async fn store_report(
    State(app): State<Arc<App>>,
    Path((agent_key, task_id)): Path<(String, i64)>,
    headers: HeaderMap,
    Json(request): Json<ReportRequest>,
) -> Result<StatusCode, ApiError> {
    authorize(&headers, &app)?;
    idempotent(
        &app.clone(),
        &headers,
        format!(
            "report:{agent_key}:{task_id}:{}",
            hash_json(&request.report)
        ),
        move || {
            store_coding_report(&app.store, &agent_key, task_id, &request.report)
                .map_err(classify)?;
            Ok(())
        },
    )
    .await?;
    Ok(StatusCode::CREATED)
}

async fn inspect_candidate(
    State(app): State<Arc<App>>,
    Path((agent_key, task_id)): Path<(String, i64)>,
    headers: HeaderMap,
) -> Result<Json<CandidateInspection>, ApiError> {
    authorize(&headers, &app)?;
    let config = app.store.clone();
    let result = spawn_blocking(move || inspect_coding_candidate(&config, &agent_key, task_id))
        .await
        .map_err(|_| ApiError::internal())?
        .map_err(classify)?;
    Ok(Json(result))
}

async fn promote_candidate(
    State(app): State<Arc<App>>,
    Path((agent_key, task_id)): Path<(String, i64)>,
    headers: HeaderMap,
    Json(request): Json<PromoteRequest>,
) -> Result<Json<PromotionResult>, ApiError> {
    authorize(&headers, &app)?;
    idempotent(
        &app.clone(),
        &headers,
        format!(
            "promote:{agent_key}:{task_id}:{}",
            request.candidate_manifest_hash
        ),
        move || {
            promote_coding_candidate(
                &app.store,
                &agent_key,
                task_id,
                &request.expected_base_manifest,
                &request.candidate_manifest_hash,
            )
            .map_err(classify)
        },
    )
    .await
    .map(Json)
}

async fn delete_candidate(
    State(app): State<Arc<App>>,
    Path((agent_key, task_id)): Path<(String, i64)>,
    headers: HeaderMap,
) -> Result<Json<DeleteResponse>, ApiError> {
    authorize(&headers, &app)?;
    idempotent(
        &app.clone(),
        &headers,
        format!("delete-candidate:{agent_key}:{task_id}"),
        move || {
            Ok(DeleteResponse {
                deleted: delete_coding_candidate(&app.store, &agent_key, task_id)
                    .map_err(classify)?,
            })
        },
    )
    .await
    .map(Json)
}

async fn recover_promotions(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
) -> Result<Json<RecoveryResponse>, ApiError> {
    authorize(&headers, &app)?;
    idempotent(
        &app.clone(),
        &headers,
        "promotion-recovery".to_string(),
        move || {
            let recovered = list_promotion_journals(&app.store)
                .map_err(classify)?
                .into_iter()
                .map(|journal| {
                    let phase =
                        recover_promotion_journal(&app.store, &journal).map_err(classify)?;
                    Ok(RecoveryTask {
                        agent_key: journal.agent_key,
                        task_id: journal.task_id,
                        phase,
                    })
                })
                .collect::<Result<Vec<_>, ApiError>>()?;
            Ok(RecoveryResponse { recovered })
        },
    )
    .await
    .map(Json)
}

async fn recover_at_startup(app: &App) {
    let config = app.store.clone();
    let _ = spawn_blocking(move || {
        let journals = list_promotion_journals(&config)?;
        for journal in journals {
            let _ = recover_promotion_journal(&config, &journal)?;
        }
        Ok::<(), anyhow::Error>(())
    })
    .await
    .inspect_err(|error| warn!(error = ?error, "workspace recovery task failed"));
}

async fn idempotent<T, F>(
    app: &App,
    headers: &HeaderMap,
    fingerprint: String,
    operation: F,
) -> Result<T, ApiError>
where
    T: Serialize + DeserializeOwned + Send + 'static,
    F: FnOnce() -> Result<T, ApiError> + Send + 'static,
{
    let key = headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .filter(|key| !key.is_empty())
        .ok_or_else(ApiError::bad_request)?
        .to_owned();
    let key_hash = hash_json(&Value::String(key));
    let path = app.idempotency_root.join(&key_hash);
    // Same-key retries share a lock, so lookup, side effects, and publication
    // remain atomic without serializing unrelated controller operations.
    let _idempotency_guard = acquire_idempotency_lock(&app.idempotency_locks, &key_hash).await;
    spawn_blocking(move || {
        match fs::read(&path) {
            Ok(bytes) => {
                let record: IdempotencyRecord<T> =
                    serde_json::from_slice(&bytes).map_err(|_| ApiError::internal())?;
                return if record.fingerprint == fingerprint {
                    Ok(record.response)
                } else {
                    Err(ApiError::conflict())
                };
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(_) => return Err(ApiError::internal()),
        }
        let response = operation()?;
        let record = IdempotencyRecordRef {
            fingerprint,
            response: &response,
        };
        let temporary = path.with_extension("tmp");
        fs::write(
            &temporary,
            serde_json::to_vec(&record).map_err(|_| ApiError::internal())?,
        )
        .map_err(|_| ApiError::internal())?;
        fs::rename(&temporary, path).map_err(|_| ApiError::internal())?;
        Ok(response)
    })
    .await
    .map_err(|_| ApiError::internal())?
}

fn idempotency_locks() -> IdempotencyLockMap {
    Arc::new(tokio::sync::Mutex::new(BTreeMap::new()))
}

async fn acquire_idempotency_lock(
    locks: &IdempotencyLockMap,
    key_hash: &str,
) -> tokio::sync::OwnedMutexGuard<()> {
    let lock = {
        let mut locks = locks.lock().await;
        // Dead weak references are removed on subsequent requests, bounding
        // this map by concurrently active idempotency keys.
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(key_hash).and_then(Weak::upgrade) {
            lock
        } else {
            let lock = Arc::new(tokio::sync::Mutex::new(()));
            locks.insert(key_hash.to_string(), Arc::downgrade(&lock));
            lock
        }
    };
    lock.lock_owned().await
}

#[derive(Deserialize)]
struct IdempotencyRecord<T> {
    fingerprint: String,
    response: T,
}

#[derive(Serialize)]
struct IdempotencyRecordRef<'a, T> {
    fingerprint: String,
    response: &'a T,
}

fn hash_json(value: &Value) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(value).unwrap_or_default())
    )
}

fn classify(error: anyhow::Error) -> ApiError {
    let message = error.to_string();
    if message.contains("already exists")
        || message.contains("changed while")
        || message.contains("changed after")
    {
        ApiError::conflict()
    } else if message.contains("agent key")
        || message.contains("agent_key")
        || message.contains("task id")
        || message.contains("run id")
        || message.contains("conversation id")
        || message.contains("symlink")
        || message.contains("not a regular file")
        || message.contains("not a directory")
        || message.contains("unsupported")
        || message.contains("invalid")
    {
        ApiError::invalid()
    } else {
        warn!(error = ?error, "workspace operation failed");
        ApiError::internal()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        process,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            let suffix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time after unix epoch")
                .as_nanos();
            let path = PathBuf::from("/tmp/opencode")
                .join(format!("workspace-controller-{}-{suffix}", process::id()));
            fs::create_dir_all(&path).expect("create temp directory");
            Self { path }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn browser_test_app(root: &std::path::Path) -> Arc<App> {
        let idempotency_root = root.join("idempotency");
        fs::create_dir_all(&idempotency_root).expect("create idempotency directory");
        Arc::new(App {
            store: OpenCodeWorkspaceConfig {
                source_root: root.join("template"),
                host_workspaces_root: root.to_path_buf(),
                container_workspaces_root: "/workspaces".to_string(),
                api_base_url: String::new(),
            },
            api_key: Arc::from("controller-key"),
            idempotency_root,
            idempotency_locks: idempotency_locks(),
        })
    }

    fn authorized_headers() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            "Bearer controller-key".parse().expect("header value"),
        );
        headers
    }

    fn idempotent_headers(key: &str) -> HeaderMap {
        let mut headers = authorized_headers();
        headers.insert(
            "idempotency-key",
            key.parse().expect("idempotency header value"),
        );
        headers
    }

    #[tokio::test]
    async fn idempotency_locks_are_keyed_and_reclaimed() {
        let locks = idempotency_locks();
        let first = acquire_idempotency_lock(&locks, "first").await;
        let second = acquire_idempotency_lock(&locks, "second").await;
        assert_eq!(locks.lock().await.len(), 2);

        drop(first);
        drop(second);
        let third = acquire_idempotency_lock(&locks, "third").await;
        assert_eq!(locks.lock().await.len(), 1);
        drop(third);
    }

    #[tokio::test]
    async fn workspace_browser_handlers_require_authentication() {
        let temp = TempDir::new();
        let app = browser_test_app(&temp.path);

        let list_error = list_workspace_browser_entries_handler(
            State(app.clone()),
            Path("agent".to_string()),
            HeaderMap::new(),
        )
        .await
        .expect_err("list without auth should fail");
        assert_eq!(
            list_error.into_response().status(),
            StatusCode::UNAUTHORIZED
        );

        let file_error = read_workspace_browser_file_handler(
            State(app),
            Path("agent".to_string()),
            HeaderMap::new(),
            Query(WorkspaceBrowserFileQuery {
                path: "file.txt".to_string(),
            }),
        )
        .await
        .expect_err("file without auth should fail");
        assert_eq!(
            file_error.into_response().status(),
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn workspace_browser_handlers_return_typed_results_without_idempotency() {
        let temp = TempDir::new();
        let app = browser_test_app(&temp.path);
        let workspace = temp.path.join("agents/agent");
        fs::create_dir_all(&workspace).expect("create workspace");
        fs::write(workspace.join("file.txt"), "contents\n").expect("write workspace file");
        let headers = authorized_headers();

        let Json(listing) = list_workspace_browser_entries_handler(
            State(app.clone()),
            Path("agent".to_string()),
            headers.clone(),
        )
        .await
        .expect("list workspace");
        assert!(listing.workspace_exists);
        assert_eq!(listing.entries[0].path, "file.txt");

        let Json(preview) = read_workspace_browser_file_handler(
            State(app.clone()),
            Path("agent".to_string()),
            headers,
            Query(WorkspaceBrowserFileQuery {
                path: "file.txt".to_string(),
            }),
        )
        .await
        .expect("read workspace file");
        assert_eq!(
            preview.status,
            workspace_store::workspace::WorkspaceFilePreviewStatus::Text
        );
        assert_eq!(preview.text.as_deref(), Some("contents\n"));

        fs::remove_file(workspace.join("file.txt")).expect("remove workspace file");
        let Json(missing) = read_workspace_browser_file_handler(
            State(app.clone()),
            Path("agent".to_string()),
            authorized_headers(),
            Query(WorkspaceBrowserFileQuery {
                path: "file.txt".to_string(),
            }),
        )
        .await
        .expect("read removed workspace file");
        assert_eq!(
            missing.status,
            workspace_store::workspace::WorkspaceFilePreviewStatus::Missing
        );
        assert_eq!(
            fs::read_dir(&app.idempotency_root)
                .expect("read idempotency directory")
                .count(),
            0
        );
    }

    #[tokio::test]
    async fn workspace_browser_handler_rejects_unsafe_file_paths() {
        let temp = TempDir::new();
        let app = browser_test_app(&temp.path);
        let workspace = temp.path.join("agents/agent");
        fs::create_dir_all(workspace.join("directory")).expect("create workspace directory");
        fs::write(workspace.join(".env"), "secret\n").expect("write env");

        for path in [".env", "../.env", "directory"] {
            let error = read_workspace_browser_file_handler(
                State(app.clone()),
                Path("agent".to_string()),
                authorized_headers(),
                Query(WorkspaceBrowserFileQuery {
                    path: path.to_string(),
                }),
            )
            .await
            .expect_err("unsafe browser read should fail");
            assert_eq!(
                error.into_response().status(),
                StatusCode::UNPROCESSABLE_ENTITY
            );
        }

        let error = list_workspace_browser_entries_handler(
            State(app),
            Path("../agent".to_string()),
            authorized_headers(),
        )
        .await
        .expect_err("unsafe agent key should fail");
        assert_eq!(
            error.into_response().status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }

    #[tokio::test]
    async fn run_workspace_handlers_recover_partial_state_and_scrub_secrets() {
        let temp = TempDir::new();
        let app = browser_test_app(&temp.path);
        fs::create_dir_all(temp.path.join("runs/agent/42")).expect("create partial run root");

        let Json(created) = create_run_workspace_handler(
            State(app.clone()),
            Path(("agent".to_string(), 42)),
            idempotent_headers("run-create"),
        )
        .await
        .expect("create run workspace");
        assert_eq!(
            created.workspace_container_path,
            "/workspaces/runs/agent/42/workspace"
        );
        let workspace = temp.path.join("runs/agent/42/workspace");
        assert!(workspace.is_dir());

        let Json(repeated) = create_run_workspace_handler(
            State(app.clone()),
            Path(("agent".to_string(), 42)),
            idempotent_headers("run-create"),
        )
        .await
        .expect("repeat run workspace create");
        assert_eq!(created, repeated);

        fs::write(workspace.join(".env"), "runtime-secret\n").expect("write runtime secret");
        let Json(before_scrub) = inspect_run_workspace_handler(
            State(app.clone()),
            Path(("agent".to_string(), 42)),
            authorized_headers(),
        )
        .await
        .expect("inspect run workspace");
        assert!(before_scrub.runtime_secrets_present);

        let Json(scrubbed) = scrub_run_workspace_runtime_secrets_handler(
            State(app.clone()),
            Path(("agent".to_string(), 42)),
            idempotent_headers("run-scrub"),
        )
        .await
        .expect("scrub run workspace");
        assert!(scrubbed.workspace_exists);
        assert!(scrubbed.removed);
        assert!(!workspace.join(".env").exists());

        let Json(deleted) = delete_run_workspace_handler(
            State(app),
            Path(("agent".to_string(), 42)),
            idempotent_headers("run-delete"),
        )
        .await
        .expect("delete run workspace");
        assert!(deleted.deleted);
    }

    #[tokio::test]
    async fn idempotency_key_serializes_conflicting_run_workspace_requests() {
        let temp = TempDir::new();
        let app = browser_test_app(&temp.path);
        let (first, second) = tokio::join!(
            create_run_workspace_handler(
                State(app.clone()),
                Path(("agent".to_string(), 43)),
                idempotent_headers("shared-run-create"),
            ),
            create_run_workspace_handler(
                State(app),
                Path(("agent".to_string(), 44)),
                idempotent_headers("shared-run-create"),
            )
        );

        let mut successes = 0;
        let mut conflicts = 0;
        for result in [first, second] {
            match result {
                Ok(_) => successes += 1,
                Err(error) => {
                    assert_eq!(error.into_response().status(), StatusCode::CONFLICT);
                    conflicts += 1;
                }
            }
        }
        assert_eq!(successes, 1);
        assert_eq!(conflicts, 1);
        assert_ne!(
            temp.path.join("runs/agent/43").is_dir(),
            temp.path.join("runs/agent/44").is_dir()
        );
    }

    #[tokio::test]
    async fn conversation_workspace_handlers_use_validated_internal_ids() {
        let temp = TempDir::new();
        let app = browser_test_app(&temp.path);
        let conversation_id = "b3ce59a8-f3b6-448d-a0c8-d44ea9d23a33";

        let Json(created) = create_conversation_workspace_handler(
            State(app.clone()),
            Path(("agent".to_string(), conversation_id.to_string())),
            idempotent_headers("conversation-create"),
        )
        .await
        .expect("create conversation workspace");
        assert_eq!(
            created.workspace_container_path,
            format!("/workspaces/conversations/agent/{conversation_id}/workspace")
        );

        let invalid = create_conversation_workspace_handler(
            State(app),
            Path(("agent".to_string(), "not-a-conversation-id".to_string())),
            idempotent_headers("conversation-invalid"),
        )
        .await
        .expect_err("invalid conversation id must fail");
        assert_eq!(
            invalid.into_response().status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
}
