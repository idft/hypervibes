use std::{
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
};

use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};

/// Application-wide coordination for access to an agent's live workspace.
///
/// Read leases allow independent live jobs to run concurrently. A write
/// lease is used only for operations that can expose a new live tree, such
/// as regeneration or coding promotion. The lock is intentionally
/// keyed by agent so unrelated workspaces never contend.
#[derive(Clone, Default)]
pub struct WorkspaceLeaseManager {
    locks: Arc<Mutex<HashMap<String, Weak<RwLock<()>>>>>,
}

impl WorkspaceLeaseManager {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock_for(&self, agent_key: &str) -> Arc<RwLock<()>> {
        let mut locks = self.locks.lock().expect("workspace lease map poisoned");
        if let Some(lock) = locks.get(agent_key).and_then(Weak::upgrade) {
            return lock;
        }

        let lock = Arc::new(RwLock::new(()));
        locks.insert(agent_key.to_string(), Arc::downgrade(&lock));
        lock
    }

    pub async fn acquire_live_read(&self, agent_key: &str) -> OwnedRwLockReadGuard<()> {
        self.lock_for(agent_key).read_owned().await
    }

    pub async fn acquire_live_write(&self, agent_key: &str) -> OwnedRwLockWriteGuard<()> {
        self.lock_for(agent_key).write_owned().await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use tokio::time::{Duration, sleep, timeout};

    use super::WorkspaceLeaseManager;

    #[tokio::test]
    async fn reads_for_same_agent_can_run_concurrently() {
        let manager = WorkspaceLeaseManager::new();
        let first = manager.acquire_live_read("agent").await;
        let second = timeout(
            Duration::from_millis(50),
            manager.acquire_live_read("agent"),
        )
        .await
        .expect("read leases should not block each other");
        drop(first);
        drop(second);
    }

    #[tokio::test]
    async fn readers_for_different_agents_do_not_contend() {
        let manager = WorkspaceLeaseManager::new();
        let first = manager.acquire_live_read("agent-a").await;
        let second = timeout(
            Duration::from_millis(50),
            manager.acquire_live_read("agent-b"),
        )
        .await
        .expect("different agents should not contend");
        drop(first);
        drop(second);
    }

    #[tokio::test]
    async fn write_waits_for_readers_and_blocks_new_readers() {
        let manager = WorkspaceLeaseManager::new();
        let read = manager.acquire_live_read("agent").await;
        let writer_started = Arc::new(AtomicBool::new(false));
        let writer_started_task = Arc::clone(&writer_started);
        let writer_manager = manager.clone();
        let writer = tokio::spawn(async move {
            writer_started_task.store(true, Ordering::SeqCst);
            let _guard = writer_manager.acquire_live_write("agent").await;
            sleep(Duration::from_millis(40)).await;
        });

        while !writer_started.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
        assert!(
            timeout(
                Duration::from_millis(20),
                manager.acquire_live_read("agent")
            )
            .await
            .is_err(),
            "a reader must not enter while a writer is waiting"
        );
        drop(read);
        timeout(Duration::from_secs(1), writer)
            .await
            .expect("writer should acquire after reader release")
            .expect("writer task");
    }
}
