import { HttpTransport, InfoClient } from "@nktkas/hyperliquid";
import { compareDecimals, parseDecimal } from "./decimal.js";

/** What the wallet can do on Hyperliquid right now. */
export type AccountMode = "unified" | "unregistered" | "unsupported" | "unavailable";

export interface OwnedSubaccount {
  name: string | null;
  address: string;
  /** Plain decimal USDC balance string, or null when unavailable. */
  balance: string | null;
}

export interface OwnedTradingAccounts {
  mainAddress: string;
  mainBalance: string | null;
  subaccounts: OwnedSubaccount[];
  subaccountCapacity: string | null;
  lookupError: string | null;
  accountMode: AccountMode;
}

export function accountModeMessage(mode: AccountMode): string {
  switch (mode) {
    case "unified": return "";
    case "unregistered":
      return "This wallet is not registered with Hyperliquid yet. Register it before setting up a trading signer, approving a builder fee, transferring funds, or creating agents.";
    case "unsupported":
      return "HyperVibes currently supports Unified Accounts only. Enable Unified Account in Hyperliquid before transferring or creating agents.";
    case "unavailable":
      return "Hyperliquid account mode could not be verified. Transfers are temporarily unavailable.";
  }
}

export function accountModeError(mode: AccountMode): string | null {
  return mode === "unified" ? null : accountModeMessage(mode);
}

function usdcBalance(balances: ReadonlyArray<{ coin: string; total?: string }>): string | null {
  const balance = balances.find((entry) => entry.coin.toLowerCase() === "usdc");
  // A registered account simply not holding USDC is zero, not unavailable.
  if (!balance) return "0";
  if (typeof balance.total !== "string") return null;
  const value = parseDecimal(balance.total);
  if (!value || value.negative) return null;
  return balance.total;
}

/** Sub-account capacity follows Hyperliquid's volume-based schedule, capped at 50. */
function subaccountCapacity(cumulativeVolume: string, existing: number): string | null {
  const comparison = compareDecimals(cumulativeVolume, "100000");
  if (comparison === null) return null;
  if (comparison < 0) return "0 remaining of 0 total";
  // One additional account per $100M of volume above the first $100k.
  const additional = wholeQuotientMinus(cumulativeVolume);
  const limit = Math.min(10 + additional, 50);
  const remaining = Math.max(limit - existing, 0);
  return `${remaining} remaining of ${limit} total`;
}

function wholeQuotientMinus(volume: string): number {
  const separator = volume.indexOf(".");
  const whole = separator === -1 ? volume : volume.slice(0, separator);
  const base = BigInt(whole) - 100_000n;
  if (base < 0n) return 0;
  return Number(base / 100_000_000n);
}

/**
 * Loads the wallet's main account and owned sub-accounts with their spot USDC
 * balances, mirroring the legacy application's unified-account checks.
 */
export async function loadOwnedTradingAccounts(
  transport: HttpTransport,
  walletAddress: string,
  options: { includeCapacity?: boolean; checkMode?: boolean } = {},
): Promise<OwnedTradingAccounts> {
  const mainAddress = walletAddress.toLowerCase();
  const info = new InfoClient({ transport });
  const checkMode = options.checkMode ?? true;

  const [spotState, subaccountsResult, userRole, abstraction] = await Promise.all([
    info.spotClearinghouseState({ user: mainAddress as `0x${string}` }).then(
      (state) => ({ ok: true as const, state }), (error: unknown) => ({ ok: false as const, error }),
    ),
    info.subAccounts({ user: mainAddress as `0x${string}` }).then(
      (accounts) => ({ ok: true as const, accounts }), (error: unknown) => ({ ok: false as const, error }),
    ),
    checkMode
      ? info.userRole({ user: mainAddress as `0x${string}` }).then(
        (role) => ({ ok: true as const, role }), (error: unknown) => ({ ok: false as const, error }),
      )
      : Promise.resolve({ ok: true as const, role: { role: "user" } as const }),
    checkMode
      ? info.userAbstraction({ user: mainAddress as `0x${string}` }).then(
        (mode) => ({ ok: true as const, mode }), (error: unknown) => ({ ok: false as const, error }),
      )
      : Promise.resolve({ ok: true as const, mode: "unifiedAccount" as const }),
  ]);

  const mainBalance = spotState.ok ? usdcBalance(spotState.state.balances) : null;
  const accountMode: AccountMode = userRole.ok && userRole.role.role === "missing"
    ? "unregistered"
    : abstraction.ok && abstraction.mode === "unifiedAccount"
      ? "unified"
      : abstraction.ok
        ? "unsupported"
        : "unavailable";

  let subaccounts: OwnedSubaccount[] = [];
  let subaccountLookupError: string | null = null;
  if (subaccountsResult.ok) {
    subaccounts = (subaccountsResult.accounts ?? [])
      .filter((account) => account.master.toLowerCase() === mainAddress)
      .map((account) => ({
        name: account.name || null,
        address: account.subAccountUser.toLowerCase(),
        // Hyperliquid omits spotState entirely for never-funded sub-accounts.
        balance: usdcBalance(account.spotState?.balances ?? []),
      }));
  } else {
    subaccountLookupError = "Hyperliquid sub-account data is temporarily unavailable.";
  }

  const lookupError = accountMode === "unregistered"
    ? null
    : spotState.ok || mainBalance !== null
      ? subaccountLookupError
      : "Hyperliquid account data is temporarily unavailable.";

  let subaccountCapacityText: string | null = null;
  if (options.includeCapacity && accountMode === "unified") {
    try {
      const rateLimit = await info.userRateLimit({ user: mainAddress as `0x${string}` });
      subaccountCapacityText = subaccountCapacity(rateLimit.cumVlm, subaccounts.length);
    } catch { /* Capacity is advisory; the page still renders without it. */ }
  }

  return {
    mainAddress,
    mainBalance,
    subaccounts,
    subaccountCapacity: subaccountCapacityText,
    lookupError,
    accountMode,
  };
}

/** Looks up the current builder-fee maximum for a user, in tenths of a bp. */
export async function maxBuilderFee(transport: HttpTransport, user: string, builder: string, signal?: AbortSignal): Promise<number> {
  const info = new InfoClient({ transport });
  return info.maxBuilderFee({ user: user as `0x${string}`, builder: builder as `0x${string}` }, signal);
}