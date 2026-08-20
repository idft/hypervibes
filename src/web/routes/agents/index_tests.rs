//! Tests for agents/index_tests.rs
use crate::web::routes::router;
use crate::web::routes::test_support::*;

use anyhow::Result;
use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::Utc;
use std::fs;
use std::sync::Arc;
use tower::util::ServiceExt;

use crate::{
    agents::{
        model::slugify_agent_key,
        store::{get_agent, list_agent_instrument_ids},
    },
    harness::backend::{DispatchRequest, DispatchResult, HarnessBackend},
    hyperliquid::live_state::{AccountKey, AccountLiveState, LiveConnectionStatus},
    opencode::client::SessionStatusKind,
    opencode::workspace::delete_agent_workspace,
};

struct RefusingDeleteAbortBackend;

#[async_trait]
impl HarnessBackend for RefusingDeleteAbortBackend {
    async fn dispatch(&self, _request: DispatchRequest) -> Result<DispatchResult> {
        Ok(DispatchResult {
            backend_run_ref: "ses_unused".to_string(),
        })
    }

    async fn abort_session(&self, _base_url: &str, _session_id: &str) -> Result<bool> {
        Ok(false)
    }

    async fn get_session_status_in_directory(
        &self,
        _base_url: &str,
        _session_id: &str,
        _workspace_container_path: Option<&str>,
    ) -> Result<Option<SessionStatusKind>> {
        Ok(Some(SessionStatusKind::Busy))
    }
}

