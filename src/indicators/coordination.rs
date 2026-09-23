use std::{sync::Arc, time::Duration};

use tokio::sync::Notify;

#[derive(Clone, Default)]
pub struct IndicatorQueueNotifier(Arc<Notify>);

impl IndicatorQueueNotifier {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn notify(&self) {
        self.0.notify_one();
    }

    pub async fn notified(&self) {
        self.0.notified().await;
    }
}

static QUEUE_NOTIFIER: std::sync::OnceLock<IndicatorQueueNotifier> = std::sync::OnceLock::new();
static COMPLETION_NOTIFIER: std::sync::OnceLock<IndicatorQueueNotifier> =
    std::sync::OnceLock::new();
static ANALYSIS_WAIT_TIMEOUT_SECONDS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(30);

pub fn install_queue_notifier(notifier: IndicatorQueueNotifier) {
    let _ = QUEUE_NOTIFIER.set(notifier);
    let _ = COMPLETION_NOTIFIER.set(IndicatorQueueNotifier::new());
}

pub fn notify_completion() {
    if let Some(notifier) = COMPLETION_NOTIFIER.get() {
        notifier.0.notify_waiters();
    }
}

pub async fn completion_notified() {
    if let Some(notifier) = COMPLETION_NOTIFIER.get() {
        notifier.notified().await;
    } else {
        std::future::pending().await
    }
}

pub fn notify_queue() {
    if let Some(notifier) = QUEUE_NOTIFIER.get() {
        notifier.notify();
    }
}

pub fn set_analysis_wait_timeout(timeout: Duration) {
    ANALYSIS_WAIT_TIMEOUT_SECONDS.store(timeout.as_secs(), std::sync::atomic::Ordering::Relaxed);
}

pub fn analysis_wait_timeout() -> Duration {
    Duration::from_secs(ANALYSIS_WAIT_TIMEOUT_SECONDS.load(std::sync::atomic::Ordering::Relaxed))
}
