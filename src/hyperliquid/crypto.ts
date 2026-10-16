import { createCipheriv, createDecipheriv, randomBytes } from "node:crypto";

const NONCE_LENGTH = 12;
const TAG_LENGTH = 16;
const KEY_LENGTH = 32;

export interface EncryptionKey {
  /** Identifier stored alongside ciphertext so keys can be rotated and audited. */
  id: string;
  bytes: Uint8Array;
}

/** Generates a fresh secp256k1 private key in Hyperliquid's `0x…` hex form. */
export function generateWalletPrivateKey(): string {
  return `0x${randomBytes(KEY_LENGTH).toString("hex")}`;
}

/** Encrypts a secret with AES-256-GCM: `nonce ‖ ciphertext ‖ tag`. */
export function encryptSecret(key: EncryptionKey, plaintext: string): Uint8Array {
  const nonce = randomBytes(NONCE_LENGTH);
  const cipher = createCipheriv("aes-256-gcm", key.bytes, nonce);
  const ciphertext = Buffer.concat([cipher.update(plaintext, "utf8"), cipher.final()]);
  return new Uint8Array(Buffer.concat([nonce, ciphertext, cipher.getAuthTag()]));
}

/**
 * Decrypts an AES-256-GCM secret. Throws on truncation, tampering or a
 * mismatched key so callers never silently accept corrupted credentials.
 */
export function decryptSecret(key: EncryptionKey, sealed: Uint8Array): string {
  if (sealed.length < NONCE_LENGTH + TAG_LENGTH) throw new Error("Encrypted secret is truncated");
  const decipher = createDecipheriv("aes-256-gcm", key.bytes, sealed.subarray(0, NONCE_LENGTH));
  decipher.setAuthTag(sealed.subarray(sealed.length - TAG_LENGTH));
  return Buffer.concat([
    decipher.update(sealed.subarray(NONCE_LENGTH, sealed.length - TAG_LENGTH)),
    decipher.final(),
  ]).toString("utf8");
}