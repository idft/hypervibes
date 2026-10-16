import { describe, expect, it } from "vitest";
import { decryptSecret, encryptSecret, generateWalletPrivateKey, type EncryptionKey } from "./crypto.js";

const key: EncryptionKey = { id: "test-key", bytes: new Uint8Array(32).fill(7) };

describe("AES-256-GCM secret storage", () => {
  it("round-trips a private key through nonce ‖ ciphertext ‖ tag", () => {
    const sealed = encryptSecret(key, "0xdeadbeef");
    expect(sealed.length).toBe(12 + 10 + 16);
    expect(new Uint8Array(sealed.subarray(0, 4))).not.toEqual(new Uint8Array(4).fill(0));
    expect(decryptSecret(key, sealed)).toBe("0xdeadbeef");
  });

  it("uses a fresh nonce per call", () => {
    expect(new Uint8Array(encryptSecret(key, "same").subarray(0, 12)))
      .not.toEqual(new Uint8Array(encryptSecret(key, "same").subarray(0, 12)));
  });

  it("rejects tampered ciphertext instead of returning corrupted secrets", () => {
    const sealed = encryptSecret(key, "0xdeadbeef");
    const tampered = Uint8Array.from(sealed);
    tampered[tampered.length - 1] = (tampered[tampered.length - 1] ?? 0) ^ 0xff;
    expect(() => decryptSecret(key, tampered)).toThrow();
  });

  it("rejects the wrong key and truncated ciphertext", () => {
    const sealed = encryptSecret(key, "0xdeadbeef");
    const other: EncryptionKey = { id: "other", bytes: new Uint8Array(32).fill(9) };
    expect(() => decryptSecret(other, sealed)).toThrow();
    expect(() => decryptSecret(key, sealed.subarray(0, 10))).toThrow();
  });

  it("generates 32-byte secp256k1 private keys", () => {
    expect(generateWalletPrivateKey()).toMatch(/^0x[0-9a-f]{64}$/);
    expect(generateWalletPrivateKey()).not.toBe(generateWalletPrivateKey());
  });
});