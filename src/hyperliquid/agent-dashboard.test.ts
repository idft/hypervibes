import { HttpTransport } from "@nktkas/hyperliquid";
import { afterEach, describe, expect, it, vi } from "vitest";
import { loadAgentDashboard } from "./agent-dashboard.js";

const address = `0x${"12".repeat(20)}`;
const fixtures: Record<string, unknown> = {
  clearinghouseState: { marginSummary: { accountValue: "9007199254740993.123456" }, assetPositions: [
    { position: { coin: "BTC", szi: "-0.01", entryPx: "100000", unrealizedPnl: "-1.123456", liquidationPx: null, leverage: { value: 2 } } },
    { position: { coin: "ETH", szi: "0.000" } },
  ] },
  spotClearinghouseState: { balances: [{ coin: "USDC", total: "10.0000001" }] },
  frontendOpenOrders: [{ oid: 123, coin: "BTC", side: "B", sz: "0.02", limitPx: "99000", orderType: "Limit" }],
};
afterEach(() => vi.unstubAllGlobals());
describe("Agent dashboard snapshots", () => {
  function mockFetch(failed?: string) {
    const calls: Record<string, unknown>[] = [];
    vi.stubGlobal("fetch", vi.fn(async (_input: unknown, init: RequestInit) => {
      const body = JSON.parse(String(init.body)) as { type: string; user: string };
      calls.push(body);
      if (body.type === failed) return new Response("unavailable", { status: 503 });
      return Response.json(fixtures[body.type]);
    }));
    return calls;
  }
  it("does not make exchange calls for chat-only agents", async () => {
    const calls = mockFetch();
    expect(await loadAgentDashboard(new HttpTransport(), null)).toMatchObject({ address: null, positions: null, orders: null });
    expect(calls).toHaveLength(0);
  });
  it("reads only the linked account and preserves decimal strings without aggregating balances", async () => {
    const calls = mockFetch();
    const result = await loadAgentDashboard(new HttpTransport(), address);
    expect(calls).toHaveLength(3);
    expect(calls.every((call) => call.user === address)).toBe(true);
    expect(calls.map((call) => call.type).sort()).toEqual(Object.keys(fixtures).sort());
    expect(result.perpEquity).toBe("9007199254740993.123456");
    expect(result.spotUsdc).toBe("10.0000001");
    expect(result.positions).toEqual([{ coin: "BTC", size: "-0.01", entryPrice: "100000", unrealizedPnl: "-1.123456", liquidationPrice: null, leverage: 2 }]);
    expect(result.orders).toEqual([{ id: 123, coin: "BTC", side: "Buy", size: "0.02", price: "99000", type: "Limit" }]);
    expect(result.errors).toEqual([]);
  });
  it("reports unavailable sections as unknown, not empty or zero, while preserving successful sections", async () => {
    mockFetch("clearinghouseState");
    const result = await loadAgentDashboard(new HttpTransport(), address);
    expect(result.perpEquity).toBeNull(); expect(result.positions).toBeNull();
    expect(result.spotUsdc).toBe("10.0000001"); expect(result.orders).toHaveLength(1);
    expect(result.errors).toEqual(["Perpetual balances and positions are temporarily unavailable."]);
  });
});
