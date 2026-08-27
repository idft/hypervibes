use anyhow::{Context, Result};
use chrono::Utc;
use sqlx::query_as;
use uuid::Uuid;

use crate::{
    db::DbPool,
    gateway::model::NotificationSeverity,
    notifications::model::{NotificationHistoryRow, NotificationRecord, NotificationRow},
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
    let row = query_as::<_, NotificationRow>(
        "INSERT INTO notifications (agent_key, title, body, severity)
         VALUES ($1, $2, $3, $4)
          RETURNING id, agent_key, title, body, severity, status",
    )
    .bind(agent_key)
    .bind(title.trim())
    .bind(body.trim())
    .bind(severity.as_str())
    .fetch_one(pool)
    .await
    .context("failed to insert notification")?;
    Ok(NotificationRecord::from(row))
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
    use super::*;
    use crate::test_db;

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
}
