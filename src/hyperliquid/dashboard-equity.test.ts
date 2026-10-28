import { describe, expect, it } from "vitest";
import type { AgentDashboard } from "./agent-dashboard.js";
import { dashboardTotalEquity } from "./dashboard-equity.js";

function snapshot(patch: Partial<AgentDashboard> = {}): AgentDashboard {
  return {
    address: `0x${"12".repeat(20)}`, accountAbstraction: "unifiedAccount", fetchedAt: 0,
    spotUsdc: "186.061799", perpEquity: "20.298475", positions: [], orders: [], errors: [],
    ...patch,
  };
}

function liveSnapshot(patch: Partial<AgentDashboard> = {}): AgentDashboard {
  return snapshot({
    live: { status: "connected", sections: {
      mode: { fresh: true, updatedAt: 1 }, spot: { fresh: true, updatedAt: 1 },
      perps: { fresh: true, updatedAt: 1 }, orders: { fresh: true, updatedAt: 1 },
    } },
    ...patch,
  });
}

describe("Dashboard total equity", () => {
  for (const accountAbstraction of ["unifiedAccount", "disabled", "default"] as const) {
    it.each(["mode", "spot"] as const)(`requires fresh %s data for live ${accountAbstraction} equity`, (section) => {
      const state = liveSnapshot({ accountAbstraction });
      state.live!.sections[section].fresh = false;
      expect(dashboardTotalEquity(state)).toBeNull();
    });

    it.each(["reconnecting", "stopped"] as const)(`returns unknown for ${accountAbstraction} equity while %s even with fresh sections`, (status) => {
      const state = liveSnapshot({ accountAbstraction });
      state.live!.status = status;
      expect(dashboardTotalEquity(state)).toBeNull();
    });

    it(`ignores stale orders when calculating live ${accountAbstraction} equity`, () => {
      const state = liveSnapshot({ accountAbstraction });
      state.live!.sections.orders.fresh = false;
      expect(dashboardTotalEquity(state)).toBe(accountAbstraction === "unifiedAccount" ? "186.061799" : "206.360274");
    });
  }

  it.each(["disabled", "default"] as const)("requires fresh perp balances for live %s equity", (accountAbstraction) => {
    const state = liveSnapshot({ accountAbstraction });
    state.live!.sections.perps.fresh = false;
    expect(dashboardTotalEquity(state)).toBeNull();
  });

  it("does not require fresh perp balances for live unified equity", () => {
    const state = liveSnapshot({ perpEquity: null, positions: null });
    state.live!.sections.perps = { fresh: false, updatedAt: null };
    expect(dashboardTotalEquity(state)).toBe("186.061799");
  });

  it.each([null, "portfolioMargin"] as const)("keeps live equity unknown for account mode %s even with fresh sections", (accountAbstraction) => {
    expect(dashboardTotalEquity(liveSnapshot({ accountAbstraction }))).toBeNull();
  });

  it("uses the staging unified spot total without counting the raw perp equity again", () => {
    expect(dashboardTotalEquity(snapshot())).toBe("186.061799");
  });

  it.each(["12.345678", "-12.345678"])("does not add unified open-position PnL %s a second time", (unrealizedPnl) => {
    const state = snapshot({ positions: [{
      coin: "BTC", size: "0.01", entryPrice: "100000", unrealizedPnl,
      liquidationPrice: null, leverage: 2,
    }] });
    expect(dashboardTotalEquity(state)).toBe("186.061799");
  });

  it("does not require a separate perp balance or positions for unified equity", () => {
    expect(dashboardTotalEquity(snapshot({ perpEquity: null, positions: null }))).toBe("186.061799");
  });

  it("returns unknown when the unified spot balance is unavailable instead of using raw perp equity", () => {
    expect(dashboardTotalEquity(snapshot({ spotUsdc: null }))).toBeNull();
  });

  it("preserves a zero unified balance and its exact decimal string", () => {
    expect(dashboardTotalEquity(snapshot({ spotUsdc: "0" }))).toBe("0");
    expect(dashboardTotalEquity(snapshot({ spotUsdc: "0.000000" }))).toBe("0.000000");
  });

  it.each(["disabled", "default"] as const)("adds separate balances for %s accounts", (accountAbstraction) => {
    expect(dashboardTotalEquity(snapshot({ accountAbstraction }))).toBe("206.360274");
  });

  it.each(["disabled", "default"] as const)("adds %s balances exactly beyond Number precision", (accountAbstraction) => {
    expect(dashboardTotalEquity(snapshot({
      accountAbstraction, perpEquity: "9007199254740993.123456789", spotUsdc: "0.000000002",
    }))).toBe("9007199254740993.123456791");
  });

  it("preserves every decimal in a unified spot balance", () => {
    expect(dashboardTotalEquity(snapshot({ spotUsdc: "9007199254740993.123456789" }))).toBe("9007199254740993.123456789");
  });

  for (const accountAbstraction of ["disabled", "default"] as const) {
    it.each([
      { perpEquity: null, spotUsdc: "2" },
      { perpEquity: "2", spotUsdc: null },
      { perpEquity: null, spotUsdc: null },
    ])(`keeps unavailable ${accountAbstraction} balances unknown: %j`, (balances) => {
      expect(dashboardTotalEquity(snapshot({ accountAbstraction, ...balances }))).toBeNull();
    });

    it(`returns zero for known zero ${accountAbstraction} balances`, () => {
      expect(dashboardTotalEquity(snapshot({ accountAbstraction, perpEquity: "0.000", spotUsdc: "0" }))).toBe("0");
    });
  }

  it.each([null, "portfolioMargin"] as const)("does not infer equity from balances with account mode %s", (accountAbstraction) => {
    expect(dashboardTotalEquity(snapshot({ accountAbstraction }))).toBeNull();
    expect(dashboardTotalEquity(snapshot({ accountAbstraction, perpEquity: "0", spotUsdc: "0" }))).toBeNull();
  });
});
