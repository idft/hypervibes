import { HyperliquidError } from "../errors.js";
import { exchangeOid } from "./order-prepare.js";
import type { FrontendOpenOrdersResponse, InfoClient } from "@nktkas/hyperliquid";
import { compare, positionSize } from "./order-math.js";

export async function verifyProtection(info: Pick<InfoClient, "orderStatus">, account: string, positions: readonly { coin: string; size: string }[], orders: FrontendOpenOrdersResponse, quotes: Readonly<Record<string, string>>, signal: AbortSignal) {
  if (orders.length > 100 || positions.length > 100) throw new HyperliquidError("Protection inspection exceeds the 100-order/position budget", 409);
  const verified: FrontendOpenOrdersResponse = [], statuses = new Map<number, string>();
  for (const order of orders) if (order.isTrigger && order.reduceOnly) {
    const oid = exchangeOid(String(order.oid));
    const response = await info.orderStatus({ user: account as `0x${string}`, oid }, signal);
    if (response.status === "order" && response.order.order.oid === oid && response.order.order.coin === order.coin) {
      // Re-read size/side/type/trigger from the individual response too: an old
      // frontendOpenOrders quantity must not mask a subsequent partial fill.
      verified.push(response.order.order); statuses.set(oid, response.order.status);
    }
  }
  return inspectProtection(positions, verified, statuses, quotes);
}
/** Only top-level live triggers with individually verified statuses count.
 * Parent.children/waitingForFill are deliberately NOT active position protection. */
export function inspectProtection(positions: readonly { coin: string; size: string }[], orders: FrontendOpenOrdersResponse, statuses: ReadonlyMap<number, string>, quotes: Readonly<Record<string, string>> = {}) {
  return positions.flatMap((position) => {
    const exposure = positionSize(position.size);
    if (exposure.side === "flat") return [];
    const side = exposure.side === "long" ? "A" : "B";
    const candidates = orders.filter((order) => order.coin === position.coin && order.side === side && order.reduceOnly && order.isTrigger && ["open", "waitingForTrigger"].includes(statuses.get(order.oid) ?? "") && positionSize(order.sz).side !== "flat" && compare(order.sz, exposure.size) >= 0);
    const quote = quotes[position.coin];
    const tp = candidates.filter((order) => (order.orderType === "Take Profit Market" || order.orderType === "Take Profit Limit") && quote !== undefined && (exposure.side === "long" ? compare(order.triggerPx, quote) > 0 : compare(order.triggerPx, quote) < 0));
    const sl = candidates.filter((order) => order.orderType === "Stop Market" && quote !== undefined && (exposure.side === "long" ? compare(order.triggerPx, quote) < 0 : compare(order.triggerPx, quote) > 0));
    return [{ instrument: position.coin, side: exposure.side, size: exposure.size, takeProfitOids: tp.map((order) => String(order.oid)), stopLossOids: sl.map((order) => String(order.oid)), covered: tp.length > 0 && sl.length > 0, warning: "Observed per-leg state only, not a perpetual coverage guarantee. Stop-limit legs are not treated as reliable full-size loss protection. Inspect partial fills and uncertain/rejected legs; repair with standalone reduce-only orders, never blind cancel-all." }];
  });
}
