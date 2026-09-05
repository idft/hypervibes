use anyhow::{Context, Result};
use chrono::Utc;
use sqlx::query_as;
use uuid::Uuid;

use crate::{
    db::DbPool,
    gateway::model::NotificationSeverity,
    notifications::model::{
        NotificationHistoryRow, NotificationProvenance, NotificationRecord, NotificationRow,
    },
};

pub const NOTIFICATION_HISTORY_LIMIT: i64 = 100;

/// Count every notification for an agent, without applying the history-page limit.
pub async fn count_notifications(pool: &DbPool, agent_key: &str) -> Result<i64> {
    let (count,): (i64,) = query_as("SELECT COUNT(*) FROM notifications WHERE agent_key = $1")
        .bind(agent_key)
        .fetch_one(pool)
        .await
        .with_context(|| format!("failed to count notifications for agent {agent_key}"))?;
    Ok(count)
}

/// Insert a new queued notification. Returns the new row's id and a typed
/// record so the caller can broadcast the event without re-querying.
pub async fn create_notification(
    pool: &DbPool,
    agent_key: &str,
    title: &str,
    body: &str,
    severity: NotificationSeverity,
) -> Result<NotificationRecord> {
    create_notification_with_optional_provenance(pool, agent_key, title, body, severity, None).await
}

/// Queue a notification after verifying the source's durable capability
/// binding. This is intentionally separate from the legacy permanent-agent-key
/// path until run- and conversation-scoped runtime credentials ship.
#[allow(
    dead_code,
    reason = "Phase 3 and Phase 4 scoped runtime credentials will call this instead of the legacy agent-key path"
)]
pub async fn create_notification_with_provenance(
    pool: &DbPool,
    agent_key: &str,
    title: &str,
    body: &str,
    severity: NotificationSeverity,
    provenance: NotificationProvenance,
) -> Result<NotificationRecord> {
    create_notification_with_optional_provenance(
        pool,
        agent_key,
        title,
        body,
        severity,
        Some(provenance),
    )
    .await
}

async fn create_notification_with_optional_provenance(
    pool: &DbPool,
    agent_key: &str,
    title: &str,
    body: &str,
    severity: NotificationSeverity,
    provenance: Option<NotificationProvenance>,
) -> Result<NotificationRecord> {
    let mut tx = pool
        .begin()
        .await
        .context("failed to begin notification create transaction")?;
    if let Some(provenance) = provenance {
        authorize_notification_provenance(&mut tx, agent_key, provenance).await?;
    }
    let (source_kind, source_run_id, source_conversation_id, source_capability_schema_version) =
        match provenance {
            Some(NotificationProvenance::Run {
                run_id,
                capability_schema_version,
            }) => (
                Some("run"),
                Some(run_id),
                None,
                Some(capability_schema_version),
            ),
            Some(NotificationProvenance::Conversation {
                conversation_id,
                capability_schema_version,
            }) => (
                Some("conversation"),
                None,
                Some(conversation_id),
                Some(capability_schema_version),
            ),
            None => (None, None, None, None),
        };
    let row = query_as::<_, NotificationRow>(
        "INSERT INTO notifications (
             agent_key, title, body, severity, source_kind, source_run_id,
             source_conversation_id, source_capability_schema_version
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
          RETURNING id, agent_key, title, body, severity, status",
    )
    .bind(agent_key)
    .bind(title.trim())
    .bind(body.trim())
    .bind(severity.as_str())
    .bind(source_kind)
    .bind(source_run_id)
    .bind(source_conversation_id)
    .bind(source_capability_schema_version)
    .fetch_one(&mut *tx)
    .await
    .context("failed to insert notification")?;
    tx.commit()
        .await
        .context("failed to commit notification create transaction")?;
    Ok(NotificationRecord::from(row))
}

