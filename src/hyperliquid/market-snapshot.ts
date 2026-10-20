import { Type } from "@earendil-works/pi-ai";
import { InfoClient, type HttpTransport } from "@nktkas/hyperliquid";
import { HyperliquidError } from "./errors.js";
import { amount } from "./journal-decimal.js";

const decimal = Type.String({ pattern: "^-?[0-9]+(\\.[0-9]+)?$" });
const nullableDecimal = Type.Union([decimal, Type.Null()]);
export const MarketSnapshotSchema = Type.Object({
  instrument: Type.String(), scope: Type.Literal("main-dex-perpetuals"),
  source: Type.Literal("Hyperliquid metaAndAssetCtxs HTTP"), observedAt: Type.Integer(),
  midPrice: nullableDecimal, markPrice: decimal, oraclePrice: decimal,
  previousDayPrice: decimal, markChange24hPercent: nullableDecimal,
  volume24hUsd: decimal, volume24hBase: decimal, fundingRateHourly: decimal,
  openInterestBase: decimal, warning: Type.String(),
});

/** A single venue response keeps metadata and its index-aligned contexts together. */
export async function loadMarketSnapshot(transport: HttpTransport, instrument: string, signal?: AbortSignal) {
  if (!/^[A-Za-z0-9][A-Za-z0-9._-]{0,31}$/.test(instrument)) throw new HyperliquidError("Select a main-DEX perpetual instrument such as BTC.");
  const requestSignal = signal ? AbortSignal.any([signal, AbortSignal.timeout(10_000)]) : AbortSignal.timeout(10_000);
  requestSignal.throwIfAborted();
  const [meta, contexts] = await new InfoClient({ transport }).metaAndAssetCtxs({ dex: "" }, requestSignal);
  const observedAt = Date.now();
  const index = meta.universe.findIndex((asset) => asset.name === instrument && !asset.isDelisted);
  if (index < 0) throw new HyperliquidError("Unknown or delisted main-DEX perpetual instrument.", 404);
  const ctx = contexts[index];
  if (!ctx || contexts.length !== meta.universe.length) throw new HyperliquidError("Market snapshot context is unavailable.", 502);
  // Reject malformed data, rather than silently turning missing values into zero.
  const nonnegative = [ctx.markPx, ctx.oraclePx, ctx.prevDayPx, ctx.dayNtlVlm, ctx.dayBaseVlm, ctx.openInterest, ...(ctx.midPx === null ? [] : [ctx.midPx])];
  if (nonnegative.some((value) => amount(value).sign < 0)) throw new HyperliquidError("Invalid market snapshot amounts.", 502);
  amount(ctx.funding);
  const previous = amount(ctx.prevDayPx);
  return {
    instrument, scope: "main-dex-perpetuals" as const,
    source: "Hyperliquid metaAndAssetCtxs HTTP" as const, observedAt,
    midPrice: ctx.midPx, markPrice: ctx.markPx, oraclePrice: ctx.oraclePx,
    previousDayPrice: ctx.prevDayPx,
    markChange24hPercent: previous.zero ? null : amount(ctx.markPx).sub(previous).mul(amount("100")).div(previous).toString(),
    volume24hUsd: ctx.dayNtlVlm, volume24hBase: ctx.dayBaseVlm,
    fundingRateHourly: ctx.funding, openInterestBase: ctx.openInterest,
    warning: "Host observation time in UTC milliseconds, not a venue timestamp. Mid/mark/oracle prices are not executable quotes or last-trade prices. 24h change uses mark versus previous-day price; null means unavailable. Funding is an hourly decimal rate, not percent or APR. Volume is rolling 24h; open interest is base-asset quantity. Snapshot only, not candle/indicator research or trade authority.",
  };
}
