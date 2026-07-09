//! Tests for agents/index_tests.rs
use crate::web::routes::router;
use crate::web::routes::test_support::*;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::Utc;
use std::fs;
use tower::util::ServiceExt;

use crate::{
    agents::{
        model::CreateAgentRuntimeForm,
        model::slugify_agent_key,
        prompts::{DEFAULT_ANALYSIS_STRATEGY_PROMPT, DEFAULT_TRADING_STRATEGY_PROMPT},
        store::{get_agent, insert_agent_runtime},
    },
    hyperliquid::live_state::{AccountKey, AccountLiveState, LiveConnectionStatus},
    opencode::workspace::OpenCodeWorkspaceRuntimeConfig,
};

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
async fn post_agents_with_invalid_private_key_returns_validation_error() {
    let state = test_state().await;

    let app = router(state.clone());
    let body =
        "display_name=Test Agent&hyperliquid_private_key=not-a-key&runtime_id=opencode-local";
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
async fn agents_new_page_renders_runtime_control() {
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
    assert!(!text.contains("name=\"backend_kind\""));
    assert!(text.contains("name=\"runtime_id\""));
    assert!(text.contains("Runtime instance"));
    assert!(text.contains("OpenCode local"));
}
#[tokio::test]
async fn post_agents_rejects_disabled_runtime() {
    let state = test_state().await;
    let runtime_id = format!("disabled-runtime-{}", chrono::Utc::now().timestamp_millis());
    insert_agent_runtime(
        &state.db_pool,
        &CreateAgentRuntimeForm {
            id: runtime_id.clone(),
            name: "Disabled runtime".to_string(),
            backend_kind: crate::agents::model::BACKEND_KIND_OPENCODE.to_string(),
            base_url: "http://localhost:14096".to_string(),
            enabled: None,
        },
    )
    .await
    .expect("insert disabled runtime");

    let app = router(state.clone());
    let private_key = random_private_key();
    let body = format!(
        "display_name=DisabledRuntimeTest&hyperliquid_private_key={private_key}&runtime_id={runtime_id}"
    );
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
    let text = response_text(response).await;
    assert!(text.contains("Selected runtime must exist and be enabled."));
}
#[tokio::test]
async fn post_agents_creates_agent_with_default_strategy_prompts() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let guard = state
        ._test_db_guard
        .as_ref()
        .cloned()
        .expect("test db guard");

    let app = router(state.clone());
    let timestamp = chrono::Utc::now().timestamp_millis();
    let display_name = format!("SoulTest{}", timestamp);
    let agent_key = slugify_agent_key(&display_name);
    let private_key = random_private_key();
    let body = format!(
        "display_name={}&hyperliquid_private_key={}&runtime_id=opencode-local",
        display_name, private_key
    );

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
    let expected_location = format!("/agents/{agent_key}");
    assert_eq!(
        response
            .headers()
            .get("location")
            .and_then(|value| value.to_str().ok()),
        Some(expected_location.as_str())
    );

    let stored = get_agent(&pool, &agent_key)
        .await
        .expect("get agent")
        .expect("agent present");
    assert_eq!(
        stored.backend_kind,
        crate::agents::model::BACKEND_KIND_OPENCODE
    );
    assert_eq!(
        stored.runtime_config["workspace_container_path"],
        serde_json::json!(format!("/workspaces/agents/{agent_key}"))
    );
    let prompts = crate::agents::strategy_prompts::list_agent_strategy_prompts(&pool, &agent_key)
        .await
        .expect("list prompts");
    let analysis = prompts
        .iter()
        .find(|row| row.prompt_kind == "analysis")
        .expect("analysis prompt row");
    let trading = prompts
        .iter()
        .find(|row| row.prompt_kind == "trading")
        .expect("trading prompt row");
    assert_eq!(analysis.prompt, DEFAULT_ANALYSIS_STRATEGY_PROMPT);
    assert_eq!(trading.prompt, DEFAULT_TRADING_STRATEGY_PROMPT);
    assert!(
        std::path::Path::new(
            stored.runtime_config["workspace_host_path"]
                .as_str()
                .expect("host path string")
        )
        .join(".env")
        .exists()
    );

    // Default OpenCode schedules and hooks should have been inserted.
    let schedules = crate::agentic::store::list_agent_schedules(&pool, &agent_key)
        .await
        .expect("list schedules");
    assert_eq!(schedules.len(), 5);
    let analysis = schedules
        .iter()
        .find(|row| row.job_key == "analysis-15m")
        .expect("analysis schedule present");
    assert!(!analysis.enabled);
    assert!(schedules.iter().any(|row| row.job_key == "analysis-1h"));
    assert!(schedules.iter().any(|row| row.job_key == "analysis-1d"));
    let trading = schedules
        .iter()
        .find(|row| row.job_key == "trading-1m")
        .expect("trading schedule present");
    assert!(!trading.enabled);

    let hooks = crate::agentic::store::list_agent_hooks(&pool, &agent_key)
        .await
        .expect("list hooks");
    assert_eq!(hooks.len(), 1);
    let hook = hooks.first().expect("default hook present");
    assert_eq!(hook.job_key, "market-analysis");
    assert!(!hook.enabled);
    drop(guard);
}
#[tokio::test]
async fn post_delete_agent_removes_agent_and_redirects() {
    let state = test_state().await;
    let pool = state.db_pool.clone();

    let app = router(state.clone());
    let timestamp = chrono::Utc::now().timestamp_millis();
    let display_name = format!("DeleteRouteTest{}", timestamp);
    let agent_key = slugify_agent_key(&display_name);
    let private_key = random_private_key();
    let body = format!(
        "display_name={}&hyperliquid_private_key={}&runtime_id=opencode-local&enabled=on",
        display_name, private_key
    );

    // Create the agent.
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

    let stored = get_agent(&pool, &agent_key)
        .await
        .expect("get stored agent before delete")
        .expect("agent present before delete");
    let live_account_key = AccountKey::new(&stored.wallet_address, &stored.environment);
    let workspace = OpenCodeWorkspaceRuntimeConfig::from_value(&stored.runtime_config)
        .expect("workspace metadata present");
    let workspace_path = std::path::PathBuf::from(&workspace.workspace_host_path);
    assert!(workspace_path.exists());
    fs::write(
        workspace_path.join("scratch/delete-sentinel.txt"),
        "cleanup",
    )
    .expect("write delete sentinel");
    state.live_accounts.replace(
        live_account_key.clone(),
        AccountLiveState {
            account_address: stored.wallet_address.clone(),
            environment: stored.environment.clone(),
            status: LiveConnectionStatus::Connected,
            updated_at: Some(Utc::now()),
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
async fn post_agents_rejects_stale_existing_workspace_directory() {
    let state = test_state().await;
    let pool = state.db_pool.clone();
    let app = router(state.clone());
    let timestamp = chrono::Utc::now().timestamp_millis();
    let display_name = format!("StaleWorkspace{}", timestamp);
    let agent_key = slugify_agent_key(&display_name);
    let workspace_path = state
        .opencode_workspace_config
        .host_workspaces_root
        .join("agents")
        .join(&agent_key);
    fs::create_dir_all(&workspace_path).expect("create stale workspace dir");
    let sentinel = workspace_path.join("scripts/user/sentinel.txt");
    fs::create_dir_all(sentinel.parent().expect("sentinel parent"))
        .expect("create sentinel parent");
    fs::write(&sentinel, "stale").expect("write sentinel");

    let private_key = random_private_key();
    let body = format!(
        "display_name={display_name}&hyperliquid_private_key={private_key}&runtime_id=opencode-local&enabled=on"
    );

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

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let text = response_text(response).await;
    assert!(text.contains("Failed to create OpenCode workspace"));
    assert!(text.contains("workspace already exists"));
    assert!(
        get_agent(&pool, &agent_key)
            .await
            .expect("get agent")
            .is_none()
    );
    assert!(sentinel.exists());
}
