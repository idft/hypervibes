use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;
use tracing::{info, warn};

use crate::{
    agentic::{
        model::{JOB_KIND_ANALYSIS, JOB_KIND_TRADING},
        store,
    },
    db::DbPool,
    opencode::{
        client::{OpenCodeClient, OpenCodeCommandRequest},
        workspace::OpenCodeWorkspaceRuntimeConfig,
    },
};

const ERROR_SUMMARY_MAX_CHARS: usize = 500;
const DEFAULT_ANALYSIS_AGENT: &str = "analysis";
const DEFAULT_ANALYSIS_COMMAND: &str = "vibetrading-analysis";
const DEFAULT_TRADING_AGENT: &str = "trading";
const DEFAULT_TRADING_COMMAND: &str = "vibetrading-trading";

#[derive(Debug, Clone)]
pub struct DispatchRequest {
    pub run_id: i64,
    pub schedule_id: i64,
    pub agent_key: String,
    pub display_name: String,
    pub job_key: String,
    pub job_kind: String,
    pub timeframe: String,
    pub operator_prompt: String,
    pub analysis_prompt: String,
    pub trading_prompt: String,
    pub system_prompt: String,
    pub environment: String,
    pub selected_instruments: Vec<String>,
    pub account_snapshot: Option<crate::hyperliquid::live_state::LiveAgentSnapshot>,
    pub model_provider_id: Option<String>,
    pub model_id: Option<String>,
    pub timeout_seconds: i32,
    pub runtime_base_url: String,
    pub runtime_config: Value,
    pub scheduled_for: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct DispatchResult {
    pub backend_run_ref: String,
}

/// Trait implemented by anything that can execute a single scheduled
/// run. The production wiring uses `OpenCodeBackend`; tests can
/// substitute a fake.
#[async_trait]
pub trait AgenticBackend: Send + Sync {
    async fn dispatch(&self, request: DispatchRequest) -> Result<DispatchResult>;
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
impl AgenticBackend for OpenCodeBackend {
    async fn dispatch(&self, request: DispatchRequest) -> Result<DispatchResult> {
        let workspace = OpenCodeWorkspaceRuntimeConfig::from_value(&request.runtime_config)
            .ok_or_else(|| anyhow!("OpenCode workspace is not configured"))?;
        let workspace_container_path = workspace.workspace_container_path.clone();
        let workspace_host_path = workspace.workspace_host_path.clone();

        let (agent_name, command_name) = resolve_opencode_job(&request.job_kind)?;

        let title = format!(
            "{} {} {}",
            request.agent_key,
            request.job_key,
            request.scheduled_for.format("%Y-%m-%dT%H:%M:%SZ")
        );

        let session = self
            .client
            .create_session(
                &request.runtime_base_url,
                &workspace_container_path,
                Some(&title),
            )
            .await
            .with_context(|| {
                format!(
                    "failed to create OpenCode session for agent {} (workspace {})",
                    request.agent_key, workspace_host_path
                )
            })?;

        info!(
            run_id = request.run_id,
            schedule_id = request.schedule_id,
            agent_key = %request.agent_key,
            display_name = %request.display_name,
            job_key = %request.job_key,
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
        };

        self.client
            .run_command(&request.runtime_base_url, &session.id, command_request)
            .await
            .with_context(|| {
                format!(
                    "OpenCode run_command failed for agent {} session {}",
                    request.agent_key, session.id
                )
            })?;

        info!(
            run_id = request.run_id,
            schedule_id = request.schedule_id,
            agent_key = %request.agent_key,
            display_name = %request.display_name,
            job_key = %request.job_key,
            session_id = %session.id,
            "opencode command dispatched"
        );

        Ok(DispatchResult {
            backend_run_ref: session.id,
        })
    }
}

fn resolve_opencode_job(job_kind: &str) -> Result<(&'static str, &'static str)> {
    match job_kind {
        JOB_KIND_ANALYSIS => Ok((DEFAULT_ANALYSIS_AGENT, DEFAULT_ANALYSIS_COMMAND)),
        JOB_KIND_TRADING => Ok((DEFAULT_TRADING_AGENT, DEFAULT_TRADING_COMMAND)),
        other => Err(anyhow!("unknown job kind: {other}")),
    }
}

fn build_command_arguments(request: &DispatchRequest) -> Result<String> {
    crate::agentic::prompt::build_prompt(request)
}

fn build_command_model(model_provider_id: Option<&str>, model_id: Option<&str>) -> Option<String> {
    match (model_provider_id, model_id) {
        (Some(provider_id), Some(model_id)) => Some(format!("{provider_id}/{model_id}")),
        _ => None,
    }
}

/// Run a single dispatch through the backend, applying the schedule's
/// timeout. On timeout, the run is marked failed and a sanitized error
/// is returned.
pub async fn dispatch_with_timeout(
    pool: &crate::db::DbPool,
    backend: Arc<dyn AgenticBackend>,
    request: DispatchRequest,
) -> Result<DispatchOutcome> {
    let run_id = request.run_id;
    let timeout_seconds = request.timeout_seconds;
    let timeout = std::time::Duration::from_secs(timeout_seconds.max(0) as u64);

    let dispatch_result = tokio::time::timeout(timeout, backend.dispatch(request)).await;

    match dispatch_result {
        Ok(Ok(result)) => {
            let _ = store::mark_run_succeeded(pool, run_id, Some(&result.backend_run_ref)).await;
            Ok(DispatchOutcome::Succeeded {
                backend_run_ref: result.backend_run_ref,
            })
        }
        Ok(Err(error)) => {
            let summary = sanitize_error(&format!("{error:#}"));
            warn!(run_id, error = %summary, "agentic run failed");
            let _ = store::mark_run_failed(pool, run_id, &summary, None).await;
            Ok(DispatchOutcome::Failed { summary })
        }
        Err(_elapsed) => {
            let summary = format!("run exceeded timeout of {}s", timeout_seconds);
            warn!(run_id, summary = %summary, "agentic run timed out");
            let _ = store::mark_run_failed(pool, run_id, &summary, None).await;
            Ok(DispatchOutcome::Failed { summary })
        }
    }
}

#[derive(Debug, Clone)]
pub enum DispatchOutcome {
    Succeeded { backend_run_ref: String },
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
    impl AgenticBackend for RecordingBackend {
        async fn dispatch(&self, request: DispatchRequest) -> Result<DispatchResult> {
            self.calls.lock().unwrap().push(request);
            Ok(DispatchResult {
                backend_run_ref: self.backend_ref.to_string(),
            })
        }
    }

