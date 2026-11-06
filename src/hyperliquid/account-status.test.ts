import { HttpTransport } from "@nktkas/hyperliquid";
import { Check } from "typebox/value";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AccountStatusSchema } from "./account-status-schema.js";
import { loadAccountStatus } from "./account-status.js";

const twap = vi.hoisted(() => ({
  read: vi.fn<(user: string, dex: string, signal: AbortSignal) => Promise<{ data: unknown; receivedAt: number }>>(),
  close: vi.fn(),
}));
vi.mock("./twap-snapshot.js", () => ({ createTwapSnapshotReader: () => twap }));
beforeEach(() => {
  twap.read.mockReset().mockImplementation(async (user, dex) => ({ data: { user, dex, states: [] }, receivedAt: Date.now() }));
  twap.close.mockClear();
});
const address = `0x${"ab".repeat(20)}`;
const precise = "9007199254740993.123456789012345678";
const transport = () => new HttpTransport();
const summary = () => ({ accountValue: precise, totalNtlPos: "30.00000000000001", totalRawUsd: "-1.234", totalMarginUsed: "12.00000000000000001" });
function state(coin = "ETH") {
  return {
    marginSummary: summary(), crossMarginSummary: summary(), crossMaintenanceMarginUsed: "1.23", withdrawable: "4.56", time: Date.now(),
    assetPositions: [{ type: "oneWay", position: {
      coin, szi: "-0.000000000001", leverage: { type: "isolated", value: 3, rawUsd: "-10.123" },
      entryPx: precise, positionValue: "12.34", unrealizedPnl: "-3.456", returnOnEquity: "-0.2", liquidationPx: null,
      marginUsed: "4.567", maxLeverage: 10, cumFunding: { allTime: "-2", sinceOpen: "1", sinceChange: "0" },
    } }],
  };
}
function order(coin = "ETH", oid = 1) {
  return {
    coin, oid, side: "B", limitPx: precise, sz: "0.123456789012345678", origSz: "1.123456789012345678", timestamp: Date.now(),
    triggerCondition: "Below 10", isTrigger: true, triggerPx: "10.00000000000001", children: [],
    isPositionTpsl: true, reduceOnly: true, orderType: "Stop Market", tif: null, cloid: `0x${"cd".repeat(16)}`,
  };
}
function twapState(user: string, coin: string) {
  return {
    user, coin, sz: precise, executedSz: "9007199254740993.123456789012345677", executedNtl: "25.1",
    side: "A", reduceOnly: false, minutes: 60, randomize: true, timestamp: Date.now() - 3_600_000,
    stopPx: "120", trigger: { px: "100", above: true },
  };
}
function spotState() {
  return {
    balances: [
      { coin: "USDC", token: 0, total: precise, hold: "12.34", entryNtl: "0", spotHold: "2.34", borrowed: "1.23", supplied: "0", ltv: "0.9" },
      { coin: "HYPE", token: 150, total: "2.123456789", hold: "0.2", entryNtl: "24.12" },
    ],
    portfolioMarginEnabled: true, portfolioMarginRatio: "0.12", tokenToAvailableAfterMaintenance: [[0, "123.456789"]],
    evmEscrows: [{ coin: "USDC", token: 0, total: "1.23" }],
  };
}
type Body = { type: string; dex?: string; user?: string };
function response(body: Body): unknown {
  if (body.type === "perpDexs") return [null, { name: "xyz" }, { name: "abc" }];
  if (body.type === "userAbstraction") return "unifiedAccount";
  if (body.type === "spotClearinghouseState") return spotState();
  if (body.type === "clearinghouseState") return state(body.dex ? `${body.dex}:BTC` : "ETH");
  if (body.type === "frontendOpenOrders") return body.dex ? [order(`${body.dex}:BTC`)] : [order(), order("@107", 2), order("PURR/USDC", 3)];
  throw new Error("Unexpected request");
}
function mock(handler: (body: Body) => unknown | Promise<unknown> = response) {
  const fetcher = vi.fn(async (_input: unknown, init: RequestInit) => Response.json(await handler(JSON.parse(String(init.body)) as Body)));
  vi.stubGlobal("fetch", fetcher);
  return fetcher;
}
afterEach(() => { vi.unstubAllGlobals(); vi.useRealTimers(); });

