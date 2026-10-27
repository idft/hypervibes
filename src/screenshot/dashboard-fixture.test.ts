import { describe, expect, it } from "vitest";
import { dashboardFixture, screenshotAddress } from "./dashboard-fixture.js";
import { amount } from "../hyperliquid/journal-decimal.js";

const now = Date.UTC(2026, 9, 27, 12);
const quote = "109440";

describe("dashboard screenshot fixture", () => {
  it("keeps anonymous account data fixed for a supplied market quote", () => {
    const first = dashboardFixture(quote, now);
    const later = dashboardFixture(quote, now + 60_000);
    expect(first.agent.name).toBe("Atlas Momentum");
    expect(first.agent.tradingAccountAddress).toBe(screenshotAddress);
    expect(first.dashboard.address).toBe(screenshotAddress);
    expect(screenshotAddress).toMatch(/^0x[a-f0-9]{40}$/);
    expect(screenshotAddress).not.toMatch(/^0x0{30}/);
    expect(Number(first.dashboard.perpEquity)).toBeGreaterThanOrEqual(1_000);
    expect(Number(first.dashboard.perpEquity)).toBeLessThanOrEqual(2_000);
    expect(first.dashboard.positions).toEqual(later.dashboard.positions);
    expect(first.dashboard.orders).toEqual(later.dashboard.orders);
    expect(first.dashboard.perpEquity).toBe(later.dashboard.perpEquity);
    expect(first.subagents[0]!.runs[0]!.createdAt).toBe(now - 720_000);
    expect(later.subagents[0]!.runs[0]!.createdAt).toBe(now - 660_000);
  });

  it.each(["1000", "45000", "60253.27", "109440", "200000", "1000000"])("derives consistent position and exit values from BTC at %s", (price) => {
    const { dashboard } = dashboardFixture(price, now);
    const position = dashboard.positions![0]!;
    const stop = dashboard.orders![0]!;
    const takeProfit = dashboard.orders![1]!;
    const mark = amount(price);
    const entry = amount(position.entryPrice);
    expect(mark.sub(entry).mul(amount(position.size)).toString()).toBe(amount(position.unrealizedPnl).toString());
    const terminal = amount(dashboard.balanceHistory!.day.at(-1)!.balance);
    expect(terminal.add(amount(position.unrealizedPnl)).toString()).toBe(amount(dashboard.perpEquity!).toString());
    expect(stop.size).toBe(position.size);
    expect(takeProfit.size).toBe(position.size);
    const margin = entry.mul(amount(position.size)).div(amount("5"));
    expect(margin.cmp(amount(dashboard.perpEquity!).mul(amount("0.25")))).toBeLessThanOrEqual(0);
    expect(amount(position.size).sign).toBe(1);
    expect(position.size.replace(/^\d+\.?/, "").length).toBeLessThanOrEqual(5);
    expect(amount(position.liquidationPrice!).cmp(amount(stop.price))).toBeLessThan(0);
    expect(amount(stop.price).cmp(amount(stop.triggerPrice!))).toBeLessThan(0);
    expect(amount(stop.triggerPrice!).cmp(entry)).toBeLessThan(0);
    expect(entry.cmp(mark)).toBeLessThan(0);
    expect(amount(takeProfit.price).cmp(mark)).toBeGreaterThan(0);
    expect(amount(takeProfit.triggerPrice!).cmp(amount(takeProfit.price))).toBeGreaterThan(0);
    // 0.2% entry discount, 0.25% SL distance, and 0.5% TP distance, rounded to whole-dollar ticks.
    expect(entry.sub(mark.mul(amount("0.998"))).abs().cmp(amount("0.5"))).toBeLessThanOrEqual(0);
    expect(entry.sub(amount(stop.triggerPrice!)).sub(entry.mul(amount("0.0025"))).abs().cmp(amount("0.5"))).toBeLessThanOrEqual(0);
    expect(amount(takeProfit.triggerPrice!).sub(entry).sub(entry.mul(amount("0.005"))).abs().cmp(amount("0.5"))).toBeLessThanOrEqual(0);
    const liquidation = entry.mul(amount("0.8")).div(amount("0.995"));
    expect(amount(position.liquidationPrice!).sub(liquidation).abs().cmp(amount("0.005"))).toBeLessThanOrEqual(0);
  });

  it("updates all price-dependent fields without changing the fake account equity or identity", () => {
    const first = dashboardFixture("60000", now);
    const later = dashboardFixture("80000", now);
    expect(first.dashboard.positions![0]!.entryPrice).not.toBe(later.dashboard.positions![0]!.entryPrice);
    expect(first.dashboard.positions![0]!.unrealizedPnl).not.toBe(later.dashboard.positions![0]!.unrealizedPnl);
    expect(first.dashboard.positions![0]!.liquidationPrice).not.toBe(later.dashboard.positions![0]!.liquidationPrice);
    expect(first.dashboard.orders!.map((order) => order.triggerPrice)).not.toEqual(later.dashboard.orders!.map((order) => order.triggerPrice));
    expect(first.dashboard.perpEquity).toBe(later.dashboard.perpEquity);
    expect(first.dashboard.address).toBe(later.dashboard.address);
  });

  it.each(["", "NaN", "Infinity", "1e5", "-100000", "0", "999", "1000001", "60000.123456789"])("rejects an invalid or unsupported quote: %s", (price) => {
    expect(() => dashboardFixture(price, now)).toThrow(/BTC quote/);
  });

  it("has positive irregular balance paths and fictitious worker activity", () => {
    const { dashboard, subagents } = dashboardFixture(quote, now);
    for (const points of [dashboard.balanceHistory!.day, dashboard.balanceHistory!.month]) {
      const balances = points.map((point) => Number(point.balance));
      expect(balances.at(-1)!).toBeGreaterThan(balances[0]!);
      const changes = balances.slice(1).map((value, index) => Math.sign(value - balances[index]!));
      expect(changes.some((change, index) => index > 0 && change !== changes[index - 1])).toBe(true);
      let peak = balances[0]!;
      let drawdown = 0;
      for (const value of balances) { peak = Math.max(peak, value); drawdown = Math.max(drawdown, peak - value); }
      expect(drawdown).toBeGreaterThan(30);
      expect(new Set(changes).size).toBe(2);
      expect(points.every((point, index) => index === 0 || point.time > points[index - 1]!.time)).toBe(true);
    }
    expect(dashboard.balanceHistory!.day[0]!.balance).toBe(dashboard.balanceHistory!.month.at(-2)!.balance);
    expect(subagents.map((worker) => worker.name)).toEqual(["Market Analysis", "Risk Management", "Daily Reviewer"]);
    expect(subagents.every((worker) => worker.runs.every((run) => run.status === "completed" && run.result))).toBe(true);
    expect(dashboard.errors).toEqual([]);
  });
});
