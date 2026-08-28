#![allow(
    dead_code,
    reason = "Phase 2 deliberately persists the artifact contract before Phase 3 dispatch starts invoking it"
)]

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::query_as;

use crate::{
    db::DbPool,
    harness::model::{RunContextSnapshot, RunWorkspaceArtifactRow},
};

#[derive(Debug, sqlx::FromRow)]
struct RunWorkspaceArtifactDbRow {
    run_id: i64,
    agent_key: String,
    context_schema_version: i32,
    context_snapshot: Value,
    capability_schema_version: i32,
    capability_snapshot: Value,
    workspace_status: String,
    workspace_created_at: Option<DateTime<Utc>>,
    runtime_secrets_scrubbed_at: Option<DateTime<Utc>>,
    terminalized_at: Option<DateTime<Utc>>,
    expires_at: Option<DateTime<Utc>>,
    deletion_started_at: Option<DateTime<Utc>>,
    deleted_at: Option<DateTime<Utc>>,
    observed_size_bytes: Option<i64>,
    observed_file_count: Option<i64>,
    error_summary: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

/// Insert the durable context binding before asking the workspace controller to
/// materialize a run. Repeating the same request is safe; changing a bound
/// context fails closed instead of silently changing a retried run.
pub async fn prepare_run_workspace_artifact(
    pool: &DbPool,
    agent_key: &str,
    run_id: i64,
    context: &RunContextSnapshot,
) -> Result<RunWorkspaceArtifactRow> {
    context.validate()?;
    let capability_snapshot = Value::Array(
        context
            .normalized_enabled_capabilities()?
            .into_iter()
            .map(Value::String)
            .collect(),
    );
    let mut tx = pool
        .begin()
        .await
        .context("failed to begin run workspace artifact preparation")?;
    let owned: Option<(i64,)> = query_as(
        "SELECT id
           FROM harness_sub_agent_runs
          WHERE id = $1 AND agent_key = $2
          FOR UPDATE",
    )
    .bind(run_id)
    .bind(agent_key)
    .fetch_optional(&mut *tx)
    .await
    .context("failed to validate run workspace artifact ownership")?;
    if owned.is_none() {
        bail!("run workspace artifact does not belong to agent");
    }

    if let Some(existing) = get_run_workspace_artifact_in_tx(&mut tx, agent_key, run_id).await? {
        if existing.context.schema_version != context.schema_version
            || existing.context.context != context.context
            || existing.context.capability_schema_version != context.capability_schema_version
            || existing.context.enabled_capabilities != context.normalized_enabled_capabilities()?
        {
            bail!("run workspace artifact context conflicts with existing binding");
        }
        if existing.workspace_status == "deleted" {
            bail!("run workspace artifact has already been deleted");
        }
        tx.commit()
            .await
            .context("failed to commit idempotent run workspace artifact preparation")?;
        return Ok(existing);
    }

    sqlx::query(
        "INSERT INTO harness_run_workspace_artifacts (
             run_id,
             context_schema_version,
             context_snapshot,
             capability_schema_version,
             capability_snapshot,
             workspace_status
         ) VALUES ($1, $2, $3, $4, $5, 'preparing')",
    )
    .bind(run_id)
    .bind(context.schema_version)
    .bind(&context.context)
    .bind(context.capability_schema_version)
    .bind(capability_snapshot)
    .execute(&mut *tx)
    .await
    .context("failed to insert run workspace artifact")?;

    let artifact = get_run_workspace_artifact_in_tx(&mut tx, agent_key, run_id)
        .await?
        .expect("inserted run workspace artifact must be readable");
    tx.commit()
        .await
        .context("failed to commit run workspace artifact preparation")?;
    Ok(artifact)
}

pub async fn get_run_workspace_artifact(
    pool: &DbPool,
    agent_key: &str,
    run_id: i64,
) -> Result<Option<RunWorkspaceArtifactRow>> {
    let row = query_as::<_, RunWorkspaceArtifactDbRow>(
        "SELECT artifacts.run_id,
                runs.agent_key,
                artifacts.context_schema_version,
                artifacts.context_snapshot,
                artifacts.capability_schema_version,
                artifacts.capability_snapshot,
                artifacts.workspace_status,
                artifacts.workspace_created_at,
                artifacts.runtime_secrets_scrubbed_at,
                artifacts.terminalized_at,
                artifacts.expires_at,
                artifacts.deletion_started_at,
                artifacts.deleted_at,
                artifacts.observed_size_bytes,
                artifacts.observed_file_count,
                artifacts.error_summary,
                artifacts.created_at,
                artifacts.updated_at
           FROM harness_run_workspace_artifacts AS artifacts
           JOIN harness_sub_agent_runs AS runs ON runs.id = artifacts.run_id
          WHERE artifacts.run_id = $1 AND runs.agent_key = $2",
    )
    .bind(run_id)
    .bind(agent_key)
    .fetch_optional(pool)
    .await
    .context("failed to load run workspace artifact")?;
    row.map(run_workspace_artifact_from_db).transpose()
}

