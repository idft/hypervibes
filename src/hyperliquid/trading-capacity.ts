import { InfoClient, type HttpTransport } from "@nktkas/hyperliquid";
import { Type, type Static } from "typebox";
import { Check } from "typebox/value";
import { HyperliquidError } from "./errors.js";

// Unlike $, this end assertion cannot accept a trailing line terminator.
const decimal = Type.String({ pattern: "^-?[0-9]+(\\.[0-9]+)?(?![\\s\\S])" });
const nonnegativeDecimal = Type.String({ pattern: "^[0-9]+(\\.[0-9]+)?(?![\\s\\S])" });
const addressSchema = Type.String({ pattern: "^0x[a-fA-F0-9]{40}(?![\\s\\S])" });
const decimalPair = Type.Tuple([nonnegativeDecimal, nonnegativeDecimal]);
const leverageSchema = Type.Union([
  Type.Object({ type: Type.Literal("cross"), value: Type.Integer({ minimum: 1 }) }),
  Type.Object({ type: Type.Literal("isolated"), value: Type.Integer({ minimum: 1 }), rawUsd: decimal }),
]);
const activeAssetDataSchema = Type.Object({
  user: addressSchema, coin: Type.String(), leverage: leverageSchema,
  maxTradeSzs: decimalPair, availableToTrade: decimalPair, markPx: nonnegativeDecimal,
});
// The official API reference includes dexAbstraction; SDK 0.33.3 omits it.
const accountModeSchema = Type.Union([
  Type.Literal("unifiedAccount"), Type.Literal("portfolioMargin"),
  Type.Literal("disabled"), Type.Literal("default"), Type.Literal("dexAbstraction"),
]);
const metadataSchema = Type.Object({ universe: Type.Array(Type.Object({
  name: Type.String(), isDelisted: Type.Optional(Type.Boolean()),
})) });
const sectionSchema = (source: string) => Type.Object({
  source: Type.Literal(source), requestedAt: Type.Union([Type.Integer(), Type.Null()]),
  observedAt: Type.Union([Type.Integer(), Type.Null()]),
  venueTimestamp: Type.Null(), fresh: Type.Boolean(),
});
const orderingSchema = Type.Tuple([Type.Literal("buy"), Type.Literal("sell")]);
const sideCapacitySchema = Type.Object({ availableToTradeUsdc: nonnegativeDecimal, maxTradeSizeBase: nonnegativeDecimal });
const reasonSchema = Type.Object({
  section: Type.Union([Type.Literal("metadata"), Type.Literal("mode"), Type.Literal("activeAssetData"), Type.Literal("interpretation")]),
  status: Type.Union([Type.Literal("unknown"), Type.Literal("unavailable")]),
  code: Type.Union([
    Type.Literal("opening_capacity_unknown"), Type.Literal("no_assigned_account"),
    Type.Literal("unsupported_instrument"), Type.Literal("instrument_unverified"),
    Type.Literal("invalid_response"), Type.Literal("identity_mismatch"),
    Type.Literal("request_failed"), Type.Literal("deadline_exceeded"),
  ]),
  message: Type.String(),
});

export const TradingCapacitySchema = Type.Object({
  address: Type.Union([addressSchema, Type.Null()]), instrument: Type.String(),
  scope: Type.Literal("main-dex-perpetuals"), observedAt: Type.Integer(),
  capacityStatus: Type.Union([Type.Literal("observed"), Type.Literal("unavailable")]),
  openingCapacityStatus: Type.Literal("unknown"),
  accountAbstraction: Type.Union([accountModeSchema, Type.Null()]),
  activeAssetData: Type.Union([activeAssetDataSchema, Type.Null()]),
  venueCapacity: Type.Union([Type.Object({ buy: sideCapacitySchema, sell: sideCapacitySchema }), Type.Null()]),
  interpretation: Type.Object({
    availableToTrade: Type.Object({ ordering: orderingSchema, units: Type.Literal("USDC") }),
    maxTradeSzs: Type.Object({ ordering: orderingSchema, units: Type.Literal("base-asset") }),
  }),
  sections: Type.Object({
    metadata: sectionSchema("Hyperliquid meta HTTP"), mode: sectionSchema("Hyperliquid userAbstraction HTTP"),
    activeAssetData: sectionSchema("Hyperliquid activeAssetData HTTP"),
  }),
  reasons: Type.Array(reasonSchema),
  references: Type.Array(Type.Object({ url: Type.String(), finding: Type.String() })),
  warning: Type.String(),
});
export type TradingCapacity = Static<typeof TradingCapacitySchema>;
type Section = keyof TradingCapacity["sections"];

