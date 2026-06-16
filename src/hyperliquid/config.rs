use std::{env, str::FromStr};

use alloy::signers::local::PrivateKeySigner;
use anyhow::{Context, Result, bail};

/// Hyperliquid network environment.
///
/// Only `Mainnet` is supported today. Testnet support was intentionally
/// removed (see plan: remove-nautilustrader-hypersdk-instruments.md). To
/// re-add testnet later:
///   1. Add a `Testnet` variant here.
///   2. Add `"testnet"`/`"sandbox"` parse aliases in `FromStr`.
///   3. Return a distinct journal string (e.g. `"sandbox"`) in `as_journal_str`
///      and widen the DB CHECK constraints in migration 0001 accordingly.
///   4. Branch on the variant in `raw_http.rs`, `live_ws.rs`, and
///      `instruments.rs` to select the testnet client/URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HyperliquidEnvironment {
    Mainnet,
}

impl HyperliquidEnvironment {
    pub fn as_journal_str(self) -> &'static str {
        match self {
            Self::Mainnet => "live",
        }
    }
}

impl FromStr for HyperliquidEnvironment {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "mainnet" | "live" => Ok(Self::Mainnet),
            other => {
                bail!("unsupported HYPERLIQUID_ENVIRONMENT '{other}' (only mainnet is supported)")
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct AccountSyncConfig {
    pub account_address: String,
    pub environment: HyperliquidEnvironment,
    pub history_start_ms: u64,
    pub overlap_ms: u64,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AppConfig {
    pub database_url: String,
    pub environment: HyperliquidEnvironment,
    pub account_address: String,
    pub history_start_ms: u64,
    pub poll_once: bool,
}

impl AppConfig {
    #[allow(dead_code)]
    pub fn to_account_sync_config(&self) -> AccountSyncConfig {
        AccountSyncConfig {
            account_address: self.account_address.clone(),
            environment: self.environment,
            history_start_ms: self.history_start_ms,
            overlap_ms: 300_000,
        }
    }
}

impl AppConfig {
    #[allow(dead_code)]
    pub fn from_env() -> Result<Self> {
        let private_key =
            env::var("HYPERLIQUID_PK").context("missing HYPERLIQUID_PK environment variable")?;
        let database_url = database_url_from_env()?;
        let environment = env::var("HYPERLIQUID_ENVIRONMENT")
            .unwrap_or_else(|_| "live".to_string())
            .parse()?;
        let history_start_ms = env::var("HYPERLIQUID_HISTORY_START_MS")
            .ok()
            .map(|value| value.parse())
            .transpose()
            .context("failed to parse HYPERLIQUID_HISTORY_START_MS")?
            .unwrap_or(0);
        let poll_once = env::var("HYPERLIQUID_POLL_ONCE")
            .ok()
            .map(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes"
                )
            })
            .unwrap_or(true);
        let account_address = derive_account_address(&private_key)?;

        Ok(Self {
            database_url,
            environment,
            account_address,
            history_start_ms,
            poll_once,
        })
    }
}

#[allow(dead_code)]
fn database_url_from_env() -> Result<String> {
    crate::config::database_url_from_env()
}

#[allow(dead_code)]
pub fn derive_account_address(private_key: &str) -> Result<String> {
    let signer = PrivateKeySigner::from_str(private_key)
        .context("failed to parse HYPERLIQUID_PK as an Ethereum private key")?;
    Ok(signer.address().to_string().to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::{HyperliquidEnvironment, derive_account_address};

    #[test]
    fn environment_parsing_accepts_live_and_mainnet() {
        assert_eq!(
            HyperliquidEnvironment::from_str("live").unwrap(),
            HyperliquidEnvironment::Mainnet
        );
        assert_eq!(
            HyperliquidEnvironment::from_str("mainnet").unwrap(),
            HyperliquidEnvironment::Mainnet
        );
    }

    #[test]
    fn environment_parsing_rejects_testnet() {
        assert!(HyperliquidEnvironment::from_str("sandbox").is_err());
        assert!(HyperliquidEnvironment::from_str("testnet").is_err());
        assert!(HyperliquidEnvironment::from_str("paper").is_err());
    }

    #[test]
    fn environment_journal_str_uses_live() {
        assert_eq!(HyperliquidEnvironment::Mainnet.as_journal_str(), "live");
    }

    #[test]
    fn derives_eth_address_from_private_key() {
        let address = derive_account_address(
            "4c0883a69102937d6231471b5dbb6204fe5129617082795f9d3d2c7e2f9f3f5b",
        )
        .expect("private key should parse");

        assert_eq!(address, "0x8f0bb61c41988b44f623a0b5390fd2b52838d20e");
    }
}
