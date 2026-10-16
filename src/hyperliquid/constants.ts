/** Mainnet-only Hyperliquid constants shared by server code and templates. */

/** The only builder fee recipient the server accepts and submits. */
export const BUILDER_RECIPIENT = "0x2ebba955c61116e1c249efb1e39d25cb4a79ea05";

/** Builder fee limits in whole basis points, stored as tenths of a basis point. */
export const MIN_BUILDER_FEE_BPS = 1;
export const MAX_BUILDER_FEE_BPS = 10;
export const DEFAULT_BUILDER_FEE_BPS = 5;
export const BUILDER_FEE_BPS_TO_TENTHS = 10;
/** The maximum builder fee Hyperliquid allows this builder, in tenths of a bp. */
export const MAX_BUILDER_FEE_TENTHS_OF_BP = MAX_BUILDER_FEE_BPS * BUILDER_FEE_BPS_TO_TENTHS;
/** The maxFeeRate string that revokes the builder fee approval. */
export const REVOKED_BUILDER_FEE_RATE = "0.00%";

/** Name recorded with Hyperliquid for every HyperVibes trading signer. */
export const USER_API_WALLET_NAME = "HyperVibes";

/** Arbitrum One: the chain every Hyperliquid action signature must target. */
export const SIGNATURE_CHAIN_ID = 42_161;
export const SIGNATURE_CHAIN_HEX = "0xa4b1";

/** Hyperliquid mainnet API endpoints. */
export const HYPERLIQUID_API_URL = "https://api.hyperliquid.xyz";

/** Approvals of trading signers expire on Hyperliquid after 180 days. */
export const API_WALLET_APPROVAL_MS = 180 * 24 * 60 * 60 * 1000;

/** `vt-` prefix plus at most 13 characters keeps subaccount names within Hyperliquid's limit. */
export function agentSubaccountName(displayName: string): string {
  return `vt-${displayName.trim().slice(0, 13).trimEnd()}`;
}