use std::sync::Arc;
use std::time::Duration;

use sqlx::postgres::PgListener;
use tokio::sync::{broadcast, watch};
use uuid::Uuid;

use crate::db::DbPool;

pub const SESSION_CHANGED_CHANNEL: &str = "agent_run_detail_session_changed";
pub const RUN_CHANGED_CHANNEL: &str = "agent_run_detail_run_changed";
pub const AGENT_CONVERSATION_CHANGED_CHANNEL: &str = "agent_conversation_changed";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunDetailDbEvent {
    SessionChanged { session_id: String },
    RunChanged { run_id: i64 },
    ConversationChanged { conversation_id: Uuid },
    Resync,
}

#[derive(Clone)]
pub struct RunDetailEventHub {
    sender: broadcast::Sender<RunDetailDbEvent>,
}

impl RunDetailEventHub {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(256);
        Self { sender }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RunDetailDbEvent> {
        self.sender.subscribe()
    }

    pub fn publish(&self, event: RunDetailDbEvent) {
        let _ = self.sender.send(event);
    }
}

impl Default for RunDetailEventHub {
    fn default() -> Self {
        Self::new()
    }
}

pub fn event_from_notification(channel: &str, payload: &str) -> Result<RunDetailDbEvent, String> {
    match channel {
        SESSION_CHANGED_CHANNEL if !payload.trim().is_empty() => {
            Ok(RunDetailDbEvent::SessionChanged {
                session_id: payload.trim().to_string(),
            })
        }
        RUN_CHANGED_CHANNEL => payload
            .trim()
            .parse::<i64>()
            .map(|run_id| RunDetailDbEvent::RunChanged { run_id })
            .map_err(|_| format!("invalid harness run id payload: {payload:?}")),
        AGENT_CONVERSATION_CHANGED_CHANNEL => Uuid::parse_str(payload.trim())
            .map(|conversation_id| RunDetailDbEvent::ConversationChanged { conversation_id })
            .map_err(|_| format!("invalid agent conversation id payload: {payload:?}")),
        SESSION_CHANGED_CHANNEL => Err("empty OpenCode session id payload".to_string()),
        _ => Err(format!("unexpected notification channel: {channel}")),
    }
}

pub async fn run_listener(
    pool: DbPool,
    hub: Arc<RunDetailEventHub>,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let mut retry_delay = Duration::from_millis(100);

    loop {
        if *shutdown_rx.borrow() {
            return;
        }

        let listener = tokio::select! {
            result = PgListener::connect_with(&pool) => match result {
                Ok(listener) => listener,
                Err(error) => {
                    tracing::error!(error = ?error, "run-detail Postgres listener connection failed");
                    if wait_before_retry(&mut shutdown_rx, retry_delay).await {
                        return;
                    }
                    retry_delay = (retry_delay * 2).min(Duration::from_secs(5));
                    continue;
                }
            },
            changed = shutdown_rx.changed() => {
                if changed.is_err() || *shutdown_rx.borrow() {
                    return;
                }
                continue;
            }
        };

        let mut listener = listener;
        if let Err(error) = listener.listen(SESSION_CHANGED_CHANNEL).await {
            tracing::error!(error = ?error, "run-detail Postgres listener subscription failed");
            if wait_before_retry(&mut shutdown_rx, retry_delay).await {
                return;
            }
            retry_delay = (retry_delay * 2).min(Duration::from_secs(5));
            continue;
        }
        if let Err(error) = listener.listen(RUN_CHANGED_CHANNEL).await {
            tracing::error!(error = ?error, "run-detail Postgres listener subscription failed");
            if wait_before_retry(&mut shutdown_rx, retry_delay).await {
                return;
            }
            retry_delay = (retry_delay * 2).min(Duration::from_secs(5));
            continue;
        }
        if let Err(error) = listener.listen(AGENT_CONVERSATION_CHANGED_CHANNEL).await {
            tracing::error!(error = ?error, "run-detail Postgres listener subscription failed");
            if wait_before_retry(&mut shutdown_rx, retry_delay).await {
                return;
            }
            retry_delay = (retry_delay * 2).min(Duration::from_secs(5));
            continue;
        }

        retry_delay = Duration::from_millis(100);
        // The web server can accept SSE connections before this spawned
        // listener has completed its first LISTEN. Refresh them once the
        // subscription is active so changes from that startup window are not lost.
        hub.publish(RunDetailDbEvent::Resync);

        loop {
            tokio::select! {
                result = listener.recv() => match result {
                    Ok(notification) => match event_from_notification(notification.channel(), notification.payload()) {
                        Ok(event) => hub.publish(event),
                        Err(error) => tracing::warn!(error = %error, "ignoring malformed run-detail Postgres notification"),
                    },
                    Err(error) => {
                        tracing::error!(error = ?error, "run-detail Postgres listener failed");
                        break;
                    }
                },
                changed = shutdown_rx.changed() => {
                    if changed.is_err() || *shutdown_rx.borrow() {
                        return;
                    }
                }
            }
        }

        if wait_before_retry(&mut shutdown_rx, retry_delay).await {
            return;
        }
        retry_delay = (retry_delay * 2).min(Duration::from_secs(5));
    }
}

