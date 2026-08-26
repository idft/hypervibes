use tokio::sync::broadcast;
use uuid::Uuid;

const UI_EVENT_CAPACITY: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiEvent {
    MemoryCreated {
        agent_key: String,
        memory_id: Uuid,
    },
    /// Notification was queued by `POST /api/v1/notifications`. The gateway
    /// service observes this to dispatch the message to the configured
    /// Telegram chat. The database trigger also emits `notification_created`
    /// pg_notify, so this hub event is purely for in-process consumers
    /// that want a typed notification without subscribing to Postgres.
    NotificationQueued {
        agent_key: String,
        notification_id: Uuid,
    },
}

#[derive(Debug)]
pub struct UiEventHub {
    sender: broadcast::Sender<UiEvent>,
}

impl Default for UiEventHub {
    fn default() -> Self {
        Self::new()
    }
}

impl UiEventHub {
    pub fn new() -> Self {
        let (sender, _receiver) = broadcast::channel(UI_EVENT_CAPACITY);
        Self { sender }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<UiEvent> {
        self.sender.subscribe()
    }

    pub fn publish(&self, event: UiEvent) {
        let _ = self.sender.send(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn publishes_events_to_subscribers() {
        let hub = UiEventHub::new();
        let mut receiver = hub.subscribe();
        let memory_id = Uuid::new_v4();

        hub.publish(UiEvent::MemoryCreated {
            agent_key: "test-agent".to_string(),
            memory_id,
        });

        assert_eq!(
            receiver.recv().await.unwrap(),
            UiEvent::MemoryCreated {
                agent_key: "test-agent".to_string(),
                memory_id,
            }
        );
    }
}
