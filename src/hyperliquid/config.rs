use std::{env, str::FromStr};

use alloy::signers::local::PrivateKeySigner;
use anyhow::{Context, Result, bail};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HyperliquidEnvironment {
    Mainnet,
    Testnet,
}

impl HyperliquidEnvironment {
    pub fn as_journal_str(self) -> &'static str {
        match self {
            Self::Mainnet => "mainnet",
            Self::Testnet => "testnet",
        }
    }

    pub fn as_nt_environment(self) -> nautilus_hyperliquid::common::enums::HyperliquidEnvironment {
        match self {
            Self::Mainnet => nautilus_hyperliquid::common::enums::HyperliquidEnvironment::Mainnet,
            Self::Testnet => nautilus_hyperliquid::common::enums::HyperliquidEnvironment::Testnet,
        }
    }
}

impl FromStr for HyperliquidEnvironment {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "mainnet" | "live" => Ok(Self::Mainnet),
            "testnet" | "sandbox" | "paper" => Ok(Self::Testnet),
            other => bail!("unsupported HYPERLIQUID_ENVIRONMENT '{other}'"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub database_url: String,
    pub environment: HyperliquidEnvironment,
    pub account_address: String,
    pub history_start_ms: u64,
    pub poll_once: bool,
}

impl AppConfig {
    pub fn from_env() -> Result<Self> {
        let private_key = env::var("HYPERLIQUID_PK")
            .context("missing HYPERLIQUID_PK environment variable")?;
        let database_url = database_url_from_env()?;
        let environment = env::var("HYPERLIQUID_ENVIRONMENT")
            .unwrap_or_else(|_| "mainnet".to_string())
            .parse()?;
        let history_start_ms = env::var("HYPERLIQUID_HISTORY_START_MS")
            .ok()
            .map(|value| value.parse())
            .transpose()
            .context("failed to parse HYPERLIQUID_HISTORY_START_MS")?
            .unwrap_or(0);
        let poll_once = env::var("HYPERLIQUID_POLL_ONCE")
            .ok()
            .map(|value| matches!(value.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
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

fn database_url_from_env() -> Result<String> {
    if let Ok(database_url) = env::var("DATABASE_URL") {
        if !database_url.trim().is_empty() {
            return Ok(database_url);
        }
    }

    let host = env::var("POSTGRES_HOST").unwrap_or_else(|_| "localhost".to_string());
    let port = env::var("POSTGRES_PORT").unwrap_or_else(|_| "5432".to_string());
    let user = env::var("POSTGRES_USER").unwrap_or_else(|_| "vibetrading".to_string());
    let password = env::var("POSTGRES_PASSWORD").unwrap_or_else(|_| "vibetrading".to_string());
    let database = env::var("POSTGRES_DB").unwrap_or_else(|_| "vibetrading".to_string());

    if host.trim().is_empty() || port.trim().is_empty() || user.trim().is_empty() || database.trim().is_empty() {
        bail!("postgres environment variables are incomplete")
    }

    Ok(format!(
        "postgres://{}:{}@{}:{}/{}",
        user, password, host, port, database
    ))
}

pub fn derive_account_address(private_key: &str) -> Result<String> {
    let signer = PrivateKeySigner::from_str(private_key)
        .context("failed to parse HYPERLIQUID_PK as an Ethereum private key")?;
    Ok(signer.address().to_string().to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::derive_account_address;

    #[test]
    fn derives_eth_address_from_private_key() {
        let address = derive_account_address(
            "4c0883a69102937d6231471b5dbb6204fe5129617082795f9d3d2c7e2f9f3f5b",
        )
        .expect("private key should parse");

        assert_eq!(address, "0x8f0bb61c41988b44f623a0b5390fd2b52838d20e");
    }
}
