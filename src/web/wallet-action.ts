import { createWalletClient, custom, type EIP1193Provider } from "viem";

/**
 * Browser-side helpers for signing Hyperliquid actions with the user's main
 * wallet. The private key never leaves the wallet; the server validates and
 * relays the signed action.
 */

const CHAIN_ID = 42_161;
const CHAIN_HEX = "0xa4b1";

export interface WalletProvider {
  provider?: Pick<EIP1193Provider, "request">;
}

async function selectSignatureChain(provider: Pick<EIP1193Provider, "request">): Promise<void> {
  try {
    await provider.request({ method: "wallet_switchEthereumChain", params: [{ chainId: CHAIN_HEX }] });
  } catch (reason: unknown) {
    if (!(typeof reason === "object" && reason !== null && "code" in reason && reason.code === 4902)) {
      throw new Error("Switch your wallet to Arbitrum Mainnet to sign Hyperliquid approvals.", { cause: reason });
    }
    await provider.request({
      method: "wallet_addEthereumChain",
      params: [{
        chainId: CHAIN_HEX,
        chainName: "Arbitrum One",
        nativeCurrency: { name: "Ether", symbol: "ETH", decimals: 18 },
        rpcUrls: ["https://arb1.arbitrum.io/rpc"],
        blockExplorerUrls: ["https://arbiscan.io"],
      }],
    });
    await provider.request({ method: "wallet_switchEthereumChain", params: [{ chainId: CHAIN_HEX }] });
  }
}

export interface SignedWalletAction {
  action: Record<string, string | number | boolean>;
  signature: `0x${string}`;
}

export async function signWalletAction(
  { provider }: WalletProvider,
  primaryType: string,
  types: Record<string, readonly { name: string; type: string }[]>,
  action: Record<string, string | number | boolean>,
): Promise<SignedWalletAction> {
  if (!provider) throw new Error("No Ethereum wallet found. Open this page in a browser with a wallet extension.");
  const client = createWalletClient({ transport: custom(provider) });
  await selectSignatureChain(provider);
  const [account] = await client.requestAddresses();
  if (!account) throw new Error("No wallet account selected.");
  const signature = await client.signTypedData({
    account,
    domain: { name: "HyperliquidSignTransaction", version: "1", chainId: CHAIN_ID, verifyingContract: "0x0000000000000000000000000000000000000000" },
    types,
    primaryType,
    message: action,
  });
  return { action, signature };
}

export function actionBase(): Record<string, string | number> {
  return { hyperliquidChain: "Mainnet", signatureChainId: CHAIN_HEX, nonce: Date.now() };
}

export function walletProvider(): Pick<EIP1193Provider, "request"> | undefined {
  // Node has no `window`; browsers always do.
  return (globalThis as typeof globalThis & { window?: Window & { ethereum?: Pick<EIP1193Provider, "request"> } }).window?.ethereum;
}

export const approveAgentTypes = {
  "HyperliquidTransaction:ApproveAgent": [
    { name: "hyperliquidChain", type: "string" },
    { name: "agentAddress", type: "address" },
    { name: "agentName", type: "string" },
    { name: "nonce", type: "uint64" },
  ],
} as const;

export const approveBuilderFeeTypes = {
  "HyperliquidTransaction:ApproveBuilderFee": [
    { name: "hyperliquidChain", type: "string" },
    { name: "maxFeeRate", type: "string" },
    { name: "builder", type: "address" },
    { name: "nonce", type: "uint64" },
  ],
} as const;