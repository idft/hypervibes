import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AppRuntime } from "../runtime.js";
import { AgentError } from "../agents/agent-input.js";
const mocks = vi.hoisted(() => ({ currentUser: vi.fn(), headers: vi.fn(), request: vi.fn() }));
vi.mock("../auth-user.js", () => ({ currentUser: mocks.currentUser }));
vi.mock("@tanstack/react-start/server", () => ({ getRequestHeaders: mocks.headers, getRequest: mocks.request }));
vi.mock("@tanstack/react-start", () => {
  const builder = (validate: (data: unknown) => unknown = (data) => data) => ({
    validator: (next: (data: unknown) => unknown) => builder(next),
    handler: (handler: (args: { context: { runtime: AppRuntime }; data: unknown }) => Promise<unknown>) => ({ invoke: (runtime: AppRuntime, input: unknown) => handler({ context: { runtime }, data: validate(input) }) }),
  });
  return { createServerFn: () => builder() };
});
import { getIndicator, getIndicatorVersions, listIndicators, previewIndicator, previewSavedIndicator, renderChartIndicators, saveIndicator } from "./indicators.js";
/** The shared mock runs validators synchronously; normalize sync throws to rejections. */
function invoke(fn: unknown, runtime: AppRuntime, input: unknown): Promise<unknown> { return (fn as { invoke(runtime: AppRuntime, input: unknown): Promise<unknown> }).invoke(runtime, input); }
async function rejected(fn: unknown, runtime: AppRuntime, input: unknown): Promise<Error> {
  try { await invoke(fn, runtime, input); }
  catch (error) { return error as Error; }
  throw new Error("Expected the server function to fail");
}
const source = '//@version=6\nindicator("T")\nplot(close)';
describe("authenticated Agent indicator server functions", () => {
  const owned = vi.fn(), list = vi.fn(), get = vi.fn(), versions = vi.fn(), run = vi.fn(), preview = vi.fn(), save = vi.fn(), renderChart = vi.fn(), chartsGet = vi.fn(), researchHistory = vi.fn();
  let runtime: AppRuntime;
  beforeEach(() => {
    vi.resetAllMocks(); mocks.currentUser.mockResolvedValue({ id: "owner" }); mocks.headers.mockReturnValue(new Headers({ origin: "http://localhost:3003" })); mocks.request.mockReturnValue({ signal: new AbortController().signal });
    owned.mockImplementation(async (_user: string, id: string) => { if (id !== "agent") throw new AgentError("Agent not found", 404); });
    list.mockResolvedValue({ indicators: [{ id: "builtin:ema", builtIn: true, name: "EMA", description: "", version: 1, sourceHash: "a".repeat(64), overlay: true }], total: 1, nextOffset: null });
    get.mockResolvedValue({ descriptor: { id: "builtin:ema", builtIn: true, name: "EMA", description: "", interpretation: "", version: 1, sourceHash: "a".repeat(64), inputs: [], overlay: true } });
    versions.mockResolvedValue({ versions: [{ version: 1, sourceHash: "a".repeat(64) }], nextOffset: null });
    run.mockResolvedValue({ indicatorId: "builtin:ema", coin: "BTC", interval: "1h", end: 3600_000, values: [] });
    preview.mockResolvedValue({ indicatorId: "preview", coin: "BTC", interval: "1h", end: 3600_000, values: [] });
    save.mockResolvedValue({ indicatorId: "custom:12345678-1234-4234-8234-123456789abc", version: 1, sourceHash: "b".repeat(64) });
    renderChart.mockResolvedValue({ layoutId: "12345678-1234-4234-8234-123456789abc", revision: 3, instances: [] });
    chartsGet.mockResolvedValue({ id: "12345678-1234-4234-8234-123456789abc", revision: 3, document: { coin: "BTC", interval: "1h", indicators: [] } });
    researchHistory.mockResolvedValue({ candles: [{ time: 0, open: 1, high: 2, low: 1, close: 2, volume: 1 }] });
    runtime = { origin: "http://localhost:3003", status: vi.fn(), agents: { owned, charts: { get: chartsGet }, indicators: { list, get, versions, run, preview, save, renderChart } }, hyperliquid: { charts: { researchHistory } } } as unknown as AppRuntime;
  });
  it("checks ownership before every read and forwards validated queries", async () => {
    expect(await invoke(listIndicators, runtime, { agentId: "agent", kind: "custom", limit: 5 })).toMatchObject({ total: 1 });
    expect(list).toHaveBeenCalledWith("agent", { agentId: "agent", kind: "custom", limit: 5, query: undefined, offset: undefined });
    await expect(invoke(listIndicators, runtime, { agentId: "foreign" })).rejects.toMatchObject({ status: 404 });
    expect(await invoke(getIndicator, runtime, { agentId: "agent", indicatorId: "builtin:ema", includeSource: true, sourceLimit: 100 })).toMatchObject({ descriptor: { id: "builtin:ema" } });
    expect(get).toHaveBeenCalledWith("agent", "builtin:ema", { agentId: "agent", indicatorId: "builtin:ema", includeSource: true, sourceLimit: 100, version: undefined, sourceOffset: undefined });
    expect(await invoke(getIndicatorVersions, runtime, { agentId: "agent", indicatorId: "builtin:ema", offset: 20 })).toMatchObject({ versions: [{ version: 1 }] });
    await expect((await rejected(listIndicators, runtime, { agentId: "agent", kind: "nope" })).message).toContain("Invalid indicator kind");
  });
  it("validates preview source and windows before touching the runtime", async () => {
    await expect((await rejected(previewIndicator, runtime, { agentId: "agent", request: { source: 'indicator("x")', coin: "BTC", interval: "1h", start: 0, end: 3600000 } })).message).toContain("Native PineScript");
    await expect((await rejected(previewIndicator, runtime, { agentId: "agent", request: { source, coin: "BTC", interval: "1h", start: 3600000, end: 0 } })).message).toContain("must end after it starts");
    expect(preview).not.toHaveBeenCalled();
    await invoke(previewIndicator, runtime, { agentId: "agent", request: { source, coin: "BTC", interval: "1h", start: 0, end: 3600000, count: 20 } });
    expect(preview).toHaveBeenCalledWith("agent", expect.objectContaining({ source, coin: "BTC", count: 20 }), expect.objectContaining({ signal: expect.anything() }));
    // Previews return candles for the read-only chart alongside the bounded result.
    expect(researchHistory).toHaveBeenCalledWith("BTC", "1h", 0, 3600000, 5000, expect.anything());
    await invoke(previewSavedIndicator, runtime, { agentId: "agent", request: { indicatorId: "builtin:ema", coin: "BTC", interval: "1h", start: 0, end: 3600000 } });
    expect(run).toHaveBeenCalledWith("agent", expect.objectContaining({ indicatorId: "builtin:ema" }), expect.anything());
  });
  it("saves with an idempotent user operation key and the authenticated identity", async () => {
    const request = { name: "Mine", description: "d", interpretation: "i", source, validation: { coin: "BTC", interval: "1h", start: 0, end: 3600000 } };
    const first = await invoke(saveIndicator, runtime, { agentId: "agent", operationKey: "00000000-0000-4000-8000-000000000001", request });
    expect(first).toMatchObject({ version: 1 });
    expect(save).toHaveBeenCalledWith("agent", request, "user:00000000-0000-4000-8000-000000000001", { kind: "user", userId: "owner" }, expect.objectContaining({ signal: expect.anything() }));
    await invoke(saveIndicator, runtime, { agentId: "agent", operationKey: "00000000-0000-4000-8000-000000000002", request });
    // Each caller-supplied key stays its own operation; retries reuse the service's own receipt.
    expect(save.mock.calls.map((call) => call[2])).toEqual(["user:00000000-0000-4000-8000-000000000001", "user:00000000-0000-4000-8000-000000000002"]);
    await expect(invoke(saveIndicator, runtime, { agentId: "foreign", operationKey: "00000000-0000-4000-8000-000000000003", request })).rejects.toMatchObject({ status: 404 });
    await expect((await rejected(saveIndicator, runtime, { agentId: "agent", operationKey: "not-a-uuid", request })).message).toContain("request ID");
  });
  it("recalculates chart instances only for a current revision of the caller's own layout", async () => {
    const result = await invoke(renderChartIndicators, runtime, { agentId: "agent", layoutId: "12345678-1234-4234-8234-123456789abc", revision: 3, end: 3600000, coin: "BTC", interval: "1h" });
    expect(result).toMatchObject({ revision: 3 });
    expect(renderChart).toHaveBeenCalledWith("agent", expect.objectContaining({ revision: 3 }), 3600000, expect.anything());
    chartsGet.mockResolvedValue({ id: "12345678-1234-4234-8234-123456789abc", revision: 4, document: { coin: "BTC", interval: "1h", indicators: [] } });
    await expect((await rejected(renderChartIndicators, runtime, { agentId: "agent", layoutId: "12345678-1234-4234-8234-123456789abc", revision: 3, end: 3600000, coin: "BTC", interval: "1h" })).message).toContain("Chart changed");
    expect(renderChart).toHaveBeenCalledTimes(1);
    chartsGet.mockResolvedValue({ id: "12345678-1234-4234-8234-123456789abc", revision: 3, document: { coin: "ETH", interval: "1h", indicators: [] } });
    await expect((await rejected(renderChartIndicators, runtime, { agentId: "agent", layoutId: "12345678-1234-4234-8234-123456789abc", revision: 3, end: 3600000, coin: "BTC", interval: "1h" })).message).toContain("Chart changed");
    await expect((await rejected(renderChartIndicators, runtime, { agentId: "agent", layoutId: "12345678-1234-4234-8234-123456789abc", revision: 3, end: 3600.5, coin: "BTC", interval: "1h" })).message).toContain("revision");
  });
  it("rejects cross-origin mutations and unauthenticated access", async () => {
    mocks.headers.mockReturnValue(new Headers({ origin: "https://evil.example" }));
    await expect(invoke(saveIndicator, runtime, { agentId: "agent", operationKey: "00000000-0000-4000-8000-000000000004", request: { name: "n", description: "", interpretation: "", source, validation: { coin: "BTC", interval: "1h", start: 0, end: 3600000 } } })).rejects.toBeDefined();
    await expect(invoke(previewIndicator, runtime, { agentId: "agent", request: { source, coin: "BTC", interval: "1h", start: 0, end: 3600000 } })).rejects.toBeDefined();
    expect(save).not.toHaveBeenCalled(); expect(preview).not.toHaveBeenCalled();
    mocks.headers.mockReturnValue(new Headers({ origin: "http://localhost:3003" })); mocks.currentUser.mockResolvedValue(null);
    await expect(invoke(listIndicators, runtime, { agentId: "agent" })).rejects.toBeDefined();
    expect(list).not.toHaveBeenCalled();
  });
});
