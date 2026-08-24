use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;
use tracing::{info, warn};

use crate::{
    db::DbPool,
    harness::{
        model::{
            SUB_AGENT_KIND_ANALYSIS, SUB_AGENT_KIND_ANALYSIS_CODING, SUB_AGENT_KIND_DAILY_REVIEW,
            SUB_AGENT_KIND_MARKET_ANALYSIS, SUB_AGENT_KIND_TRADING,
        },
        store,
    },
    opencode::{
        client::{OpenCodeClient, OpenCodeCommandRequest, SessionStatusKind},
        store as opencode_store,
        workspace::OpenCodeWorkspaceRuntimeConfig,
    },
};

const ERROR_SUMMARY_MAX_CHARS: usize = 500;
#[cfg(not(test))]
const RETRY_STATUS_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);
#[cfg(test)]
const RETRY_STATUS_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(10);
const DEFAULT_ANALYSIS_AGENT: &str = "analysis";
const DEFAULT_ANALYSIS_COMMAND: &str = "hypervibes-analysis";
const DEFAULT_MARKET_ANALYSIS_AGENT: &str = "market-analysis";
const DEFAULT_MARKET_ANALYSIS_COMMAND: &str = "hypervibes-market-analysis";
const DEFAULT_DAILY_REVIEW_AGENT: &str = "daily-review";
const DEFAULT_DAILY_REVIEW_COMMAND: &str = "hypervibes-daily-review";
const DEFAULT_ANALYSIS_CODING_AGENT: &str = "analysis-coding";
const DEFAULT_ANALYSIS_CODING_COMMAND: &str = "hypervibes-analysis-coding";
const DEFAULT_TRADING_AGENT: &str = "trading";
const DEFAULT_TRADING_COMMAND: &str = "hypervibes-trading";
const MODEL_ACTIVITY_POLL_ATTEMPTS: usize = 10;
const MODEL_ACTIVITY_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(200);

#[derive(Debug, Clone)]
pub struct DispatchRequest {
    pub run_id: i64,
    pub sub_agent_id: i64,
    pub agent_key: String,
    pub display_name: String,
    pub sub_agent_key: String,
    pub sub_agent_kind: String,
    pub timeframe: Option<String>,
    pub operator_prompt: String,
    pub strategy_prompt: String,
    pub accumulated_learnings: Option<String>,
    pub system_prompt: String,
    pub environment: String,
    pub selected_instruments: Vec<String>,
    pub account_snapshot: Option<crate::hyperliquid::live_state::LiveAgentSnapshot>,
    pub model_provider_id: Option<String>,
    pub model_id: Option<String>,
    pub model_variant: Option<String>,
    pub timeout_seconds: i32,
    pub opencode_base_url: String,
    pub runtime_config: Value,
    pub scheduled_for: DateTime<Utc>,
    pub review_window_start: Option<DateTime<Utc>>,
    pub review_window_end: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct DispatchResult {
    pub backend_run_ref: String,
}

/// Trait implemented by anything that can execute a single scheduled
/// run. The production wiring uses `OpenCodeBackend`; tests can
/// substitute a fake.
///
/// The optional `abort_session` and `get_session_status` methods have
/// safe default implementations (returning `Ok(false)` / `Ok(None)`)
/// so test fakes can opt in only when they need to exercise
/// cancellation behaviour.
#[async_trait]
pub trait HarnessBackend: Send + Sync {
    async fn dispatch(&self, request: DispatchRequest) -> Result<DispatchResult>;

    /// Abort an in-flight OpenCode session. Returns `Ok(true)` if the
    /// server accepted the abort signal. Returns `Ok(false)` if the
    /// backend does not implement cancel (e.g. test fakes that never
    /// produce a real session). Errors are reserved for genuine
    /// transport/server failures.
    async fn abort_session(&self, _base_url: &str, _session_id: &str) -> Result<bool> {
        Ok(false)
    }

    /// Probe the live status of a previously-created OpenCode session.
    /// Returns `Ok(None)` when the backend / server does not know about
    /// the session (treated as terminal). Errors are reserved for
    /// transport/server failures; "session not found" is a probe, not
    /// an error.
    async fn get_session_status(
        &self,
        _base_url: &str,
        _session_id: &str,
    ) -> Result<Option<SessionStatusKind>> {
        Ok(None)
    }

