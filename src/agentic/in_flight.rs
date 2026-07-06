use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use tokio::sync::Notify;

/// Shared counter of in-flight agentic dispatches, used to drain work
/// on graceful shutdown.
///
/// Every `tokio::spawn` that dispatches an agentic run takes a guard
/// via [`InFlightTracker::track`]. The guard increments the counter on
/// construction and decrements it (notifying any waiters if it was
/// the last one) on `Drop`. The scheduler's `run` method (and `main`)
/// call [`InFlightTracker::wait_idle`] after the shutdown signal to
/// ensure every spawned task has finished before the process exits.
///
/// Cheap: a single `fetch_add`/`fetch_sub` per dispatch, no locks.
#[derive(Clone, Debug, Default)]
pub struct InFlightTracker {
    inner: Arc<InFlightInner>,
}

#[derive(Debug)]
struct InFlightInner {
    count: AtomicUsize,
    idle: Notify,
}

impl Default for InFlightInner {
    fn default() -> Self {
        Self {
            count: AtomicUsize::new(0),
            idle: Notify::new(),
        }
    }
}

impl InFlightTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Increment the in-flight count and return a guard that
    /// decrements on `Drop`. Move the guard into the spawned task so
    /// the decrement fires when the task body finishes.
    pub fn track(&self) -> InFlightGuard {
        self.inner.count.fetch_add(1, Ordering::SeqCst);
        InFlightGuard {
            inner: Arc::clone(&self.inner),
        }
    }

    /// Current in-flight count (for diagnostics/tests).
    pub fn in_flight(&self) -> usize {
        self.inner.count.load(Ordering::SeqCst)
    }

    /// Wait until every guard has been dropped.
    pub async fn wait_idle(&self) {
        while self.inner.count.load(Ordering::SeqCst) > 0 {
            // Re-check inside the loop: a guard may have dropped
            // between the load and the await, in which case the
            // `notified()` future would otherwise miss the wakeup.
            self.inner.idle.notified().await;
        }
    }

    /// Wait up to `timeout` for the tracker to drain. Returns `true`
    /// if it drained, `false` if the timeout elapsed first.
    pub async fn wait_idle_with_timeout(&self, timeout: Duration) -> bool {
        match tokio::time::timeout(timeout, self.wait_idle()).await {
            Ok(()) => true,
            Err(_) => false,
        }
    }
}

/// RAII guard that decrements the parent [`InFlightTracker`] on drop.
#[must_use = "the in-flight tracker guard must be held for the duration of the task; dropping it early will mark the task as complete"]
pub struct InFlightGuard {
    inner: Arc<InFlightInner>,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        if self.inner.count.fetch_sub(1, Ordering::SeqCst) == 1 {
            // We were the last in-flight task; wake any waiters.
            self.inner.idle.notify_waiters();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn track_increments_and_guard_drop_decrements() {
        let tracker = InFlightTracker::new();
        assert_eq!(tracker.in_flight(), 0);

        let g1 = tracker.track();
        assert_eq!(tracker.in_flight(), 1);

        let g2 = tracker.track();
        assert_eq!(tracker.in_flight(), 2);

        drop(g1);
        assert_eq!(tracker.in_flight(), 1);

        drop(g2);
        assert_eq!(tracker.in_flight(), 0);
    }

    #[tokio::test]
    async fn wait_idle_resolves_immediately_when_empty() {
        let tracker = InFlightTracker::new();
        // Should not block.
        tokio::time::timeout(Duration::from_millis(50), tracker.wait_idle())
            .await
            .expect("wait_idle should resolve immediately when count is 0");
    }

    #[tokio::test]
    async fn wait_idle_blocks_until_last_guard_drops() {
        let tracker = InFlightTracker::new();
        let g1 = tracker.track();
        let g2 = tracker.track();

        let tracker_for_wait = tracker.clone();
        let waiter = tokio::spawn(async move {
            tracker_for_wait.wait_idle().await;
        });

        // Give the waiter a moment to start blocking.
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!waiter.is_finished(), "waiter should still be blocked");

        drop(g1);
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(
            !waiter.is_finished(),
            "waiter should still be blocked with one guard remaining"
        );

        drop(g2);
        // The waiter should now be unblocked.
        tokio::time::timeout(Duration::from_millis(100), waiter)
            .await
            .expect("waiter should resolve after the last guard drops")
            .expect("waiter task should not panic");
    }

    #[tokio::test]
    async fn wait_idle_with_timeout_returns_false_when_guards_persist() {
        let tracker = InFlightTracker::new();
        let _guard = tracker.track();

        let drained = tracker
            .wait_idle_with_timeout(Duration::from_millis(30))
            .await;
        assert!(!drained, "wait should have timed out with a guard held");
        assert_eq!(tracker.in_flight(), 1);
    }

    #[tokio::test]
    async fn wait_idle_with_timeout_returns_true_after_drain() {
        let tracker = InFlightTracker::new();
        let guard = tracker.track();
        let tracker_for_wait = tracker.clone();
        let waiter = tokio::spawn(async move {
            tracker_for_wait
                .wait_idle_with_timeout(Duration::from_secs(1))
                .await
        });

        tokio::time::sleep(Duration::from_millis(20)).await;
        drop(guard);
        let drained = waiter.await.expect("waiter task should not panic");
        assert!(drained);
    }

    #[tokio::test]
    async fn concurrent_tracks_and_drops_drain_correctly() {
        let tracker = InFlightTracker::new();
        let mut handles = Vec::new();
        for _ in 0..50 {
            let t = tracker.clone();
            handles.push(tokio::spawn(async move {
                let _g = t.track();
                tokio::time::sleep(Duration::from_millis(5)).await;
            }));
        }
        tracker.wait_idle().await;
        for h in handles {
            h.await.expect("task should not panic");
        }
        assert_eq!(tracker.in_flight(), 0);
    }

    #[tokio::test]
    async fn tracker_is_clone_shares_count() {
        let t1 = InFlightTracker::new();
        let t2 = t1.clone();
        let g1 = t1.track();
        let g2 = t2.track();
        assert_eq!(t1.in_flight(), 2);
        assert_eq!(t2.in_flight(), 2);
        drop(g1);
        assert_eq!(t1.in_flight(), 1);
        assert_eq!(t2.in_flight(), 1);
        drop(g2);
    }
}
