use std::{future::Future, pin::Pin};

use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::{Value, json};

/// The only referral code HyperVibes offers to eligible users.
pub const REFERRAL_CODE: &str = "HYPERVIBES";
pub const MAX_REFERRAL_APPLICATION_VOLUME: Decimal = Decimal::from_parts(10_000, 0, 0, false, 0);

/// Ordered fields are required because Hyperliquid hashes the MessagePack bytes.
#[derive(Serialize)]
struct SetReferrerAction {
    #[serde(rename = "type")]
    action_type: &'static str,
    code: &'static str,
}

pub(crate) fn set_referrer_action_bytes() -> anyhow::Result<Vec<u8>> {
    rmp_serde::to_vec_named(&SetReferrerAction {
        action_type: "setReferrer",
        code: REFERRAL_CODE,
    })
    .map_err(Into::into)
}

pub fn set_referrer_action() -> Value {
    json!({"type": "setReferrer", "code": REFERRAL_CODE})
}

pub type ReferralFuture<'a> = Pin<Box<dyn Future<Output = Result<Value, String>> + Send + 'a>>;

/// The small signed and unsigned surface required for referral application.
pub trait ReferralExchange: Send + Sync {
    fn referral_state<'a>(&'a self, user: &'a str) -> ReferralFuture<'a>;
    fn relay_set_referrer<'a>(
        &'a self,
        action: &'a Value,
        nonce: u64,
        signature: Value,
    ) -> ReferralFuture<'a>;
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

    fn relay_set_referrer<'a>(
        &'a self,
        action: &'a Value,
        nonce: u64,
        signature: Value,
    ) -> ReferralFuture<'a> {
        Box::pin(async move {
            self.client
                .post("https://api.hyperliquid.xyz/exchange")
                .json(&json!({"action": action, "nonce": nonce, "signature": signature}))
                .send()
                .await
                .map_err(|_| "Hyperliquid exchange is temporarily unavailable.".to_string())?
                .json()
                .await
                .map_err(|_| "Hyperliquid exchange is temporarily unavailable.".to_string())
        })
    }
}