async fn authorize_notification_provenance(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    agent_key: &str,
    provenance: NotificationProvenance,
) -> Result<()> {
    if !(1..=1_000_000).contains(&provenance.capability_schema_version()) {
        anyhow::bail!("notification provenance has an invalid capability schema version");
    }
    let authorized: Option<(i32,)> = match provenance {
        NotificationProvenance::Run {
            run_id,
            capability_schema_version,
        } => {
            let owned: Option<(i64,)> = query_as(
                "SELECT id
                   FROM harness_sub_agent_runs
                  WHERE id = $1 AND agent_key = $2
                  FOR SHARE",
            )
            .bind(run_id)
            .bind(agent_key)
            .fetch_optional(&mut **tx)
            .await
            .context("failed to lock run notification provenance")?;
            if owned.is_none() {
                None
            } else {
                query_as(
                    "SELECT 1
                       FROM harness_run_workspace_artifacts AS artifacts
                      WHERE artifacts.run_id = $1
                        AND artifacts.capability_schema_version = $2
                        AND artifacts.workspace_status = 'ready'
                        AND artifacts.capability_snapshot @> $3
                      FOR SHARE",
                )
                .bind(run_id)
                .bind(capability_schema_version)
                .bind(serde_json::json!([
                    crate::harness::model::CAPABILITY_NOTIFICATION_SEND
                ]))
                .fetch_optional(&mut **tx)
                .await
                .context("failed to lock run notification capability binding")?
            }
        }
        NotificationProvenance::Conversation {
            conversation_id,
            capability_schema_version,
        } => {
            let owned: Option<(Uuid,)> = query_as(
                "SELECT id
                   FROM agent_conversations
                  WHERE id = $1 AND agent_key = $2
                  FOR SHARE",
            )
            .bind(conversation_id)
            .bind(agent_key)
            .fetch_optional(&mut **tx)
            .await
            .context("failed to lock conversation notification provenance")?;
            if owned.is_none() {
                None
            } else {
                query_as(
                    "SELECT 1
                       FROM agent_conversation_workspaces AS workspaces
                       JOIN agent_conversation_tool_policies AS policies
                         ON policies.conversation_id = workspaces.conversation_id
                      WHERE workspaces.conversation_id = $1
                        AND workspaces.capability_schema_version = $2
                        AND workspaces.workspace_status = 'ready'
                        AND policies.tool_group = 'notifications'
                        AND policies.policy = 'allow'
                      FOR SHARE OF workspaces, policies",
                )
                .bind(conversation_id)
                .bind(capability_schema_version)
                .fetch_optional(&mut **tx)
                .await
                .context("failed to lock conversation notification capability binding")?
            }
        }
    };
    if authorized.is_none() {
        anyhow::bail!("notification provenance is not authorized for this agent");
    }
    Ok(())
}

/// List the most recent notifications for an agent's operator history.
pub async fn list_notification_history(
    pool: &DbPool,
    agent_key: &str,
) -> Result<Vec<NotificationHistoryRow>> {
    query_as::<_, NotificationHistoryRow>(
        "SELECT id, title, body, severity, status, created_at, sent_at, error
           FROM notifications
          WHERE agent_key = $1
          ORDER BY created_at DESC, id DESC
          LIMIT $2",
    )
    .bind(agent_key)
    .bind(NOTIFICATION_HISTORY_LIMIT)
    .fetch_all(pool)
    .await
    .with_context(|| format!("failed to list notification history for agent {agent_key}"))
}

/// Delete one notification, scoped to its owning agent.
pub async fn delete_notification(pool: &DbPool, agent_key: &str, id: Uuid) -> Result<bool> {
    let result = sqlx::query("DELETE FROM notifications WHERE id = $1 AND agent_key = $2")
        .bind(id)
        .bind(agent_key)
        .execute(pool)
        .await
        .with_context(|| format!("failed to delete notification {id}"))?;

    Ok(result.rows_affected() > 0)
}

/// Delete selected notifications, scoped to their owning agent.
pub async fn delete_notifications(pool: &DbPool, agent_key: &str, ids: &[Uuid]) -> Result<u64> {
    if ids.is_empty() {
        return Ok(0);
    }

    let result = sqlx::query("DELETE FROM notifications WHERE agent_key = $1 AND id = ANY($2)")
        .bind(agent_key)
        .bind(ids.to_vec())
        .execute(pool)
        .await
        .with_context(|| format!("failed to delete notifications for agent {agent_key}"))?;

    Ok(result.rows_affected())
}

/// Mark a notification as sent with the current timestamp.
pub async fn mark_sent(pool: &DbPool, id: Uuid) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE notifications
            SET status = 'sent', sent_at = $2
          WHERE id = $1",
    )
    .bind(id)
    .bind(Utc::now())
    .execute(pool)
    .await
    .context("failed to mark notification sent")?;
    Ok(result.rows_affected() > 0)
}