describe("account-wide read-only account status", () => {
  it("discovers every DEX and preserves exact financial values, margin modes, balances, holds, and order IDs", async () => {
    const fetcher = mock();
    const result = await loadAccountStatus(transport(), address);
    expect(Check(AccountStatusSchema, result)).toBe(true);
    expect(result.coverage).toMatchObject({ complete: true, perpDexs: ["", "xyz", "abc"], discovery: { fresh: true, source: "http" } });
    expect(result.perps.map((perp) => perp.dex)).toEqual(["", "xyz", "abc"]);
    expect(result.perps[1]?.state?.assetPositions[0]?.position).toMatchObject({ coin: "xyz:BTC", szi: "-0.000000000001", leverage: { type: "isolated", rawUsd: "-10.123" }, marginUsed: "4.567" });
    expect(result.perps[0]?.state?.marginSummary.accountValue).toBe(precise);
    expect(result.perps[0]?.orders?.[0]).toMatchObject({ oid: 1, reduceOnly: true, triggerPx: "10.00000000000001", cloid: `0x${"cd".repeat(16)}`, parentOid: null });
    expect(result.spot.state).toEqual(spotState());
    expect(result.spot.orders?.map((entry) => entry.coin)).toEqual(["@107", "PURR/USDC"]);
    expect(result.orders?.map((entry) => entry.coin)).toEqual(["ETH"]);
    expect(result).toMatchObject({ perpEquity: precise, spotUsdc: precise, spotUsdcHold: "12.34", sections: { spot: { fresh: true, source: "http" } }, errors: [] });
    expect(result.equity.value).toBeNull();
    expect(result.availableMargin.value).toBeNull();
    expect(fetcher).toHaveBeenCalledTimes(9);
    const requests = fetcher.mock.calls.map(([, init]) => JSON.parse(String(init.body)) as Body);
    expect(requests.filter((body) => body.type === "clearinghouseState").map((body) => body.dex)).toEqual(["", "xyz", "abc"]);
  });

  it.each(["unifiedAccount", "portfolioMargin", "disabled", "default", "dexAbstraction"])("retains %s without inventing equity or capacity", async (mode) => {
    mock((body) => body.type === "userAbstraction" ? mode : response(body));
    const result = await loadAccountStatus(transport(), address);
    expect(result.accountAbstraction).toBe(mode);
    expect(result.equity).toMatchObject({ value: null, reason: expect.stringContaining("unknown") });
    expect(result.availableMargin).toMatchObject({ value: null, reason: expect.stringContaining("unknown") });
  });

  it("keeps child trigger IDs and parent IDs without losing reduce-only flags", async () => {
    mock((body) => body.type === "frontendOpenOrders" && body.dex === "" ? [{ ...order(), children: [order("ETH", 9)] }] : response(body));
    const result = await loadAccountStatus(transport(), address);
    expect(result.perps[0]?.orders?.map(({ oid, parentOid, reduceOnly }) => ({ oid, parentOid, reduceOnly }))).toEqual([
      { oid: 1, parentOid: null, reduceOnly: true }, { oid: 9, parentOid: 1, reduceOnly: true },
    ]);
  });

  it("supports documented orders without optional SDK fields and cross-margin positions", async () => {
    mock((body) => {
      if (body.type === "clearinghouseState") {
        const data = state(body.dex ? `${body.dex}:BTC` : "ETH");
        return { ...data, assetPositions: data.assetPositions.map(({ position }) => ({ type: "oneWay", position: { ...position, leverage: { type: "cross", value: 2 } } })) };
      }
      if (body.type === "frontendOpenOrders") {
        return [Object.fromEntries(Object.entries(order(body.dex ? `${body.dex}:BTC` : "ETH")).filter(([key]) => !["children", "cloid", "tif"].includes(key)))];
      }
      return response(body);
    });
    const result = await loadAccountStatus(transport(), address);
    expect(result.coverage.complete).toBe(true);
    expect(result.perps[0]?.state?.assetPositions[0]?.position.leverage.type).toBe("cross");
  });

  it.each([null, [], [null, null], [null, { name: "" }], [null, { name: "xyz" }, { name: "xyz" }]].map((discovery) => ({ discovery })))("does not treat malformed discovery %j as an empty account", async ({ discovery }) => {
    mock((body) => body.type === "perpDexs" ? discovery : response(body));
    const result = await loadAccountStatus(transport(), address);
    expect(result.coverage).toMatchObject({ complete: false, perpDexs: null, discovery: { fresh: false } });
    expect(result.perps).toHaveLength(1);
    expect(result.perps[0]?.state).not.toBeNull();
    expect(result.errors).toHaveLength(1);
  });

  it("distinguishes successful empty accounts from discovery and section failures", async () => {
    mock((body) => {
      if (body.type === "perpDexs") return [null];
      if (body.type === "clearinghouseState") return { ...state(), assetPositions: [] };
      if (body.type === "spotClearinghouseState") return { balances: [] };
      if (body.type === "frontendOpenOrders") return [];
      return response(body);
    });
    const result = await loadAccountStatus(transport(), address);
    expect(result).toMatchObject({ coverage: { complete: true }, positions: [], orders: [], spotUsdc: "0", spotUsdcHold: "0", spot: { state: { balances: [] }, orders: [] }, errors: [] });
  });

  it("keeps successful DEX data when another DEX fails and sanitizes transport errors", async () => {
    mock((body) => {
      if (body.dex === "xyz") throw new Error("sensitive transport details");
      return response(body);
    });
    const result = await loadAccountStatus(transport(), address);
    expect(result.coverage.complete).toBe(false);
    expect(result.perps[1]).toMatchObject({ state: null, orders: null, sections: { state: { fresh: false }, orders: { fresh: false } } });
    expect(result.perps[2]?.sections.state.fresh).toBe(true);
    expect(result.errors).toHaveLength(2);
    expect(JSON.stringify(result)).not.toContain("sensitive");
  });

  it.each(["number", "nan", "exponent", "negative-margin", "unsafe-id", "missing-hold", "unknown-mode", "bad-child"])("rejects malformed financial responses: %s", async (kind) => {
    mock((body) => {
      if (body.type === "clearinghouseState" && body.dex === "") {
        if (kind === "number") return { ...state(), withdrawable: 1.23 };
        if (kind === "nan") return { ...state(), withdrawable: "NaN" };
        if (kind === "exponent") return { ...state(), withdrawable: "1e10" };
        if (kind === "negative-margin") return { ...state(), crossMaintenanceMarginUsed: "-1" };
      }
      if (body.type === "frontendOpenOrders" && body.dex === "") {
        if (kind === "unsafe-id") return [{ ...order(), oid: Number.MAX_SAFE_INTEGER + 1 }];
        if (kind === "bad-child") return [{ ...order(), children: [{ ...order(), sz: "broken" }] }];
      }
      if (body.type === "spotClearinghouseState" && kind === "missing-hold") return { balances: [{ coin: "USDC", token: 0, total: "10", entryNtl: "0" }] };
      if (body.type === "userAbstraction" && kind === "unknown-mode") return "new-mode";
      return response(body);
    });
    const result = await loadAccountStatus(transport(), address);
    expect(result.coverage.complete).toBe(false);
    expect(result.errors).toHaveLength(1);
    if (kind === "missing-hold") expect(result).toMatchObject({ spotUsdc: null, spotUsdcHold: null, spot: { state: null } });
    if (kind === "unsafe-id" || kind === "bad-child") expect(result.spot.orders).toBeNull();
    expect(Check(AccountStatusSchema, result)).toBe(true);
  });

  it("marks old and future exchange state as not fresh", async () => {
    mock((body) => body.type === "clearinghouseState" ? { ...state(body.dex ? `${body.dex}:BTC` : "ETH"), time: Date.now() + (body.dex ? 60_000 : -60_000) } : response(body));
    const result = await loadAccountStatus(transport(), address);
    expect(result.coverage.complete).toBe(false);
    expect(result.perps.every((perp) => !perp.sections.state.fresh && perp.state !== null)).toBe(true);
    expect(result.sections.perps.fresh).toBe(false);
  });

  it("bounds request concurrency to 4 across many DEXs", async () => {
    let active = 0; let maximum = 0;
    mock(async (body) => {
      active++; maximum = Math.max(maximum, active);
      await new Promise((resolve) => setTimeout(resolve, 2));
      active--;
      return body.type === "perpDexs" ? [null, ...Array.from({ length: 15 }, (_, i) => ({ name: `dex${i}` }))] : response(body);
    });
    const result = await loadAccountStatus(transport(), address);
    expect(result.perps).toHaveLength(16);
    expect(result.coverage.complete).toBe(true);
    expect(maximum).toBe(4);
  });

  it("returns partial coverage after the total HTTP deadline", async () => {
    vi.useFakeTimers();
    mock((body) => body.type === "frontendOpenOrders" ? new Promise(() => {}) : response(body));
    // AbortSignal.timeout uses the native clock, so provide a controlled deadline.
    const timeout = new AbortController();
    const spy = vi.spyOn(AbortSignal, "timeout").mockReturnValue(timeout.signal);
    try {
      const pending = loadAccountStatus(transport(), address);
      await vi.advanceTimersByTimeAsync(0);
      timeout.abort();
      const result = await pending;
      expect(result.coverage.complete).toBe(false);
      expect(result.errors.length).toBeGreaterThan(0);
      expect(result.spot.orders).toBeNull();
    } finally { spy.mockRestore(); }
  });

  it("does not read the network for unassigned or already cancelled accounts", async () => {
    const fetcher = mock();
    const result = await loadAccountStatus(transport(), null);
    expect(result).toMatchObject({ address: null, coverage: { complete: false, perpDexs: null }, spotUsdc: null, positions: null, orders: null });
    expect(fetcher).not.toHaveBeenCalled();
    const abort = new AbortController(); abort.abort();
    await expect(loadAccountStatus(transport(), address, abort.signal)).rejects.toThrow("aborted");
    expect(fetcher).not.toHaveBeenCalled();
  });

  it("preserves outcome orders separately and exposes explicit spot order freshness", async () => {
    mock((body) => body.type === "frontendOpenOrders" && body.dex === "" ? [order("#1230", 20), order("@107", 21)] : response(body));
    const result = await loadAccountStatus(transport(), address);
    expect(result.outcomeOrders.orders?.map((entry) => entry.coin)).toEqual(["#1230"]);
    expect(result.outcomeOrders.section).toMatchObject({ fresh: true, source: "http" });
    expect(result.spot.sections.orders).toMatchObject({ fresh: true, source: "http" });
    expect(result.spot.orders?.map((entry) => entry.coin)).toEqual(["@107"]);
    expect(result.perps[0]?.orders).toEqual([]);
    expect(result.coverage.complete).toBe(true);
    expect(result.coverage).not.toHaveProperty("openingRiskVerified");
    expect(Check(AccountStatusSchema, result)).toBe(true);
  });

  it("rejects invalid assigned addresses without network reads", async () => {
    const fetcher = mock();
    for (const invalid of ["0x1234", "not-an-address", `0x${"zz".repeat(20)}`, `${address}\n`]) {
      const result = await loadAccountStatus(transport(), invalid);
      expect(result.coverage.complete).toBe(false);
      expect(result.errors).toEqual(["The assigned trading account address is invalid."]);
    }
    expect(fetcher).not.toHaveBeenCalled();
  });

  it("rechecks exchange freshness at completion instead of only at receipt", async () => {
    vi.useFakeTimers();
    const start = Date.now();
    mock((body) => {
      if (body.type === "clearinghouseState") return { ...state(body.dex ? `${body.dex}:BTC` : "ETH"), time: start - 29_000 };
      if (body.type === "frontendOpenOrders") vi.setSystemTime(start + 2_000);
      return response(body);
    });
    const result = await loadAccountStatus(transport(), address);
    expect(result.perps.every((perp) => !perp.sections.state.fresh)).toBe(true);
    expect(result.coverage.complete).toBe(false);
  });

  it("rechecks receipt freshness for every section at completion", async () => {
    vi.useFakeTimers();
    const start = Date.now();
    mock((body) => {
      if (body.type === "frontendOpenOrders" && body.dex === "abc") vi.setSystemTime(start + 31_000);
      return response(body);
    });
    const result = await loadAccountStatus(transport(), address);
    expect(result.coverage.discovery.fresh).toBe(false);
    expect(result.sections.spot.fresh).toBe(false);
    expect(result.spot.sections.balances.fresh).toBe(false);
    expect(result.sections.mode.fresh).toBe(false);
    expect(result.twaps.some((twaps) => !twaps.section.fresh)).toBe(true);
    expect(result.coverage.complete).toBe(false);
  });

  it.each(["", "xyz"])("rejects positions and orders from another DEX when reading %j", async (dex) => {
    mock((body) => {
      if (body.dex === dex && body.type === "clearinghouseState") return state("abc:BTC");
      if (body.dex === dex && body.type === "frontendOpenOrders") return [order("abc:BTC")];
      return response(body);
    });
    const result = await loadAccountStatus(transport(), address);
    expect(result.perps.find((perp) => perp.dex === dex)).toMatchObject({ state: null, orders: null });
    expect(result.errors).toHaveLength(2);
    expect(result.coverage.complete).toBe(false);
  });

  it.each([
    [{ coin: "USDC", token: 0, total: "1", hold: "0", entryNtl: "0" }, { coin: "USDC", token: 0, total: "2", hold: "0", entryNtl: "0" }],
    [{ coin: "HYPE", token: 0, total: "1", hold: "0", entryNtl: "0" }],
    [{ coin: "USDC", token: 150, total: "1", hold: "0", entryNtl: "0" }],
    [{ coin: "HYPE", token: 150, total: "1", hold: "0", entryNtl: "0" }, { coin: "OTHER", token: 150, total: "1", hold: "0", entryNtl: "0" }],
  ].map((balances) => ({ balances })))("rejects duplicate token IDs and inconsistent USDC identities: %j", async ({ balances }) => {
    mock((body) => body.type === "spotClearinghouseState" ? { balances } : response(body));
    const result = await loadAccountStatus(transport(), address);
    expect(result).toMatchObject({ spotUsdc: null, spotUsdcHold: null, spot: { state: null, sections: { balances: { fresh: false } } } });
    expect(result.coverage.complete).toBe(false);
  });

  it.each(["1\n", "1\r", "1\r\n", "1\u2028", "1 "])("rejects financial strings with trailing whitespace: %j", async (amount) => {
    mock((body) => {
      if (body.type === "clearinghouseState") return { ...state(), withdrawable: amount };
      if (body.type === "spotClearinghouseState") return { balances: [{ coin: "USDC", token: 0, total: amount, hold: "0", entryNtl: "0" }] };
      return response(body);
    });
    const result = await loadAccountStatus(transport(), address);
    expect(result.spot.state).toBeNull();
    expect(result.perps.every((perp) => perp.state === null)).toBe(true);
    expect(result.coverage.complete).toBe(false);
  });

  it.each([false, true])("deduplicates identical positive IDs and retains child linkage, child first = %s", async (childFirst) => {
    const child = order("ETH", 9);
    const parent = { ...order(), children: [child] };
    mock((body) => body.type === "frontendOpenOrders" && body.dex === "" ? (childFirst ? [child, parent] : [parent, child]) : response(body));
    const result = await loadAccountStatus(transport(), address);
    expect(result.perps[0]?.orders).toHaveLength(2);
    expect(result.perps[0]?.orders?.find((entry) => entry.oid === 9)).toMatchObject({ parentOid: 1, sourcePath: [childFirst ? 1 : 0, 0], orderIdKnown: true });
    expect(result.coverage.complete).toBe(true);
  });

  it("rejects conflicting duplicate order IDs even between top-level and children", async () => {
    const child = order("ETH", 9);
    mock((body) => body.type === "frontendOpenOrders" && body.dex === "" ? [{ ...order(), children: [child] }, { ...child, sz: "123" }] : response(body));
    const result = await loadAccountStatus(transport(), address);
    expect(result.perps[0]?.orders).toBeNull();
    expect(result.spot.orders).toBeNull();
    expect(result.coverage.complete).toBe(false);
  });

  it("does not merge distinct zero-ID placeholders and marks their IDs unknown", async () => {
    mock((body) => body.type === "frontendOpenOrders" && body.dex === "" ? [{ ...order(), children: [order("ETH", 0), { ...order("ETH", 0), triggerPx: "20" }] }] : response(body));
    const result = await loadAccountStatus(transport(), address);
    expect(result.perps[0]?.orders).toHaveLength(3);
    expect(result.perps[0]?.orders?.filter((entry) => entry.oid === 0)).toMatchObject([
      { orderIdKnown: false, parentOid: 1, sourcePath: [0, 0] },
      { orderIdKnown: false, parentOid: 1, sourcePath: [0, 1] },
    ]);
    expect(result.coverage.complete).toBe(true);
    expect(Check(AccountStatusSchema, result)).toBe(true);
  });

  it("reads current TWAPs across DEXs, including spot, and subtracts executed quantity exactly", async () => {
    mock();
    twap.read.mockImplementation(async (user, dex) => ({
      data: { user, dex, states: [[1, twapState(user, dex ? `${dex}:BTC` : "ETH")],
        ...(dex ? [] : [[2, { ...twapState(user, "@107"), reduceOnly: true }]])] }, receivedAt: Date.now(),
    }));
    const result = await loadAccountStatus(transport(), address);
    expect(result.coverage.complete).toBe(true);
    expect(result.twaps.map(({ dex }) => dex)).toEqual(["", "xyz", "abc"]);
    expect(result.twaps[0]?.states?.[0]).toMatchObject({
      twapId: 1, sz: precise, executedSz: "9007199254740993.123456789012345677", remainingSz: "0.000000000000000001",
      side: "A", reduceOnly: false, trigger: { px: "100", above: true }, stopPx: "120",
    });
    expect(result.twaps[0]?.states?.[1]).toMatchObject({ coin: "@107", reduceOnly: true });
    expect(result.twaps.every(({ section }) => section.fresh && section.source === "websocket")).toBe(true);
    expect(Check(AccountStatusSchema, result)).toBe(true);
    expect(twap.close).toHaveBeenCalledOnce();
  });

  it("distinguishes an empty current TWAP event from unavailable schedules", async () => {
    mock();
    twap.read.mockImplementation(async (user, dex) => {
      if (dex === "xyz") throw new Error("sensitive websocket details");
      return { data: { user, dex, states: [] }, receivedAt: Date.now() };
    });
    const result = await loadAccountStatus(transport(), address);
    expect(result.twaps[0]).toMatchObject({ states: [], section: { fresh: true } });
    expect(result.twaps[1]).toMatchObject({ states: null, section: { fresh: false, updatedAt: null, source: "unavailable" } });
    expect(result.perps.every(({ sections }) => sections.state.fresh && sections.orders.fresh)).toBe(true);
    expect(result.coverage.complete).toBe(false);
    expect(result.errors).toEqual(["TWAP schedules (xyz) unavailable: request failed, timed out, or returned malformed data."]);
    expect(JSON.stringify(result)).not.toContain("sensitive");
    expect(twap.close).toHaveBeenCalledOnce();
  });

  it.each(["event-user", "event-dex", "state-user", "coin", "duplicate-id", "unsafe-id", "negative", "over-executed", "exponent", "number", "oversized", "missing-flag", "old-receipt", "future-receipt", "missing-states"])("rejects invalid TWAP evidence: %s", async (kind) => {
    mock();
    twap.read.mockImplementation(async (user, dex) => {
      const state: Record<string, unknown> = twapState(user, dex ? `${dex}:BTC` : "ETH");
      const data: Record<string, unknown> = { user, dex, states: [[1, state]] };
      if (dex !== "") return { data, receivedAt: Date.now() };
      if (kind === "event-user") data.user = `0x${"cd".repeat(20)}`;
      if (kind === "event-dex") data.dex = "xyz";
      if (kind === "state-user") state.user = `0x${"cd".repeat(20)}`;
      if (kind === "coin") state.coin = "xyz:BTC";
      if (kind === "duplicate-id") data.states = [[1, state], [1, state]];
      if (kind === "unsafe-id") data.states = [[Number.MAX_SAFE_INTEGER + 1, state]];
      if (kind === "negative") state.executedSz = "-1";
      if (kind === "over-executed") state.sz = "1";
      if (kind === "exponent") state.sz = "1e20";
      if (kind === "number") state.sz = 100;
      if (kind === "oversized") state.sz = "1".repeat(201);
      if (kind === "missing-flag") delete state.reduceOnly;
      if (kind === "missing-states") delete data.states;
      return { data, receivedAt: Date.now() + (kind === "old-receipt" ? -31_000 : kind === "future-receipt" ? 31_000 : 0) };
    });
    const result = await loadAccountStatus(transport(), address);
    expect(result.twaps[0]).toMatchObject({ states: null, section: { fresh: false } });
    expect(result.twaps[1]?.section.fresh).toBe(true);
    expect(result.coverage.complete).toBe(false);
    expect(result.errors).toHaveLength(1);
    expect(Check(AccountStatusSchema, result)).toBe(true);
  });

  it("bounds silent TWAP reads by the account deadline and closes the reader", async () => {
    mock();
    twap.read.mockImplementation(() => new Promise(() => {}));
    const timeout = new AbortController();
    const spy = vi.spyOn(AbortSignal, "timeout").mockReturnValue(timeout.signal);
    try {
      const pending = loadAccountStatus(transport(), address);
      await vi.waitFor(() => expect(twap.read).toHaveBeenCalled());
      timeout.abort();
      const result = await pending;
      expect(result.twaps.every(({ states, section }) => states === null && !section.fresh)).toBe(true);
      expect(result.coverage.complete).toBe(false);
      expect(twap.close).toHaveBeenCalledOnce();
    } finally { spy.mockRestore(); }
  });

  it("does not let silent TWAP subscriptions starve HTTP reads on later DEXs", async () => {
    const fetcher = mock((body) => body.type === "perpDexs"
      ? [null, ...Array.from({ length: 5 }, (_, index) => ({ name: `dex${index}` }))] : response(body));
    twap.read.mockImplementation(() => new Promise(() => {}));
    const timeout = new AbortController();
    const spy = vi.spyOn(AbortSignal, "timeout").mockReturnValue(timeout.signal);
    try {
      const pending = loadAccountStatus(transport(), address);
      await vi.waitFor(() => expect(fetcher).toHaveBeenCalledTimes(15));
      expect(twap.read).toHaveBeenCalledTimes(4);
      timeout.abort();
      const result = await pending;
      expect(result.perps).toHaveLength(6);
      expect(result.perps.every(({ sections }) => sections.state.fresh && sections.orders.fresh)).toBe(true);
      expect(result.twaps.every(({ states }) => states === null)).toBe(true);
      expect(result.coverage.complete).toBe(false);
      expect(twap.close).toHaveBeenCalledOnce();
    } finally { spy.mockRestore(); }
  });

  it("closes TWAP reads on caller cancellation", async () => {
    mock();
    twap.read.mockImplementation(() => new Promise(() => {}));
    const abort = new AbortController();
    const pending = loadAccountStatus(transport(), address, abort.signal);
    await vi.waitFor(() => expect(twap.read).toHaveBeenCalled());
    abort.abort();
    await expect(pending).rejects.toThrow("aborted");
    expect(twap.close).toHaveBeenCalledOnce();
  });

  it("propagates in-flight cancellation even when a transport ignores its signal", async () => {
    const abort = new AbortController();
    mock(() => new Promise(() => {}));
    const pending = loadAccountStatus(transport(), address, abort.signal);
    abort.abort();
    await expect(pending).rejects.toThrow("aborted");
  });
});