/**
 * Read-only, host-bound account observations. No account balances are converted into buying power.
 * Verified from the exchange's public frontend (no wallet or account requests):
 * https://app.hyperliquid.xyz/assets/config-D-6TkbK9.js
 * SHA-256: eac53ecca4f5d9b47b343edbf70f941df648401cc43e59244d730be82c614d46
 * x3(isBuy) returns {name:`Long`,side:0} or {name:`Short`,side:1}.
 * d8 receives side from x3(isBuy) and indexes BOTH maxTradeSzs and availableToTrade by it.
 * Its perp branch computes availableToTrade / markPx * leverage into base size, and
 * returns maxTradeSzs[side] as maxTradeSz (with a user-limit-price adjustment).
 * rQ displays availableToTrade using Y8(...).name: the perp collateral token.
 * Official contract specifications establish USDC collateral and 1 underlying unit
 * per contract for these main-DEX crypto perps. Thus the pairs are [buy, sell],
 * availableToTrade is USDC, and maxTradeSzs is base quantity, NOT [min, max].
 * HTTP examples and the official Python SDK establish decimal-string pairs but do
 * not document their semantics. The frontend establishes side/units, not that a
 * value is additional opening exposure: opposing trades can reduce/reverse a position.
 * We preserve the venue figures; we do not copy frontend floating-point arithmetic,
 * adjust for a proposed price, or promise an executable maximum opening order.
 */