/// Mark a notification as failed with the provided error message.
pub async fn mark_failed(pool: &DbPool, id: Uuid, error: &str) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE notifications
            SET status = 'failed', error = $2
          WHERE id = $1",
    )
    .bind(id)
    .bind(error)
    .execute(pool)
    .await
    .context("failed to mark notification failed")?;
    Ok(result.rows_affected() > 0)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{
        harness::{
            model::{
                CAPABILITY_NOTIFICATION_SEND, CAPABILITY_SCHEMA_VERSION,
                RUN_CONTEXT_SNAPSHOT_SCHEMA_VERSION, RunContextSnapshot,
            },
            store::artifacts::{mark_run_workspace_ready, prepare_run_workspace_artifact},
        },
        test_db,
    };

    async fn seed_agent(pool: &DbPool, key: &str) {
        let now = chrono::Utc::now();
        crate::agents::store::insert_agent(
            pool,
            &crate::agents::model::AgentRegistryRow {
                agent_key: key.to_string(),
                user_id: test_db::test_user_id(),
                created_at: now,
                updated_at: now,
                enabled: true,
                lifecycle: crate::agents::model::AGENT_LIFECYCLE_ACTIVE.to_string(),
                display_name: key.to_string(),
                trading_account_address: Some(format!("0x{:040x}", uuid::Uuid::new_v4().as_u128())),
                environment: "live".to_string(),
                api_key: format!("vta_{key}"),
                api_key_last_used_at: None,
                runtime_config: serde_json::json!({}),
            },
        )
        .await
        .expect("insert agent");
    }

    async fn seed_run(pool: &DbPool, agent_key: &str) -> i64 {
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

    #[tokio::test]
    async fn create_notification_persists_queued_row() {
        let pool = test_db::pool().await;
        let key = format!(
            "note-create-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        seed_agent(&pool, &key).await;

        let record =
            create_notification(&pool, &key, "title", "body", NotificationSeverity::Warning)
                .await
                .expect("create notification");
        assert_eq!(record.agent_key, key);
        assert_eq!(record.severity, NotificationSeverity::Warning);
    }

    #[tokio::test]
    async fn notification_history_is_scoped_to_the_agent() {
        let pool = test_db::pool().await;
        let first_key = format!(
            "note-history-first-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        let second_key = format!(
            "note-history-second-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        seed_agent(&pool, &first_key).await;
        seed_agent(&pool, &second_key).await;

        create_notification(
            &pool,
            &first_key,
            "First notification",
            "First body",
            NotificationSeverity::Warning,
        )
        .await
        .expect("create first notification");
        create_notification(
            &pool,
            &second_key,
            "Second notification",
            "Second body",
            NotificationSeverity::Error,
        )
        .await
        .expect("create second notification");

        let history = list_notification_history(&pool, &first_key)
            .await
            .expect("list notification history");

        assert_eq!(history.len(), 1);
        assert_eq!(history[0].title, "First notification");
        assert_eq!(history[0].severity, "warning");
        assert_eq!(history[0].status, "queued");
    }

    #[tokio::test]
    async fn notification_count_is_exact_and_scoped_to_the_agent() {
        let pool = test_db::pool().await;
        let first_key = format!(
            "note-count-first-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        let second_key = format!(
            "note-count-second-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        seed_agent(&pool, &first_key).await;
        seed_agent(&pool, &second_key).await;

        sqlx::query(
            "INSERT INTO notifications (agent_key, title, body)
             SELECT $1, 'Notification', 'Body'
             FROM generate_series(1, 101)",
        )
        .bind(&first_key)
        .execute(&pool)
        .await
        .expect("insert notifications for first agent");
        create_notification(
            &pool,
            &second_key,
            "Other notification",
            "Other body",
            NotificationSeverity::Info,
        )
        .await
        .expect("create notification for second agent");

        assert_eq!(
            count_notifications(&pool, &first_key)
                .await
                .expect("count first agent notifications"),
            101
        );
        assert_eq!(
            count_notifications(&pool, &second_key)
                .await
                .expect("count second agent notifications"),
            1
        );
        assert_eq!(
            list_notification_history(&pool, &first_key)
                .await
                .expect("list first agent notification history")
                .len(),
            NOTIFICATION_HISTORY_LIMIT as usize
        );
    }

    #[tokio::test]
    async fn delete_notification_is_scoped_to_the_agent() {
        let pool = test_db::pool().await;
        let owner_key = format!(
            "note-delete-owner-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        let other_key = format!(
            "note-delete-other-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        seed_agent(&pool, &owner_key).await;
        seed_agent(&pool, &other_key).await;
        let notification = create_notification(
            &pool,
            &owner_key,
            "Owner notification",
            "Owner body",
            NotificationSeverity::Info,
        )
        .await
        .expect("create owner notification");

        assert!(
            !delete_notification(&pool, &other_key, notification.id)
                .await
                .expect("try to delete other agent notification")
        );
        assert_eq!(
            list_notification_history(&pool, &owner_key)
                .await
                .expect("list owner notification history")
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn mark_failed_records_error_message() {
        let pool = test_db::pool().await;
        let key = format!(
            "note-failed-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        seed_agent(&pool, &key).await;

        let record = create_notification(&pool, &key, "title", "body", NotificationSeverity::Error)
            .await
            .expect("create notification");
        assert!(
            mark_failed(&pool, record.id, "telegram unreachable")
                .await
                .expect("mark failed")
        );
        let (status, error): (String, Option<String>) = sqlx::query_as(
            "SELECT status, error
               FROM notifications
               WHERE id = $1",
        )
        .bind(record.id)
        .fetch_one(&pool)
        .await
        .expect("fetch row");
        assert_eq!(status, "failed");
        assert_eq!(error.as_deref(), Some("telegram unreachable"));
    }

    #[tokio::test]
    async fn scoped_run_notifications_require_and_preserve_capability_provenance() {
        let pool = test_db::pool().await;
        let key = format!(
            "note-provenance-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        seed_agent(&pool, &key).await;
        let run_id = seed_run(&pool, &key).await;
        assert!(
            sqlx::query(
                "INSERT INTO notifications (
                     agent_key, title, body, source_run_id, source_capability_schema_version
                 ) VALUES ($1, 'invalid', 'invalid', $2, 1)",
            )
            .bind(&key)
            .bind(run_id)
            .execute(&pool)
            .await
            .is_err(),
            "nullable source_kind must not bypass provenance validation"
        );
        prepare_run_workspace_artifact(
            &pool,
            &key,
            run_id,
            &RunContextSnapshot {
                schema_version: RUN_CONTEXT_SNAPSHOT_SCHEMA_VERSION,
                context: json!({
                    "provider_id": "test",
                    "model_id": "test",
                    "model_variant": null,
                    "timeout_seconds": 60,
                    "selected_instruments": [],
                    "strategy_prompt_revision": {"target_sub_agent_id": 1, "revision_id": 1},
                    "additional_instructions": "",
                    "accumulated_learning_memory_id": null,
                    "system_prompt_version": "v1",
                    "mcp_installations": [],
                    "notification_send_enabled": true,
                    "scheduled_candle_boundary": null,
                    "account_snapshot_metadata": null
                }),
                capability_schema_version: CAPABILITY_SCHEMA_VERSION,
                enabled_capabilities: vec![CAPABILITY_NOTIFICATION_SEND.to_string()],
            },
        )
        .await
        .expect("prepare run artifact");

        assert!(
            create_notification_with_provenance(
                &pool,
                &key,
                "title",
                "body",
                NotificationSeverity::Info,
                NotificationProvenance::Run {
                    run_id,
                    capability_schema_version: CAPABILITY_SCHEMA_VERSION,
                },
            )
            .await
            .is_err()
        );

        mark_run_workspace_ready(&pool, &key, run_id)
            .await
            .expect("mark run workspace ready");
        let notification = create_notification_with_provenance(
            &pool,
            &key,
            "title",
            "body",
            NotificationSeverity::Info,
            NotificationProvenance::Run {
                run_id,
                capability_schema_version: CAPABILITY_SCHEMA_VERSION,
            },
        )
        .await
        .expect("create scoped notification");
        let (source_kind, source_run_id, source_version): (String, Option<i64>, i32) = query_as(
            "SELECT source_kind, source_run_id, source_capability_schema_version
               FROM notifications
              WHERE id = $1",
        )
        .bind(notification.id)
        .fetch_one(&pool)
        .await
        .expect("load notification provenance");
        assert_eq!(source_kind, "run");
        assert_eq!(source_run_id, Some(run_id));
        assert_eq!(source_version, CAPABILITY_SCHEMA_VERSION);

        sqlx::query("DELETE FROM harness_sub_agent_runs WHERE id = $1")
            .bind(run_id)
            .execute(&pool)
            .await
            .expect("delete source run");
        let (retained_kind, removed_run_id): (String, Option<i64>) =
            query_as("SELECT source_kind, source_run_id FROM notifications WHERE id = $1")
                .bind(notification.id)
                .fetch_one(&pool)
                .await
                .expect("load retained notification");
        assert_eq!(retained_kind, "run");
        assert!(removed_run_id.is_none());
    }
}
