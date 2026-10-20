import { createWalletClient, custom, getAddress, isAddress } from "viem";
import type { EIP1193Provider } from "viem";
import { createSiweMessage } from "viem/siwe";

interface WalletLoginOptions {
  provider?: Pick<EIP1193Provider, "request">;
  origin: string;
  nonce(): Promise<string>;
  verify(message: string, signature: `0x${string}`): Promise<void>;
  onStatus(status: string): void;
}

export async function walletLogin({ provider, origin, nonce, verify, onStatus }: WalletLoginOptions): Promise<void> {
  if (!provider) throw new Error("No Ethereum wallet found. Open this page in a browser with a wallet extension.");
  const wallet = createWalletClient({ transport: custom(provider) });
  onStatus("Connecting wallet…");
  const [selected] = await wallet.requestAddresses();
  if (!selected || !isAddress(selected)) throw new Error("No Ethereum wallet account selected.");
  const address = getAddress(selected);
  onStatus("Preparing sign-in…");
  const challenge = await nonce();
  const issuedAt = new Date();
  const url = new URL(origin);
  const message = createSiweMessage({
    address,
    chainId: 1,
    domain: url.host,
    uri: `${url.origin}/login`,
    version: "1",
    nonce: challenge,
    issuedAt,
    expirationTime: new Date(issuedAt.getTime() + 300_000),
    statement: "Sign in to HyperVibes. This does not authorize trading.",
  });
  onStatus("Sign message in your wallet…");
  const signature = await wallet.signMessage({ account: address, message });
  onStatus("Verifying sign-in…");
  await verify(message, signature);
}

export function loginError(reason: unknown): string {
  let error = reason;
  for (let depth = 0; depth < 5 && error && typeof error === "object"; depth++) {
    if ("code" in error && error.code === 4001) return "Sign-in cancelled. Please try again when you're ready.";
    error = "cause" in error ? error.cause : undefined;
  }
  return reason instanceof Error ? reason.message : "Sign-in failed. Please try again.";
}
