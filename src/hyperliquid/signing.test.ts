import { privateKeyToAccount } from "viem/accounts";
import { describe, expect, it } from "vitest";
import {
  parseSignedAction,
  signatureMatches,
  validApproveAgentAction,
  validApproveBuilderFeeAction,
} from "./signing.js";
import { BUILDER_RECIPIENT } from "./constants.js";

const signer = privateKeyToAccount(`0x${"46".repeat(32)}` as `0x${string}`);
const other = privateKeyToAccount(`0x${"47".repeat(32)}` as `0x${string}`);
const domain = { name: "HyperliquidSignTransaction", version: "1", chainId: 42_161, verifyingContract: "0x0000000000000000000000000000000000000000" as const };
const approveAgentTypes = {
  "HyperliquidTransaction:ApproveAgent": [
    { name: "hyperliquidChain", type: "string" },
    { name: "agentAddress", type: "address" },
    { name: "agentName", type: "string" },
    { name: "nonce", type: "uint64" },
  ],
} as const;

const agentAction = {
  type: "approveAgent",
  hyperliquidChain: "Mainnet",
  signatureChainId: "0xa4b1",
  agentAddress: "0x1111111111111111111111111111111111111111",
  agentName: "HyperVibes",
  nonce: 1_700_000_000_000,
} as const;

describe("signed-action validation", () => {
  it("accepts the exact ApproveAgent shape for the expected signer address", () => {
    expect(validApproveAgentAction(agentAction, agentAction.agentAddress)).toBe(true);
  });

  it.each([
    ["wrong chain", { ...agentAction, hyperliquidChain: "Testnet" }],
    ["wrong signature chain id", { ...agentAction, signatureChainId: "0x6666" }],
    ["zero nonce", { ...agentAction, nonce: 0 }],
    ["extra field", { ...agentAction, extra: "field" }],
    ["wrong agent name", { ...agentAction, agentName: "Someone Else" }],
    ["different address", { ...agentAction, agentAddress: "0x2222222222222222222222222222222222222222" }],
    ["mixed-case address", { ...agentAction, agentAddress: "0xAbCdEf1111111111111111111111111111111111" }],
    ["string nonce", { ...agentAction, nonce: "1700000000000" }],
  ])("rejects %s", (_name, action) => {
    expect(validApproveAgentAction(action, agentAction.agentAddress)).toBe(false);
  });

  it("accepts only the exact ApproveBuilderFee shape for this builder", () => {
    const action = {
      type: "approveBuilderFee", hyperliquidChain: "Mainnet", signatureChainId: "0xa4b1",
      maxFeeRate: "0.05%", builder: BUILDER_RECIPIENT, nonce: 1234,
    };
    expect(validApproveBuilderFeeAction(action, "0.05%")).toBe(true);
    expect(validApproveBuilderFeeAction({ ...action, builder: "0x000000000000000000000000000000000000dead" }, "0.05%")).toBe(false);
    expect(validApproveBuilderFeeAction(action, "0.03%")).toBe(false);
    expect(validApproveBuilderFeeAction({ ...action, maxFeeRate: "0.00%" }, "0.05%")).toBe(false);
  });

  it("recovers the signing address and rejects forgeries", async () => {
    // The wallet provider receives the nonce as a JSON number; a locally signed
    // bigint nonce produces the identical EIP-712 hash.
    const signature = await signer.signTypedData({
      domain, types: approveAgentTypes, primaryType: "HyperliquidTransaction:ApproveAgent",
      message: { ...agentAction, nonce: BigInt(agentAction.nonce) },
    });
    expect(await signatureMatches(agentAction, signature, signer.address.toLowerCase(), "approveAgent")).toBe(true);
    expect(await signatureMatches(agentAction, signature, other.address.toLowerCase(), "approveAgent")).toBe(false);

    const forged = { ...agentAction, agentAddress: "0x3333333333333333333333333333333333333333" };
    expect(await signatureMatches(forged, signature, signer.address.toLowerCase(), "approveAgent")).toBe(false);
    expect(await signatureMatches(agentAction, `0x${"ab".repeat(65)}`, signer.address.toLowerCase(), "approveAgent")).toBe(false);
  });

  it("narrows request bodies through parseSignedAction", () => {
    expect(parseSignedAction({ action: agentAction, signature: `0x${"ab".repeat(65)}` })).toEqual({
      action: agentAction,
      signature: `0x${"ab".repeat(65)}`,
    });
    expect(() => parseSignedAction("nope")).toThrow("Invalid request");
    expect(() => parseSignedAction({ action: [agentAction], signature: `0x${"ab".repeat(65)}` })).toThrow("Invalid request");
    expect(() => parseSignedAction({ action: agentAction, signature: "0x1234" })).toThrow("Invalid request");
    expect(() => parseSignedAction({ action: agentAction, signature: `0x${"ab".repeat(66)}` })).toThrow("Invalid request");
  });
});