pub async fn mark_run_workspace_ready(pool: &DbPool, agent_key: &str, run_id: i64) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE harness_run_workspace_artifacts AS artifacts
            SET workspace_status = 'ready',
                workspace_created_at = COALESCE(workspace_created_at, now())
           FROM harness_sub_agent_runs AS runs
          WHERE artifacts.run_id = $1
            AND runs.id = artifacts.run_id
            AND runs.agent_key = $2
            AND artifacts.workspace_status IN ('preparing', 'ready')",
    )
    .bind(run_id)
    .bind(agent_key)
    .execute(pool)
    .await
    .context("failed to mark run workspace ready")?;
    Ok(result.rows_affected() > 0)
}

pub async fn record_run_workspace_secret_scrub(
    pool: &DbPool,
    agent_key: &str,
    run_id: i64,
) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE harness_run_workspace_artifacts AS artifacts
            SET runtime_secrets_scrubbed_at = COALESCE(runtime_secrets_scrubbed_at, now())
           FROM harness_sub_agent_runs AS runs
          WHERE artifacts.run_id = $1
            AND runs.id = artifacts.run_id
            AND runs.agent_key = $2
            AND artifacts.workspace_status <> 'deleted'",
    )
    .bind(run_id)
    .bind(agent_key)
    .execute(pool)
    .await
    .context("failed to record run workspace runtime-secret scrub")?;
    Ok(result.rows_affected() > 0)
}

pub async fn record_run_workspace_stats(
    pool: &DbPool,
    agent_key: &str,
    run_id: i64,
    size_bytes: u64,
    file_count: u64,
) -> Result<bool> {
    let size_bytes = i64::try_from(size_bytes).context("workspace size exceeds database range")?;
    let file_count =
        i64::try_from(file_count).context("workspace file count exceeds database range")?;
    let result = sqlx::query(
        "UPDATE harness_run_workspace_artifacts AS artifacts
            SET observed_size_bytes = $3,
                observed_file_count = $4
           FROM harness_sub_agent_runs AS runs
          WHERE artifacts.run_id = $1
            AND runs.id = artifacts.run_id
            AND runs.agent_key = $2
            AND artifacts.workspace_status <> 'deleted'",
    )
    .bind(run_id)
    .bind(agent_key)
    .bind(size_bytes)
    .bind(file_count)
    .execute(pool)
    .await
    .context("failed to record run workspace statistics")?;
    Ok(result.rows_affected() > 0)
}

async fn get_run_workspace_artifact_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    agent_key: &str,
    run_id: i64,
) -> Result<Option<RunWorkspaceArtifactRow>> {
    let row = query_as::<_, RunWorkspaceArtifactDbRow>(
        "SELECT artifacts.run_id,
                runs.agent_key,
                artifacts.context_schema_version,
                artifacts.context_snapshot,
                artifacts.capability_schema_version,
                artifacts.capability_snapshot,
                artifacts.workspace_status,
                artifacts.workspace_created_at,
                artifacts.runtime_secrets_scrubbed_at,
                artifacts.terminalized_at,
                artifacts.expires_at,
                artifacts.deletion_started_at,
                artifacts.deleted_at,
                artifacts.observed_size_bytes,
                artifacts.observed_file_count,
                artifacts.error_summary,
                artifacts.created_at,
                artifacts.updated_at
           FROM harness_run_workspace_artifacts AS artifacts
           JOIN harness_sub_agent_runs AS runs ON runs.id = artifacts.run_id
          WHERE artifacts.run_id = $1 AND runs.agent_key = $2",
    )
    .bind(run_id)
    .bind(agent_key)
    .fetch_optional(&mut **tx)
    .await
    .context("failed to load run workspace artifact in transaction")?;
    row.map(run_workspace_artifact_from_db).transpose()
}

