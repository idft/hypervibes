import type { InfoClient } from "@nktkas/hyperliquid";
import type { WorkerWorkflow } from "../../agents/worker-workflow.js";
import { HyperliquidError } from "../errors.js";
import { compare, decimal, guardPrice, notionalAtLeast, notionalWithin, positionSize, price, size } from "./order-math.js";
import { instrument, instruments } from "./order-prepare.js";
import type { OrderInput } from "./order-contracts.js";

export interface ProtectedEntryInput { instrument: string; side: "buy" | "sell"; entry_style: "guarded_ioc" | "limit"; size?: string; limit_price?: string; take_profit_price?: string; stop_loss_price?: string }
export interface PositionProtectionInput { instrument: string; take_profit_price?: string; stop_loss_price?: string }
export type TradingWorkflow = Extract<WorkerWorkflow, { purpose: "trading" }>;
export function protectionPrices(entry: string, buy: boolean, decimals: number, tpBps: number, slBps: number) {
  if (![tpBps, slBps].every((bps) => Number.isInteger(bps) && bps > 0 && bps < 10000)) throw new HyperliquidError("Invalid configured protection distances");
  const tp = guardPrice(entry, buy, decimals, tpBps);
  const sl = guardPrice(entry, !buy, decimals, slBps);
  if (compare(tp, entry) === 0 || compare(sl, entry) === 0) throw new HyperliquidError("Protection distance is too small for exchange tick precision");
  return { tp, sl };
}
export async function preparePositionProtection(info: InfoClient, input: PositionProtectionInput, policy: TradingWorkflow | null, userDirected: boolean, account: string, signal: AbortSignal) {
  const observedAt = Date.now();
  const [meta, state, mids] = await Promise.all([instruments(info, signal), info.clearinghouseState({ user: account as `0x${string}`, dex: "" }, signal), info.allMids({ dex: "" }, signal)]);
  const asset = instrument(meta, input.instrument), position = state.assetPositions.find(({ position }) => position.coin === input.instrument)?.position;
  const exposure = positionSize(position?.szi ?? "0");
  if (exposure.side === "flat") throw new HyperliquidError("No current position exists for protective planning");
  const quote = mids[input.instrument];
  if (typeof quote !== "string") throw new HyperliquidError("Current string-valued quote unavailable", 503);
  decimal(quote);
  const long = exposure.side === "long", quantity = size(exposure.size, asset.szDecimals);
  if (!userDirected && (input.take_profit_price !== undefined || input.stop_loss_price !== undefined)) throw new HyperliquidError("Automatic protection planning uses configured distances, not authored overrides");
  const calculated = policy && position?.entryPx ? protectionPrices(position.entryPx, long, asset.szDecimals, policy.takeProfitBps, policy.stopLossBps) : null;
  const tp = price(input.take_profit_price ?? calculated?.tp ?? "", asset.szDecimals), sl = price(input.stop_loss_price ?? calculated?.sl ?? "", asset.szDecimals);
  if (calculated && (long ? compare(sl, calculated.sl) < 0 : compare(sl, calculated.sl) > 0)) throw new HyperliquidError("Protective stop cannot widen the configured maximum loss distance");
  if (long ? compare(tp, quote) <= 0 || compare(sl, quote) >= 0 : compare(tp, quote) >= 0 || compare(sl, quote) <= 0) throw new HyperliquidError("Configured protective levels have crossed the current market; explicit review is required, not unapproved flattening or invented replacement prices");
  const orders: OrderInput[] = [
    { symbol: input.instrument, side: long ? "sell" : "buy", size: quantity, order_type: "take_profit", trigger_price: tp, reduce_only: true },
    { symbol: input.instrument, side: long ? "sell" : "buy", size: quantity, order_type: "stop_loss", trigger_price: sl, reduce_only: true },
  ];
  return { orders, quote, quoteSource: "Hyperliquid main-DEX allMids and clearinghouseState HTTP (host observation, not venue timestamp)", observedAt, validUntil: observedAt + 15000, calculationBasis: "Actual current signed position quantity and exact entry-price/basis-point arithmetic; no new exposure or sizing increase", warning: "Draft only. Inspect known protective orders first and submit ONLY missing/appropriate legs; never duplicate or cancel live protection blindly. Crossed/unknown levels require review. No perpetual coverage guarantee." };
}
export async function prepareProtectedEntry(info: InfoClient, input: ProtectedEntryInput, policy: TradingWorkflow | null, userDirected: boolean, signal: AbortSignal) {
  if (!["buy", "sell"].includes(input.side) || !["guarded_ioc", "limit"].includes(input.entry_style)) throw new HyperliquidError("Unsupported entry side/style");
  if (!userDirected && (!policy || !policy.instruments.includes(input.instrument))) throw new HyperliquidError("Automatic planning requires an applicable saved trading workflow", 409);
  if (policy && !policy.instruments.includes(input.instrument)) throw new HyperliquidError("Instrument is outside this worker's policy");
  const entry = instrument(await instruments(info, signal), input.instrument);
  const observedAt = Date.now();
  const quote = (await info.allMids({ dex: "" }, signal))[input.instrument];
  if (typeof quote !== "string" || !quote) throw new HyperliquidError("Current string-valued market quote unavailable", 503);
  decimal(quote);
  const buy = input.side === "buy";
  if (input.entry_style === "guarded_ioc" && input.limit_price !== undefined) throw new HyperliquidError("Omit limit_price for guarded IOC planning");
  const px = input.entry_style === "limit" ? price(input.limit_price ?? "", entry.szDecimals) : guardPrice(quote, buy, entry.szDecimals);
  const qty = size(input.size ?? policy?.baseSizeByInstrument[input.instrument] ?? "", entry.szDecimals);
  if (!notionalAtLeast(qty, px, "10")) throw new HyperliquidError("Draft is below the $10 minimum entry notional; do not silently increase size");
  if (policy && (compare(qty, policy.baseSizeByInstrument[input.instrument] ?? "") > 0 || !notionalWithin(qty, px, policy.maxEntryNotional))) throw new HyperliquidError("Draft exceeds configured base size/notional cap");
  if (!userDirected && input.size !== undefined && compare(input.size, policy!.baseSizeByInstrument[input.instrument]!) !== 0) throw new HyperliquidError("Automatic sizing must use the configured base quantity");
  const calculated = policy ? protectionPrices(px, buy, entry.szDecimals, policy.takeProfitBps, policy.stopLossBps) : null;
  if (!userDirected && (input.take_profit_price !== undefined || input.stop_loss_price !== undefined)) throw new HyperliquidError("Automatic protection comes from the saved workflow");
  const tp = price(input.take_profit_price ?? calculated?.tp ?? "", entry.szDecimals), sl = price(input.stop_loss_price ?? calculated?.sl ?? "", entry.szDecimals);
  if ((buy ? compare(tp, px) <= 0 || compare(sl, px) >= 0 : compare(tp, px) >= 0 || compare(sl, px) <= 0)) throw new HyperliquidError("TP/SL is on the wrong side of the guarded/explicit entry");
  if (calculated && (buy ? compare(sl, calculated.sl) < 0 : compare(sl, calculated.sl) > 0)) throw new HyperliquidError("Explicit stop-loss cannot exceed the configured maximum loss distance");
  const order: OrderInput = { symbol: input.instrument, side: input.side, order_type: "limit", size: qty, price: px, time_in_force: input.entry_style === "guarded_ioc" ? "ioc" : "gtc", take_profits: [{ trigger_price: tp, size: qty }], stop_losses: [{ trigger_price: sl, size: qty }] };
  return { orders: [order], quote, quoteSource: "Hyperliquid main-DEX allMids HTTP (host observation, not venue timestamp)", observedAt, validUntil: observedAt + 15000, calculationBasis: input.entry_style === "guarded_ioc" ? "0.5% guarded IOC limit; TP/SL calculated relative to this guard, not floating-point TA" : "Explicit exact-decimal limit price", warning: "Draft only; no authority, entry claim, or protection guarantee. Submit revalidates prices, current state and research. Never silently increase size.", };
}
