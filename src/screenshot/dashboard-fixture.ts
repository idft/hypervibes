import type { AgentRow } from "../db/agent-schema.js";
import type { AgentDashboard, BalancePoint } from "../hyperliquid/agent-dashboard.js";
import type { DashboardSubagent } from "../agents/subagent-dashboard.js";
import { amount, type Amount } from "../hyperliquid/journal-decimal.js";

export const screenshotAddress = "0x9b8437e9abb0dbef8a9b3078ea9f47f3ea555a87";

function roundPositive(value: Amount, places = 0, rounding: "nearest" | "down" = "nearest"): string {
  if (value.scale <= places) return value.toString();
  const divisor = 10n ** BigInt(value.scale - places);
  const bias = rounding === "down" ? 0n : divisor / 2n;
  const units = ((value.units + bias) / divisor).toString().padStart(places + 1, "0");
  return places ? `${units.slice(0, -places)}.${units.slice(-places)}` : units;
}

/** Only the public market quote is real. Identity, equity, history, and activity are fictitious. */
export function dashboardFixture(btcPrice: string, now = Date.now()) {
  if (typeof btcPrice !== "string" || !/^\d{1,9}(?:\.\d{1,8})?$/.test(btcPrice)) throw new Error("A valid fresh BTC quote is required.");
  const mark = amount(btcPrice);
  if (mark.cmp(amount("1000")) < 0 || mark.cmp(amount("1000000")) > 0) throw new Error("BTC quote is outside the screenshot scenario's supported range.");
  const entryPrice = roundPositive(mark.mul(amount("0.998")));
  const equity = "1472.53";
  // Allocate at most 25% of the fake equity as isolated margin at 5x leverage.
  const size = roundPositive(amount(equity).mul(amount("1.25")).div(amount(entryPrice)), 5, "down");
  const unrealizedPnl = mark.sub(amount(entryPrice)).mul(amount(size)).toString();
  const realizedBalance = amount(equity).sub(amount(unrealizedPnl));
  const stopTrigger = roundPositive(amount(entryPrice).mul(amount("0.9975")));
  const takeProfitTrigger = roundPositive(amount(entryPrice).mul(amount("1.005")));
  const stopLimit = amount(stopTrigger).sub(amount("1")).toString();
  const takeProfitLimit = amount(takeProfitTrigger).sub(amount("1")).toString();
  // Isolated 5x long with a 0.5% maintenance rate; exact decimal arithmetic until display rounding.
  const liquidationPrice = roundPositive(amount(entryPrice).mul(amount("0.8")).div(amount("0.995")), 2);
  const agent: AgentRow & { tradingEnabled: boolean } = {
    id: "screenshot-atlas", userId: "screenshot-user", name: "Atlas Momentum",
    creationRequestId: "screenshot-creation", state: "ready", enabled: true, tradingEnabled: true,
    conversationId: null, initialProvider: null, initialModelId: null, error: null,
    tradingAccountAddress: screenshotAddress, createdAt: now, updatedAt: now,
  };
  // Hand-authored sample paths include a selloff, a recovery, and uneven pullbacks.
  // Both windows end at realized balance; the last month interval matches the day window.
  const dayOffsets = [-18, -14, -21, -20, -34, -43, -46, -29, -32, -28, -25, -39, -47, -37, -20, -16, -23, -18, -21, -8, -4, -11, -6, -9, 0];
  const monthOffsets = [-82, -69, -73, -48, -55, -19, -28, -5, 17, 9, -12, -35, -31, -67, -104, -91, -128, -116, -135, -111, -96, -105, -73, -57, -66, -35, -42, -14, -23, -18, 0];
  const history = (offsets: number[], duration: number): BalancePoint[] => offsets.map((offset, index) => ({
    time: now - duration + index / (offsets.length - 1) * duration,
    balance: realizedBalance.add(amount(String(offset))).toString(),
  }));
  const dashboard: AgentDashboard = {
    address: screenshotAddress, fetchedAt: now, perpEquity: equity, spotUsdc: "0", spotUsdcHold: "0", errors: [],
    balanceHistory: { day: history(dayOffsets, 86_400_000), month: history(monthOffsets, 30 * 86_400_000) },
    live: { status: "connected", sections: {
      perps: { updatedAt: now, fresh: true }, spot: { updatedAt: now, fresh: true }, orders: { updatedAt: now, fresh: true },
    } },
    positions: [{ coin: "BTC", size, leverage: 5, entryPrice, unrealizedPnl, liquidationPrice }],
    orders: [
      { id: 1, coin: "BTC", side: "Sell", type: "Stop Limit", size, price: stopLimit, triggerPrice: stopTrigger, positionTpsl: true },
      { id: 2, coin: "BTC", side: "Sell", type: "Take Profit Limit", size, price: takeProfitLimit, triggerPrice: takeProfitTrigger, positionTpsl: true },
    ],
  };
  const subagents: DashboardSubagent[] = [
    { id: "market-analysis", name: "Market Analysis", enabled: true, schedule: "", nextRunAt: null,
      runs: [{ id: "analysis-demo", status: "completed", createdAt: now - 720_000, result: "Momentum remains constructive above the session range." }] },
    { id: "risk-management", name: "Risk Management", enabled: true, schedule: "", nextRunAt: null,
      runs: [{ id: "risk-demo", status: "completed", createdAt: now - 480_000, result: "Risk controls staged for the current BTC position." }] },
    { id: "daily-reviewer", name: "Daily Reviewer", enabled: true, schedule: "1d", nextRunAt: now + 20 * 3_600_000,
      runs: [{ id: "review-demo", status: "completed", createdAt: now - 4 * 3_600_000, result: "Reviewed the last 24 hours; retained risk limits and updated execution guidance." }] },
  ];
  return { agent, dashboard, subagents };
}
