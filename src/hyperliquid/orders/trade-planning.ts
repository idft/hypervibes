import type { InfoClient } from "@nktkas/hyperliquid";
import { HyperliquidError } from "../errors.js";
import { compare, decimal, guardPrice, notionalAtLeast, positionSize, price, size } from "./order-math.js";
import { instrument, instruments } from "./order-prepare.js";
import type { OrderInput } from "./order-contracts.js";

export interface ProtectedEntryInput { instrument: string; side: "buy" | "sell"; entry_style: "guarded_ioc" | "limit"; size: string; limit_price?: string; take_profit_price: string; stop_loss_price: string }
export interface PositionProtectionInput { instrument: string; take_profit_price: string; stop_loss_price: string }

/** Run before schema validation as well, so callers see the allowed values. */
export function assertEntryStyle(value: unknown): void {
  if (value !== "guarded_ioc" && value !== "limit") throw new HyperliquidError(`entry_style must be "guarded_ioc" or "limit"; received ${JSON.stringify(value) ?? "missing"}. Use "guarded_ioc" for a market-like 0.5%-guarded IOC entry, not "market". Use "limit" with an explicit limit_price for GTC.`);
}

/**
 * Read-only protected-entry planner. Strategy, sizing and protection levels
 * are model/worker decisions from instructions and insights; this only
 * enforces exact venue arithmetic and side sanity, never policy.
 */
export async function prepareProtectedEntry(info: InfoClient, input: ProtectedEntryInput, signal: AbortSignal) {
  assertEntryStyle(input.entry_style);
  if (!["buy", "sell"].includes(input.side)) throw new HyperliquidError('side must be "buy" or "sell"');
  if (input.entry_style === "guarded_ioc" && input.limit_price !== undefined) throw new HyperliquidError("Omit limit_price for guarded IOC planning");
  if (input.entry_style === "limit" && input.limit_price === undefined) throw new HyperliquidError("Explicit limit_price is required for limit planning");
  const entry = instrument(await instruments(info, signal), input.instrument);
  const observedAt = Date.now();
  const quote = (await info.allMids({ dex: "" }, signal))[input.instrument];
  if (typeof quote !== "string" || !quote) throw new HyperliquidError("Current string-valued market quote unavailable", 503);
  decimal(quote);
  const buy = input.side === "buy";
  const px = input.entry_style === "limit" ? price(input.limit_price!, entry.szDecimals) : guardPrice(quote, buy, entry.szDecimals);
  const qty = size(input.size, entry.szDecimals);
  if (!notionalAtLeast(qty, px, "10")) throw new HyperliquidError("Draft is below the $10 minimum entry notional; do not silently increase size");
  const tp = price(input.take_profit_price, entry.szDecimals), sl = price(input.stop_loss_price, entry.szDecimals);
  if (buy ? compare(tp, px) <= 0 || compare(sl, px) >= 0 : compare(tp, px) >= 0 || compare(sl, px) <= 0) throw new HyperliquidError("TP/SL is on the wrong side of the guarded/explicit entry");
  const order: OrderInput = { symbol: input.instrument, side: input.side, order_type: "limit", size: qty, price: px, time_in_force: input.entry_style === "guarded_ioc" ? "ioc" : "gtc", take_profits: [{ trigger_price: tp, size: qty }], stop_losses: [{ trigger_price: sl, size: qty }] };
  return { orders: [order], quote, quoteSource: "Hyperliquid main-DEX allMids HTTP (host observation, not venue timestamp)", observedAt, validUntil: observedAt + 15000, calculationBasis: input.entry_style === "guarded_ioc" ? "0.5% guarded IOC limit" : "Explicit exact-decimal limit price", warning: "Draft only; no authority or protection guarantee. Submit revalidates prices and current state. Entry size and TP/SL levels are your strategy decisions within the Agent's risk settings; never silently increase size." };
}

/** Read-only planner for standalone reduce-only protection of an existing position. */
export async function preparePositionProtection(info: InfoClient, input: PositionProtectionInput, account: string, signal: AbortSignal) {
  const observedAt = Date.now();
  const [meta, state, mids] = await Promise.all([instruments(info, signal), info.clearinghouseState({ user: account as `0x${string}`, dex: "" }, signal), info.allMids({ dex: "" }, signal)]);
  const asset = instrument(meta, input.instrument), position = state.assetPositions.find(({ position }) => position.coin === input.instrument)?.position;
  const exposure = positionSize(position?.szi ?? "0");
  if (exposure.side === "flat") throw new HyperliquidError("No current position exists for protective planning");
  const quote = mids[input.instrument];
  if (typeof quote !== "string") throw new HyperliquidError("Current string-valued quote unavailable", 503);
  decimal(quote);
  const long = exposure.side === "long", quantity = size(exposure.size, asset.szDecimals);
  const tp = price(input.take_profit_price, asset.szDecimals), sl = price(input.stop_loss_price, asset.szDecimals);
  if (long ? compare(tp, quote) <= 0 || compare(sl, quote) >= 0 : compare(tp, quote) >= 0 || compare(sl, quote) <= 0) throw new HyperliquidError("Protection levels have crossed the current market; explicit review is required, not unapproved flattening or invented replacement prices");
  const orders: OrderInput[] = [
    { symbol: input.instrument, side: long ? "sell" : "buy", size: quantity, order_type: "take_profit", trigger_price: tp, reduce_only: true },
    { symbol: input.instrument, side: long ? "sell" : "buy", size: quantity, order_type: "stop_loss", trigger_price: sl, reduce_only: true },
  ];
  return { orders, quote, quoteSource: "Hyperliquid main-DEX allMids and clearinghouseState HTTP (host observation, not venue timestamp)", observedAt, validUntil: observedAt + 15000, calculationBasis: "Actual current signed position quantity; no new exposure or sizing increase", warning: "Draft only. Inspect known protective orders first and submit ONLY missing/appropriate legs; never duplicate or cancel live protection blindly. No perpetual coverage guarantee." };
}