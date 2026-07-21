use std::str::FromStr;

use alloy::signers::local::PrivateKeySigner;
use anyhow::{Context, Result};
use rand::RngExt;

/// Generate a fresh cryptographically random Ethereum private key.
pub fn generate_wallet_private_key() -> String {
    loop {
        let mut bytes = [0u8; 32];
        rand::rng().fill(&mut bytes);
        let private_key = format!("0x{}", hex::encode(bytes));
        if derive_wallet_address(&private_key).is_ok() {
            return private_key;
        }
    }
}

/// Derive a checksummed Ethereum address from a private key string and return
/// it in normalized lowercase form.
///
/// The input is trimmed and may be provided with or without a `0x` prefix.
pub fn derive_wallet_address(private_key: &str) -> Result<String> {
    let signer = PrivateKeySigner::from_str(private_key.trim())
        .context("failed to parse Hyperliquid private key")?;
    Ok(signer.address().to_string().to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_known_eth_address() {
        // Test vector borrowed from the Hyperliquid journal configuration tests.
        let address = derive_wallet_address(
            "4c0883a69102937d6231471b5dbb6204fe5129617082795f9d3d2c7e2f9f3f5b",
        )
        .expect("private key should parse");

        assert_eq!(address, "0x8f0bb61c41988b44f623a0b5390fd2b52838d20e");
    }

    #[test]
    fn accepts_0x_prefixed_private_key() {
        let address = derive_wallet_address(
            "0x4c0883a69102937d6231471b5dbb6204fe5129617082795f9d3d2c7e2f9f3f5b",
        )
        .expect("private key should parse");

        assert_eq!(address, "0x8f0bb61c41988b44f623a0b5390fd2b52838d20e");
    }

    #[test]
    fn rejects_invalid_private_key() {
        assert!(derive_wallet_address("not-a-private-key").is_err());
    }

    #[test]
    fn generated_private_keys_are_valid_and_fresh() {
        let first = generate_wallet_private_key();
        let second = generate_wallet_private_key();

        assert!(derive_wallet_address(&first).is_ok());
        assert!(derive_wallet_address(&second).is_ok());
        assert_ne!(first, second);
    }
}
