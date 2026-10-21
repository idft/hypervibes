// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { defaultChartOverlays, type ChartFillsPage } from "../charts/trading-contracts.js";

const mocks = vi.hoisted(() => ({ fills: vi.fn() }));
vi.mock("../web/charts.js", () => ({ getChartFills: mocks.fills }));
import { useChartAccount } from "./use-chart-account.js";
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
class Source extends EventTarget {
  static instances: Source[] = [];
  close = vi.fn(); onerror: (() => void) | null = null;
  constructor(readonly url: string) { super(); Source.instances.push(this); }
}
const page = (price = "100"): ChartFillsPage => ({ fills: [{ id: "a".repeat(64), time: 1, side: "Buy", size: "0.01", price }], nextCursor: null, historyLimited: false, sync: { updatedAt: 1, error: null } });
describe("chart account data", () => {
  let root: Root, host: HTMLDivElement, client: QueryClient;
  let current: ReturnType<typeof useChartAccount>;
  function Probe({ id, coin, from = 0 }: { id: string; coin: string; from?: number }) {
    current = useChartAccount(id, coin, from, 100, defaultChartOverlays);
    return <>{current.messages.join(" · ")}</>;
  }
  const render = async (id = "agent", coin = "BTC", from = 0) => { await act(async () => { root.render(<QueryClientProvider client={client}><Probe id={id} coin={coin} from={from} /></QueryClientProvider>); }); await act(async () => vi.advanceTimersByTimeAsync(1)); };
  beforeEach(() => {
    vi.useFakeTimers(); Source.instances = []; vi.stubGlobal("EventSource", Source);
    mocks.fills.mockReset().mockResolvedValue(page());
    client = new QueryClient({ defaultOptions: { queries: { retryDelay: 0 } } });
    host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  });
  afterEach(async () => { await act(async () => root.unmount()); client.clear(); host.remove(); vi.unstubAllGlobals(); vi.useRealTimers(); });
  it("refreshes journal markers without exchange calls and retains a stale snapshot after query failure", async () => {
    await render();
    expect(current.fills[0]?.price).toBe("100");
    mocks.fills.mockResolvedValue(page("101.00000001"));
    await act(async () => vi.advanceTimersByTimeAsync(5_001));
    expect(current.fills[0]?.price).toBe("101.00000001");
    mocks.fills.mockRejectedValue(new Error("offline"));
    await act(async () => vi.advanceTimersByTimeAsync(5_100));
    expect(current.fills[0]?.price).toBe("101.00000001");
    expect(host.textContent).toContain("Trade history unavailable");
    expect(current.canRetry).toBe(true);
    mocks.fills.mockResolvedValue(page("102"));
    await act(async () => current.retry());
    await act(async () => vi.advanceTimersByTimeAsync(1));
    expect(current.fills[0]?.price).toBe("102");
    expect(host.textContent).not.toContain("Trade history unavailable");
  });
  it("cancels obsolete range queries and never carries markers across instruments or Agents", async () => {
    await render();
    expect(current.fills).toHaveLength(1);
    let complete!: (value: ChartFillsPage) => void;
    mocks.fills.mockImplementationOnce(() => new Promise((resolve) => { complete = resolve; }));
    await render("agent", "ETH");
    expect(current.fills).toEqual([]);
    const obsoleteSignal = mocks.fills.mock.calls.at(-1)![0].signal as AbortSignal;
    mocks.fills.mockResolvedValue({ ...page(), fills: [] });
    await render("agent", "ETH", 10);
    expect(obsoleteSignal.aborted).toBe(true);
    await act(async () => complete(page("999")));
    expect(current.fills).toEqual([]);
    expect(Source.instances).toHaveLength(1); // Symbol/timeframe changes reuse the linked account stream.
    await render("other", "ETH", 10);
    expect(current.snapshot).toBeNull(); expect(current.fills).toEqual([]);
    expect(Source.instances[0]!.close).toHaveBeenCalledOnce();
    expect(Source.instances[1]!.url).toBe("/api/agents/other/dashboard/events");
  });
});
