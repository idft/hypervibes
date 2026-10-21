import { describe, expect, it } from "vitest";
import type { AgentDashboard, DashboardOrder } from "../hyperliquid/agent-dashboard.js";
import { chartFillRange, defaultChartOverlays, type ChartFill } from "./trading-contracts.js";
import { accountPriceLines, tradeMarkers } from "./trading-overlays.js";

const fill = (id: string, time: number, side: ChartFill["side"] = "Buy", price = "100"): ChartFill => ({ id, time, side, price, size: "0.00000001" });
const candle = (time: number) => ({ time, open: 100, close: 101, high: 102, low: 99, volume: 1 });
const order = (id: number, overrides: Partial<DashboardOrder> = {}): DashboardOrder => ({ id, coin: "BTC", side: "Sell", price: "80", size: "0.1", type: "Stop Market", triggerPrice: "90", ...overrides });
const account = (): AgentDashboard => ({ address: "0xaccount", fetchedAt: 1, perpEquity: "1000", spotUsdc: "0", errors: [],
  positions: [{ coin: "BTC", size: "-0.00000001", entryPrice: "100.00000001", unrealizedPnl: "-1.23456789", liquidationPrice: null, leverage: 2 }], orders: [],
  live: { status: "connected", sections: { perps: { fresh: true, updatedAt: 1 }, orders: { fresh: true, updatedAt: 1 }, spot: { fresh: false, updatedAt: null } } },
});

describe("read-only trading overlays", () => {
  it("validates safe half-open ranges and tied-time cursors", () => {
    expect(chartFillRange({ from: 0, to: 100 })).toEqual({ from: 0, to: 100 });
    expect(chartFillRange({ from: 0, to: 100, cursor: { time: 0, id: "a".repeat(64) } }).cursor?.time).toBe(0);
    for (const data of [{ from: -1, to: 1 }, { from: 1, to: 1 }, { from: 0, to: Infinity }, { from: 1.5, to: 10 }, { from: 0, to: Number.MAX_SAFE_INTEGER + 1 }, { from: 0, to: 100, cursor: { time: 100, id: "a".repeat(64) } }, { from: 0, to: 100, cursor: { time: 1, id: "invalid" } }]) {
      expect(() => chartFillRange(data)).toThrow("Invalid trade history");
    }
  });
  it("puts every fill on its candle at its fill price, sorted and without inventing missing candles", () => {
    const markers = tradeMarkers([fill("sell", 7_200_001, "Sell", "101"), fill("buy2", 3_600_999), fill("buy1", 3_600_001), fill("gap", 10_800_001), fill("before", 1), fill("invalid", 3_600_001, "Buy", "Infinity")], [candle(3600), candle(7200), candle(14400)], 3600);
    expect(markers.map((marker) => [marker.id, marker.time, marker.shape, marker.price])).toEqual([["buy1", 3600, "arrowUp", 100], ["buy2", 3600, "arrowUp", 100], ["sell", 7200, "arrowDown", 101]]);
  });
  it("preserves signed positions, sizes and P&L; isolates symbols and freshness by section", () => {
    const snapshot = account();
    snapshot.orders = [order(1), order(2, { coin: "ETH" })];
    const position = accountPriceLines("BTC", snapshot, defaultChartOverlays, false).get("position")!;
    expect(position.price).toBe(100.00000001);
    expect(position.title).toBe("Short · 0.00000001 BTC · P&L (1.2346) USDC");
    expect(position.title).not.toContain("STALE"); // A stale spot balance is irrelevant to overlays.
    snapshot.live!.sections.orders.fresh = false;
    const lines = accountPriceLines("BTC", snapshot, defaultChartOverlays, false);
    expect(lines.size).toBe(2); expect(lines.get("position")!.title).not.toContain("STALE");
    expect(lines.get("order:1")).toMatchObject({ price: 90, title: expect.stringContaining("STALE") });
    expect(accountPriceLines("BTC", snapshot, defaultChartOverlays, true).get("position")!.title).toContain("STALE");
    expect(accountPriceLines("SOL", snapshot, defaultChartOverlays, false).size).toBe(0);
  });
  it("includes attached triggers, deduplicates live children and describes position-wide TP/SL accurately", () => {
    const snapshot = account();
    snapshot.orders = [order(1, { type: "Limit", triggerPrice: undefined, children: [order(2), order(3, { positionTpsl: true, size: "0" })] }), order(2)];
    const lines = accountPriceLines("BTC", snapshot, defaultChartOverlays, false);
    expect(lines.size).toBe(4);
    expect(lines.get("order:2")!.title).not.toContain("Pending");
    expect(lines.get("order:3")!.title).toBe("Pending · Sell Stop Market trigger · Entire position · #3");
    expect(lines.get("order:3")!.price).toBe(90);
  });
  it("never plots flat positions, invalid prices or hidden sections", () => {
    const snapshot = account();
    snapshot.positions![0]!.size = "0.0000";
    snapshot.orders = [order(1, { triggerPrice: "0" }), order(2, { price: "NaN", triggerPrice: undefined })];
    expect(accountPriceLines("BTC", snapshot, defaultChartOverlays, false).size).toBe(0);
    expect(accountPriceLines("BTC", account(), { trades: true, position: false, orders: false }, false).size).toBe(0);
  });
});
