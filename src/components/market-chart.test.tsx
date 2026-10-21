// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ChartCandle, ChartSnapshot } from "../hyperliquid/chart-contracts.js";
import { newDrawing, type ChartLayout, type DrawingKind } from "../charts/contracts.js";
import type { MarketChartHandle } from "./market-chart.js";

const mocks = vi.hoisted(() => ({ history: vi.fn(), charts: [] as ReturnType<typeof makeChart>[] }));
function makeChart() {
  let listener: ((range: { from: number; to: number }) => void) | undefined;
  const range = { from: 0, to: 120 };
  const timeScale = { setVisibleLogicalRange: vi.fn(), getVisibleLogicalRange: () => range,
    subscribeVisibleLogicalRangeChange: vi.fn((value) => { listener = value; }), unsubscribeVisibleLogicalRangeChange: vi.fn(), subscribeSizeChange: vi.fn(), unsubscribeSizeChange: vi.fn(), height: () => 30,
    logicalToCoordinate: (n: number) => Number.isInteger(n) ? n * 5 : 0, coordinateToLogical: (n: number) => Math.ceil(n / 5) };
  const series = () => ({ setData: vi.fn(), update: vi.fn(), applyOptions: vi.fn(), priceScale: () => ({ applyOptions: vi.fn() }), priceToCoordinate: (n: number) => 400 - n, coordinateToPrice: (n: number) => 400 - n });
  const price = series(), volume = series();
  return { addSeries: vi.fn().mockReturnValueOnce(price).mockReturnValueOnce(volume), timeScale: () => timeScale, price, volume,
    priceScale: () => ({ width: () => 50 }), chartElement: () => ({ clientWidth: 900, clientHeight: 500 }), subscribeCrosshairMove: vi.fn(), unsubscribeCrosshairMove: vi.fn(),
    remove: vi.fn(), scroll: (from: number) => listener?.({ from, to: from + 120 }) };
}
vi.mock("lightweight-charts", () => ({ CandlestickSeries: "candles", HistogramSeries: "volume", ColorType: { Solid: "solid" }, createChart: (host: HTMLElement) => {
  // Match the real library: positioned canvases with positive z-index values.
  for (const layer of [1, 2]) {
    const canvas = document.createElement("canvas");
    canvas.style.position = "absolute"; canvas.style.zIndex = String(layer); host.append(canvas);
  }
  const chart = makeChart(); chart.remove.mockImplementation(() => host.replaceChildren()); mocks.charts.push(chart); return chart;
} }));
vi.mock("../web/charts.js", () => ({ getChartHistory: mocks.history }));
import { MarketChart } from "./market-chart.js";
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const candle = (time: number, close = 11): ChartCandle => ({ time, open: 10, high: 15, low: 9, close, volume: 20 });
class FakeEventSource extends EventTarget {
  static instances: FakeEventSource[] = [];
  close = vi.fn(); onerror: (() => void) | null = null;
  constructor(readonly url: string) { super(); FakeEventSource.instances.push(this); }
  send(snapshot: ChartSnapshot) { this.dispatchEvent(new MessageEvent("chart", { data: JSON.stringify(snapshot) })); }
}
describe("annotated market chart", () => {
  let root: Root, container: HTMLDivElement, chartMount: HTMLDivElement, statusHost: HTMLDivElement, handle: MarketChartHandle | null;
  let layout: ChartLayout;
  beforeEach(() => {
    layout = { id: crypto.randomUUID(), revision: 0, document: { name: "Test", coin: "BTC", interval: "1h", drawings: [], notes: "", magnet: true, hideDrawings: false }, updatedAt: 0, canUndo: false, canRedo: false, lastChange: "Created" };
    mocks.history.mockReset().mockResolvedValue({ ok: true, data: { candles: [candle(3600), candle(7200)], hasMore: true } });
    mocks.charts.length = 0; FakeEventSource.instances = []; handle = null;
    vi.stubGlobal("EventSource", FakeEventSource);
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => window.setTimeout(() => callback(0), 0));
    vi.stubGlobal("cancelAnimationFrame", window.clearTimeout.bind(window));
    container = document.createElement("div"); statusHost = document.createElement("div"); chartMount = document.createElement("div");
    statusHost.setAttribute("role", "toolbar"); statusHost.setAttribute("aria-label", "Chart controls");
    container.append(statusHost, chartMount); document.body.append(container); root = createRoot(chartMount);
  });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.unstubAllGlobals(); });
  const render = async (tool: DrawingKind | "select" = "select", onSave = vi.fn()) => { await act(async () => { root.render(<MarketChart id="fixture" layout={layout} tool={tool} selected="" disabled={false} statusHost={statusHost} onSelect={() => {}} onSave={onSave} onDrawn={() => {}} onEdit={() => {}} ref={(value) => { handle = value; }} />); await new Promise((resolve) => setTimeout(resolve, 5)); }); };
  it("renders connection and history status in the toolbar without an extra row above the chart", async () => {
    await render();
    const indicator = statusHost.querySelector<HTMLElement>("[aria-label='Chart connection']")!;
    expect(indicator.textContent).toContain("Connecting…");
    expect(chartMount.querySelector("[aria-label='Chart connection']")).toBeNull();
    expect(chartMount.firstElementChild!.children).toHaveLength(1);
    expect(chartMount.firstElementChild!.firstElementChild!.getAttribute("aria-label")).toBe("BTC 1h candlestick and volume chart");
    await act(async () => FakeEventSource.instances[0]!.send({ status: "live", candles: [] }));
    expect(indicator.textContent).toContain("Connected");
    mocks.history.mockResolvedValueOnce({ ok: true, data: { candles: [candle(0)], hasMore: false } });
    await act(async () => mocks.charts[0]!.scroll(0));
    expect(indicator.title).toContain("No older candles available");
    await act(async () => FakeEventSource.instances[0]!.onerror?.());
    expect(indicator.textContent).toContain("Disconnected"); expect(indicator.title).toContain("Data may be stale");
    expect(FakeEventSource.instances).toHaveLength(1); expect(mocks.history).toHaveBeenCalledTimes(2);
  });
  it("keeps saved agent annotations above the library's canvas layers", async () => {
    const level = { ...newDrawing("horizontal", "BTC", [{ time: 3600, price: 12 }]), author: "agent" as const, name: "Agent support" };
    layout = { ...layout, revision: 1, document: { ...layout.document, drawings: [level] } };
    await render();
    const canvas = container.querySelector("canvas")!, host = canvas.parentElement!;
    const overlay = container.querySelector<SVGSVGElement>("svg[aria-label='Chart drawings']")!;
    expect(canvas.style.zIndex).toBe("1");
    // The host must isolate ALL library descendants below the SVG, not just rely on DOM order.
    expect(host.classList.contains("z-0")).toBe(true);
    expect(overlay.classList.contains("z-10")).toBe(true);
    expect(overlay.querySelector("g[aria-label='Agent support'] path")).not.toBeNull();
    expect(overlay.style.pointerEvents).toBe("none");
    expect(overlay.querySelector<SVGPathElement>("path[stroke='transparent']")!.style.pointerEvents).toBe("stroke");
  });
  it("provides a hit surface for drawing on empty chart space and renders the saved result", async () => {
    const save = vi.fn(); await render("horizontal", save);
    const surface = container.querySelector<SVGRectElement>("[data-drawing-surface]");
    expect(surface).not.toBeNull();
    expect(surface!.getAttribute("fill")).toBe("transparent");
    expect(surface!.style.pointerEvents).toBe("all");
    await act(async () => surface!.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true, button: 0, clientX: 5, clientY: 389 })));
    expect(save).toHaveBeenCalledWith(expect.objectContaining({ coin: "BTC", kind: "horizontal", points: [{ time: 7200, price: 11 }] }));
    const level = save.mock.calls[0]![0];
    layout = { ...layout, revision: 1, document: { ...layout.document, drawings: [level] } };
    await render();
    expect(container.querySelector("g[aria-label='horizontal'] path")).not.toBeNull();
    expect(container.querySelector("[data-drawing-surface]")).toBeNull();
  });
  it("interpolates saved anchors between candles instead of passing fractional indices to Lightweight Charts", async () => {
    const level = newDrawing("trend", "BTC", [{ time: 1800, price: 12 }, { time: 9000, price: 13 }]);
    layout = { ...layout, document: { ...layout.document, drawings: [level] } };
    await render();
    expect(container.querySelector("g[aria-label='trend'] path")!.getAttribute("d")).toBe("M -2.5 388 L 7.5 387");
  });
  it("preserves intraday anchor positions when displayed on a coarser timeframe", async () => {
    mocks.history.mockResolvedValue({ ok: true, data: { candles: [candle(0), candle(14400)], hasMore: false } });
    const level = newDrawing("trend", "BTC", [{ time: 3600, price: 12 }, { time: 10800, price: 13 }]);
    layout = { ...layout, document: { ...layout.document, interval: "4h", drawings: [level] } };
    await render();
    expect(container.querySelector("g[aria-label='trend'] path")!.getAttribute("d")).toBe("M 1.25 388 L 3.75 387");
  });
  it("places unsnapped anchors at the pointer rather than rounding up to a candle", async () => {
    layout = { ...layout, document: { ...layout.document, magnet: false } };
    const save = vi.fn(); await render("horizontal", save);
    await act(async () => container.querySelector("[data-drawing-surface]")!.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true, button: 0, clientX: 2.5, clientY: 389 })));
    expect(save).toHaveBeenCalledWith(expect.objectContaining({ points: [{ time: 5400, price: 11 }] }));
  });
  it("draws candles/volume, streams updates, and exposes a frozen viewport capture", async () => {
    await render();
    expect(FakeEventSource.instances[0]!.url).toBe("/api/agents/fixture/chart/events?coin=BTC&interval=1h");
    const chart = mocks.charts[0]!;
    expect(chart.price.setData).toHaveBeenLastCalledWith([candle(3600), candle(7200)]);
    await act(async () => FakeEventSource.instances[0]!.send({ status: "live", candles: [candle(7200, 14)] }));
    expect(chart.price.update).toHaveBeenLastCalledWith(candle(7200, 14), true);
    expect(container.textContent).toContain("Connected");
    const capture = await handle!.capture(layout, "", false);
    expect(capture).toMatchObject({ layoutId: layout.id, revision: 0, status: "live", candles: [candle(3600), candle(7200, 14)] });
    await act(async () => FakeEventSource.instances[0]!.send({ status: "live", candles: [candle(7200, 12)] }));
    expect(capture.candles.at(-1)!.close).toBe(14);
    await act(async () => FakeEventSource.instances[0]!.onerror?.());
    expect(container.textContent).toContain("Disconnected");
  });
  it("loads older candles and preserves viewport; ignores obsolete history after selection changes", async () => {
    await render();
    mocks.history.mockResolvedValueOnce({ ok: true, data: { candles: [candle(0)], hasMore: false } });
    const chart = mocks.charts[0]!;
    await act(async () => chart.scroll(0));
    expect(mocks.history).toHaveBeenLastCalledWith({ data: { id: "fixture", coin: "BTC", interval: "1h", before: 3_600_000 } });
    expect(chart.timeScale().setVisibleLogicalRange).toHaveBeenLastCalledWith({ from: 1, to: 121 });
    let resolve!: (value: unknown) => void;
    mocks.history.mockImplementationOnce(() => new Promise((done) => { resolve = done; }));
    layout = { ...layout, document: { ...layout.document, coin: "ETH" } }; await render();
    const obsolete = mocks.charts[1]!;
    layout = { ...layout, document: { ...layout.document, interval: "5m" } }; await render();
    await act(async () => FakeEventSource.instances[1]!.onerror?.());
    expect(statusHost.querySelector("[aria-label='Chart connection']")!.textContent).toContain("Connecting…");
    await act(async () => resolve({ ok: true, data: { candles: [candle(999)], hasMore: false } }));
    expect(obsolete.price.setData).not.toHaveBeenCalled();
    expect(obsolete.remove).toHaveBeenCalledOnce();
    expect(FakeEventSource.instances[1]!.close).toHaveBeenCalledOnce();
  });
  it("offers retry on history failure and cleans up subscriptions", async () => {
    mocks.history.mockResolvedValueOnce({ ok: false, error: "History offline" }); await render();
    expect(container.querySelector("[role=alert]")!.textContent).toContain("History offline");
    await act(async () => container.querySelector<HTMLButtonElement>("button")!.click());
    expect(mocks.history).toHaveBeenCalledTimes(2);
    expect(mocks.charts[0]!.remove).toHaveBeenCalledOnce();
    expect(mocks.charts[0]!.unsubscribeCrosshairMove).toHaveBeenCalledOnce();
    expect(container.querySelector("[role=alert]")).toBeNull();
  });
});
