import { InfoClient, type HttpTransport, type UserAbstractionResponse } from "@nktkas/hyperliquid";
import { dashboardOrders, dashboardPerps, dashboardSpot, type AgentDashboard, type DashboardOrder, type DashboardPosition } from "./agent-dashboard.js";

export interface AccountStatus {
  address: string | null;
  scope: "main-dex-perpetuals-and-spot-usdc";
  observedAt: number;
  accountAbstraction: UserAbstractionResponse | null;
  /** Raw main-DEX summary, NOT account equity under unified/portfolio margin. */
  perpEquity: string | null;
  spotUsdc: string | null;
  spotUsdcHold: string | null;
  positions: DashboardPosition[] | null;
  orders: DashboardOrder[] | null;
  sections: Record<"perps" | "spot" | "orders" | "mode", { updatedAt: number | null; fresh: boolean; source: "live" | "http" | "unavailable" }>;
  errors: string[];
}

function perpetualOrder(coin: string) { return !coin.startsWith("@") && !coin.includes("/"); }

/** A bounded-age read of the shared feed, repaired without modifying its state. */
export async function loadAccountStatus(transport: HttpTransport, address: string | null, cached?: AgentDashboard, cancellation?: AbortSignal): Promise<AccountStatus> {
  const now = Date.now();
  const result: AccountStatus = {
    address, scope: "main-dex-perpetuals-and-spot-usdc", observedAt: now, accountAbstraction: null,
    perpEquity: cached?.perpEquity ?? null, positions: cached?.positions ?? null,
    spotUsdc: cached?.spotUsdc ?? null, spotUsdcHold: cached?.spotUsdcHold ?? null,
    orders: cached?.orders?.filter((order) => perpetualOrder(order.coin)) ?? null,
    sections: {
      perps: { updatedAt: null, fresh: false, source: "unavailable" },
      spot: { updatedAt: null, fresh: false, source: "unavailable" },
      orders: { updatedAt: null, fresh: false, source: "unavailable" },
      mode: { updatedAt: null, fresh: false, source: "unavailable" },
    }, errors: [],
  };
  if (!address) { result.errors.push("No trading account is assigned to this Agent."); return result; }
  for (const section of ["perps", "spot", "orders"] as const) {
    const state = cached?.live?.sections[section];
    if (state?.updatedAt !== null && state?.updatedAt !== undefined) {
      const complete = section === "perps" ? result.perpEquity !== null && result.positions !== null
        : section === "spot" ? result.spotUsdc !== null && result.spotUsdcHold !== null : result.orders !== null;
      result.sections[section] = { updatedAt: state.updatedAt, fresh: complete && state.fresh && cached?.live?.status === "connected" && now - state.updatedAt <= 30_000, source: "live" };
    }
  }
  const client = new InfoClient({ transport });
  const user = address as `0x${string}`;
  const signal = AbortSignal.any([AbortSignal.timeout(8_000), ...(cancellation ? [cancellation] : [])]);
  const errors = {
    perps: "Perpetual balances and positions are unavailable; any retained values are stale.",
    spot: "Spot USDC balance and holds are unavailable; any retained values are stale.",
    orders: "Open perpetual orders are unavailable; any retained values are stale.",
    mode: "Account mode is unavailable. Do not infer collateral availability from balances or the raw perp summary.",
  };
  await Promise.all((["perps", "spot", "orders", "mode"] as const).map(async (section) => {
    if (result.sections[section].fresh) return;
    try {
      if (section === "perps") Object.assign(result, dashboardPerps(await client.clearinghouseState({ user, dex: "" }, signal)));
      else if (section === "spot") Object.assign(result, dashboardSpot(await client.spotClearinghouseState({ user }, signal)));
      else if (section === "mode") result.accountAbstraction = await client.userAbstraction({ user }, signal);
      else {
        const orders = await client.frontendOpenOrders({ user, dex: "" }, signal);
        result.orders = dashboardOrders(orders.filter((order) => perpetualOrder(order.coin))).orders;
      }
      result.sections[section] = { updatedAt: Date.now(), fresh: true, source: "http" };
    } catch {
      if (cancellation?.aborted) throw new Error("Account status request aborted");
      result.errors.push(errors[section]);
    }
  }));
  result.observedAt = Date.now();
  return result;
}
