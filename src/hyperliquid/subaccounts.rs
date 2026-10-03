use std::{collections::HashSet, sync::Arc};

use alloy::primitives::Address;
use dashmap::DashMap;
use hypersdk::hypercore::{ApiError, HttpClient, PrivateKeySigner};
use tokio::sync::Mutex;
use tracing::warn;

#[derive(Debug, thiserror::Error)]
pub enum CreateSubaccountError {
    #[error("Hyperliquid account data is temporarily unavailable.")]
    LookupUnavailable,
    #[error("{0}")]
    Rejected(String),
    #[error(
        "Sub-Account creation could not be confirmed. Refresh the account list before retrying."
    )]
    Unconfirmed,
}

#[cfg(test)]
pub(crate) mod tests;

#[derive(Debug, PartialEq, Eq)]
pub struct CreatedSubaccount {
    pub address: Address,
    pub created: bool,
}

/// Serializes creation for each owner and remembers uncertain submissions for
/// the lifetime of the server. A cancelled request also remains uncertain.
pub struct SubaccountCreator {
    client: HttpClient,
    pending: DashMap<Address, Arc<Mutex<HashSet<String>>>>,
}

impl SubaccountCreator {
    pub fn mainnet() -> Self {
        Self::new(hypersdk::hypercore::mainnet())
    }

    pub fn new(client: HttpClient) -> Self {
        Self {
            client,
            pending: DashMap::new(),
        }
    }

    async fn discover(&self, owner: Address, name: &str) -> anyhow::Result<Option<Address>> {
        Ok(self
            .client
            .subaccounts(owner)
            .await?
            .into_iter()
            .find(|account| account.master == owner && account.name == name)
            .map(|account| account.sub_account_user))
    }

    pub async fn create(
        &self,
        signer: &PrivateKeySigner,
        owner: Address,
        name: &str,
    ) -> Result<CreatedSubaccount, CreateSubaccountError> {
        let owner_pending = Arc::clone(self.pending.entry(owner).or_default().value());
        let mut pending = owner_pending.lock().await;
        let existing = self.discover(owner, name).await.map_err(|error| {
            warn!(%owner, subaccount_name = name, %error, "subaccount discovery failed");
            if pending.contains(name) {
                CreateSubaccountError::Unconfirmed
            } else {
                CreateSubaccountError::LookupUnavailable
            }
        })?;
        if let Some(address) = existing {
            pending.remove(name);
            return Ok(CreatedSubaccount {
                address,
                created: false,
            });
        }
        if pending.contains(name) {
            return Err(CreateSubaccountError::Unconfirmed);
        }

        // Record this before awaiting submission so cancellation cannot enable
        // another request to blindly submit the same creation.
        pending.insert(name.to_string());
        let nonce = u64::try_from(chrono::Utc::now().timestamp_millis())
            .expect("current timestamp is after the Unix epoch");
        match self
            .client
            .create_sub_account(signer, name.to_string(), nonce, None)
            .await
        {
            Ok(address) => {
                pending.remove(name);
                Ok(CreatedSubaccount {
                    address,
                    created: true,
                })
            }
            Err(error) => {
                // hypersdk 0.2.15 uses ApiError for explicit exchange rejections,
                // HTTP failures, and unexpected decoded responses. Only the
                // first category definitively means creation was rejected.
                if let Some(ApiError(message)) = error.downcast_ref::<ApiError>()
                    && !message.starts_with("HTTP ")
                    && !message.starts_with("unexpected response:")
                {
                    pending.remove(name);
                    warn!(%owner, subaccount_name = name, %error, "Hyperliquid rejected new Sub-Account creation");
                    return Err(CreateSubaccountError::Rejected(message.clone()));
                }
                warn!(%owner, subaccount_name = name, %error, "uncertain Sub-Account creation result; reconciling");
                if let Ok(Some(address)) = self.discover(owner, name).await {
                    pending.remove(name);
                    return Ok(CreatedSubaccount {
                        address,
                        created: true,
                    });
                }
                Err(CreateSubaccountError::Unconfirmed)
            }
        }
    }
}
