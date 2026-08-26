use anyhow::{Context, Result};
use chrono::Utc;
use sqlx::query_as;
use uuid::Uuid;

use crate::{
    db::DbPool,
    gateway::model::NotificationSeverity,
    notifications::model::{NotificationRecord, NotificationRow},
};

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