async fn wait_before_retry(shutdown_rx: &mut watch::Receiver<bool>, delay: Duration) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(delay) => *shutdown_rx.borrow(),
        changed = shutdown_rx.changed() => changed.is_err() || *shutdown_rx.borrow(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use sqlx::{Row, query};
    use std::sync::Arc;
    use uuid::Uuid;

    async fn listener(pool: &DbPool) -> PgListener {
        let mut listener = PgListener::connect_with(pool)
            .await
            .expect("connect notification listener");
        listener
            .listen(SESSION_CHANGED_CHANNEL)
            .await
            .expect("listen for session changes");
        listener
            .listen(RUN_CHANGED_CHANNEL)
            .await
            .expect("listen for run changes");
        listener
    }

    async fn next_notification(listener: &mut PgListener) -> (String, String) {
        let notification = tokio::time::timeout(Duration::from_secs(2), listener.recv())
            .await
            .expect("notification timeout")
            .expect("notification receive");
        (
            notification.channel().to_string(),
            notification.payload().to_string(),
        )
    }

    #[test]
    fn converts_valid_notifications() {
        assert_eq!(
            event_from_notification(SESSION_CHANGED_CHANNEL, "ses_1").expect("session event"),
            RunDetailDbEvent::SessionChanged {
                session_id: "ses_1".to_string()
            }
        );
        assert_eq!(
            event_from_notification(RUN_CHANGED_CHANNEL, "42").expect("run event"),
            RunDetailDbEvent::RunChanged { run_id: 42 }
        );
    }

    #[test]
    fn rejects_malformed_notifications() {
        assert!(event_from_notification(SESSION_CHANGED_CHANNEL, "").is_err());
        assert!(event_from_notification(RUN_CHANGED_CHANNEL, "not-a-run").is_err());
        assert!(event_from_notification("other", "42").is_err());
    }

    #[tokio::test]
    async fn broadcasts_events_to_subscribers() {
        let hub = RunDetailEventHub::new();
        let mut receiver = hub.subscribe();
        hub.publish(RunDetailDbEvent::Resync);
        assert_eq!(
            receiver.recv().await.expect("event"),
            RunDetailDbEvent::Resync
        );
    }

    #[tokio::test]
    async fn listener_resyncs_subscribers_after_initial_subscription() {
        let pool = crate::test_db::pool().await;
        let hub = Arc::new(RunDetailEventHub::new());
        let mut receiver = hub.subscribe();
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let listener_task = tokio::spawn(run_listener(
            pool.as_ref().clone(),
            Arc::clone(&hub),
            shutdown_rx,
        ));

        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), receiver.recv())
                .await
                .expect("listener should subscribe")
                .expect("listener should publish resync"),
            RunDetailDbEvent::Resync
        );

        shutdown_tx.send(true).expect("signal listener shutdown");
        tokio::time::timeout(Duration::from_secs(2), listener_task)
            .await
            .expect("listener should stop")
            .expect("listener task should not panic");
    }

    #[tokio::test]
    async fn opencode_triggers_notify_the_affected_session() {
        let pool = crate::test_db::pool().await;
        let mut listener = listener(&pool).await;
        let session_id = format!("trigger-session-{}", Uuid::new_v4());
        let message_id = format!("trigger-message-{}", Uuid::new_v4());
        let part_id = format!("trigger-part-{}", Uuid::new_v4());

        query("INSERT INTO opencode.sessions (id) VALUES ($1)")
            .bind(&session_id)
            .execute(&pool)
            .await
            .expect("insert session");
        assert_eq!(
            next_notification(&mut listener).await,
            (SESSION_CHANGED_CHANNEL.to_string(), session_id.clone())
        );

        query("UPDATE opencode.sessions SET title = 'updated' WHERE id = $1")
            .bind(&session_id)
            .execute(&pool)
            .await
            .expect("update session");
        assert_eq!(
            next_notification(&mut listener).await,
            (SESSION_CHANGED_CHANNEL.to_string(), session_id.clone())
        );

        query(
            "INSERT INTO opencode.messages (id, session_id, role)
             VALUES ($1, $2, 'assistant')",
        )
        .bind(&message_id)
        .bind(&session_id)
        .execute(&pool)
        .await
        .expect("insert message");
        assert_eq!(
            next_notification(&mut listener).await,
            (SESSION_CHANGED_CHANNEL.to_string(), session_id.clone())
        );

        query(
            "INSERT INTO opencode.message_parts
                (id, message_id, part_type, content)
             VALUES ($1, $2, 'tool', $3)",
        )
        .bind(&part_id)
        .bind(&message_id)
        .bind(json!({"tool": "read"}))
        .execute(&pool)
        .await
        .expect("insert message part");
        assert_eq!(
            next_notification(&mut listener).await,
            (SESSION_CHANGED_CHANNEL.to_string(), session_id.clone())
        );

        query("UPDATE opencode.message_parts SET content = $1 WHERE id = $2")
            .bind(json!({"tool": "write"}))
            .bind(&part_id)
            .execute(&pool)
            .await
            .expect("update message part");
        assert_eq!(
            next_notification(&mut listener).await,
            (SESSION_CHANGED_CHANNEL.to_string(), session_id.clone())
        );

        query(
            "INSERT INTO opencode.tool_executions
                (session_id, correlation_id, tool_name)
             VALUES ($1, 'trigger-correlation', 'read')",
        )
        .bind(&session_id)
        .execute(&pool)
        .await
        .expect("insert tool execution");
        assert_eq!(
            next_notification(&mut listener).await,
            (SESSION_CHANGED_CHANNEL.to_string(), session_id.clone())
        );

        query(
            "INSERT INTO opencode.session_errors (session_id, error_message)
             VALUES ($1, 'trigger error')",
        )
        .bind(&session_id)
        .execute(&pool)
        .await
        .expect("insert session error");
        assert_eq!(
            next_notification(&mut listener).await,
            (SESSION_CHANGED_CHANNEL.to_string(), session_id)
        );
    }

    #[tokio::test]
    async fn harness_run_triggers_notify_the_run_id() {
        let pool = crate::test_db::pool().await;
        let mut listener = listener(&pool).await;
        let suffix = Uuid::new_v4().to_string();
        let agent_key = format!("trigger-agent-{suffix}");
        let api_key = format!("trigger-api-{suffix}");
        let now = chrono::Utc::now();
        let user_id = crate::test_db::test_user_id();

        query(
            "INSERT INTO agents
                (agent_key, user_id, created_at, updated_at, display_name,
                 api_key, lifecycle)
             VALUES ($1, $2, $3, $3, $4, $5, 'pending_subaccount_creation')",
        )
        .bind(&agent_key)
        .bind(user_id)
        .bind(now)
        .bind(&agent_key)
        .bind(&api_key)
        .execute(&pool)
        .await
        .expect("insert trigger agent");
        let job_id: i64 = query(
            "INSERT INTO harness_jobs
                (agent_key, job_key, job_kind, trigger_type, timeframe, trigger_delay_seconds,
                 next_run_at, timeout_seconds)
             VALUES ($1, 'trigger-job', 'analysis', 'candle_closed', '15m', 0, $2, 60)
             RETURNING id",
        )
        .bind(&agent_key)
        .bind(now)
        .fetch_one(&pool)
        .await
        .expect("insert trigger job")
        .get("id");
        let run_id: i64 = query(
            "INSERT INTO harness_runs
                (job_id, agent_key, job_key, job_kind, trigger_type, timeframe, status,
                 scheduled_for, timeout_seconds)
             VALUES ($1, $2, 'trigger-job', 'analysis', 'candle_closed', '15m', 'queued', $3, 60)
             RETURNING id",
        )
        .bind(job_id)
        .bind(&agent_key)
        .bind(now)
        .fetch_one(&pool)
        .await
        .expect("insert trigger run")
        .get("id");
        assert_eq!(
            next_notification(&mut listener).await,
            (RUN_CHANGED_CHANNEL.to_string(), run_id.to_string())
        );

        query(
            "UPDATE harness_runs
                SET status = 'running', backend_run_ref = 'trigger-session'
              WHERE id = $1",
        )
        .bind(run_id)
        .execute(&pool)
        .await
        .expect("update trigger run");
        assert_eq!(
            next_notification(&mut listener).await,
            (RUN_CHANGED_CHANNEL.to_string(), run_id.to_string())
        );
    }
}
