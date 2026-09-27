use std::{future::Future, pin::Pin};

use rust_decimal::Decimal;
use serde_json::{Value, json};

/// The only referral code HyperVibes offers to eligible users.
pub const REFERRAL_CODE: &str = "HYPERVIBES";
pub const MAX_REFERRAL_APPLICATION_VOLUME: Decimal = Decimal::from_parts(10_000, 0, 0, false, 0);

pub type ReferralFuture<'a> = Pin<Box<dyn Future<Output = Result<Value, String>> + Send + 'a>>;

/// Read-only referral state used to offer the Hyperliquid referral link.
pub trait ReferralExchange: Send + Sync {
    fn referral_state<'a>(&'a self, user: &'a str) -> ReferralFuture<'a>;
}

pub struct HyperliquidReferralExchange {
    client: reqwest::Client,
}

impl HyperliquidReferralExchange {
    pub fn mainnet() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

impl ReferralExchange for HyperliquidReferralExchange {
    fn referral_state<'a>(&'a self, user: &'a str) -> ReferralFuture<'a> {
        Box::pin(async move {
            self.client
                .post("https://api.hyperliquid.xyz/info")
                .json(&json!({"type": "referral", "user": user}))
                .send()
                .await
                .map_err(|_| "Hyperliquid referral data is temporarily unavailable.".to_string())?
                .error_for_status()
                .map_err(|_| "Hyperliquid referral data is temporarily unavailable.".to_string())?
                .json()
                .await
                .map_err(|_| "Hyperliquid referral data is temporarily unavailable.".to_string())
        })
    }
}