    struct FailingBackend;

    #[async_trait]
    impl AgenticBackend for FailingBackend {
        async fn dispatch(&self, _request: DispatchRequest) -> Result<DispatchResult> {
            Err(anyhow!("simulated dispatch failure"))
        }
    }

    fn make_request() -> DispatchRequest {
        DispatchRequest {
            run_id: 1,
            schedule_id: 2,
            agent_key: "btc-2".to_string(),
            display_name: "BTC 2".to_string(),
            job_key: "analysis-15m".to_string(),
            job_kind: JOB_KIND_ANALYSIS.to_string(),
            timeframe: "15m".to_string(),
            operator_prompt: String::new(),
            analysis_prompt: "Analyze trends.".to_string(),
            trading_prompt: "Trade breakouts.".to_string(),
            system_prompt: "You are a crypto trading assistant.".to_string(),
            environment: "live".to_string(),
            selected_instruments: Vec::new(),
            account_snapshot: None,
            model_provider_id: None,
            model_id: None,
            timeout_seconds: 10,
            runtime_base_url: "http://localhost:14096".to_string(),
            runtime_config: serde_json::json!({
                "workspace_host_path": "workspaces/agents/btc-2",
                "workspace_container_path": "/workspaces/agents/btc-2",
                "profile_source": "agent-runtime/workspace-template"
            }),
            scheduled_for: Utc::now(),
        }
    }

    #[test]
    fn resolve_opencode_job_maps_kinds_to_agent_and_command() {
        assert_eq!(
            resolve_opencode_job(JOB_KIND_ANALYSIS).unwrap(),
            (DEFAULT_ANALYSIS_AGENT, DEFAULT_ANALYSIS_COMMAND)
        );
        assert_eq!(
            resolve_opencode_job(JOB_KIND_TRADING).unwrap(),
            (DEFAULT_TRADING_AGENT, DEFAULT_TRADING_COMMAND)
        );
        assert!(resolve_opencode_job("unknown").is_err());
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
        assert!(!args.contains("VIBETRADING_API_KEY"));
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
        // Insert a minimal agent + schedule + queued run, then dispatch.
        let key = format!(
            "backend-ok-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        crate::agentic::store::insert_default_opencode_schedules(&pool, &key)
            .await
            .ok();

        // We can't create an agent without going through the full insert path,
        // so the integration test for `mark_run_succeeded` is in the store tests.
        // Here we just verify that the helper returns the expected outcome.
        let backend = std::sync::Arc::new(RecordingBackend {
            calls: Mutex::new(Vec::new()),
            backend_ref: "ses_test",
        });
        let request = make_request();
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
        let backend: Arc<dyn AgenticBackend> = Arc::new(FailingBackend);
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
}
