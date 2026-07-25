use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use tokio::sync::Mutex;
use uuid::Uuid;

use crate::opencode::client::OpenCodeOAuthCompletionMode;

pub const OAUTH_ATTEMPT_TTL: Duration = Duration::from_secs(15 * 60);

#[derive(Clone)]
pub struct ProviderConnectionsState {
    attempts: Arc<Mutex<HashMap<String, ProviderConnection>>>,
}

impl Default for ProviderConnectionsState {
    fn default() -> Self {
        Self::new()
    }
}

impl ProviderConnectionsState {
    pub fn new() -> Self {
        Self {
            attempts: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn get(&self, provider_id: &str) -> Option<OAuthAttempt> {
        let mut attempts = self.attempts.lock().await;
        prune_expired(&mut attempts);
        match attempts.get(provider_id) {
            Some(ProviderConnection::Pending(attempt)) => Some(attempt.clone()),
            Some(ProviderConnection::Reserving { .. }) | None => None,
        }
    }

    /// Reserve OpenCode's provider-global OAuth state before authorization.
    /// OpenCode permits only one authorization flow for each provider at once.
    pub async fn reserve(&self, provider_id: &str, user_id: Uuid) -> bool {
        let mut attempts = self.attempts.lock().await;
        prune_expired(&mut attempts);
        if attempts.contains_key(provider_id) {
            return false;
        }
        attempts.insert(
            provider_id.to_string(),
            ProviderConnection::Reserving {
                user_id,
                expires_at: Instant::now() + OAUTH_ATTEMPT_TTL,
            },
        );
        true
    }

    pub async fn complete_reservation(&self, attempt: OAuthAttempt) -> Result<(), OAuthAttempt> {
        let mut attempts = self.attempts.lock().await;
        prune_expired(&mut attempts);
        let reserved_by_user = matches!(
            attempts.get(&attempt.provider_id),
            Some(ProviderConnection::Reserving { user_id, .. }) if *user_id == attempt.user_id
        );
        if !reserved_by_user {
            return Err(attempt);
        }
        attempts.insert(
            attempt.provider_id.clone(),
            ProviderConnection::Pending(attempt),
        );
        Ok(())
    }

    pub async fn release_reservation(&self, provider_id: &str, user_id: Uuid) {
        let mut attempts = self.attempts.lock().await;
        prune_expired(&mut attempts);
        if matches!(
            attempts.get(provider_id),
            Some(ProviderConnection::Reserving { user_id: reserved_by, .. }) if *reserved_by == user_id
        ) {
            attempts.remove(provider_id);
        }
    }

    pub async fn remove_if_matches(&self, provider_id: &str, attempt: &OAuthAttempt) -> bool {
        let mut attempts = self.attempts.lock().await;
        prune_expired(&mut attempts);
        if matches!(attempts.get(provider_id), Some(ProviderConnection::Pending(current)) if current == attempt)
        {
            attempts.remove(provider_id);
            true
        } else {
            false
        }
    }

    pub async fn cancel_for_user(&self, provider_id: &str, user_id: Uuid) -> bool {
        let mut attempts = self.attempts.lock().await;
        prune_expired(&mut attempts);
        let owned_by_user = match attempts.get(provider_id) {
            Some(ProviderConnection::Reserving {
                user_id: reserved_by,
                ..
            }) => *reserved_by == user_id,
            Some(ProviderConnection::Pending(attempt)) => attempt.user_id == user_id,
            None => false,
        };
        if owned_by_user {
            attempts.remove(provider_id);
        }
        owned_by_user
    }

    pub async fn remove(&self, provider_id: &str) {
        let mut attempts = self.attempts.lock().await;
        prune_expired(&mut attempts);
        attempts.remove(provider_id);
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct OAuthAttempt {
    pub user_id: Uuid,
    pub provider_id: String,
    pub method: usize,
    pub directory: String,
    pub completion_mode: OpenCodeOAuthCompletionMode,
    pub authorization_url: String,
    pub instructions: String,
    pub created_at: Instant,
    pub expires_at: Instant,
}

#[derive(Clone)]
enum ProviderConnection {
    Reserving { user_id: Uuid, expires_at: Instant },
    Pending(OAuthAttempt),
}

impl OAuthAttempt {
    pub fn new(
        user_id: Uuid,
        provider_id: String,
        method: usize,
        directory: String,
        completion_mode: OpenCodeOAuthCompletionMode,
        authorization_url: String,
        instructions: String,
    ) -> Self {
        let created_at = Instant::now();
        Self {
            user_id,
            provider_id,
            method,
            directory,
            completion_mode,
            authorization_url,
            instructions,
            created_at,
            expires_at: created_at + OAUTH_ATTEMPT_TTL,
        }
    }
}

fn prune_expired(attempts: &mut HashMap<String, ProviderConnection>) {
    let now = Instant::now();
    attempts.retain(|_, attempt| match attempt {
        ProviderConnection::Reserving { expires_at, .. } => *expires_at > now,
        ProviderConnection::Pending(attempt) => attempt.expires_at > now,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reservation_prevents_concurrent_oauth_authorization() {
        let state = ProviderConnectionsState::new();
        let first_user = Uuid::new_v4();
        let second_user = Uuid::new_v4();

        assert!(state.reserve("openai", first_user).await);
        assert!(!state.reserve("openai", second_user).await);

        let attempt = OAuthAttempt::new(
            first_user,
            "openai".to_string(),
            0,
            "/workspaces".to_string(),
            OpenCodeOAuthCompletionMode::Code,
            "https://example.test/authorize".to_string(),
            "Complete authorization".to_string(),
        );
        assert!(state.complete_reservation(attempt.clone()).await.is_ok());

        assert!(state.get("openai").await == Some(attempt));
    }

    #[tokio::test]
    async fn releasing_failed_authorization_allows_retry() {
        let state = ProviderConnectionsState::new();
        let user = Uuid::new_v4();

        assert!(state.reserve("openai", user).await);
        state.release_reservation("openai", user).await;

        assert!(state.reserve("openai", Uuid::new_v4()).await);
    }

    #[tokio::test]
    async fn cancelling_own_pending_attempt_allows_retry() {
        let state = ProviderConnectionsState::new();
        let user = Uuid::new_v4();
        let attempt = OAuthAttempt::new(
            user,
            "openai".to_string(),
            0,
            "/workspaces".to_string(),
            OpenCodeOAuthCompletionMode::Code,
            "https://example.test/authorize".to_string(),
            "Complete authorization".to_string(),
        );

        assert!(state.reserve("openai", user).await);
        assert!(state.complete_reservation(attempt).await.is_ok());
        assert!(state.cancel_for_user("openai", user).await);
        assert!(state.reserve("openai", Uuid::new_v4()).await);
    }

    #[tokio::test]
    async fn cancelling_another_users_attempt_is_rejected() {
        let state = ProviderConnectionsState::new();
        let owner = Uuid::new_v4();
        let attempt = OAuthAttempt::new(
            owner,
            "openai".to_string(),
            0,
            "/workspaces".to_string(),
            OpenCodeOAuthCompletionMode::Code,
            "https://example.test/authorize".to_string(),
            "Complete authorization".to_string(),
        );

        assert!(state.reserve("openai", owner).await);
        assert!(state.complete_reservation(attempt.clone()).await.is_ok());
        assert!(!state.cancel_for_user("openai", Uuid::new_v4()).await);
        assert!(state.get("openai").await == Some(attempt));
    }
}
