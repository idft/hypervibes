import { ApiRequestError, ExchangeClient, HttpTransport, InfoClient } from "@nktkas/hyperliquid";
import { privateKeyToAccount } from "viem/accounts";
import { HYPERLIQUID_API_URL } from "./constants.js";

/** Splits a 65-byte secp256k1 signature into Hyperliquid's `r`/`s`/`v` fields. */
export function signatureParts(signature: string): { r: string; s: string; v: number } {
  const raw = signature.slice(2);
  if (!/^[0-9a-fA-F]{130}$/.test(raw)) throw new Error("Invalid signature");
  return {
    r: `0x${raw.slice(0, 64)}`,
    s: `0x${raw.slice(64, 128)}`,
    v: Number.parseInt(raw.slice(128, 130), 16),
  };
}

/** An exchange response succeeded only when Hyperliquid reports `status: "ok"`. */
export function exchangeSuccess(response: unknown): response is { status: "ok" } {
  return typeof response === "object" && response !== null
    && (response as { status?: unknown }).status === "ok";
}

/** The message Hyperliquid attached to a rejected exchange response. */
export function exchangeErrorMessage(response: unknown): string {
  if (typeof response !== "object" || response === null) return "Hyperliquid rejected the request.";
  const inner = (response as { response?: unknown }).response;
  const nested = typeof inner === "object" && inner !== null ? (inner as { data?: { status?: { error?: unknown } } }).data?.status?.error : undefined;
  const message = typeof nested === "string" ? nested
    : typeof inner === "string" ? inner
      : typeof (response as { message?: unknown }).message === "string" ? (response as { message: string }).message
        : undefined;
  return message ?? "Hyperliquid rejected the request.";
}

/**
 * Relays a browser-signed action to the exchange endpoint unchanged. The
 * caller has already validated the action object and recovered its signer.
 */
export async function relaySignedAction(
  transport: HttpTransport,
  action: Record<string, unknown>,
  signature: string,
): Promise<unknown> {
  const nonce = action.nonce;
  if (typeof nonce !== "number" || !Number.isSafeInteger(nonce)) throw new Error("Invalid action nonce");
  const response = await fetch(`${HYPERLIQUID_API_URL}/exchange`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ action, nonce, signature: signatureParts(signature) }),
  });
  if (!response.ok) throw new Error("Hyperliquid exchange unavailable");
  return response.json();
}

export interface CreatedSubaccount {
  address: string;
  created: boolean;
}

/** Creation outcomes that callers must report distinctly to the user. */
export type SubaccountFailure =
  | { kind: "lookup-unavailable" }
  | { kind: "rejected"; message: string }
  | { kind: "unconfirmed" };

export type SubaccountResult = { ok: true; subaccount: CreatedSubaccount } | { ok: false; failure: SubaccountFailure };

interface OwnerState {
  pending: Set<string>;
  queue: Promise<unknown>;
}

/**
 * Serializes creation for each owner and remembers uncertain submissions for
 * the lifetime of the service. A cancelled request also remains uncertain.
 */
export class SubaccountCreator {
  private readonly owners = new Map<string, OwnerState>();

  constructor(private readonly transport: HttpTransport, private readonly nonceManager?: (signer: string) => Promise<number>) {}

  private async discover(owner: string, name: string): Promise<string | null> {
    const info = new InfoClient({ transport: this.transport });
    const accounts = await info.subAccounts({ user: owner as `0x${string}` });
    return accounts?.find((account) => account.master.toLowerCase() === owner && account.name === name)
      ?.subAccountUser.toLowerCase() ?? null;
  }

  async create(owner: string, name: string, privateKey: string): Promise<SubaccountResult> {
    const state = this.owners.get(owner) ?? { pending: new Set<string>(), queue: Promise.resolve() };
    this.owners.set(owner, state);
    const attempt = state.queue.then(() => this.attempt(state, owner, name, privateKey));
    state.queue = attempt.then(() => {}, () => {});
    return attempt;
  }

  private async attempt(state: OwnerState, owner: string, name: string, privateKey: string): Promise<SubaccountResult> {
    let existing: string | null;
    try {
      existing = await this.discover(owner, name);
    } catch {
      return state.pending.has(name)
        ? { ok: false, failure: { kind: "unconfirmed" } }
        : { ok: false, failure: { kind: "lookup-unavailable" } };
    }
    if (existing) {
      state.pending.delete(name);
      return { ok: true, subaccount: { address: existing, created: false } };
    }
    if (state.pending.has(name)) return { ok: false, failure: { kind: "unconfirmed" } };

    // Record the name before awaiting submission so a cancelled request cannot
    // enable another request to blindly submit the same creation.
    state.pending.add(name);
    try {
      const wallet = privateKeyToAccount(privateKey as `0x${string}`);
      const exchange = new ExchangeClient({ transport: this.transport, wallet, nonceManager: this.nonceManager });
      const response = await exchange.createSubAccount({ name });
      state.pending.delete(name);
      const address = typeof response.response?.data === "string" ? response.response.data.toLowerCase() : null;
      if (!address) return { ok: false, failure: { kind: "unconfirmed" } };
      return { ok: true, subaccount: { address, created: true } };
    } catch (error) {
      if (error instanceof ApiRequestError) {
        // An explicit exchange rejection definitively means nothing was created.
        state.pending.delete(name);
        return { ok: false, failure: { kind: "rejected", message: error.message } };
      }
      // Transport or unexpected failures are uncertain; reconcile by discovery.
      const reconciled = await this.discover(owner, name).catch(() => null);
      if (reconciled) {
        state.pending.delete(name);
        return { ok: true, subaccount: { address: reconciled, created: true } };
      }
      return { ok: false, failure: { kind: "unconfirmed" } };
    }
  }
}