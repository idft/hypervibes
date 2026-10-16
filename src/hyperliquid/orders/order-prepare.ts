import { randomBytes, randomUUID } from "node:crypto";
import type { InfoClient, FrontendOpenOrdersResponse } from "@nktkas/hyperliquid";
import { HyperliquidError } from "../errors.js";
import { compare, guardPrice, price, size } from "./order-math.js";
import type { OrderInput, OrderView } from "./order-contracts.js";
import type { PreparedAction, WireOrder } from "./order-store.js";

export function newView(requestId: string, symbol: string, kind: string): OrderView {
  return { id: randomUUID(), requestId, symbol, side: "", kind, size: null, price: null, triggerPrice: null, reduceOnly: false, cloid: null, exchangeOid: null,
    commandStatus: "not_sent", orderStatus: "not_sent", filledSize: null, averageFillPrice: null, updatedAt: Date.now(), venueTimestamp: null, source: "intent", error: null };
}
export function liveView(order: FrontendOpenOrdersResponse[number], status = "open", timestamp: number | null = null): OrderView {
  if (!Number.isSafeInteger(order.oid) || order.oid <= 0) throw new HyperliquidError("Exchange returned an unsafe order ID.", 502);
  return { ...newView("", order.coin, order.orderType), id: String(order.oid), requestId: null,
    side: order.side === "B" ? "buy" : "sell", size: order.origSz ?? order.sz, price: order.limitPx, triggerPrice: order.isTrigger ? order.triggerPx : null,
    reduceOnly: order.reduceOnly, cloid: order.cloid ?? null, exchangeOid: String(order.oid), commandStatus: "reconciled", orderStatus: status,
    updatedAt: Date.now(), venueTimestamp: timestamp, source: "exchange" };
}
export function isPerpetual(coin: string) { return !coin.startsWith("@") && !coin.includes("/") && !coin.includes(":") && !coin.startsWith("+") && !/^o\d+$/.test(coin); }
export function exchangeOid(oid: string): number {
  if (typeof oid !== "string" || !/^[1-9]\d{0,15}$/.test(oid)) throw new HyperliquidError("Use an exchange order ID as a decimal string.");
  const number = Number(oid);
  if (!Number.isSafeInteger(number)) throw new HyperliquidError("Exchange order ID exceeds safe integer range.");
  return number;
}
export async function instruments(info: InfoClient, signal: AbortSignal) {
  const meta = await info.meta({ dex: "" }, signal);
  return new Map(meta.universe.map((entry, asset) => [entry.name, { ...entry, asset }]));
}
export type Instruments = Awaited<ReturnType<typeof instruments>>;
export function instrument(meta: Instruments, symbol: string) {
  const entry = meta.get(symbol);
  if (!entry || !isPerpetual(symbol) || entry.isDelisted || !Number.isInteger(entry.szDecimals) || entry.szDecimals < 0 || entry.szDecimals > 6) throw new HyperliquidError(`Unknown, delisted, or unsupported main-DEX perpetual: ${symbol}`);
  return entry;
}
export async function prepareSubmit(info: InfoClient, requestId: string, inputs: OrderInput[], builder: { b: `0x${string}`; f: number }, signal: AbortSignal): Promise<PreparedAction[]> {
  const meta = await instruments(info, signal);
  const mids = inputs.some((input) => input.order_type === "market") ? await info.allMids({ dex: "" }, signal) : {};
  return inputs.map((input) => {
    const entry = instrument(meta, input.symbol);
    const buy = input.side === "buy";
    if (!["buy", "sell"].includes(input.side)) throw new HyperliquidError("Invalid order side.");
    const amount = size(input.size, entry.szDecimals);
    const attached = [...(input.take_profits ?? []), ...(input.stop_losses ?? [])];
    if ((input.take_profits?.length ?? 0) > 1 || (input.stop_losses?.length ?? 0) > 1) throw new HyperliquidError("Native parent TP/SL supports one take-profit and one stop-loss per entry.");
    if (input.reduce_only && attached.length) throw new HyperliquidError("Attach TP/SL only to an opening entry; use standalone reduce-only triggers to protect an existing position.");
    const orders: WireOrder[] = [], items: OrderView[] = [];
    const add = (kind: string, side: boolean, qty: string, px: string, reduce: boolean, type: WireOrder["t"], trigger: string | null = null) => {
      const cloid = `0x${randomBytes(16).toString("hex")}` as const;
      orders.push({ a: entry.asset, b: side, p: px, s: qty, r: reduce, t: type, c: cloid });
      items.push({ ...newView(requestId, input.symbol, kind), side: side ? "buy" : "sell", size: qty, price: px, triggerPrice: trigger, reduceOnly: reduce, cloid });
    };
    if (input.order_type === "limit" || input.order_type === "market") {
      if (input.trigger_price !== undefined) throw new HyperliquidError("Use trigger_price only for standalone triggers.");
      if (input.order_type === "market" && (input.price !== undefined || input.time_in_force !== undefined)) throw new HyperliquidError("Market entries use a 0.5% guarded IOC; omit price/time_in_force.");
      const px = input.order_type === "market" ? guardPrice(mids[input.symbol] ?? "", buy, entry.szDecimals) : price(input.price ?? "", entry.szDecimals);
      const tif = input.order_type === "market" ? "Ioc" : input.time_in_force === "ioc" ? "Ioc" : input.time_in_force === "alo" ? "Alo" : "Gtc";
      add(input.order_type, buy, amount, px, input.reduce_only ?? false, { limit: { tif } });
      const trigger = (leg: NonNullable<OrderInput["take_profits"]>[number], kind: "tp" | "sl") => {
        const triggerPx = price(leg.trigger_price, entry.szDecimals);
        const qty = size(leg.size ?? amount, entry.szDecimals);
        if (compare(qty, amount) > 0) throw new HyperliquidError("Attached trigger size cannot exceed entry size.");
        const direction = compare(triggerPx, px);
        if ((kind === "tp" ? buy ? direction <= 0 : direction >= 0 : buy ? direction >= 0 : direction <= 0)) throw new HyperliquidError("Attached TP/SL trigger is on the wrong side of the entry price.");
        if (kind === "sl" && leg.limit_price !== undefined) throw new HyperliquidError("Attached stop-loss is market-on-trigger; omit limit_price.");
        const isMarket = leg.limit_price === undefined;
        const limitPx = isMarket ? guardPrice(triggerPx, !buy, entry.szDecimals, 1000) : price(leg.limit_price!, entry.szDecimals);
        add(kind === "tp" ? "take_profit" : "stop_loss", !buy, qty, limitPx, true, { trigger: { triggerPx, isMarket, tpsl: kind } }, triggerPx);
      };
      for (const leg of input.take_profits ?? []) trigger(leg, "tp");
      for (const leg of input.stop_losses ?? []) trigger(leg, "sl");
    } else if (input.order_type === "take_profit" || input.order_type === "stop_loss") {
      if (attached.length || input.time_in_force !== undefined || input.reduce_only === false) throw new HyperliquidError("Standalone TP/SL must be reduce-only, without attached legs or time_in_force.");
      const triggerPx = price(input.trigger_price ?? "", entry.szDecimals);
      const isMarket = input.price === undefined;
      const px = isMarket ? guardPrice(triggerPx, buy, entry.szDecimals, 1000) : price(input.price!, entry.szDecimals);
      add(input.order_type, buy, amount, px, true, { trigger: { triggerPx, isMarket, tpsl: input.order_type === "take_profit" ? "tp" : "sl" } }, triggerPx);
    } else throw new HyperliquidError("Unsupported order_type.");
    return { payload: { kind: "submit", orders, grouping: attached.length ? "normalTpsl" : "na", builder }, items };
  });
}

/** Reject numeric money before Durable's permissive schema coercion can stringify it. */
export function assertStringAmounts(value: unknown): void {
  if (Array.isArray(value)) { for (const item of value) assertStringAmounts(item); return; }
  if (!value || typeof value !== "object") return;
  for (const [key, item] of Object.entries(value)) {
    if (["size", "price", "trigger_price", "limit_price", "oid"].includes(key) && typeof item !== "string") throw new HyperliquidError(`${key} must be an exact decimal string, not a JSON number.`);
    assertStringAmounts(item);
  }
}