    /// Probe a session within its workspace-scoped OpenCode instance. The
    /// default preserves compatibility with fakes that only implement the
    /// unscoped probe.
    async fn get_session_status_in_directory(
        &self,
        base_url: &str,
        session_id: &str,
        _workspace_container_path: Option<&str>,
    ) -> Result<Option<SessionStatusKind>> {
        self.get_session_status(base_url, session_id).await
    }
}

#[derive(Clone)]
pub struct OpenCodeBackend {
    pool: DbPool,
    client: Arc<OpenCodeClient>,
}

impl OpenCodeBackend {
    pub fn new(pool: DbPool, client: Arc<OpenCodeClient>) -> Self {
        Self { pool, client }
    }
}

#[async_trait]
impl HarnessBackend for OpenCodeBackend {
    async fn dispatch(&self, request: DispatchRequest) -> Result<DispatchResult> {
        let workspace = OpenCodeWorkspaceRuntimeConfig::from_value(&request.runtime_config)
            .ok_or_else(|| anyhow!("OpenCode workspace is not configured"))?;
        let workspace_container_path = workspace.workspace_container_path.clone();

        let (agent_name, command_name) = resolve_opencode_sub_agent(&request.sub_agent_kind)?;

        let title = format!(
            "{} {} {}",
            request.agent_key,
            request.sub_agent_key,
            request.scheduled_for.format("%Y-%m-%dT%H:%M:%SZ")
        );

        let session = self
            .client
            .create_session(
                &request.opencode_base_url,
                &workspace_container_path,
                Some(&title),
            )
            .await
            .with_context(|| {
                format!(
                    "failed to create OpenCode session for agent {} (workspace {})",
                    request.agent_key, workspace_container_path
                )
            })?;

        info!(
            run_id = request.run_id,
            sub_agent_id = request.sub_agent_id,
            agent_key = %request.agent_key,
            display_name = %request.display_name,
            sub_agent_key = %request.sub_agent_key,
            session_id = %session.id,
            "opencode session created"
        );

        store::mark_run_running(&self.pool, request.run_id, Some(&session.id))
            .await
            .with_context(|| {
                format!(
                    "failed to persist OpenCode session {} for run {}",
                    session.id, request.run_id
                )
            })?;

        let command_arguments = build_command_arguments(&request)?;

        let command_request = OpenCodeCommandRequest {
            command: command_name.to_string(),
            arguments: command_arguments,
            agent: Some(agent_name.to_string()),
            model: build_command_model(
                request.model_provider_id.as_deref(),
                request.model_id.as_deref(),
            ),
            variant: request.model_variant.clone(),
        };

        self.client
            .run_command(&request.opencode_base_url, &session.id, command_request)
            .await
            .with_context(|| {
                format!(
                    "OpenCode run_command failed for agent {} session {}",
                    request.agent_key, session.id
                )
            })?;

        if !wait_for_model_activity(&self.pool, &session.id).await? {
            let message = opencode_store::latest_session_error_message(&self.pool, &session.id)
                .await?
                .unwrap_or_else(|| "model request failed".to_string());
            return Err(anyhow!(message));
        }

        info!(
            run_id = request.run_id,
            sub_agent_id = request.sub_agent_id,
            agent_key = %request.agent_key,
            display_name = %request.display_name,
            sub_agent_key = %request.sub_agent_key,
            session_id = %session.id,
            "opencode command dispatched"
        );

        Ok(DispatchResult {
            backend_run_ref: session.id,
        })
    }

    async fn abort_session(&self, base_url: &str, session_id: &str) -> Result<bool> {
        self.client.abort_session(base_url, session_id).await
    }

    async fn get_session_status(
        &self,
        base_url: &str,
        session_id: &str,
    ) -> Result<Option<SessionStatusKind>> {
        self.client.get_session_status(base_url, session_id).await
    }

