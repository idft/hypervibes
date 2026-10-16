import { InfoClient, type HttpTransport } from "@nktkas/hyperliquid";
import { compareDecimals } from "./decimal.js";

export interface DashboardPosition {
  coin: string; size: string; entryPrice: string; unrealizedPnl: string;
  liquidationPrice: string | null; leverage: number;
}
export interface DashboardOrder {
  id: number; coin: string; side: string; size: string; price: string; type: string;
}
export interface AgentDashboard {
  address: string | null;
  fetchedAt: number;
  perpEquity: string | null;
  spotUsdc: string | null;
  positions: DashboardPosition[] | null;
  orders: DashboardOrder[] | null;
  errors: string[];
}

/** Read-only main-DEX snapshots. No signer, persistence, or trading authority. */
export async function loadAgentDashboard(transport: HttpTransport, address: string | null): Promise<AgentDashboard> {
  const result: AgentDashboard = { address, fetchedAt: Date.now(), perpEquity: null, spotUsdc: null, positions: null, orders: null, errors: [] };
  if (!address) return result;
  const client = new InfoClient({ transport });
  const user = address as `0x${string}`;
  const signal = AbortSignal.timeout(8_000);
  const [perps, spot, orders] = await Promise.allSettled([
    client.clearinghouseState({ user }, signal),
    client.spotClearinghouseState({ user }, signal),
    client.frontendOpenOrders({ user }, signal),
  ]);
  if (perps.status === "fulfilled") {
    result.perpEquity = perps.value.marginSummary.accountValue;
    result.positions = perps.value.assetPositions.filter(({ position }) => {
      const size = compareDecimals(position.szi, "0");
      return size !== null && size !== 0;
    }).map(({ position: p }) => ({ coin: p.coin, size: p.szi, entryPrice: p.entryPx, unrealizedPnl: p.unrealizedPnl, liquidationPrice: p.liquidationPx, leverage: p.leverage.value }));
  } else result.errors.push("Perpetual balances and positions are temporarily unavailable.");
  if (spot.status === "fulfilled") result.spotUsdc = spot.value.balances.find((balance) => balance.coin === "USDC")?.total ?? "0";
  else result.errors.push("Spot USDC balance is temporarily unavailable.");
  if (orders.status === "fulfilled") result.orders = orders.value.map((order) => ({ id: order.oid, coin: order.coin, side: order.side === "B" ? "Buy" : "Sell", size: order.sz, price: order.limitPx, type: order.orderType }));
  else result.errors.push("Open orders are temporarily unavailable.");
  result.fetchedAt = Date.now();
  return result;
}
