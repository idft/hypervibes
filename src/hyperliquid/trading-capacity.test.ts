import { HttpTransport } from "@nktkas/hyperliquid";
import { Check } from "typebox/value";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { loadTradingCapacity, TradingCapacitySchema } from "./trading-capacity.js";

const address = "0x1234567890abcdef1234567890abcdef12345678";
const foreignAddress = "0xabcdefabcdefabcdefabcdefabcdefabcdefabcd";
const activeData = (coin = "BTC") => ({
  user: address, coin, leverage: { type: "cross", value: 3 },
  maxTradeSzs: ["0.12345678901234567890", "0.000000000000000001"],
  availableToTrade: ["123456789.12345678901234567890", "0.000000000000000003"],
  markPx: "105000.12345678901234567890",
});
type RequestBody = { type: string; coin?: string; user?: string; dex?: string };
const requestBody = (init?: RequestInit) => JSON.parse(String(init?.body)) as RequestBody;

// All requests use mocked fetch. No wallets, credentials, database or live account reads.
describe("read-only instrument-specific trading capacity", () => {
  const transport = new HttpTransport();
  let data: unknown;
  let metadata: unknown;
  let mode: unknown;
  let fetcher: ReturnType<typeof vi.fn<(input: string | URL | Request, init?: RequestInit) => Promise<Response>>>;
  beforeEach(() => {
    data = activeData();
    metadata = { universe: [{ name: "ETH" }, { name: "BTC" }, { name: "kPEPE" }, { name: "OLD", isDelisted: true }] };
    mode = "unifiedAccount";
    fetcher = vi.fn(async (_input, init) => {
      const body = requestBody(init);
      if (body.type === "meta") return Response.json(metadata);
      if (body.type === "userAbstraction") return Response.json(mode);
      if (body.type === "activeAssetData") return Response.json(data);
      throw new Error(`Unexpected request: ${body.type}`);
    });
    vi.stubGlobal("fetch", fetcher);
  });
  afterEach(() => { vi.unstubAllGlobals(); vi.restoreAllMocks(); });

  it.each(["BTC", "ETH", "kPEPE"])("preserves exact strings and verified array sides for %s", async (instrument) => {
    data = activeData(instrument);
    const before = Date.now();
    const result = await loadTradingCapacity(transport, address, instrument);
    expect(Check(TradingCapacitySchema, result)).toBe(true);
    expect(result).toMatchObject({
      address, instrument, scope: "main-dex-perpetuals", capacityStatus: "observed", openingCapacityStatus: "unknown",
      accountAbstraction: "unifiedAccount", activeAssetData: data,
      venueCapacity: {
        buy: { availableToTradeUsdc: activeData().availableToTrade[0], maxTradeSizeBase: activeData().maxTradeSzs[0] },
        sell: { availableToTradeUsdc: activeData().availableToTrade[1], maxTradeSizeBase: activeData().maxTradeSzs[1] },
      },
      interpretation: { availableToTrade: { ordering: ["buy", "sell"], units: "USDC" }, maxTradeSzs: { ordering: ["buy", "sell"], units: "base-asset" } },
    });
    expect(result.reasons).toEqual([expect.objectContaining({ code: "opening_capacity_unknown", status: "unknown" })]);
    expect(result.references.some((reference) => reference.url === "https://app.hyperliquid.xyz/assets/config-D-6TkbK9.js")).toBe(true);
    expect(result.warning).toContain("not maximum opening size");
    expect(result.warning).toContain("not atomic");
    for (const section of Object.values(result.sections)) {
      expect(section.fresh).toBe(true);
      expect(section.requestedAt).toBeGreaterThanOrEqual(before);
      expect(section.observedAt).toBeGreaterThanOrEqual(section.requestedAt!);
      expect(section.observedAt).toBeLessThanOrEqual(result.observedAt);
      expect(section.venueTimestamp).toBeNull();
    }
    const bodies = fetcher.mock.calls.map(([, init]) => requestBody(init));
    expect(bodies).toEqual([
      { type: "meta", dex: "" }, { type: "userAbstraction", user: address },
      { type: "activeAssetData", user: address, coin: instrument },
    ]);
    for (const [input, init] of fetcher.mock.calls) {
      expect(String(input)).toMatch(/\/info$/);
      expect(init?.method).toBe("POST");
      expect(init?.signal).toBeInstanceOf(AbortSignal);
    }
  });

  it.each(["default", "disabled", "unifiedAccount", "portfolioMargin", "dexAbstraction"])("queries the assigned account for mode %s", async (accountMode) => {
    mode = accountMode;
    data = { ...activeData(), user: address.toUpperCase().replace("0X", "0x") };
    const result = await loadTradingCapacity(transport, address, "BTC");
    expect(result.accountAbstraction).toBe(accountMode);
    expect(result.sections.mode.fresh).toBe(true);
    expect(result.activeAssetData).not.toBeNull();
    expect(Check(TradingCapacitySchema, result)).toBe(true);
  });

  it("keeps signed isolated rawUsd and explicit zero values unchanged", async () => {
    data = { ...activeData(), leverage: { type: "isolated", value: 2, rawUsd: "-0.12345678901234567890" }, maxTradeSzs: ["0.0", "0"], availableToTrade: ["0", "0.00"] };
    const result = await loadTradingCapacity(transport, address, "BTC");
    expect(result.activeAssetData).toEqual(data);
    expect(result.capacityStatus).toBe("observed");
    expect(result.venueCapacity?.buy.maxTradeSizeBase).toBe("0.0");
    expect(Check(TradingCapacitySchema, result)).toBe(true);
  });

  it.each([
    ["maxTradeSzs", ["1"]], ["maxTradeSzs", ["1", "2", "3"]], ["maxTradeSzs", [1, "2"]],
    ["maxTradeSzs", ["-1", "2"]], ["maxTradeSzs", ["1e3", "2"]], ["maxTradeSzs", null],
    ["availableToTrade", ["1", "NaN"]], ["availableToTrade", [" 1", "2"]], ["availableToTrade", ["1", "Infinity"]],
    ["availableToTrade", ["1", ".2"]], ["availableToTrade", ["1", "2\n"]], ["availableToTrade", ["1", "-0"]],
    ["markPx", "NaN"], ["markPx", -1], ["markPx", "-1"], ["markPx", null],
    ["leverage", { type: "cross", value: 0 }], ["leverage", { type: "cross", value: 1.5 }],
    ["leverage", { type: "isolated", value: 3 }], ["leverage", { type: "isolated", value: 3, rawUsd: "1e5" }],
    ["user", "malformed"], ["coin", null],
  ])("rejects malformed %s=%j without manufacturing capacity", async (field, value) => {
    data = { ...activeData(), [field as string]: value };
    const result = await loadTradingCapacity(transport, address, "BTC");
    expect(result.activeAssetData).toBeNull();
    expect(result.venueCapacity).toBeNull();
    expect(result.capacityStatus).toBe("unavailable");
    expect(result.sections.activeAssetData).toMatchObject({ fresh: false, observedAt: null });
    expect(result.reasons).toContainEqual(expect.objectContaining({ section: "activeAssetData", code: "invalid_response" }));
    expect(Check(TradingCapacitySchema, result)).toBe(true);
  });

  it.each([{ user: foreignAddress }, { coin: "ETH" }, { coin: "btc" }])("rejects a response with foreign identity %j", async (identity) => {
    data = { ...activeData(), ...identity };
    const result = await loadTradingCapacity(transport, address, "BTC");
    expect(result.activeAssetData).toBeNull();
    expect(result.venueCapacity).toBeNull();
    expect(result.reasons).toContainEqual(expect.objectContaining({ code: "identity_mismatch" }));
  });

  it.each([null, {}, "futureMode"])("keeps unknown mode %j separate from valid venue data", async (value) => {
    mode = value;
    const result = await loadTradingCapacity(transport, address, "BTC");
    expect(result.accountAbstraction).toBeNull();
    expect(result.sections.mode.fresh).toBe(false);
    expect(result.activeAssetData).toEqual(data);
    expect(result.reasons).toContainEqual(expect.objectContaining({ section: "mode", code: "invalid_response" }));
  });

  it.each(["DOGE", "OLD", "btc"])("does not request capacity for unsupported instrument %s", async (instrument) => {
    const result = await loadTradingCapacity(transport, address, instrument);
    expect(result.capacityStatus).toBe("unavailable");
    expect(result.reasons).toContainEqual(expect.objectContaining({ code: "unsupported_instrument" }));
    expect(result.sections.activeAssetData.requestedAt).toBeNull();
    expect(fetcher.mock.calls.map(([, init]) => requestBody(init).type)).not.toContain("activeAssetData");
  });

  it.each([null, {}, { universe: [{ name: "BTC" }, { name: "BTC" }] }, { universe: [{ name: "BTC", isDelisted: "false" }] }])("fails closed when main-DEX metadata is malformed: %j", async (value) => {
    metadata = value;
    const result = await loadTradingCapacity(transport, address, "BTC");
    expect(result.activeAssetData).toBeNull();
    expect(result.accountAbstraction).toBe("unifiedAccount");
    expect(result.reasons).toContainEqual(expect.objectContaining({ section: "metadata", code: "invalid_response" }));
    expect(result.reasons).toContainEqual(expect.objectContaining({ section: "activeAssetData", code: "instrument_unverified" }));
  });

  it.each(["", " BTC ", "xyz:BTC", "@1", "BTC/USDC", "BTC\n"])("rejects malformed or foreign-DEX input %j before HTTP", async (instrument) => {
    await expect(loadTradingCapacity(transport, address, instrument)).rejects.toThrow();
    expect(fetcher).not.toHaveBeenCalled();
  });

  it("returns structured absence without requests when no account is assigned", async () => {
    const result = await loadTradingCapacity(transport, null, "BTC");
    expect(Check(TradingCapacitySchema, result)).toBe(true);
    expect(result.activeAssetData).toBeNull();
    expect(result.accountAbstraction).toBeNull();
    expect(result.reasons.filter((reason) => reason.code === "no_assigned_account")).toHaveLength(3);
    expect(fetcher).not.toHaveBeenCalled();
    await expect(loadTradingCapacity(transport, "invalid", "BTC")).rejects.toThrow("Invalid assigned");
    expect(fetcher).not.toHaveBeenCalled();
  });

  it.each(["meta", "userAbstraction", "activeAssetData"])("reports an HTTP failure for %s without stale or zero fallback", async (failedType) => {
    const original = fetcher.getMockImplementation()!;
    fetcher.mockImplementation(async (input, init) => {
      if (requestBody(init).type === failedType) throw new Error("offline");
      return original(input, init);
    });
    const result = await loadTradingCapacity(transport, address, "BTC");
    const section = failedType === "meta" ? "metadata" : failedType === "userAbstraction" ? "mode" : "activeAssetData";
    expect(result.sections[section].fresh).toBe(false);
    expect(result.reasons).toContainEqual(expect.objectContaining({ section, code: "request_failed" }));
    expect(Check(TradingCapacitySchema, result)).toBe(true);
    if (failedType === "userAbstraction") expect(result.activeAssetData).toEqual(data);
    else expect(result.activeAssetData).toBeNull();
  });

  it("fetches each observation anew, rather than returning cached capacity", async () => {
    await loadTradingCapacity(transport, address, "BTC");
    data = { ...activeData(), maxTradeSzs: ["1", "2"] };
    expect((await loadTradingCapacity(transport, address, "BTC")).venueCapacity?.buy.maxTradeSizeBase).toBe("1");
    expect(fetcher).toHaveBeenCalledTimes(6);
  });

  it("propagates caller cancellation before making requests", async () => {
    const controller = new AbortController(); controller.abort(new Error("Canceled"));
    await expect(loadTradingCapacity(transport, address, "BTC", controller.signal)).rejects.toThrow("Canceled");
    expect(fetcher).not.toHaveBeenCalled();
  });

  it.each(["caller", "deadline"])("bounds in-flight HTTP reads with %s cancellation", async (kind) => {
    const controller = new AbortController();
    let notifyStarted!: () => void;
    const started = new Promise<void>((resolve) => { notifyStarted = resolve; });
    if (kind === "deadline") vi.spyOn(AbortSignal, "timeout").mockReturnValue(controller.signal);
    const original = fetcher.getMockImplementation()!;
    fetcher.mockImplementation(async (input, init) => {
      if (requestBody(init).type !== "activeAssetData") return original(input, init);
      const pending = new Promise<Response>((_resolve, reject) => {
        init!.signal!.addEventListener("abort", () => reject(init!.signal!.reason), { once: true });
      });
      notifyStarted();
      return pending;
    });
    const task = loadTradingCapacity(transport, address, "BTC", kind === "caller" ? controller.signal : undefined);
    await started;
    controller.abort(new Error(kind));
    if (kind === "caller") await expect(task).rejects.toThrow("caller");
    else {
      const result = await task;
      expect(AbortSignal.timeout).toHaveBeenCalledWith(10_000);
      expect(result.activeAssetData).toBeNull();
      expect(result.sections.activeAssetData.fresh).toBe(false);
      expect(result.reasons).toContainEqual(expect.objectContaining({ section: "activeAssetData", code: "deadline_exceeded" }));
    }
  });
});
