import { InfoClient, type ClearinghouseStateResponse, type FrontendOpenOrdersResponse, type SpotClearinghouseStateResponse, type HttpTransport, type UserAbstractionResponse } from "@nktkas/hyperliquid";
import { compareDecimals } from "./decimal.js";

export interface DashboardPosition {
  coin: string; size: string; entryPrice: string; unrealizedPnl: string;
  liquidationPrice: string | null; leverage: number;
}
export interface DashboardOrder {
  id: number; coin: string; side: string; size: string; price: string; type: string;
  /** Trigger orders must be plotted at the trigger, not their execution limit. */
  triggerPrice?: string;
  timestamp?: number;
  positionTpsl?: true;
  children?: DashboardOrder[];
}
export interface BalancePoint { time: number; balance: string }
export interface BalanceHistory {
  day: BalancePoint[];
  month: BalancePoint[];
}
export type DashboardSection = "perps" | "spot" | "orders" | "mode";
export interface AgentDashboard {
  accountAbstraction: UserAbstractionResponse | null;
  balanceHistory?: BalanceHistory | null;
  address: string | null;
  fetchedAt: number;
  /** Raw main-DEX summary; not separate equity in unified/portfolio modes. */
  perpEquity: string | null;
  spotUsdc: string | null;
  spotUsdcHold?: string | null;
  positions: DashboardPosition[] | null;
  orders: DashboardOrder[] | null;
  errors: string[];
  live?: {
    status: "connecting" | "connected" | "reconnecting" | "stopped";
    sections: Record<DashboardSection, { updatedAt: number | null; fresh: boolean }>;
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
  const usdc = state.balances.find((balance) => balance.coin === "USDC");
  return { spotUsdc: usdc?.total ?? "0", spotUsdcHold: usdc ? usdc.hold ?? null : "0" };
}
export function dashboardOrders(orders: FrontendOpenOrdersResponse) {
  const convert = (order: FrontendOpenOrdersResponse[number]): DashboardOrder => ({ id: order.oid, coin: order.coin, side: order.side === "B" ? "Buy" : "Sell", size: order.sz, price: order.limitPx, type: order.orderType,
    ...(Number.isFinite(order.timestamp) ? { timestamp: order.timestamp } : {}),
    ...(order.isTrigger ? { triggerPrice: order.triggerPx } : {}), ...(order.isPositionTpsl ? { positionTpsl: true as const } : {}),
    ...(order.children?.length ? { children: order.children.map(convert) } : {}),
  });
  return { orders: orders.map(convert) };
}

/** Read-only main-DEX snapshots. No signer, persistence, or trading authority. */
export async function loadAgentDashboard(transport: HttpTransport, address: string | null, cancellation?: AbortSignal): Promise<AgentDashboard> {
  const result: AgentDashboard = { address, accountAbstraction: null, fetchedAt: Date.now(), perpEquity: null, spotUsdc: null, positions: null, orders: null, errors: [] };
  if (!address) return result;
  const client = new InfoClient({ transport });
  const user = address as `0x${string}`;
  const signal = AbortSignal.any([AbortSignal.timeout(8_000), ...(cancellation ? [cancellation] : [])]);
  const [perps, spot, orders, mode] = await Promise.allSettled([
    client.clearinghouseState({ user }, signal),
    client.spotClearinghouseState({ user }, signal),
    client.frontendOpenOrders({ user }, signal),
    client.userAbstraction({ user }, signal),
  ]);
  if (perps.status === "fulfilled") {
    Object.assign(result, dashboardPerps(perps.value));
  } else result.errors.push("Perpetual balances and positions are temporarily unavailable.");
  if (spot.status === "fulfilled") Object.assign(result, dashboardSpot(spot.value));
  else result.errors.push("Spot USDC balance is temporarily unavailable.");
  if (orders.status === "fulfilled") Object.assign(result, dashboardOrders(orders.value));
  else result.errors.push("Open orders are temporarily unavailable.");
  if (mode.status === "fulfilled") result.accountAbstraction = mode.value;
  else result.errors.push("Account mode is temporarily unavailable; total equity cannot be determined.");
  result.fetchedAt = Date.now();
  return result;
}
