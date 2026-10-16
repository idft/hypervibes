import { InfoClient, type ClearinghouseStateResponse, type FrontendOpenOrdersResponse, type SpotClearinghouseStateResponse, type HttpTransport } from "@nktkas/hyperliquid";
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
  live?: {
    status: "connecting" | "connected" | "reconnecting" | "stopped";
    sections: Record<"perps" | "spot" | "orders", { updatedAt: number | null; fresh: boolean }>;
  };
}

export function dashboardPerps(state: ClearinghouseStateResponse) {
  return {
    perpEquity: state.marginSummary.accountValue,
    positions: state.assetPositions.filter(({ position }) => {
      const size = compareDecimals(position.szi, "0");
      return size !== null && size !== 0;
    }).map(({ position: p }) => ({ coin: p.coin, size: p.szi, entryPrice: p.entryPx, unrealizedPnl: p.unrealizedPnl, liquidationPrice: p.liquidationPx, leverage: p.leverage.value })),
  };
}
export function dashboardSpot(state: SpotClearinghouseStateResponse) {
  return { spotUsdc: state.balances.find((balance) => balance.coin === "USDC")?.total ?? "0" };
}
export function dashboardOrders(orders: FrontendOpenOrdersResponse) {
  return { orders: orders.map((order) => ({ id: order.oid, coin: order.coin, side: order.side === "B" ? "Buy" : "Sell", size: order.sz, price: order.limitPx, type: order.orderType })) };
}

/** Read-only main-DEX snapshots. No signer, persistence, or trading authority. */
export async function loadAgentDashboard(transport: HttpTransport, address: string | null, cancellation?: AbortSignal): Promise<AgentDashboard> {
  const result: AgentDashboard = { address, fetchedAt: Date.now(), perpEquity: null, spotUsdc: null, positions: null, orders: null, errors: [] };
  if (!address) return result;
  const client = new InfoClient({ transport });
  const user = address as `0x${string}`;
  const signal = AbortSignal.any([AbortSignal.timeout(8_000), ...(cancellation ? [cancellation] : [])]);
  const [perps, spot, orders] = await Promise.allSettled([
    client.clearinghouseState({ user }, signal),
    client.spotClearinghouseState({ user }, signal),
    client.frontendOpenOrders({ user }, signal),
  ]);
  if (perps.status === "fulfilled") {
    Object.assign(result, dashboardPerps(perps.value));
  } else result.errors.push("Perpetual balances and positions are temporarily unavailable.");
  if (spot.status === "fulfilled") Object.assign(result, dashboardSpot(spot.value));
  else result.errors.push("Spot USDC balance is temporarily unavailable.");
  if (orders.status === "fulfilled") Object.assign(result, dashboardOrders(orders.value));
  else result.errors.push("Open orders are temporarily unavailable.");
  result.fetchedAt = Date.now();
  return result;
}
