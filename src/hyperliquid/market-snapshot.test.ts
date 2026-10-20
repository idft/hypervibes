import { BACKGROUND_CONTEXT as context } from "@earendil-works/chord/context";
import { HttpTransport } from "@nktkas/hyperliquid";
import { createModels } from "@earendil-works/pi-ai/models";
import { fauxAssistantMessage, fauxProvider, fauxToolCall } from "@earendil-works/pi-ai/providers/faux";
import { createRegistry, Harness, MemoryStorage } from "@earendil-works/pi-durable";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createHyperliquidExtension } from "./durable-extension.js";
import { loadMarketSnapshot } from "./market-snapshot.js";
import type { OrderService } from "./orders/order-service.js";

const assetContext = (markPx = "105000.123456789") => ({
  markPx, midPx: "105001.123456789" as string | null, oraclePx: "104999.9", prevDayPx: "100000",
  dayNtlVlm: "123456789.123456789", dayBaseVlm: "1234.56789", funding: "-0.00001234", openInterest: "4567.89123",
  premium: null, impactPxs: null,
});
const response = (): [{ universe: { name: string; isDelisted?: boolean }[] }, ReturnType<typeof assetContext>[]] => [{ universe: [{ name: "ETH" }, { name: "BTC" }, { name: "OLD", isDelisted: true }, { name: "kPEPE" }] }, [assetContext("2000"), assetContext(), assetContext(), assetContext("0.01")]];

describe("read-only main-DEX market snapshots", () => {
  const transport = new HttpTransport();
  let fetcher: ReturnType<typeof vi.fn>;
  beforeEach(() => {
    fetcher = vi.fn(async () => Response.json(response()));
    vi.stubGlobal("fetch", fetcher);
  });
  afterEach(() => { vi.unstubAllGlobals(); vi.restoreAllMocks(); });

  it("uses one index-aligned HTTP response and preserves decimal precision and units", async () => {
    const before = Date.now();
    const snapshot = await loadMarketSnapshot(transport, "BTC");
    expect(snapshot).toMatchObject({
      instrument: "BTC", scope: "main-dex-perpetuals", source: "Hyperliquid metaAndAssetCtxs HTTP",
      midPrice: "105001.123456789", markPrice: "105000.123456789", oraclePrice: "104999.9",
      previousDayPrice: "100000", markChange24hPercent: "5.000123456789",
      volume24hUsd: "123456789.123456789", volume24hBase: "1234.56789",
      fundingRateHourly: "-0.00001234", openInterestBase: "4567.89123",
    });
    expect(snapshot.observedAt).toBeGreaterThanOrEqual(before);
    expect(snapshot.observedAt).toBeLessThanOrEqual(Date.now());
    expect(snapshot.warning).toContain("not executable quotes");
    expect(fetcher).toHaveBeenCalledOnce();
    const init = fetcher.mock.calls[0]![1] as RequestInit;
    expect(JSON.parse(String(init.body))).toEqual({ type: "metaAndAssetCtxs", dex: "" });
    expect((await loadMarketSnapshot(transport, "kPEPE")).markPrice).toBe("0.01");
  });

  it("keeps a missing mid and an undefined zero-baseline change unknown", async () => {
    const raw = response();
    raw[1][1] = { ...assetContext(), midPx: null, prevDayPx: "0" };
    fetcher.mockResolvedValueOnce(Response.json(raw));
    expect(await loadMarketSnapshot(transport, "BTC")).toMatchObject({ midPrice: null, markChange24hPercent: null });
  });

  it.each(["DOGE", "OLD", "@1", "xyz:BTC", "btc", "", " BTC "])("rejects unsupported instrument %s", async (instrument) => {
    await expect(loadMarketSnapshot(transport, instrument)).rejects.toThrow();
  });

  it("rejects missing, mismatched and malformed context rather than guessing", async () => {
    fetcher.mockResolvedValueOnce(Response.json([{ universe: [{ name: "BTC" }] }, []]));
    await expect(loadMarketSnapshot(transport, "BTC")).rejects.toThrow("unavailable");
    fetcher.mockResolvedValueOnce(Response.json([{ universe: [{ name: "BTC" }] }, [{ ...assetContext(), markPx: "NaN" }]]));
    await expect(loadMarketSnapshot(transport, "BTC")).rejects.toThrow();
    fetcher.mockResolvedValueOnce(Response.json([{ universe: [{ name: "BTC" }] }, [{ ...assetContext(), openInterest: "-1" }]]));
    await expect(loadMarketSnapshot(transport, "BTC")).rejects.toThrow("Invalid");
  });

  it("propagates cancellation and failures, and fetches anew on subsequent calls", async () => {
    const controller = new AbortController(); controller.abort(new Error("Canceled"));
    await expect(loadMarketSnapshot(transport, "BTC", controller.signal)).rejects.toThrow("Canceled");
    expect(fetcher).not.toHaveBeenCalled();
    fetcher.mockRejectedValueOnce(new Error("offline"));
    await expect(loadMarketSnapshot(transport, "BTC")).rejects.toThrow();
    expect((await loadMarketSnapshot(transport, "BTC")).instrument).toBe("BTC");
    await loadMarketSnapshot(transport, "BTC");
    expect(fetcher).toHaveBeenCalledTimes(3);
  });

  it("executes through Durable with schema-validated output and no account or trading authority", async () => {
    const registry = createRegistry();
    const snapshot = vi.fn((instrument: string, signal?: AbortSignal) => loadMarketSnapshot(transport, instrument, signal));
    registry.install(createHyperliquidExtension({
      marketSnapshot: snapshot,
      agentAccountStatus: async () => { throw new Error("No account access expected"); },
    }, "owner", "agent", {} as OrderService));
    const models = createModels(), faux = fauxProvider(); models.setProvider(faux.provider);
    faux.setResponses([
      fauxAssistantMessage(fauxToolCall("get_market_snapshot", { instrument: "BTC" }), { stopReason: "toolUse" }),
      (transcript) => {
        const result = transcript.messages.find((message) => message.role === "toolResult");
        expect(result).toMatchObject({ toolName: "get_market_snapshot", isError: false });
        expect(result?.content).toEqual([{ type: "text", text: expect.stringContaining('"markPrice":"105000.123456789"') }]);
        return fauxAssistantMessage("Fresh BTC snapshot");
      },
    ]);
    const harness = await Harness.open(new MemoryStorage(), { models, registry }, context);
    try {
      const extension = registry.snapshot().extension("hyperliquid")!;
      const tool = extension.tools!.find((tool) => tool.name === "get_market_snapshot")!;
      const root = await harness.root(context, { agent: { model: { provider: "faux", modelId: "faux-1" }, extensions: [extension], tools: [tool] } });
      const settled = await (await root.submit({ type: "input", content: "What's BTC doing?" }, context)).wait(context);
      expect(settled.status).toBe("done");
      expect(snapshot).toHaveBeenCalledWith("BTC", expect.any(AbortSignal));
    } finally { await harness.close(context); }
  });
});
