// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ChartCandle, ChartSnapshot } from "../hyperliquid/chart-contracts.js";

const mocks = vi.hoisted(() => ({ history: vi.fn(), charts: [] as ReturnType<typeof makeChart>[] }));
function makeChart() {
  let listener: ((range: { from: number; to: number }) => void) | undefined;
  const range = { from: 0, to: 120 };
  const timeScale = { setVisibleLogicalRange: vi.fn(), getVisibleLogicalRange: () => range,
    subscribeVisibleLogicalRangeChange: vi.fn((value) => { listener = value; }), unsubscribeVisibleLogicalRangeChange: vi.fn() };
  const series = () => ({ setData: vi.fn(), update: vi.fn(), applyOptions: vi.fn(), priceScale: () => ({ applyOptions: vi.fn() }) });
  const price = series(); const volume = series();
  return { addSeries: vi.fn().mockReturnValueOnce(price).mockReturnValueOnce(volume), timeScale: () => timeScale, price, volume,
    remove: vi.fn(), scroll: (from: number) => listener?.({ from, to: from + 120 }) };
}
vi.mock("lightweight-charts", () => ({ CandlestickSeries: "candles", HistogramSeries: "volume", ColorType: { Solid: "solid" }, createChart: () => {
  const chart = makeChart(); mocks.charts.push(chart); return chart;
} }));
vi.mock("../web/charts.js", () => ({ getChartHistory: mocks.history }));
import { AgentChartPage } from "./agent-chart-page.js";
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const instruments = ["BTC", "ETH"];
const candle = (time: number, close = 11): ChartCandle => ({ time, open: 10, high: 15, low: 9, close, volume: 20 });
class FakeEventSource extends EventTarget {
  static instances: FakeEventSource[] = [];
  close = vi.fn();
  onerror: (() => void) | null = null;
  constructor(readonly url: string) { super(); FakeEventSource.instances.push(this); }
  send(snapshot: ChartSnapshot) { this.dispatchEvent(new MessageEvent("chart", { data: JSON.stringify(snapshot) })); }
}
describe("Agent trading chart", () => {
  let root: Root;
  let container: HTMLDivElement;
  beforeEach(() => {
    localStorage.clear();
    mocks.history.mockReset().mockResolvedValue({ ok: true, data: { candles: [candle(3600), candle(7200)], hasMore: true } });
    mocks.charts.length = 0;
    FakeEventSource.instances = [];
    vi.stubGlobal("EventSource", FakeEventSource);
    container = document.createElement("div"); document.body.append(container); root = createRoot(container);
  });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.unstubAllGlobals(); });
  const render = async (id = "fixture") => { await act(async () => root.render(<AgentChartPage key={id} id={id} instruments={instruments} />)); };
  const change = async (label: string, value: string) => { await act(async () => {
    const select = container.querySelector<HTMLSelectElement>(`[aria-label="${label}"]`)!;
    select.value = value; select.dispatchEvent(new Event("change", { bubbles: true }));
  }); };
  it("defaults to BTC/1h, draws candles and volume, and streams through the backend", async () => {
    await render();
    expect(FakeEventSource.instances[0]!.url).toBe("/api/agents/fixture/chart/events?coin=BTC&interval=1h");
    expect(mocks.history).toHaveBeenCalledWith({ data: { id: "fixture", coin: "BTC", interval: "1h" } });
    const chart = mocks.charts[0]!;
    expect(chart.price.setData).toHaveBeenLastCalledWith([candle(3600), candle(7200)]);
    expect(chart.volume.setData.mock.calls.at(-1)![0]).toHaveLength(2);
    await act(async () => FakeEventSource.instances[0]!.send({ status: "live", candles: [candle(7200, 14)] }));
    expect(chart.price.update).toHaveBeenLastCalledWith(candle(7200, 14), true);
    const indicator = container.querySelector('[role="status"]')!;
    expect(indicator.textContent).toContain("Connected");
    expect(indicator.querySelector(".bg-emerald-400")).not.toBeNull();
    expect(indicator.className).toContain("right-0");
    expect(container.textContent).not.toContain("Live");
    await act(async () => FakeEventSource.instances[0]!.onerror?.());
    expect(indicator.textContent).toContain("Disconnected");
    expect(indicator.querySelector(".bg-red-400")).not.toBeNull();
    expect(indicator.getAttribute("title")).toContain("Data may be stale");
  });
  it("loads older candles with an exclusive cursor and preserves the viewport", async () => {
    await render();
    mocks.history.mockResolvedValueOnce({ ok: true, data: { candles: [candle(0)], hasMore: false } });
    const chart = mocks.charts[0]!;
    await act(async () => chart.scroll(0));
    expect(mocks.history).toHaveBeenLastCalledWith({ data: { id: "fixture", coin: "BTC", interval: "1h", before: 3_600_000 } });
    expect(chart.price.setData).toHaveBeenLastCalledWith([candle(0), candle(3600), candle(7200)]);
    expect(chart.timeScale().setVisibleLogicalRange).toHaveBeenLastCalledWith({ from: 1, to: 121 });
    await act(async () => chart.scroll(0));
    expect(mocks.history).toHaveBeenCalledTimes(2);
    expect(container.textContent).toContain("No older candles");
  });
  it("remembers selections per agent, cleans up replaced charts, and ignores obsolete history", async () => {
    let resolve!: (value: unknown) => void;
    mocks.history.mockImplementationOnce(() => new Promise((done) => { resolve = done; }));
    await render();
    const oldChart = mocks.charts[0]!;
    const oldSource = FakeEventSource.instances[0]!;
    await change("Instrument", "ETH");
    expect(oldChart.remove).toHaveBeenCalledOnce();
    expect(oldSource.close).toHaveBeenCalledOnce();
    await act(async () => resolve({ ok: true, data: { candles: [candle(999)], hasMore: false } }));
    expect(oldChart.price.setData).not.toHaveBeenCalled();
    await change("Timeframe", "5m");
    expect(JSON.parse(localStorage.getItem("hypervibes:chart:fixture")!)).toEqual({ coin: "ETH", interval: "5m" });
    await render("other");
    expect(FakeEventSource.instances.at(-1)!.url).toContain("coin=BTC&interval=1h");
    await render();
    expect(FakeEventSource.instances.at(-1)!.url).toContain("coin=ETH&interval=5m");
    const latest = mocks.charts.at(-1)!;
    await act(async () => root.unmount()); root = createRoot(container);
    expect(latest.remove).toHaveBeenCalledOnce();
    expect(FakeEventSource.instances.at(-1)!.close).toHaveBeenCalledOnce();
  });
  it("recovers from invalid browser preferences and exposes a retry for history failures", async () => {
    localStorage.setItem("hypervibes:chart:fixture", JSON.stringify({ coin: "@1", interval: "nonsense" }));
    mocks.history.mockResolvedValueOnce({ ok: false, error: "History offline" });
    await render();
    expect(FakeEventSource.instances[0]!.url).toContain("coin=BTC&interval=1h");
    expect(container.querySelector("[role=alert]")!.textContent).toContain("History offline");
    await act(async () => container.querySelector<HTMLButtonElement>("button")!.click());
    expect(mocks.history).toHaveBeenCalledTimes(2);
    expect(container.querySelector("[role=alert]")).toBeNull();
  });
});