fn run_workspace_artifact_from_db(
    row: RunWorkspaceArtifactDbRow,
) -> Result<RunWorkspaceArtifactRow> {
    let capabilities = row
        .capability_snapshot
        .as_array()
        .context("run workspace artifact capability snapshot is not an array")?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(ToOwned::to_owned)
                .context("run workspace artifact capability snapshot contains a non-string")
        })
        .collect::<Result<Vec<_>>>()?;
    let context = RunContextSnapshot {
        schema_version: row.context_schema_version,
        context: row.context_snapshot,
        capability_schema_version: row.capability_schema_version,
        enabled_capabilities: capabilities,
    };
    context.validate()?;
    Ok(RunWorkspaceArtifactRow {
        run_id: row.run_id,
        agent_key: row.agent_key,
        context,
        workspace_status: row.workspace_status,
        workspace_created_at: row.workspace_created_at,
        runtime_secrets_scrubbed_at: row.runtime_secrets_scrubbed_at,
        terminalized_at: row.terminalized_at,
        expires_at: row.expires_at,
        deletion_started_at: row.deletion_started_at,
        deleted_at: row.deleted_at,
        observed_size_bytes: row.observed_size_bytes,
        observed_file_count: row.observed_file_count,
        error_summary: row.error_summary,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use serde_json::json;

    use super::*;
    use crate::{
        agents::{model::AgentRegistryRow, store::insert_agent},
        harness::model::{
            CAPABILITY_NOTIFICATION_SEND, CAPABILITY_SCHEMA_VERSION,
            RUN_CONTEXT_SNAPSHOT_SCHEMA_VERSION,
        },
        test_db,
    };

    async fn seed_run(pool: &DbPool, agent_key: &str) -> i64 {
        let now = Utc::now();
        insert_agent(
            pool,
            &AgentRegistryRow {
                agent_key: agent_key.to_string(),
                user_id: test_db::test_user_id(),
                created_at: now,
                updated_at: now,
                enabled: true,
                lifecycle: crate::agents::model::AGENT_LIFECYCLE_ACTIVE.to_string(),
                display_name: agent_key.to_string(),
                trading_account_address: Some(format!("0x{:040x}", uuid::Uuid::new_v4().as_u128())),
                environment: "live".to_string(),
                api_key: format!("vta_{agent_key}"),
                api_key_last_used_at: None,
                runtime_config: json!({}),
            },
        )
        .await
        .expect("insert agent");
        let (sub_agent_id,): (i64,) = query_as(
            "INSERT INTO harness_sub_agents (
                 agent_key, sub_agent_key, sub_agent_kind, timeout_seconds
             ) VALUES ($1, 'analysis-15m', 'analysis', 60)
             RETURNING id",
        )
        .bind(agent_key)
        .fetch_one(pool)
        .await
        .expect("insert sub-agent");
        let (run_id,): (i64,) = query_as(
            "INSERT INTO harness_sub_agent_runs (
                 sub_agent_id, agent_key, sub_agent_key, sub_agent_kind, status,
                 scheduled_for, timeout_seconds
             ) VALUES ($1, $2, 'analysis-15m', 'analysis', 'queued', now(), 60)
             RETURNING id",
        )
        .bind(sub_agent_id)
        .bind(agent_key)
        .fetch_one(pool)
        .await
        .expect("insert run");
        run_id
    }

    fn snapshot() -> RunContextSnapshot {
        RunContextSnapshot {
            schema_version: RUN_CONTEXT_SNAPSHOT_SCHEMA_VERSION,
            context: json!({
                "provider_id": "anthropic",
                "model_id": "claude-sonnet-4",
                "model_variant": null,
                "timeout_seconds": 60,
                "selected_instruments": ["BTC"],
                "strategy_prompt_revisions": {"analysis": 1},
                "additional_instructions": "",
                "accumulated_learning_memory_id": null,
                "system_prompt_version": "v1",
                "quantitative_package": null,
                "mcp_installations": [],
                "notification_send_enabled": true,
                "scheduled_candle_boundary": null,
                "account_snapshot_metadata": null
            }),
            capability_schema_version: CAPABILITY_SCHEMA_VERSION,
            enabled_capabilities: vec![CAPABILITY_NOTIFICATION_SEND.to_string()],
        }
    }

    #[tokio::test]
    async fn preparation_binds_an_immutable_versioned_context() {
        let pool = test_db::pool().await;
        let agent_key = format!("artifact-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0));
        let run_id = seed_run(&pool, &agent_key).await;

        let first = prepare_run_workspace_artifact(&pool, &agent_key, run_id, &snapshot())
            .await
            .expect("prepare artifact");
        let second = prepare_run_workspace_artifact(&pool, &agent_key, run_id, &snapshot())
            .await
            .expect("repeat preparation");

        assert_eq!(first, second);
        assert_eq!(first.workspace_status, "preparing");
        assert!(first.context.notification_send_enabled());
        assert!(
            prepare_run_workspace_artifact(&pool, &agent_key, run_id, &{
                let mut changed = snapshot();
                changed.context["provider_id"] = json!("other");
                changed
            },)
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn workspace_lifecycle_updates_are_agent_scoped() {
        let pool = test_db::pool().await;
        let agent_key = format!(
            "artifact-life-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        let run_id = seed_run(&pool, &agent_key).await;
        prepare_run_workspace_artifact(&pool, &agent_key, run_id, &snapshot())
            .await
            .expect("prepare artifact");

        assert!(
            !mark_run_workspace_ready(&pool, "other-agent", run_id)
                .await
                .expect("scope ready update")
        );
        assert!(
            mark_run_workspace_ready(&pool, &agent_key, run_id)
                .await
                .expect("mark ready")
        );
        assert!(
            record_run_workspace_secret_scrub(&pool, &agent_key, run_id)
                .await
                .expect("record scrub")
        );
        assert!(
            record_run_workspace_stats(&pool, &agent_key, run_id, 123, 4)
                .await
                .expect("record stats")
        );

        let artifact = get_run_workspace_artifact(&pool, &agent_key, run_id)
            .await
            .expect("get artifact")
            .expect("artifact exists");
        assert_eq!(artifact.workspace_status, "ready");
        assert!(artifact.workspace_created_at.is_some());
        assert!(artifact.runtime_secrets_scrubbed_at.is_some());
        assert_eq!(artifact.observed_size_bytes, Some(123));
        assert_eq!(artifact.observed_file_count, Some(4));
    }

    #[test]
    fn context_snapshot_rejects_secret_and_gateway_fields() {
        let invalid = RunContextSnapshot {
            context: json!({"telegram": {"bot_token": "never-store"}}),
            ..snapshot()
        };

        assert!(invalid.validate().is_err());
    }

    #[test]
    fn context_snapshot_rejects_normalized_secret_fields_and_known_credentials() {
        for replacement in [
            json!({
                "captured_at_ms": 1,
                "account_address": "0x0000000000000000000000000000000000000000",
                "botToken": "123456:abcdefghijklmnopqrstuvwxyz"
            }),
            json!({
                "captured_at_ms": 1,
                "account_address": "0x0000000000000000000000000000000000000000",
                "privateKey": "not-permitted"
            }),
            json!({
                "captured_at_ms": 1,
                "account_address": "vta_runtime_credential"
            }),
        ] {
            let mut invalid = snapshot();
            invalid.context["account_snapshot_metadata"] = replacement;
            assert!(invalid.validate().is_err());
        }

        for secret in [
            "TELEGRAM_BOT_TOKEN=123456:abcdefghijklmnopqrstuvwxyz",
            "https://api.telegram.org/bot123456:abcdefghijklmnopqrstuvwxyz/sendMessage",
        ] {
            let mut credential_instructions = snapshot();
            credential_instructions.context["additional_instructions"] = json!(secret);
            assert!(credential_instructions.validate().is_err());
        }
    }

    #[test]
    fn context_snapshot_rejects_unsupported_versions_and_arbitrary_nested_values() {
        let mut unsupported_version = snapshot();
        unsupported_version.schema_version = RUN_CONTEXT_SNAPSHOT_SCHEMA_VERSION + 1;
        assert!(unsupported_version.validate().is_err());

        let mut unauthorized_mcp_field = snapshot();
        unauthorized_mcp_field.context["mcp_installations"] = json!([{
            "installation_id": "market-data",
            "version": "1",
            "tools": [],
            "authorization": "Bearer sk-not-permitted"
        }]);
        assert!(unauthorized_mcp_field.validate().is_err());
    }

    #[test]
    fn context_snapshot_requires_capability_and_context_authority_to_match() {
        let mut mismatch = snapshot();
        mismatch.context["notification_send_enabled"] = json!(false);
        assert!(mismatch.validate().is_err());

        let mut unknown_capability = snapshot();
        unknown_capability.context["notification_send_enabled"] = json!(false);
        unknown_capability.enabled_capabilities =
            vec!["123456:abcdefghijklmnopqrstuvwxyz".to_string()];
        assert!(unknown_capability.validate().is_err());
    }

    #[test]
    fn context_snapshot_scans_long_numeric_instructions_in_linear_time() {
        let mut snapshot = snapshot();
        snapshot.context["additional_instructions"] = json!("1".repeat(65_536));
        assert!(snapshot.validate().is_ok());
    }
}