export async function loadTradingCapacity(transport: HttpTransport, address: string | null, instrument: string, cancellation?: AbortSignal): Promise<TradingCapacity> {
  cancellation?.throwIfAborted();
  if (!/^[A-Za-z0-9][A-Za-z0-9._-]{0,31}(?![\s\S])/.test(instrument)) throw new HyperliquidError("Select a main-DEX perpetual instrument such as BTC.");
  if (address !== null && !Check(addressSchema, address)) throw new HyperliquidError("Invalid assigned trading account address.");
  const emptySection = (source: string) => ({ source, requestedAt: null, observedAt: null, venueTimestamp: null, fresh: false });
  const result: TradingCapacity = {
    address, instrument, scope: "main-dex-perpetuals", observedAt: Date.now(), capacityStatus: "unavailable", openingCapacityStatus: "unknown",
    accountAbstraction: null, activeAssetData: null, venueCapacity: null,
    interpretation: { availableToTrade: { ordering: ["buy", "sell"], units: "USDC" }, maxTradeSzs: { ordering: ["buy", "sell"], units: "base-asset" } },
    sections: {
      metadata: emptySection("Hyperliquid meta HTTP"), mode: emptySection("Hyperliquid userAbstraction HTTP"),
      activeAssetData: emptySection("Hyperliquid activeAssetData HTTP"),
    },
    reasons: [{ section: "interpretation", status: "unknown", code: "opening_capacity_unknown", message: "Venue side-specific trade capacity is not isolated additional opening exposure. This read does not separate closing/reversing an existing position or validate a proposed order." }],
    references: [
      { url: "https://app.hyperliquid.xyz/assets/config-D-6TkbK9.js", finding: "Exchange frontend x3 maps buy to 0 and sell to 1. d8 indexes both arrays by side and treats maxTradeSzs as base size; rQ/Y8 display availableToTrade in the collateral token. These are not min/max pairs." },
      { url: "https://hyperliquid.gitbook.io/hyperliquid-docs/trading/contract-specifications", finding: "Main-DEX crypto perps use USDC collateral and a contract represents 1 unit of the underlying asset." },
      { url: "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/info-endpoint/perpetuals#retrieve-users-active-asset-data", finding: "HTTP examples show identity, leverage, markPx and two decimal strings per array, without array ordering or unit definitions." },
      { url: "https://github.com/hyperliquid-dex/hyperliquid-python-sdk/blob/master/hyperliquid/utils/types.py", finding: "ActiveAssetData declares maxTradeSzs and availableToTrade as Tuple[str, str], without ordering or unit definitions." },
      { url: "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/info-endpoint#query-a-users-abstraction-state", finding: "userAbstraction takes the exact user address and returns an account mode, including dexAbstraction." },
    ],
    warning: "Read-only venue observations, not a recommendation, executable quote, order approval or execution guarantee. Buy/sell denote order direction, not guaranteed new long/short exposure. maxTradeSizeBase is the raw venue side limit, not maximum opening size or a limit adjusted for a proposed price. availableToTradeUsdc is not freely withdrawable cash or an account-wide spending budget. Do not add sides together. Account mode is queried for this exact account, not inferred from balances. HTTP reads are separate, not atomic. Fresh means fetched during this call; timestamps are host UTC milliseconds, not venue timestamps. Data can change immediately. Other DEXs and spot instruments are outside this view.",
  };
  if (address === null) {
    for (const section of ["metadata", "mode", "activeAssetData"] as const) {
      result.reasons.push({ section, status: "unavailable", code: "no_assigned_account", message: "No trading account is assigned to this Agent." });
    }
    return result;
  }

  const signal = AbortSignal.any([AbortSignal.timeout(10_000), ...(cancellation ? [cancellation] : [])]);
  const client = new InfoClient({ transport });
  const user = address as `0x${string}`;
  const unavailable = (section: Section, code: Static<typeof reasonSchema>["code"], message: string) => {
    result.reasons.push({ section, status: "unavailable", code, message });
    return false;
  };
  const request = async (section: Section, read: () => Promise<unknown>, accept: (raw: unknown) => boolean) => {
    result.sections[section].requestedAt = Date.now();
    try {
      signal.throwIfAborted();
      const raw = await read();
      signal.throwIfAborted();
      if (!accept(raw)) return false;
      result.sections[section].observedAt = Date.now();
      result.sections[section].fresh = true;
      return true;
    } catch {
      cancellation?.throwIfAborted();
      return unavailable(section, signal.aborted ? "deadline_exceeded" : "request_failed",
        signal.aborted ? "The capacity read exceeded its deadline." : "The Hyperliquid HTTP read failed; missing data is not zero.");
    }
  };

  const [instrumentVerified] = await Promise.all([
    request("metadata", () => client.meta({ dex: "" }, signal), (raw) => {
      if (!Check(metadataSchema, raw)) return unavailable("metadata", "invalid_response", "Main-DEX metadata is malformed.");
      const matches = raw.universe.filter((asset) => asset.name === instrument);
      if (matches.length > 1) return unavailable("metadata", "invalid_response", "Main-DEX metadata contains duplicate instrument identities.");
      if (matches.length === 0 || matches[0]!.isDelisted) return unavailable("metadata", "unsupported_instrument", "Unknown or delisted main-DEX perpetual instrument.");
      return true;
    }),
    request("mode", () => client.userAbstraction({ user }, signal), (raw) => {
      if (!Check(accountModeSchema, raw)) return unavailable("mode", "invalid_response", "The account mode is unknown or malformed; do not infer a mode.");
      result.accountAbstraction = raw;
      return true;
    }),
  ]);
  if (instrumentVerified) {
    await request("activeAssetData", () => client.activeAssetData({ user, coin: instrument }, signal), (raw) => {
      if (!Check(activeAssetDataSchema, raw)) return unavailable("activeAssetData", "invalid_response", "Active asset data has malformed identity, decimal fields, leverage or array lengths.");
      if (raw.user.toLowerCase() !== address.toLowerCase() || raw.coin !== instrument) {
        return unavailable("activeAssetData", "identity_mismatch", "Active asset data does not match the assigned account and selected instrument.");
      }
      result.activeAssetData = {
        user: raw.user, coin: raw.coin,
        leverage: raw.leverage.type === "cross" ? { type: "cross", value: raw.leverage.value }
          : { type: "isolated", value: raw.leverage.value, rawUsd: raw.leverage.rawUsd },
        maxTradeSzs: [...raw.maxTradeSzs], availableToTrade: [...raw.availableToTrade], markPx: raw.markPx,
      };
      result.venueCapacity = {
        buy: { availableToTradeUsdc: raw.availableToTrade[0], maxTradeSizeBase: raw.maxTradeSzs[0] },
        sell: { availableToTradeUsdc: raw.availableToTrade[1], maxTradeSizeBase: raw.maxTradeSzs[1] },
      };
      result.capacityStatus = "observed";
      return true;
    });
  } else {
    unavailable("activeAssetData", "instrument_unverified", "Active asset data was not requested because the main-DEX instrument could not be verified.");
  }
  cancellation?.throwIfAborted();
  result.observedAt = Date.now();
  return result;
}
