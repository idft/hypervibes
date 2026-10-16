import { isAddress, recoverTypedDataAddress, type Address, type Hex } from "viem";
import {
  BUILDER_RECIPIENT,
  SIGNATURE_CHAIN_ID,
  USER_API_WALLET_NAME,
} from "./constants.js";

/** Browser-wallet-signed actions the server validates before relaying. */
export type SignedActionKind = "approveAgent" | "approveBuilderFee";

export interface SignedAction {
  action: Record<string, unknown>;
  signature: string;
}

const ZERO_ADDRESS = "0x0000000000000000000000000000000000000000";

const FIELD_SETS = {
  approveAgent: ["type", "hyperliquidChain", "signatureChainId", "agentAddress", "agentName", "nonce"],
  approveBuilderFee: ["type", "hyperliquidChain", "signatureChainId", "maxFeeRate", "builder", "nonce"],
} as const satisfies Record<SignedActionKind, readonly string[]>;

/** Narrow, validate and sanitize a client request body into a SignedAction. */
export function parseSignedAction(value: unknown): SignedAction {
  if (typeof value !== "object" || value === null) throw new Error("Invalid request");
  const action = Reflect.get(value, "action");
  const signature = Reflect.get(value, "signature");
  if (typeof action !== "object" || action === null || Array.isArray(action)) throw new Error("Invalid request");
  if (typeof signature !== "string" || !/^0x[0-9a-fA-F]{130}$/.test(signature)) throw new Error("Invalid request");
  return { action: { ...(action as Record<string, unknown>) }, signature };
}

function hasExactFields(action: Record<string, unknown>, kind: SignedActionKind): boolean {
  const expected = FIELD_SETS[kind];
  const keys = Object.keys(action);
  return keys.length === expected.length && expected.every((field) => field in action);
}

function validCommon(action: Record<string, unknown>): boolean {
  const nonce = action.nonce;
  return action.hyperliquidChain === "Mainnet"
    && action.signatureChainId === "0xa4b1"
    && typeof nonce === "number" && Number.isSafeInteger(nonce) && nonce > 0;
}

function isLowercaseAddress(value: unknown): value is string {
  return typeof value === "string" && isAddress(value) && value === value.toLowerCase();
}

/** The action must approve exactly this server-generated trading signer. */
export function validApproveAgentAction(action: Record<string, unknown>, expectedAddress: string): boolean {
  return hasExactFields(action, "approveAgent")
    && validCommon(action)
    && action.type === "approveAgent"
    && action.agentName === USER_API_WALLET_NAME
    && isLowercaseAddress(action.agentAddress)
    && action.agentAddress === expectedAddress;
}

/** The action must approve or revoke exactly this server's builder at the expected rate. */
export function validApproveBuilderFeeAction(action: Record<string, unknown>, maxFeeRate: string): boolean {
  return hasExactFields(action, "approveBuilderFee")
    && validCommon(action)
    && action.type === "approveBuilderFee"
    && action.builder === BUILDER_RECIPIENT
    && action.maxFeeRate === maxFeeRate;
}

/**
 * Recovers the signing address from an EIP-712 Hyperliquid action signature and
 * compares it with the expected address. Invalid structures never match.
 */
export async function signatureMatches(
  action: Record<string, unknown>,
  signature: string,
  expectedAddress: string,
  kind: SignedActionKind,
): Promise<boolean> {
  const primaryType = kind === "approveAgent"
    ? "HyperliquidTransaction:ApproveAgent"
    : "HyperliquidTransaction:ApproveBuilderFee";
  try {
    const recovered: Address = await recoverTypedDataAddress({
      domain: {
        name: "HyperliquidSignTransaction",
        version: "1",
        chainId: SIGNATURE_CHAIN_ID,
        verifyingContract: ZERO_ADDRESS,
      },
      types:
        kind === "approveAgent"
          ? { [primaryType]: [
            { name: "hyperliquidChain", type: "string" },
            { name: "agentAddress", type: "address" },
            { name: "agentName", type: "string" },
            { name: "nonce", type: "uint64" },
          ] }
          : { [primaryType]: [
            { name: "hyperliquidChain", type: "string" },
            { name: "maxFeeRate", type: "string" },
            { name: "builder", type: "address" },
            { name: "nonce", type: "uint64" },
          ] },
      primaryType,
      message: action as Record<string, string | number>,
      signature: signature as Hex,
    });
    return recovered.toLowerCase() === expectedAddress.toLowerCase();
  } catch {
    return false;
  }
}