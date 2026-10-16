import { privateKeyToAccount } from "viem/accounts";
import type { EIP1193Provider } from "viem";
import { describe, expect, it, vi } from "vitest";
import { validApproveAgentAction, validApproveBuilderFeeAction, signatureMatches } from "../hyperliquid/signing.js";
import { BUILDER_RECIPIENT } from "../hyperliquid/constants.js";
import { actionBase, signWalletAction, walletProvider } from "./wallet-action.js";

const wallet = privateKeyToAccount(`0x${"11".repeat(32)}` as `0x${string}`);

/** Minimal injected wallet: returns real signatures over exactly what it receives. */
function injectedProvider(): { provider: Pick<EIP1193Provider, "request">; requests: string[] } {
  const requests: string[] = [];
  return {
    requests,
    provider: {
      request: vi.fn(async ({ method, params = [] }: { method: string; params?: unknown[] }) => {
        requests.push(method);
        if (method === "eth_requestAccounts") return [wallet.address];
        if (method === "wallet_switchEthereumChain") return null;
        if (method === "eth_signTypedData_v4") {
          const typedData = JSON.parse(String(params[1])) as {
            domain: { name: string; version: string; chainId: number; verifyingContract: `0x${string}` };
            types: Record<string, readonly { name: string; type: string }[]>;
            primaryType: string;
            message: Record<string, string | number>;
          };
          return wallet.signTypedData({
            domain: typedData.domain,
            types: typedData.types,
            primaryType: typedData.primaryType,
            // The provider receives JSON numbers; signing accepts them as-is.
            message: typedData.message as never,
          });
        }
        throw new Error(`unexpected wallet method ${method}`);
      }) as unknown as EIP1193Provider["request"],
    },
  };
}

describe("browser wallet action signing", () => {
  it("produces actions the server validators accept and can recover", async () => {
    const { provider } = injectedProvider();
    const action = { type: "approveAgent", ...actionBase(), agentAddress: "0x1111111111111111111111111111111111111111", agentName: "HyperVibes" };
    const signed = await signWalletAction({ provider }, "HyperliquidTransaction:ApproveAgent", {
      "HyperliquidTransaction:ApproveAgent": [
        { name: "hyperliquidChain", type: "string" },
        { name: "agentAddress", type: "address" },
        { name: "agentName", type: "string" },
        { name: "nonce", type: "uint64" },
      ],
    }, action);
    expect(validApproveAgentAction(signed.action, action.agentAddress)).toBe(true);
    expect(await signatureMatches(signed.action, signed.signature, wallet.address.toLowerCase(), "approveAgent")).toBe(true);

    const feeAction = { type: "approveBuilderFee", ...actionBase(), maxFeeRate: "0.05%", builder: BUILDER_RECIPIENT };
    const feeSigned = await signWalletAction({ provider }, "HyperliquidTransaction:ApproveBuilderFee", {
      "HyperliquidTransaction:ApproveBuilderFee": [
        { name: "hyperliquidChain", type: "string" },
        { name: "maxFeeRate", type: "string" },
        { name: "builder", type: "address" },
        { name: "nonce", type: "uint64" },
      ],
    }, feeAction);
    expect(validApproveBuilderFeeAction(feeSigned.action, "0.05%")).toBe(true);
    expect(await signatureMatches(feeSigned.action, feeSigned.signature, wallet.address.toLowerCase(), "approveBuilderFee")).toBe(true);
  });

  it("switches to Arbitrum One before signing and reports missing wallets", async () => {
    const { provider, requests } = injectedProvider();
    await signWalletAction({ provider }, "HyperliquidTransaction:ApproveBuilderFee", {
      "HyperliquidTransaction:ApproveBuilderFee": [
        { name: "hyperliquidChain", type: "string" },
        { name: "maxFeeRate", type: "string" },
        { name: "builder", type: "address" },
        { name: "nonce", type: "uint64" },
      ],
    }, { type: "approveBuilderFee", ...actionBase(), maxFeeRate: "0.00%", builder: BUILDER_RECIPIENT });
    expect(requests).toEqual(["wallet_switchEthereumChain", "eth_requestAccounts", "eth_signTypedData_v4"]);
    await expect(signWalletAction({}, "X", {}, {})).rejects.toThrow("No Ethereum wallet found");
  });

  it("reads window.ethereum when present", () => {
    expect(walletProvider()).toBeUndefined();
    vi.stubGlobal("window", { ethereum: { request: vi.fn() } });
    expect(walletProvider()).toBeDefined();
  });
});