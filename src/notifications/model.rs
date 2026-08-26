use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Severity levels for notifications. Mirrors the `notifications.severity`
/// CHECK constraint from migration `0016`.
pub use crate::gateway::model::NotificationSeverity;

/// Database row shape for the `notifications` table.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct NotificationRow {
    pub id: Uuid,
    pub agent_key: String,
    pub title: String,
    pub body: String,
    pub severity: String,
    pub status: String,
}

/// Owned notification record produced after inserting a new row.
#[derive(Debug, Clone)]
pub struct NotificationRecord {
    pub id: Uuid,
    pub agent_key: String,
    pub title: String,
    pub body: String,
    pub severity: NotificationSeverity,
}

impl From<NotificationRow> for NotificationRecord {
    fn from(row: NotificationRow) -> Self {
        Self {
            id: row.id,
            agent_key: row.agent_key,
            title: row.title,
            body: row.body,
            severity: NotificationSeverity::parse(&row.severity)
                .unwrap_or(NotificationSeverity::Info),
        }
    }
}

/// JSON body for `POST /api/v1/notifications`.
#[derive(Debug, Clone, Deserialize)]
pub struct CreateNotification {
    pub title: String,
    pub body: String,
    #[serde(default = "default_severity")]
    pub severity: String,
}

fn default_severity() -> String {
    "info".to_string()
}

impl CreateNotification {
    pub fn validate(&self) -> Result<(), String> {
        let mut errors = Vec::new();
        if self.title.trim().is_empty() {
            errors.push("title is required.".to_string());
        }
        if self.body.trim().is_empty() {
            errors.push("body is required.".to_string());
        }
        if NotificationSeverity::parse(self.severity.trim()).is_none() {
            errors.push("severity must be one of: info, warning, error.".to_string());
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join(" "))
        }
    }

    pub fn severity(&self) -> NotificationSeverity {
        NotificationSeverity::parse(self.severity.trim()).unwrap_or(NotificationSeverity::Info)
    }
}

/// JSON response for `POST /api/v1/notifications`.
#[derive(Debug, Clone, Serialize)]
pub struct NotificationResponse {
    pub id: Uuid,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_notification_validation_rejects_blank_fields() {
        let input = CreateNotification {
            title: String::new(),
            body: String::new(),
            severity: "info".to_string(),
        };
        assert!(input.validate().is_err());
    }

    #[test]
    fn create_notification_validation_rejects_unknown_severity() {
        let input = CreateNotification {
            title: "title".to_string(),
            body: "body".to_string(),
            severity: "critical".to_string(),
        };
        let err = input.validate().expect_err("should error");
        assert!(err.contains("severity"));
    }

    #[test]
    fn create_notification_validation_accepts_known_severity() {
        let input = CreateNotification {
            title: "title".to_string(),
            body: "body".to_string(),
            severity: "warning".to_string(),
        };
        assert!(input.validate().is_ok());
        assert_eq!(input.severity(), NotificationSeverity::Warning);
    }
}
