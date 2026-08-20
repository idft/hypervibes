use alloy::primitives::B256;
use anyhow::{Context, Result};
use serde::Serialize;

use super::referral::{set_referrer_action, set_referrer_action_bytes};

/// The exact field order used by Hyperliquid's `createSubAccount` L1 action.
#[derive(Debug, Serialize)]
struct CreateSubAccountAction<'a> {
    #[serde(rename = "type")]
    action_type: &'static str,
    name: &'a str,
}

/// Hash a Hyperliquid L1 action using its MessagePack wire representation.
///
/// Hyperliquid appends the nonce as a big-endian uint64 and a zero byte for
/// the absent vault address before hashing. The encoding is kept here rather
/// than in browser code so every server-side L1 action uses one implementation.
pub fn create_subaccount_action_hash(name: &str, nonce: u64) -> Result<B256> {
    let action = CreateSubAccountAction {
        action_type: "createSubAccount",
        name,
    };
    let mut bytes =
        rmp_serde::to_vec_named(&action).context("failed to encode createSubAccount action")?;
    bytes.extend(nonce.to_be_bytes());
    bytes.push(0);
    Ok(alloy::primitives::keccak256(bytes))
}

/// Hash Hyperliquid's fixed `setReferrer` L1 action for a main-wallet signature.
pub fn set_referrer_action_hash(nonce: u64) -> Result<B256> {
    let mut bytes = set_referrer_action_bytes().context("failed to encode setReferrer action")?;
    bytes.extend(nonce.to_be_bytes());
    bytes.push(0);
    Ok(alloy::primitives::keccak256(bytes))
}

/// Build the canonical L1 Agent typed-data payload for browser wallet signing.
pub fn set_referrer_signing_payload(nonce: u64) -> Result<serde_json::Value> {
    let hash = set_referrer_action_hash(nonce)?;
    Ok(serde_json::json!({
        "action": set_referrer_action(),
        "nonce": nonce,
        "domain": {
            "name": "Exchange",
            "version": "1",
            "chainId": 1337,
            "verifyingContract": "0x0000000000000000000000000000000000000000"
        },
        "primaryType": "Agent",
        "types": {
            "Agent": [
                {"name": "source", "type": "string"},
                {"name": "connectionId", "type": "bytes32"}
            ]
        },
        "message": {"source": "a", "connectionId": hash}
    }))
}

/// Sign a `createSubAccount` L1 action with a server-held Hyperliquid signer.
pub async fn sign_create_subaccount(
    signer: &hypersdk::hypercore::PrivateKeySigner,
    name: &str,
    nonce: u64,
) -> Result<(B256, hypersdk::hypercore::types::Signature)> {
    let hash = create_subaccount_action_hash(name, nonce)?;
    let signature = hypersdk::hypercore::signing::sign_l1_action(
        signer,
        hypersdk::hypercore::Chain::Mainnet,
        hash,
    )
    .await
    .context("failed to sign createSubAccount action")?;
    Ok((hash, signature))
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use alloy::signers::local::PrivateKeySigner;

    use super::*;

    #[tokio::test]
    async fn create_subaccount_signature_recovers_the_server_signer() {
        let signer = PrivateKeySigner::from_str(
            "4c0883a69102937d6231471b5dbb6204fe5129617082795f9d3d2c7e2f9f3f5b",
        )
        .expect("test signer");
        let nonce = 1_752_886_867_123u64;
        let (hash, signature) = sign_create_subaccount(&signer, "HyperVibes - BTC", nonce)
            .await
            .expect("sign action");
        let recovered = signature
            .to_string()
            .parse::<alloy::primitives::Signature>()
            .expect("signature format")
            .recover_address_from_prehash(&hypersdk::hypercore::signing::agent_signing_hash(
                hypersdk::hypercore::Chain::Mainnet,
                hash,
            ))
            .expect("recover signer");
        assert_eq!(recovered, signer.address());
        assert!(
            serde_json::to_value(signature)
                .expect("serialize signature")
                .is_object()
        );
    }

    #[tokio::test]
    async fn set_referrer_hash_and_agent_digest_recover_main_wallet() {
        let signer = PrivateKeySigner::from_str(
            "4c0883a69102937d6231471b5dbb6204fe5129617082795f9d3d2c7e2f9f3f5b",
        )
        .expect("test signer");
        let nonce = 1_752_886_867_123u64;
        let hash = set_referrer_action_hash(nonce).expect("hash action");
        assert_eq!(
            hex::encode(hash),
            "41dd55b9daee2c5bf9f55d4345c362ef5a7231d2cf911983ba628a3092739fbb"
        );
        let signature = hypersdk::hypercore::signing::sign_l1_action(
            &signer,
            hypersdk::hypercore::Chain::Mainnet,
            hash,
        )
        .await
        .expect("sign action");
        let digest = hypersdk::hypercore::signing::agent_signing_hash(
            hypersdk::hypercore::Chain::Mainnet,
            hash,
        );
        let recovered = signature
            .to_string()
            .parse::<alloy::primitives::Signature>()
            .expect("signature format")
            .recover_address_from_prehash(&digest)
            .expect("recover signer");
        assert_eq!(recovered, signer.address());
    }
}
