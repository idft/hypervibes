import { HttpTransport } from "@nktkas/hyperliquid";
import { afterEach, describe, expect, it, vi } from "vitest";
import { loadAccountStatus } from "./account-status.js";
import type { AgentDashboard } from "./agent-dashboard.js";

const address = `0x${"ab".repeat(20)}`;
function snapshot(): AgentDashboard {
  const updatedAt = Date.now();
  return {
    address, accountAbstraction: "unifiedAccount", fetchedAt: updatedAt, perpEquity: "9007199254740993.123456", positions: [], spotUsdc: "1234.567890123456789", spotUsdcHold: "12.34",
    orders: [{ id: 1, coin: "ETH", side: "Buy", size: "0.001", price: "1000", type: "Limit" }, { id: 2, coin: "@107", side: "Buy", size: "1", price: "10", type: "Limit" }, { id: 3, coin: "PURR/USDC", side: "Buy", size: "1", price: "10", type: "Limit" }],
    errors: [], live: { status: "connected", sections: { perps: { updatedAt, fresh: true }, orders: { updatedAt, fresh: true }, spot: { updatedAt, fresh: true }, mode: { updatedAt, fresh: true } } },
  };
}
const transport = () => new HttpTransport();
afterEach(() => vi.unstubAllGlobals());

describe("perpetual account status", () => {
  it("reuses fresh live balances including shared spot collateral, and queries the actual account mode", async () => {
    const fetcher = vi.fn(async (_input: unknown, init: RequestInit) => {
      expect(JSON.parse(String(init.body))).toEqual({ type: "userAbstraction", user: address });
      return Response.json("unifiedAccount");
    }); vi.stubGlobal("fetch", fetcher);
    const result = await loadAccountStatus(transport(), address, snapshot());
    expect(fetcher).toHaveBeenCalledOnce();
    expect(result.accountAbstraction).toBe("unifiedAccount");
    expect(result.spotUsdc).toBe("1234.567890123456789");
    expect(result.spotUsdcHold).toBe("12.34");
    expect(result.sections.spot).toMatchObject({ fresh: true, source: "live" });
    expect(result.perpEquity).toBe("9007199254740993.123456");
    expect(result.orders?.map((order) => order.coin)).toEqual(["ETH"]);
    expect(result.sections.perps).toMatchObject({ fresh: true, source: "live" });
    expect(result.sections.mode).toMatchObject({ fresh: true, source: "http" });
  });

  it.each(["unifiedAccount", "portfolioMargin", "disabled", "default"] as const)("reports actual %s mode with funded spot even when the raw perp summary is zero", async (mode) => {
    const cached = snapshot(); cached.perpEquity = "0.0";
    vi.stubGlobal("fetch", vi.fn(async () => Response.json(mode)));
    const result = await loadAccountStatus(transport(), address, cached);
    expect(result).toMatchObject({ accountAbstraction: mode, perpEquity: "0.0", spotUsdc: "1234.567890123456789", spotUsdcHold: "12.34", errors: [] });
  });

  it("retains stale spot on repair failure and does not guess an unavailable account mode", async () => {
    const cached = snapshot(); cached.live!.sections.spot.fresh = false;
    vi.stubGlobal("fetch", vi.fn(async () => { throw new Error("offline"); }));
    const result = await loadAccountStatus(transport(), address, cached);
    expect(result.spotUsdc).toBe(cached.spotUsdc);
    expect(result.spotUsdcHold).toBe("12.34");
    expect(result.sections.spot).toMatchObject({ fresh: false, source: "live" });
    expect(result.accountAbstraction).toBeNull();
    expect(result.sections.mode).toMatchObject({ fresh: false, source: "unavailable" });
    expect(result.errors).toHaveLength(2);
  });

  it("reports successful empty spot balances as zero, not unavailable", async () => {
    const cached = snapshot(); cached.live!.sections.spot.fresh = false;
    vi.stubGlobal("fetch", vi.fn(async (_input: unknown, init: RequestInit) => {
      const { type } = JSON.parse(String(init.body)) as { type: string };
      return Response.json(type === "userAbstraction" ? "unifiedAccount" : { balances: [] });
    }));
    const result = await loadAccountStatus(transport(), address, cached);
    expect(result).toMatchObject({ spotUsdc: "0", spotUsdcHold: "0", sections: { spot: { fresh: true, source: "http" } }, errors: [] });
  });

  it("reports an unassigned account without network access", async () => {
    const fetcher = vi.fn(); vi.stubGlobal("fetch", fetcher);
    const result = await loadAccountStatus(transport(), null);
    expect(result).toMatchObject({ address: null, perpEquity: null, positions: null, orders: null });
    expect(result.errors).toEqual(["No trading account is assigned to this Agent."]);
    expect(fetcher).not.toHaveBeenCalled();
  });

  it("repairs only stale sections and refreshes account mode without refetching fresh spot data", async () => {
    const cached = snapshot(); cached.live!.sections.orders.updatedAt = Date.now() - 31_000;
    const fetcher = vi.fn(async (_input: unknown, init: RequestInit) => {
      const body = JSON.parse(String(init.body)) as { type: string };
      if (body.type === "userAbstraction") return Response.json("unifiedAccount");
      expect(body).toMatchObject({ type: "frontendOpenOrders", user: address, dex: "" });
      return Response.json([]);
    }); vi.stubGlobal("fetch", fetcher);
    const result = await loadAccountStatus(transport(), address, cached);
    expect(fetcher).toHaveBeenCalledTimes(2);
    expect(result.orders).toEqual([]);
    expect(result.sections.orders).toMatchObject({ source: "http", fresh: true });
    expect(result.sections.perps.source).toBe("live");
    expect(cached.orders).toHaveLength(3);
  });

  it("repairs disconnected data and exposes partial failures as stale, not zero", async () => {
    const cached = snapshot(); cached.live!.status = "reconnecting";
    vi.stubGlobal("fetch", vi.fn(async (_input: unknown, init: RequestInit) => {
      const { type } = JSON.parse(String(init.body)) as { type: string };
      if (type === "clearinghouseState") throw new Error("sensitive transport details");
      if (type === "userAbstraction") return Response.json("unifiedAccount");
      if (type === "spotClearinghouseState") return Response.json({ balances: [{ coin: "USDC", total: "100", hold: "5" }] });
      return Response.json([]);
    }));
    const result = await loadAccountStatus(transport(), address, cached);
    expect(result.perpEquity).toBe(cached.perpEquity);
    expect(result.sections.perps).toMatchObject({ source: "live", fresh: false });
    expect(result.sections.orders).toMatchObject({ source: "http", fresh: true });
    expect(result.errors).toHaveLength(1);
    expect(JSON.stringify(result)).not.toContain("sensitive");
  });

  it("leaves unavailable data null and propagates caller cancellation", async () => {
    vi.stubGlobal("fetch", vi.fn(async () => { throw new Error("offline"); }));
    const result = await loadAccountStatus(transport(), address);
    expect(result).toMatchObject({ perpEquity: null, positions: null, orders: null });
    expect(result.errors).toHaveLength(4);
    expect(result.spotUsdc).toBeNull();
    expect(result.accountAbstraction).toBeNull();
    const abort = new AbortController(); abort.abort();
    await expect(loadAccountStatus(transport(), address, undefined, abort.signal)).rejects.toThrow("aborted");
  });
});
