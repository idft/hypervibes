use aes_gcm::{
    Aes256Gcm,
    aead::{Aead, KeyInit},
};
use anyhow::{Context, Result, ensure};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::RngExt;

const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;

/// Master encryption key used to protect agent Hyperliquid private keys.
#[derive(Debug, Clone)]
pub struct EncryptionKey {
    pub key_id: String,
    pub bytes: [u8; KEY_LEN],
}

impl EncryptionKey {
    /// Parse a 32-byte (64 hex character) master key.
    #[cfg(test)]
    pub fn from_hex(key_id: impl Into<String>, hex_str: &str) -> Result<Self> {
        let bytes =
            hex::decode(hex_str.trim()).context("failed to decode agents encryption key as hex")?;
        ensure!(
            bytes.len() == KEY_LEN,
            "agents encryption key must decode to exactly {} bytes (got {})",
            KEY_LEN,
            bytes.len()
        );
        let mut arr = [0u8; KEY_LEN];
        arr.copy_from_slice(&bytes);
        Ok(Self {
            key_id: key_id.into(),
            bytes: arr,
        })
    }

    /// Build directly from bytes for callers that already parsed the key.
    pub fn new(key_id: impl Into<String>, bytes: [u8; KEY_LEN]) -> Self {
        Self {
            key_id: key_id.into(),
            bytes,
        }
    }
}

/// Encrypt a plaintext private key string into a ciphertext blob.
///
/// The returned bytes contain the 12-byte nonce followed by the AEAD ciphertext
/// (which includes the authentication tag). This single blob is what should be
/// stored in `hyperliquid_private_key_ciphertext`.
pub fn encrypt(key: &EncryptionKey, plaintext: &str) -> Result<Vec<u8>> {
    let cipher =
        Aes256Gcm::new_from_slice(&key.bytes).context("failed to initialize AES-256-GCM cipher")?;

    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::rng().fill(&mut nonce_bytes);
    let nonce: aes_gcm::Nonce<aes_gcm::aead::consts::U12> = nonce_bytes.into();
    let ciphertext = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .map_err(|e| anyhow::anyhow!("encryption failed: {:?}", e))?;

    let mut out = Vec::with_capacity(nonce.len() + ciphertext.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// Decrypt a ciphertext blob produced by [`encrypt`] back into a private key string.
pub fn decrypt(key: &EncryptionKey, ciphertext: &[u8]) -> Result<String> {
    ensure!(
        ciphertext.len() >= NONCE_LEN + 16,
        "ciphertext is too short"
    );

    let (nonce_bytes, sealed) = ciphertext.split_at(NONCE_LEN);
    let cipher =
        Aes256Gcm::new_from_slice(&key.bytes).context("failed to initialize AES-256-GCM cipher")?;
    let nonce_array: [u8; NONCE_LEN] = nonce_bytes.try_into().expect("nonce length checked");
    let nonce: aes_gcm::Nonce<aes_gcm::aead::consts::U12> = nonce_array.into();

    let plaintext = cipher
        .decrypt(&nonce, sealed)
        .map_err(|e| anyhow::anyhow!("decryption failed: {:?}", e))?;

    String::from_utf8(plaintext).context("decrypted plaintext is not valid UTF-8")
}

/// Generate an opaque high-entropy app API key for a new agent.
pub fn generate_api_key() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    format!("vta_{}", URL_SAFE_NO_PAD.encode(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> EncryptionKey {
        EncryptionKey::new(
            "test-key-id",
            [
                0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22,
                23, 24, 25, 26, 27, 28, 29, 30, 31,
            ],
        )
    }

    #[test]
    fn encryption_round_trip() {
        let key = test_key();
        let plaintext = "0x4c0883a69102937d6231471b5dbb6204fe5129617082795f9d3d2c7e2f9f3f5b";

        let ciphertext = encrypt(&key, plaintext).expect("encryption should succeed");
        assert!(ciphertext.len() > NONCE_LEN + 16);

        let decrypted = decrypt(&key, &ciphertext).expect("decryption should succeed");
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn decryption_fails_with_wrong_key() {
        let key = test_key();
        let plaintext = "secret-private-key";
        let ciphertext = encrypt(&key, plaintext).unwrap();

        let other_key = EncryptionKey::new(
            "other-key-id",
            [
                31, 30, 29, 28, 27, 26, 25, 24, 23, 22, 21, 20, 19, 18, 17, 16, 15, 14, 13, 12, 11,
                10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0,
            ],
        );

        assert!(decrypt(&other_key, &ciphertext).is_err());
    }

    #[test]
    fn decryption_fails_with_tampered_ciphertext() {
        let key = test_key();
        let mut ciphertext = encrypt(&key, "secret").unwrap();
        ciphertext[NONCE_LEN] ^= 0xff;

        assert!(decrypt(&key, &ciphertext).is_err());
    }

    #[test]
    fn from_hex_accepts_64_character_key() {
        let hex = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
        let key = EncryptionKey::from_hex("id", hex).unwrap();
        assert_eq!(key.bytes[0], 0x00);
        assert_eq!(key.bytes[31], 0x1f);
    }

    #[test]
    fn from_hex_rejects_wrong_length() {
        assert!(EncryptionKey::from_hex("id", "deadbeef").is_err());
    }

    #[test]
    fn generated_api_keys_are_prefixed_and_unique() {
        let a = generate_api_key();
        let b = generate_api_key();
        assert!(a.starts_with("vta_"));
        assert!(b.starts_with("vta_"));
        assert_ne!(a, b);
    }
}
