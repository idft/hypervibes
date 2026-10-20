import { hexToString } from "viem";
import type { EIP1193Provider, Hex } from "viem";
import { privateKeyToAccount } from "viem/accounts";
import { parseSiweMessage } from "viem/siwe";
import { describe, expect, it, vi } from "vitest";
import { loginError, walletLogin } from "./wallet-login.js";

const wallet = privateKeyToAccount(`0x${"44".repeat(32)}`);

describe("browser wallet login", () => {
  it("connects, signs a bounded SIWE message and verifies it without requesting trading authority", async () => {
    const request = vi.fn(async ({ method, params }: { method: string; params?: unknown }) => {
      if (method === "eth_requestAccounts") return [wallet.address.toLowerCase()];
      if (method === "personal_sign") {
        const [hex] = params as [Hex, string];
        const message = hexToString(hex);
        expect(parseSiweMessage(message)).toMatchObject({ address: wallet.address, domain: "localhost:3003", chainId: 1, uri: "http://localhost:3003/login", nonce: "a1234567890abcdef" });
        expect(message).toContain("This does not authorize trading.");
        return wallet.signMessage({ message });
      }
      throw new Error(`Unexpected wallet method ${method}`);
    });
    const verify = vi.fn(async () => {});
    const status = vi.fn();
    await walletLogin({ provider: { request } as Pick<EIP1193Provider, "request">, origin: "http://localhost:3003", nonce: async () => "a1234567890abcdef", verify, onStatus: status });
    expect(verify).toHaveBeenCalledOnce();
    expect(request.mock.calls.map(([args]) => args.method)).toEqual(["eth_requestAccounts", "personal_sign"]);
    expect(status.mock.calls.map(([value]) => value)).toEqual(["Connecting wallet…", "Preparing sign-in…", "Sign message in your wallet…", "Verifying sign-in…"]);
  });

  it("reports a missing provider without making an auth request", async () => {
    const nonce = vi.fn(async () => "unused");
    await expect(walletLogin({ origin: "http://localhost:3003", nonce, verify: async () => {}, onStatus: () => {} })).rejects.toThrow("No Ethereum wallet found");
    expect(nonce).not.toHaveBeenCalled();
  });

  it("rejects empty wallet selections before requesting a challenge", async () => {
    const nonce = vi.fn(async () => "unused");
    await expect(walletLogin({ provider: { request: async () => [] } as Pick<EIP1193Provider, "request">, origin: "http://localhost:3003", nonce, verify: async () => {}, onStatus: () => {} })).rejects.toThrow("No Ethereum wallet account selected");
    expect(nonce).not.toHaveBeenCalled();
  });

  it("makes wallet cancellation understandable and keeps generic failures safe", () => {
    expect(loginError({ cause: { code: 4001 } })).toContain("Sign-in cancelled");
    expect(loginError(null)).toBe("Sign-in failed. Please try again.");
    expect(loginError(new Error("No wallet"))).toBe("No wallet");
  });
});