    async fn get_session_status_in_directory(
        &self,
        base_url: &str,
        session_id: &str,
        workspace_container_path: Option<&str>,
    ) -> Result<Option<SessionStatusKind>> {
        self.client
            .get_session_status_in_directory(base_url, session_id, workspace_container_path)
            .await
    }
}

async fn wait_for_model_activity(pool: &DbPool, session_id: &str) -> Result<bool> {
    for attempt in 0..MODEL_ACTIVITY_POLL_ATTEMPTS {
        if opencode_store::session_has_model_activity(pool, session_id).await? {
            return Ok(true);
        }
        if attempt + 1 < MODEL_ACTIVITY_POLL_ATTEMPTS {
            tokio::time::sleep(MODEL_ACTIVITY_POLL_INTERVAL).await;
        }
    }
    Ok(false)
}

fn resolve_opencode_sub_agent(sub_agent_kind: &str) -> Result<(&'static str, &'static str)> {
    match sub_agent_kind {
        SUB_AGENT_KIND_ANALYSIS => Ok((DEFAULT_ANALYSIS_AGENT, DEFAULT_ANALYSIS_COMMAND)),
        SUB_AGENT_KIND_MARKET_ANALYSIS => Ok((
            DEFAULT_MARKET_ANALYSIS_AGENT,
            DEFAULT_MARKET_ANALYSIS_COMMAND,
        )),
        SUB_AGENT_KIND_DAILY_REVIEW => {
            Ok((DEFAULT_DAILY_REVIEW_AGENT, DEFAULT_DAILY_REVIEW_COMMAND))
        }
        SUB_AGENT_KIND_ANALYSIS_CODING => Ok((
            DEFAULT_ANALYSIS_CODING_AGENT,
            DEFAULT_ANALYSIS_CODING_COMMAND,
        )),
        SUB_AGENT_KIND_TRADING => Ok((DEFAULT_TRADING_AGENT, DEFAULT_TRADING_COMMAND)),
        other => Err(anyhow!("unknown job kind: {other}")),
    }
}

fn build_command_arguments(request: &DispatchRequest) -> Result<String> {
    crate::harness::prompt::build_prompt(request)
}

fn build_command_model(model_provider_id: Option<&str>, model_id: Option<&str>) -> Option<String> {
    match (model_provider_id, model_id) {
        (Some(provider_id), Some(model_id)) => Some(format!("{provider_id}/{model_id}")),
        _ => None,
    }
}

/// Run a single dispatch through the backend, applying the job's
/// timeout. While OpenCode is retrying a provider request, its retry
/// message is mirrored to the running job. On timeout, the backend's `abort_session` /
/// `get_session_status` capabilities are used to confirm that the
/// previously-created OpenCode session is no longer executing before
/// marking the run `failed`. If cancellation cannot be confirmed (e.g.
/// the abort endpoint is unavailable or the session is reported as
/// `Busy` after abort), the run is left in `running` so recovery can
/// retry the probe later; in that case `Err` is returned so callers
/// do not trigger success-path follow-up events.
pub async fn dispatch_with_timeout(
    pool: &crate::db::DbPool,
    backend: Arc<dyn HarnessBackend>,
    request: DispatchRequest,
) -> Result<DispatchOutcome> {
    dispatch_with_timeout_mode(pool, backend, request, true).await
}

/// Execute an coding model session without terminalizing success. The
/// coding worker owns final success/failure after report validation and
/// promotion; timeout and backend failures still become terminal immediately.
pub async fn dispatch_with_timeout_for_coding(
    pool: &crate::db::DbPool,
    backend: Arc<dyn HarnessBackend>,
    request: DispatchRequest,
) -> Result<DispatchOutcome> {
    dispatch_with_timeout_mode(pool, backend, request, false).await
}

async fn dispatch_with_timeout_mode(
    pool: &crate::db::DbPool,
    backend: Arc<dyn HarnessBackend>,
    request: DispatchRequest,
    finalize_success: bool,
) -> Result<DispatchOutcome> {
    let run_id = request.run_id;
    let timeout_seconds = request.timeout_seconds;
    let timeout = std::time::Duration::from_secs(timeout_seconds.max(0) as u64);
    let opencode_base_url = request.opencode_base_url.clone();
    let workspace_container_path =
        OpenCodeWorkspaceRuntimeConfig::from_value(&request.runtime_config)
            .map(|workspace| workspace.workspace_container_path);

    let dispatch = tokio::time::timeout(timeout, backend.dispatch(request));
    tokio::pin!(dispatch);
    let mut retry_status_poll = tokio::time::interval(RETRY_STATUS_POLL_INTERVAL);
    retry_status_poll.tick().await;

    let dispatch_result = loop {
        tokio::select! {
            result = &mut dispatch => break result,
            _ = retry_status_poll.tick() => {
                let Some(session_id) = store::get_run(pool, run_id)
                    .await?
                    .and_then(|run| run.backend_run_ref)
                else {
                    continue;
                };
                let status = match backend
                    .get_session_status_in_directory(
                        &opencode_base_url,
                        &session_id,
                        workspace_container_path.as_deref(),
                    )
                    .await
                {
                    Ok(status) => status,
                    Err(error) => {
                        warn!(run_id, session_id, error = ?error, "failed to probe OpenCode retry status");
                        continue;
                    }
                };
                let retry_summary = match status {
                    Some(SessionStatusKind::Retry { message }) if !message.trim().is_empty() => {
                        Some(sanitize_error(&message))
                    }
                    Some(SessionStatusKind::Retry { .. } | SessionStatusKind::Idle | SessionStatusKind::Busy) | None => None,
                };
                if let Err(error) = store::set_run_error_summary(pool, run_id, retry_summary.as_deref()).await {
                    warn!(run_id, error = ?error, "failed to publish OpenCode retry status");
                }
            }
        }
    };

    match dispatch_result {
        Ok(Ok(result)) => {
            if finalize_success {
                if !store::mark_run_succeeded(pool, run_id, Some(&result.backend_run_ref)).await? {
                    return Ok(DispatchOutcome::Cancelled);
                }
            } else if !store::get_run(pool, run_id)
                .await?
                .is_some_and(|run| run.status == crate::harness::model::RUN_STATUS_RUNNING)
            {
                return Ok(DispatchOutcome::Cancelled);
            }
            Ok(DispatchOutcome::Succeeded {
                backend_run_ref: result.backend_run_ref,
            })
        }
        Ok(Err(error)) => {
            let summary = sanitize_error(&format!("{error:#}"));
            warn!(run_id, error = %summary, "harness run failed");
            let _ = store::mark_run_failed(pool, run_id, &summary, None).await;
            Ok(DispatchOutcome::Failed { summary })
        }
        Err(_elapsed) => {
            // The backend future was dropped without producing a
            // `DispatchResult`. The backend may already have persisted
            // a `backend_run_ref` (session id) into the run row before
            // the command itself blocked. Load it so we can confirm
            // cancellation before terminalizing.
            let session_id = store::get_run(pool, run_id)
                .await?
                .and_then(|row| row.backend_run_ref);

            let Some(session_id) = session_id else {
                // No session id was persisted: the dispatch was stuck
                // during session creation. Safe to mark failed.
                let summary = format!("run exceeded timeout of {timeout_seconds}s");
                warn!(
                    run_id,
                    summary = %summary,
                    "harness run timed out before session was created"
                );
                let _ = store::mark_run_failed(pool, run_id, &summary, None).await;
                return Ok(DispatchOutcome::Failed { summary });
            };

            match confirm_session_terminated(
                &backend,
                &opencode_base_url,
                &session_id,
                workspace_container_path.as_deref(),
            )
            .await
            {
                Ok(TerminationOutcome::AlreadyTerminal { provider_error }) => {
                    let summary = timeout_failure_summary(
                        timeout_seconds,
                        "OpenCode session already terminal",
                        provider_error.as_deref(),
                    );
                    warn!(run_id, summary = %summary, "harness run timed out; session terminal");
                    let _ = store::mark_run_failed(pool, run_id, &summary, None).await;
                    Ok(DispatchOutcome::Failed { summary })
                }
                Ok(TerminationOutcome::Aborted { provider_error }) => {
                    let summary = timeout_failure_summary(
                        timeout_seconds,
                        "OpenCode session aborted",
                        provider_error.as_deref(),
                    );
                    warn!(run_id, summary = %summary, "harness run timed out; session aborted");
                    let _ = store::mark_run_failed(pool, run_id, &summary, None).await;
                    Ok(DispatchOutcome::Failed { summary })
                }
                Ok(TerminationOutcome::StillActive) => {
                    warn!(
                        run_id,
                        session_id = %session_id,
                        "could not terminate OpenCode session after \
                         timeout; leaving run running for recovery"
                    );
                    Err(anyhow!(
                        "timeout cancellation unconfirmed for session {session_id}"
                    ))
                }
                Err(error) => {
                    warn!(
                        run_id,
                        session_id = %session_id,
                        error = ?error,
                        "session termination probe failed; leaving run \
                         running for recovery"
                    );
                    Err(error)
                }
            }
        }
    }
}

#[derive(Debug)]
enum TerminationOutcome {
    AlreadyTerminal { provider_error: Option<String> },
    Aborted { provider_error: Option<String> },
    StillActive,
}

const POST_ABORT_PROBE_ATTEMPTS: usize = 3;
const POST_ABORT_PROBE_DELAY: std::time::Duration = std::time::Duration::from_secs(1);

/// Probe `get_session_status`. If the session is already `Idle` (or
/// the server does not know about it), return `AlreadyTerminal`.
/// Otherwise call `abort_session`; if the server accepts the abort,
/// probe the status a handful of times to wait for the session to
/// drain. If the abort is reported unsupported or the session remains
/// `Busy`/`Retry` afterwards, return `StillActive`.
async fn confirm_session_terminated(
    backend: &Arc<dyn HarnessBackend>,
    base_url: &str,
    session_id: &str,
    workspace_container_path: Option<&str>,
) -> Result<TerminationOutcome> {
    let initial = backend
        .get_session_status_in_directory(base_url, session_id, workspace_container_path)
        .await?;
    let provider_error = provider_retry_message(initial.as_ref());
    if !initial.is_some_and(|status| status.is_active()) {
        // Either Idle, None (unknown), or no status known. All are
        // treated as terminal for our purposes.
        return Ok(TerminationOutcome::AlreadyTerminal { provider_error });
    }

    let aborted = backend.abort_session(base_url, session_id).await?;
    if !aborted {
        return Ok(TerminationOutcome::StillActive);
    }

    for _ in 0..POST_ABORT_PROBE_ATTEMPTS {
        tokio::time::sleep(POST_ABORT_PROBE_DELAY).await;
        let status = backend
            .get_session_status_in_directory(base_url, session_id, workspace_container_path)
            .await?;
        if !status.is_some_and(|status| status.is_active()) {
            return Ok(TerminationOutcome::Aborted { provider_error });
        }
    }
    Ok(TerminationOutcome::StillActive)
}

/// Abort a session when needed and confirm it is no longer executing.
///
/// Callers must not remove the workspace or its database records when this
/// returns `false`, because OpenCode may still be using them.
pub(crate) async fn abort_and_confirm_session_terminated(
    backend: &Arc<dyn HarnessBackend>,
    base_url: &str,
    session_id: &str,
    workspace_container_path: Option<&str>,
) -> Result<bool> {
    Ok(!matches!(
        confirm_session_terminated(backend, base_url, session_id, workspace_container_path,)
            .await?,
        TerminationOutcome::StillActive
    ))
}

fn provider_retry_message(status: Option<&SessionStatusKind>) -> Option<String> {
    match status {
        Some(SessionStatusKind::Retry { message }) if !message.trim().is_empty() => {
            Some(sanitize_error(message))
        }
        Some(
            SessionStatusKind::Idle | SessionStatusKind::Busy | SessionStatusKind::Retry { .. },
        )
        | None => None,
    }
}

fn timeout_failure_summary(
    timeout_seconds: i32,
    session_outcome: &str,
    provider_error: Option<&str>,
) -> String {
    let timeout_summary = format!("run exceeded timeout of {timeout_seconds}s; {session_outcome}");
    match provider_error {
        Some(message) => format!("OpenCode provider failure: {message}; {timeout_summary}"),
        None => timeout_summary,
    }
}

#[derive(Debug, Clone)]
pub enum DispatchOutcome {
    Succeeded { backend_run_ref: String },
    Cancelled,
    Failed { summary: String },
}

fn sanitize_error(input: &str) -> String {
    let mut out = String::with_capacity(input.len().min(ERROR_SUMMARY_MAX_CHARS));
    for ch in input.chars() {
        if ch.is_control() {
            out.push(' ');
        } else {
            out.push(ch);
        }
        if out.chars().count() >= ERROR_SUMMARY_MAX_CHARS {
            out.push('…');
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Mutex;

    use async_trait::async_trait;

    struct RecordingBackend {
        calls: Mutex<Vec<DispatchRequest>>,
        backend_ref: &'static str,
    }

    #[test]
    fn build_command_model_uses_provider_and_model_slash_format() {
        assert_eq!(
            build_command_model(Some("anthropic"), Some("claude-sonnet-4")),
            Some("anthropic/claude-sonnet-4".to_string())
        );
    }

    #[test]
    fn build_command_model_requires_both_fields() {
        assert_eq!(build_command_model(Some("anthropic"), None), None);
        assert_eq!(build_command_model(None, Some("claude-sonnet-4")), None);
    }

    #[async_trait]
    impl HarnessBackend for RecordingBackend {
        async fn dispatch(&self, request: DispatchRequest) -> Result<DispatchResult> {
            self.calls.lock().unwrap().push(request);
            Ok(DispatchResult {
                backend_run_ref: self.backend_ref.to_string(),
            })
        }
    }

    struct FailingBackend;

    #[async_trait]
    impl HarnessBackend for FailingBackend {
        async fn dispatch(&self, _request: DispatchRequest) -> Result<DispatchResult> {
            Err(anyhow!("simulated dispatch failure"))
        }
    }

    struct CancellingBackend {
        pool: DbPool,
    }

    #[async_trait]
    impl HarnessBackend for CancellingBackend {
        async fn dispatch(&self, request: DispatchRequest) -> Result<DispatchResult> {
            store::mark_run_aborted(&self.pool, request.run_id, "cancelled by operator", None)
                .await?;
            Ok(DispatchResult {
                backend_run_ref: "ses_cancelled".to_string(),
            })
        }
    }

    fn make_request() -> DispatchRequest {
        DispatchRequest {
            run_id: 1,
            sub_agent_id: 2,
            agent_key: "btc-2".to_string(),
            display_name: "BTC 2".to_string(),
            sub_agent_key: "analysis-15m".to_string(),
            sub_agent_kind: SUB_AGENT_KIND_ANALYSIS.to_string(),
            timeframe: Some("15m".to_string()),
            operator_prompt: String::new(),
            strategy_prompt: "Analyze trends.".to_string(),
            accumulated_learnings: None,
            system_prompt: "You are a crypto trading assistant.".to_string(),
            environment: "live".to_string(),
            selected_instruments: Vec::new(),
            account_snapshot: None,
            model_provider_id: None,
            model_id: None,
            model_variant: None,
            timeout_seconds: 10,
            opencode_base_url: "http://localhost:14096".to_string(),
            runtime_config: serde_json::json!({
                "workspace_host_path": "workspaces/agents/btc-2",
                "workspace_container_path": "/workspaces/agents/btc-2",
                "profile_source": "agent-runtime/workspace-template"
            }),
            scheduled_for: Utc::now(),
            review_window_start: None,
            review_window_end: None,
        }
    }

    #[test]
    fn resolve_opencode_sub_agent_maps_kinds_to_agent_and_command() {
        assert_eq!(
            resolve_opencode_sub_agent(SUB_AGENT_KIND_ANALYSIS).unwrap(),
            (DEFAULT_ANALYSIS_AGENT, DEFAULT_ANALYSIS_COMMAND)
        );
        assert_eq!(
            resolve_opencode_sub_agent(SUB_AGENT_KIND_TRADING).unwrap(),
            (DEFAULT_TRADING_AGENT, DEFAULT_TRADING_COMMAND)
        );
        assert_eq!(
            resolve_opencode_sub_agent(SUB_AGENT_KIND_MARKET_ANALYSIS).unwrap(),
            (
                DEFAULT_MARKET_ANALYSIS_AGENT,
                DEFAULT_MARKET_ANALYSIS_COMMAND,
            )
        );
        assert_eq!(
            resolve_opencode_sub_agent(SUB_AGENT_KIND_ANALYSIS_CODING).unwrap(),
            (
                DEFAULT_ANALYSIS_CODING_AGENT,
                DEFAULT_ANALYSIS_CODING_COMMAND,
            )
        );
        assert!(resolve_opencode_sub_agent("unknown").is_err());
    }

    #[test]
    fn command_arguments_contain_strategy_and_agent_details() {
        let request = make_request();
        let args = build_command_arguments(&request).expect("build args");
        assert!(args.contains("Agent key: btc-2"));
        assert!(args.contains("## Analysis strategy"));
        assert!(args.contains("Analyze trends."));
        assert!(args.contains("## Instructions"));
        assert!(args.contains("(none)"));
        // Ensure no api-key-like token is present
        assert!(!args.contains("HYPERVIBES_API_KEY"));
        assert!(!args.contains("vta_"));
    }

    #[test]
    fn sanitize_error_truncates_and_strips_control_chars() {
        let huge = "x".repeat(1000);
        let sanitized = sanitize_error(&huge);
        assert!(sanitized.chars().count() <= ERROR_SUMMARY_MAX_CHARS + 1);
        assert!(sanitized.ends_with('…'));
    }

    #[test]
    fn sanitize_error_replaces_control_chars() {
        let result = sanitize_error("hello\nworld\t!`\u{0007}`");
        assert!(!result.contains('\n'));
        assert!(!result.contains('\t'));
        assert!(!result.contains('\u{0007}'));
        assert!(result.contains("hello"));
        assert!(result.contains("world"));
    }

    #[tokio::test]
    async fn dispatch_with_timeout_marks_succeeded_when_backend_ok() {
        let pool = crate::test_db::pool().await;
        let (sub_agent_id, run_id, agent_key) =
            seed_run_for_timeout_test(&pool, "backend-ok").await;

        let backend = std::sync::Arc::new(RecordingBackend {
            calls: Mutex::new(Vec::new()),
            backend_ref: "ses_test",
        });
        let mut request = make_request();
        request.sub_agent_id = sub_agent_id;
        request.run_id = run_id;
        request.agent_key = agent_key;
        let outcome = dispatch_with_timeout(&pool, backend.clone(), request)
            .await
            .expect("dispatch");
        match outcome {
            DispatchOutcome::Succeeded { backend_run_ref } => {
                assert_eq!(backend_run_ref, "ses_test");
            }
            other => panic!("expected Succeeded, got {other:?}"),
        }
        let calls = backend.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
    }

    #[tokio::test]
    async fn dispatch_with_timeout_marks_failed_when_backend_errors() {
        let pool = crate::test_db::pool().await;
        let backend: Arc<dyn HarnessBackend> = Arc::new(FailingBackend);
        let request = make_request();
        let outcome = dispatch_with_timeout(&pool, backend, request)
            .await
            .expect("dispatch");
        match outcome {
            DispatchOutcome::Failed { summary } => {
                assert!(summary.contains("simulated dispatch failure"));
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn dispatch_with_timeout_does_not_succeed_after_cancellation() {
        let pool = crate::test_db::pool().await;
        let (sub_agent_id, run_id, agent_key) =
            seed_run_for_timeout_test(&pool, "cancelled-dispatch").await;
        let backend: Arc<dyn HarnessBackend> = Arc::new(CancellingBackend { pool: pool.clone() });
        let mut request = make_request();
        request.sub_agent_id = sub_agent_id;
        request.run_id = run_id;
        request.agent_key = agent_key;

        let outcome = dispatch_with_timeout(&pool, backend, request)
            .await
            .expect("dispatch returned outcome");

        assert!(matches!(outcome, DispatchOutcome::Cancelled));
        let run = store::get_run(&pool, run_id)
            .await
            .expect("fetch run")
            .expect("run exists");
        assert_eq!(run.status, crate::harness::model::RUN_STATUS_ABORTED);
    }

    /// Fake backend that simulates an OpenCode dispatch that:
    /// 1. persists a `backend_run_ref` immediately (so the run row has
    ///    a session id before the command blocks), then
    /// 2. blocks forever until the dispatch future is cancelled by the
    ///    timeout algorithm.
    ///
    /// Its `abort_session` and `get_session_status` overrides let each
    /// test fixture drive the cancellation outcome deterministically
    /// without touching a real OpenCode server.
    struct BlockingBackend {
        pool: DbPool,
        session_id: String,
        initial_status: SessionStatusKind,
        post_abort_status: SessionStatusKind,
        abort_returns: bool,
        abort_calls: Arc<std::sync::Mutex<Vec<String>>>,
        status_calls: Arc<std::sync::Mutex<usize>>,
    }

    #[async_trait]
    impl HarnessBackend for BlockingBackend {
        async fn dispatch(&self, request: DispatchRequest) -> Result<DispatchResult> {
            let _ =
                store::mark_run_running(&self.pool, request.run_id, Some(&self.session_id)).await;
            std::future::pending::<()>().await;
            unreachable!("dispatch should never return; timeout cancels it");
        }

        async fn abort_session(&self, _base_url: &str, session_id: &str) -> Result<bool> {
            self.abort_calls
                .lock()
                .expect("lock abort calls")
                .push(session_id.to_string());
            Ok(self.abort_returns)
        }

        async fn get_session_status(
            &self,
            _base_url: &str,
            session_id: &str,
        ) -> Result<Option<SessionStatusKind>> {
            *self.status_calls.lock().expect("lock status calls") += 1;
            assert_eq!(session_id, self.session_id);
            let aborts = self.abort_calls.lock().expect("count aborts").len();
            Ok(Some(if aborts > 0 {
                self.post_abort_status.clone()
            } else {
                self.initial_status.clone()
            }))
        }
    }

    /// Helper: seed an agent + job + queued run row, returning both
    /// the job id and the run row's id plus an `agent_key` for
    /// verification.
    async fn seed_run_for_timeout_test(pool: &DbPool, key: &str) -> (i64, i64, String) {
        use crate::agents::{
            keys::derive_wallet_address, model::AgentRegistryRow, store::insert_agent,
        };
        use crate::harness::store::insert_default_harness_sub_agents;
        // Seed a deterministic private key.
        fn deterministic_private_key(key: &str) -> String {
            use rand::rngs::StdRng;
            use rand::{RngExt, SeedableRng};
            let seed = key
                .bytes()
                .fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
            let mut rng = StdRng::seed_from_u64(seed);
            let bytes: [u8; 32] = rng.random();
            format!("0x{}", hex::encode(bytes))
        }
        let private_key = deterministic_private_key(key);
        let wallet = derive_wallet_address(&private_key).expect("wallet");
        let now = Utc::now();
        insert_agent(
            pool,
            &AgentRegistryRow {
                agent_key: key.to_string(),
                user_id: crate::test_db::test_user_id(),
                created_at: now,
                updated_at: now,
                enabled: true,
                lifecycle: crate::agents::model::AGENT_LIFECYCLE_ACTIVE.to_string(),
                display_name: format!("Test {key}"),
                trading_account_address: Some(wallet),
                environment: "live".to_string(),
                api_key: format!("vta_{key}"),
                api_key_last_used_at: None,
                runtime_config: serde_json::json!({
                    "workspace_host_path": format!("workspaces/agents/{key}"),
                    "workspace_container_path": format!("/workspaces/agents/{key}"),
                    "profile_source": "agent-runtime/workspace-template",
                }),
            },
        )
        .await
        .expect("insert agent");
        insert_default_harness_sub_agents(pool, key)
            .await
            .expect("insert default jobs");

        let (sub_agent_id,): (i64,) = sqlx::query_as(
            "SELECT id FROM harness_sub_agents
              WHERE agent_key = $1 AND sub_agent_key = 'analysis-15m'",
        )
        .bind(key)
        .fetch_one(pool)
        .await
        .expect("fetch job id");

        // Insert a queued run row directly so dispatch_with_timeout has
        // something to load and persist into.
        let (run_id,): (i64,) = sqlx::query_as(
            "INSERT INTO harness_sub_agent_runs (
                 sub_agent_id, agent_key, sub_agent_key, sub_agent_kind, timeframe,
                  status, scheduled_for, timeout_seconds
             ) VALUES ($1, $2, 'analysis-15m', 'analysis', '15m',
                        'queued', now(), $3)
             RETURNING id",
        )
        .bind(sub_agent_id)
        .bind(key)
        .bind(2_i32) // 2 second timeout for the test
        .fetch_one(pool)
        .await
        .expect("insert queued run");

        (sub_agent_id, run_id, key.to_string())
    }

    #[tokio::test]
    async fn dispatch_with_timeout_aborts_when_session_already_terminal() {
        let pool = crate::test_db::pool().await;
        let key = format!(
            "abort-terminal-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        let (sub_agent_id, run_id, agent_key) = seed_run_for_timeout_test(&pool, &key).await;
        let session_id = format!("ses_terminal_{}", run_id);

        let backend = Arc::new(BlockingBackend {
            pool: pool.clone(),
            session_id: session_id.clone(),
            initial_status: SessionStatusKind::Idle,
            post_abort_status: SessionStatusKind::Idle,
            abort_returns: true,
            abort_calls: Arc::new(std::sync::Mutex::new(Vec::new())),
            status_calls: Arc::new(std::sync::Mutex::new(0)),
        });
        let backend_arc: Arc<dyn HarnessBackend> = backend.clone();

        let request = DispatchRequest {
            run_id,
            sub_agent_id,
            agent_key,
            display_name: key.clone(),
            sub_agent_key: "analysis-15m".to_string(),
            sub_agent_kind: SUB_AGENT_KIND_ANALYSIS.to_string(),
            timeframe: Some("15m".to_string()),
            operator_prompt: String::new(),
            strategy_prompt: String::new(),
            accumulated_learnings: None,
            system_prompt: String::new(),
            environment: "live".to_string(),
            selected_instruments: Vec::new(),
            account_snapshot: None,
            model_provider_id: None,
            model_id: None,
            model_variant: None,
            timeout_seconds: 1,
            opencode_base_url: "http://localhost:14096".to_string(),
            runtime_config: serde_json::json!({}),
            scheduled_for: Utc::now(),
            review_window_start: None,
            review_window_end: None,
        };

        let outcome = dispatch_with_timeout(&pool, backend_arc, request)
            .await
            .expect("dispatch returned outcome");

        match outcome {
            DispatchOutcome::Failed { summary } => {
                assert!(
                    summary.contains("already terminal"),
                    "expected 'already terminal' summary, got {summary:?}"
                );
            }
            other => panic!("expected Failed, got {other:?}"),
        }

        let run = store::get_run(&pool, run_id)
            .await
            .expect("fetch run")
            .expect("run row");
        assert_eq!(run.status, crate::harness::model::RUN_STATUS_FAILED);
        assert_eq!(run.backend_run_ref.as_deref(), Some(session_id.as_str()));
        // The session was already idle so no abort should be issued.
        assert!(
            backend.abort_calls.lock().expect("abort calls").is_empty(),
            "abort should not be called for already-terminal session"
        );
    }

    #[tokio::test]
    async fn dispatch_with_timeout_aborts_active_session_and_marks_failed() {
        let pool = crate::test_db::pool().await;
        let key = format!("abort-ok-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0));
        let (sub_agent_id, run_id, agent_key) = seed_run_for_timeout_test(&pool, &key).await;
        let session_id = format!("ses_abort_{}", run_id);

        let backend = Arc::new(BlockingBackend {
            pool: pool.clone(),
            session_id: session_id.clone(),
            initial_status: SessionStatusKind::Retry {
                message: "You exceeded your current quota".to_string(),
            },
            post_abort_status: SessionStatusKind::Idle,
            abort_returns: true,
            abort_calls: Arc::new(std::sync::Mutex::new(Vec::new())),
            status_calls: Arc::new(std::sync::Mutex::new(0)),
        });
        let backend_arc: Arc<dyn HarnessBackend> = backend.clone();

        let request = DispatchRequest {
            run_id,
            sub_agent_id,
            agent_key,
            display_name: key.clone(),
            sub_agent_key: "analysis-15m".to_string(),
            sub_agent_kind: SUB_AGENT_KIND_ANALYSIS.to_string(),
            timeframe: Some("15m".to_string()),
            operator_prompt: String::new(),
            strategy_prompt: String::new(),
            accumulated_learnings: None,
            system_prompt: String::new(),
            environment: "live".to_string(),
            selected_instruments: Vec::new(),
            account_snapshot: None,
            model_provider_id: None,
            model_id: None,
            model_variant: None,
            timeout_seconds: 1,
            opencode_base_url: "http://localhost:14096".to_string(),
            runtime_config: serde_json::json!({}),
            scheduled_for: Utc::now(),
            review_window_start: None,
            review_window_end: None,
        };

        let outcome = dispatch_with_timeout(&pool, backend_arc, request)
            .await
            .expect("dispatch returned outcome");

        match outcome {
            DispatchOutcome::Failed { summary } => {
                assert!(
                    summary.contains("You exceeded your current quota"),
                    "expected provider error summary, got {summary:?}"
                );
            }
            other => panic!("expected Failed, got {other:?}"),
        }

        assert_eq!(
            backend.abort_calls.lock().expect("abort calls").len(),
            1,
            "abort should fire exactly once"
        );
        let run = store::get_run(&pool, run_id)
            .await
            .expect("fetch run")
            .expect("run row");
        assert_eq!(run.status, crate::harness::model::RUN_STATUS_FAILED);
        assert!(
            run.error_summary
                .as_deref()
                .is_some_and(|summary| summary.contains("You exceeded your current quota")),
            "provider error should be persisted: {:?}",
            run.error_summary
        );
    }

    #[tokio::test]
    async fn dispatch_publishes_provider_retry_message_while_run_is_active() {
        let pool = crate::test_db::pool().await;
        let key = format!(
            "retry-status-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        let (sub_agent_id, run_id, agent_key) = seed_run_for_timeout_test(&pool, &key).await;
        let session_id = format!("ses_retry_{}", run_id);
        let backend = Arc::new(BlockingBackend {
            pool: pool.clone(),
            session_id,
            initial_status: SessionStatusKind::Retry {
                message: "Your Ollama Cloud usage is exhausted".to_string(),
            },
            post_abort_status: SessionStatusKind::Idle,
            abort_returns: true,
            abort_calls: Arc::new(std::sync::Mutex::new(Vec::new())),
            status_calls: Arc::new(std::sync::Mutex::new(0)),
        });
        let request = DispatchRequest {
            run_id,
            sub_agent_id,
            agent_key,
            display_name: key.clone(),
            sub_agent_key: "analysis-15m".to_string(),
            sub_agent_kind: SUB_AGENT_KIND_ANALYSIS.to_string(),
            timeframe: Some("15m".to_string()),
            operator_prompt: String::new(),
            strategy_prompt: String::new(),
            accumulated_learnings: None,
            system_prompt: String::new(),
            environment: "live".to_string(),
            selected_instruments: Vec::new(),
            account_snapshot: None,
            model_provider_id: None,
            model_id: None,
            model_variant: None,
            timeout_seconds: 2,
            opencode_base_url: "http://localhost:14096".to_string(),
            runtime_config: serde_json::json!({}),
            scheduled_for: Utc::now(),
            review_window_start: None,
            review_window_end: None,
        };
        let dispatch_pool = pool.clone();
        let dispatch =
            tokio::spawn(
                async move { dispatch_with_timeout(&dispatch_pool, backend, request).await },
            );

        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
        loop {
            let run = store::get_run(&pool, run_id)
                .await
                .expect("fetch run")
                .expect("run row");
            if run.error_summary.as_deref() == Some("Your Ollama Cloud usage is exhausted") {
                assert_eq!(run.status, crate::harness::model::RUN_STATUS_RUNNING);
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "provider retry message was not published: {:?}",
                run.error_summary
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        dispatch.abort();
    }

    #[tokio::test]
    async fn dispatch_with_timeout_keeps_run_running_when_abort_unconfirmed() {
        let pool = crate::test_db::pool().await;
        let key = format!(
            "abort-fail-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        let (sub_agent_id, run_id, agent_key) = seed_run_for_timeout_test(&pool, &key).await;
        let session_id = format!("ses_active_{}", run_id);

        let backend = Arc::new(BlockingBackend {
            pool: pool.clone(),
            session_id: session_id.clone(),
            initial_status: SessionStatusKind::Busy,
            // After "abort" the session stays busy (abort signals
            // unsupported or refused) -- the post-abort probe loop
            // should still report Busy and dispatch_with_timeout must
            // leave the run running.
            post_abort_status: SessionStatusKind::Busy,
            abort_returns: false,
            abort_calls: Arc::new(std::sync::Mutex::new(Vec::new())),
            status_calls: Arc::new(std::sync::Mutex::new(0)),
        });
        let backend_arc: Arc<dyn HarnessBackend> = backend.clone();

        let request = DispatchRequest {
            run_id,
            sub_agent_id,
            agent_key,
            display_name: key.clone(),
            sub_agent_key: "analysis-15m".to_string(),
            sub_agent_kind: SUB_AGENT_KIND_ANALYSIS.to_string(),
            timeframe: Some("15m".to_string()),
            operator_prompt: String::new(),
            strategy_prompt: String::new(),
            accumulated_learnings: None,
            system_prompt: String::new(),
            environment: "live".to_string(),
            selected_instruments: Vec::new(),
            account_snapshot: None,
            model_provider_id: None,
            model_id: None,
            model_variant: None,
            timeout_seconds: 1,
            opencode_base_url: "http://localhost:14096".to_string(),
            runtime_config: serde_json::json!({}),
            scheduled_for: Utc::now(),
            review_window_start: None,
            review_window_end: None,
        };

        let outcome = dispatch_with_timeout(&pool, backend_arc, request).await;

        assert!(
            outcome.is_err(),
            "expected dispatch_with_timeout to surface Err when abort cannot be confirmed, got {outcome:?}"
        );

        let run = store::get_run(&pool, run_id)
            .await
            .expect("fetch run")
            .expect("run row");
        assert_eq!(
            run.status,
            crate::harness::model::RUN_STATUS_RUNNING,
            "run should remain running while session cancellation is unconfirmed"
        );
    }
}