#[tokio::test]
async fn get_agents_renders_db_data() {
    let state = test_state().await;

    let app = router(state.clone());
    let response = app
        .oneshot(
            Request::builder()
                .uri("/agents")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn get_agents_renders_unavailable_agent_notice() {
    let state = test_state().await;

    let response = router(state)
        .oneshot(
            Request::builder()
                .uri("/agents?notice=agent-unavailable")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response_text(response)
            .await
            .contains("The requested page is not available.")
    );
}

#[tokio::test]
async fn wallet_route_is_not_registered() {
    let state = test_state().await;
    let response = router(state)
        .oneshot(
            Request::builder()
                .uri("/wallet")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
#[tokio::test]
async fn post_agents_requires_a_name() {
    let state = test_state().await;

    let app = router(state.clone());
    let body = "display_name=";
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/agents")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn agents_new_page_renders_simplified_form() {
    let state = test_state().await;

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/agents/new")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let text = response_text(response).await;
    assert!(!text.contains("Runtime instance"));
    assert!(!text.contains("name=\"enabled\""));
    assert!(!text.contains("Generate new Agent wallet"));
    assert!(!text.contains("Import existing Private Key"));
}
#[tokio::test]
async fn post_user_subaccounts_requires_a_display_name() {
    let state = test_state().await;

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/account/subaccounts")
                .header("content-type", "application/json")
                .body(Body::from("{\"displayName\":\"\"}"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let text = response_text(response).await;
    assert!(text.contains("Enter an agent name first."));
}

#[tokio::test]
async fn post_user_subaccounts_rejects_unusable_display_name() {
    let state = test_state().await;

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/account/subaccounts")
                .header("content-type", "application/json")
                .body(Body::from("{\"displayName\":\"!!!\"}"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let text = response_text(response).await;
    assert!(text.contains("Enter an agent name first."));
}

#[tokio::test]
async fn post_user_subaccounts_requires_a_ready_signer() {
    let state = test_state().await;
    sqlx::query("UPDATE users SET api_wallet_approved_at = NULL WHERE id = $1")
        .bind(crate::test_db::test_user_id())
        .execute(&state.db_pool)
        .await
        .expect("clear test signer approval");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/account/subaccounts")
                .header("content-type", "application/json")
                .body(Body::from("{\"displayName\":\"BTC Momentum\"}"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let text = response_text(response).await;
    assert!(text.contains("trading signer needs attention"));
}

#[tokio::test]
async fn post_agents_creates_agent_active_with_default_prompts_and_jobs() {
    let state = test_state().await;
    seed_instrument(&state, "BTC", true).await;
    let guard = state
        ._test_db_guard
        .as_ref()
        .cloned()
        .expect("test db guard");

    let app = router(state.clone());
    let timestamp = chrono::Utc::now().timestamp_millis();
    let display_name = format!("SoulTest{}", timestamp);
    let body = format!("display_name={display_name}&trading_account_selection=main");

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/agents")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let agent_key = slugify_agent_key(&display_name);
    let agent = get_agent(&state.db_pool, &agent_key)
        .await
        .expect("load active agent")
        .expect("active agent exists");
    assert_eq!(
        agent.lifecycle,
        crate::agents::model::AGENT_LIFECYCLE_ACTIVE
    );
    assert_eq!(
        agent.trading_account_address.as_deref(),
        Some("0x0000000000000000000000000000000000000000")
    );
    assert_eq!(
        list_agent_instrument_ids(&state.db_pool, &agent_key)
            .await
            .expect("list default instruments"),
        vec!["BTC".to_string()]
    );
    assert!(
        !agent
            .runtime_config
            .as_object()
            .expect("runtime config object")
            .is_empty(),
        "expected activate_new_agent to populate runtime_config"
    );
    let prompt_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM agent_strategy_prompts WHERE agent_key = $1")
            .bind(&agent_key)
            .fetch_one(&state.db_pool)
            .await
            .expect("count prompts");
    assert!(
        prompt_count.0 > 0,
        "expected default strategy prompts to be inserted"
    );
    drop(guard);
}
#[tokio::test]
async fn post_delete_agent_removes_agent_and_redirects() {
    let state = test_state().await;
    let pool = state.db_pool.clone();

    let app = router(state.clone());
    let (agent_key, _) = insert_test_opencode_agent(&state)
        .await
        .expect("insert agent");
    generate_test_agent_workspace(&state, &agent_key).await;

    let stored = get_agent(&pool, &agent_key)
        .await
        .expect("get stored agent before delete")
        .expect("agent present before delete");
    let trading_account = stored
        .trading_account_address
        .as_deref()
        .expect("trading account");
    let live_account_key = AccountKey::new(trading_account, &stored.environment);
    let workspace_path = crate::opencode::workspace::agent_workspace_host_path(
        &state.opencode_workspace_config,
        &agent_key,
    )
    .expect("workspace path");
    assert!(workspace_path.exists());
    fs::write(
        workspace_path.join("scratch/delete-sentinel.txt"),
        "cleanup",
    )
    .expect("write delete sentinel");
    state.live_accounts.replace(
        live_account_key.clone(),
        AccountLiveState {
            account_address: trading_account.to_string(),
            environment: stored.environment.clone(),
            status: LiveConnectionStatus::Connected,
            clearinghouse_updated_at: Some(Utc::now()),
            open_orders_updated_at: Some(Utc::now()),
            spot_updated_at: Some(Utc::now()),
            ..Default::default()
        },
    );

    // Verify the agent exists.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{}", agent_key))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Delete the agent.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{}/delete", agent_key))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let location = response
        .headers()
        .get("location")
        .expect("redirect location header")
        .to_str()
        .unwrap();
    assert_eq!(location, "/agents");

    // Verify the agent is gone.
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{}", agent_key))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(!workspace_path.exists());
    assert!(state.live_accounts.get(&live_account_key).is_none());
}

#[tokio::test]
async fn post_delete_agent_aborts_queued_run_before_deleting() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let (agent_key, _) = insert_test_opencode_agent(&state)
        .await
        .expect("insert agent");
    generate_test_agent_workspace(&state, &agent_key).await;
    let job_id = crate::harness::store::list_agent_jobs(&pool, &agent_key)
        .await
        .expect("list jobs")
        .first()
        .expect("default job")
        .id;
    crate::harness::store::insert_test_run(&pool, job_id, "queued")
        .await
        .expect("insert queued run");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/delete"))
                .body(Body::empty())
                .expect("build delete request"),
        )
        .await
        .expect("delete agent");

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert!(
        get_agent(&pool, &agent_key)
            .await
            .expect("get deleted agent")
            .is_none()
    );
}

#[tokio::test]
async fn post_delete_agent_preserves_disabled_agent_when_run_cannot_abort() {
    let state = test_state_with_backend(Arc::new(RefusingDeleteAbortBackend)).await;
    let pool = state.db_pool.clone();
    let (agent_key, _) = insert_test_opencode_agent(&state)
        .await
        .expect("insert agent");
    generate_test_agent_workspace(&state, &agent_key).await;
    let job_id = crate::harness::store::list_agent_jobs(&pool, &agent_key)
        .await
        .expect("list jobs")
        .first()
        .expect("default job")
        .id;
    let run_id = crate::harness::store::insert_test_run(&pool, job_id, "running")
        .await
        .expect("insert running run");
    crate::harness::store::mark_run_running(&pool, run_id, Some("ses_still_active"))
        .await
        .expect("mark running");

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{agent_key}/delete"))
                .body(Body::empty())
                .expect("build delete request"),
        )
        .await
        .expect("delete agent");

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let agent = get_agent(&pool, &agent_key)
        .await
        .expect("get preserved agent")
        .expect("agent remains after failed deletion");
    assert!(
        !agent.enabled,
        "failed deletion must leave the agent disabled"
    );
}
#[tokio::test]
async fn post_agents_creates_the_opencode_workspace() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let app = router(state.clone());
    let timestamp = chrono::Utc::now().timestamp_millis();
    let display_name = format!("FreshWorkspace{}", timestamp);
    let agent_key = slugify_agent_key(&display_name);
    let workspace_path = state
        .opencode_workspace_config
        .host_workspaces_root
        .join("agents")
        .join(&agent_key);
    assert!(
        !workspace_path.exists(),
        "workspace must not exist pre-create"
    );

    let body = format!("display_name={display_name}&trading_account_selection=main");

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/agents")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert!(
        get_agent(&pool, &agent_key)
            .await
            .expect("get agent")
            .is_some()
    );
    assert!(
        workspace_path.exists(),
        "create_agent should generate the workspace"
    );
}

#[tokio::test]
async fn post_agents_does_not_persist_agent_when_workspace_already_exists() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let app = router(state.clone());
    let timestamp = chrono::Utc::now().timestamp_millis();
    let display_name = format!("ExistingWorkspace{}", timestamp);
    let agent_key = slugify_agent_key(&display_name);
    let workspace_path = state
        .opencode_workspace_config
        .host_workspaces_root
        .join("agents")
        .join(&agent_key);
    fs::create_dir_all(&workspace_path).expect("create existing workspace");
    let sentinel_path = workspace_path.join("sentinel.txt");
    fs::write(&sentinel_path, "keep").expect("write workspace sentinel");

    let body = format!("display_name={display_name}&trading_account_selection=main");
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/agents")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        get_agent(&pool, &agent_key)
            .await
            .expect("get failed agent")
            .is_none(),
        "agent row must not survive workspace creation failure"
    );
    assert!(
        sentinel_path.exists(),
        "existing workspace must be preserved"
    );

    delete_agent_workspace(&state.opencode_workspace_config, &agent_key)
        .expect("clean up existing workspace");
}
