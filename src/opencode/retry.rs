use std::{future::Future, time::Duration};

use anyhow::Result;
use tokio::sync::watch;
use tracing::warn;

/// Retry a startup dependency until it becomes available or shutdown begins.
///
/// Startup ordering in Compose does not guarantee that a dependency is already
/// listening when this process starts, so connection failures are expected.
pub async fn retry_until_ready<T, Operation, OperationFuture>(
    operation_name: &str,
    retry_interval: Duration,
    shutdown_rx: &mut watch::Receiver<bool>,
    mut operation: Operation,
) -> Option<T>
where
    Operation: FnMut() -> OperationFuture,
    OperationFuture: Future<Output = Result<T>>,
{
    loop {
        if *shutdown_rx.borrow() {
            return None;
        }

        match operation().await {
            Ok(value) => return Some(value),
            Err(error) => {
                warn!(
                    operation = operation_name,
                    retry_in = ?retry_interval,
                    error = ?error,
                    "startup dependency unavailable; retrying"
                );
            }
        }

        tokio::select! {
            _ = tokio::time::sleep(retry_interval) => {}
            changed = shutdown_rx.changed() => {
                if changed.is_err() || *shutdown_rx.borrow() {
                    return None;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use anyhow::anyhow;
    use tokio::sync::watch;

    use super::retry_until_ready;

    #[tokio::test]
    async fn retries_until_startup_dependency_is_ready() {
        let (_shutdown_tx, mut shutdown_rx) = watch::channel(false);
        let mut attempts = 0;

        let result = retry_until_ready(
            "test dependency",
            Duration::from_millis(1),
            &mut shutdown_rx,
            || {
                attempts += 1;
                let attempt = attempts;
                async move {
                    if attempt < 3 {
                        Err(anyhow!("not ready"))
                    } else {
                        Ok("ready")
                    }
                }
            },
        )
        .await;

        assert_eq!(result, Some("ready"));
        assert_eq!(attempts, 3);
    }
}
