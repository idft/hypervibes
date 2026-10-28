// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { type QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createTestQueryClient } from "../test/query.js";
import type { AgentDashboard } from "../hyperliquid/agent-dashboard.js";
import { defaultChartOverlays, type ChartOverlayVisibility } from "../charts/trading-contracts.js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ChartCandle, ChartSnapshot } from "../hyperliquid/chart-contracts.js";
import { newDrawing, type ChartLayout, type DrawingKind } from "../charts/contracts.js";
import type { MarketChartHandle } from "./market-chart.js";

const mocks = vi.hoisted(() => ({ history: vi.fn(), fills: vi.fn(), charts: [] as ReturnType<typeof makeChart>[], plugins: [] as { setMarkers: ReturnType<typeof vi.fn>; detach: ReturnType<typeof vi.fn> }[] }));
function makeChart() {
  let listener: ((range: { from: number; to: number }) => void) | undefined;
  const range = { from: 0, to: 120 };
  const timeScale = { setVisibleLogicalRange: vi.fn(), getVisibleLogicalRange: () => range,
    subscribeVisibleLogicalRangeChange: vi.fn((value) => { listener = value; }), unsubscribeVisibleLogicalRangeChange: vi.fn(), subscribeSizeChange: vi.fn(), unsubscribeSizeChange: vi.fn(), height: () => 30,
    logicalToCoordinate: (n: number) => Number.isInteger(n) ? n * 5 : 0, coordinateToLogical: (n: number) => Math.ceil(n / 5) };
  const series = () => ({ setData: vi.fn(), update: vi.fn(), applyOptions: vi.fn(), createPriceLine: vi.fn(() => ({ applyOptions: vi.fn() })), removePriceLine: vi.fn(), priceScale: () => ({ applyOptions: vi.fn() }), priceToCoordinate: (n: number) => 400 - n, coordinateToPrice: (n: number) => 400 - n });
  const price = series(), volume = series();
  type Hover = { hoveredInfo?: { objectId: string; type: "marker" }; point?: { x: number; y: number } };
  const crosshairs = new Set<(event: Hover) => void>();
  return { addSeries: vi.fn().mockReturnValueOnce(price).mockReturnValueOnce(volume), timeScale: () => timeScale, price, volume,
    panes: () => [{ getHeight: () => 470 }], priceScale: () => ({ width: () => 50 }), chartElement: () => ({ clientWidth: 900, clientHeight: 500 }), subscribeCrosshairMove: vi.fn((callback) => crosshairs.add(callback)), unsubscribeCrosshairMove: vi.fn((callback) => crosshairs.delete(callback)),
    hover: (event: Hover) => crosshairs.forEach((callback) => callback(event)),
    remove: vi.fn(), scroll: (from: number) => listener?.({ from, to: from + 120 }) };
}
vi.mock("lightweight-charts", () => ({ LineStyle: { Solid: 0, Dotted: 1, Dashed: 2 }, createSeriesMarkers: () => { const plugin = { setMarkers: vi.fn(), detach: vi.fn() }; mocks.plugins.push(plugin); return plugin; }, CandlestickSeries: "candles", HistogramSeries: "volume", ColorType: { Solid: "solid" }, createChart: (host: HTMLElement) => {
  // Match the real library: positioned canvases with positive z-index values.
  for (const layer of [1, 2]) {
    const canvas = document.createElement("canvas");
    canvas.style.position = "absolute"; canvas.style.zIndex = String(layer); host.append(canvas);
  }
  const chart = makeChart(); chart.remove.mockImplementation(() => host.replaceChildren()); mocks.charts.push(chart); return chart;
} }));
vi.mock("../web/charts.js", () => ({ getChartHistory: mocks.history, getChartFills: mocks.fills }));
import { MarketChart } from "./market-chart.js";
import { AgentEventsProvider } from "./agent-events.js";
const candle = (time: number, close = 11): ChartCandle => ({ time, open: 10, high: 15, low: 9, close, volume: 20 });
class FakeEventSource extends EventTarget {
  static instances: FakeEventSource[] = [];
  close = vi.fn(); onerror: (() => void) | null = null;
  constructor(readonly url: string) { super(); FakeEventSource.instances.push(this); }
  send(snapshot: ChartSnapshot) { this.dispatchEvent(new MessageEvent("chart", { data: JSON.stringify(snapshot) })); }
  dashboard(snapshot: AgentDashboard) { this.dispatchEvent(new MessageEvent("dashboard", { data: JSON.stringify(snapshot) })); }
}
const chartSources = () => FakeEventSource.instances.filter((source) => source.url.includes("/stream"));
const stream = () => FakeEventSource.instances.at(-1)!;
const streamParams = (url: string) => new URL(url, "http://localhost").searchParams;
const sendChart = (snapshot: ChartSnapshot) => stream().send(snapshot);
const sendDashboard = (snapshot: AgentDashboard) => stream().dashboard(snapshot);
const snapshot = (): AgentDashboard => ({ address: "0xaccount", accountAbstraction: "disabled", fetchedAt: 0, perpEquity: "100", spotUsdc: "0", errors: [],
  positions: [{ coin: "BTC", size: "1", entryPrice: "11", unrealizedPnl: "2", liquidationPrice: null, leverage: 2 }],
  orders: [{ id: 1, coin: "BTC", side: "Buy", type: "Limit", price: "10", size: "0.5" }, { id: 2, coin: "BTC", side: "Sell", type: "Stop Market", price: "6", triggerPrice: "9", size: "1" }],
  live: { status: "connected", sections: { perps: { fresh: true, updatedAt: 1 }, spot: { fresh: true, updatedAt: 1 }, orders: { fresh: true, updatedAt: 1 }, mode: { fresh: true, updatedAt: 1 } } },
});
describe("annotated market chart", () => {
  let root: Root, container: HTMLDivElement, chartMount: HTMLDivElement, statusHost: HTMLDivElement, handle: MarketChartHandle | null;
  let layout: ChartLayout, client: QueryClient;
  beforeEach(() => {
    layout = { id: crypto.randomUUID(), revision: 0, document: { name: "Test", coin: "BTC", interval: "1h", drawings: [], indicators: [], notes: "", magnet: true, hideDrawings: false }, updatedAt: 0, canUndo: false, canRedo: false, lastChange: "Created" };
    mocks.history.mockReset().mockResolvedValue({ candles: [candle(3600), candle(7200)], hasMore: true });
    mocks.fills.mockReset().mockResolvedValue({ fills: [], nextCursor: null, historyLimited: false, sync: { updatedAt: 1, error: null } });
    client = createTestQueryClient();
    mocks.charts.length = 0; mocks.plugins.length = 0; FakeEventSource.instances = []; handle = null;
    vi.stubGlobal("EventSource", FakeEventSource);
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => window.setTimeout(() => callback(0), 0));
    vi.stubGlobal("cancelAnimationFrame", window.clearTimeout.bind(window));
    container = document.createElement("div"); statusHost = document.createElement("div"); chartMount = document.createElement("div");
    statusHost.setAttribute("role", "toolbar"); statusHost.setAttribute("aria-label", "Chart controls");
    container.append(statusHost, chartMount); document.body.append(container); root = createRoot(chartMount);
  });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.unstubAllGlobals(); });
  const render = async (tool: DrawingKind | "select" = "select", onSave = vi.fn(), visibility: ChartOverlayVisibility = defaultChartOverlays) => { await act(async () => { root.render(<QueryClientProvider client={client}><AgentEventsProvider agentId="fixture"><MarketChart id="fixture" layout={layout} tool={tool} selected="" disabled={false} statusHost={statusHost} overlayVisibility={visibility} onSelect={() => {}} onSave={onSave} onDrawn={() => {}} onEdit={() => {}} ref={(value) => { handle = value; }} /></AgentEventsProvider></QueryClientProvider>); await new Promise((resolve) => setTimeout(resolve, 5)); }); await act(async () => {}); };
  it("renders connection and history status in the toolbar without an extra row above the chart", async () => {
    await render();
    const indicator = statusHost.querySelector<HTMLElement>("[aria-label='Chart connection']")!;
    expect(indicator.textContent).toContain("Loading");
    expect(chartMount.querySelector("[aria-label='Chart connection']")).toBeNull();
    expect(chartMount.querySelector("[aria-label='Account overlay status']")).toBeNull();
    const details = statusHost.querySelector("details")!;
    expect(details.open).toBe(false);
    expect(statusHost.querySelector("[aria-label='Chart data status']")?.textContent).toContain("Connecting account");
    await act(async () => statusHost.querySelector("summary")!.click());
    expect(details.open).toBe(true);
    expect(chartMount.firstElementChild!.children).toHaveLength(1);
    expect(chartMount.firstElementChild!.firstElementChild!.getAttribute("aria-label")).toBe("BTC 1h candlestick and volume chart");
    await act(async () => sendChart({ status: "live", candles: [] }));
    expect(indicator.textContent).toContain("Loading");
    expect(indicator.querySelector(".bg-amber-400")).not.toBeNull();
    await act(async () => sendDashboard(snapshot()));
    expect(indicator.textContent).toContain("Connected");
    expect(indicator.querySelector(".bg-emerald-400")).not.toBeNull();
    mocks.history.mockResolvedValueOnce({ candles: [candle(0)], hasMore: false });
    await act(async () => mocks.charts[0]!.scroll(0));
    expect(indicator.title).toContain("No older candles available");
    await act(async () => stream().onerror?.());
    expect(indicator.textContent).toContain("Disconnected"); expect(indicator.title).toContain("Data may be stale");
    expect(indicator.querySelector(".bg-red-400")).not.toBeNull();
    expect(chartSources()).toHaveLength(1); expect(FakeEventSource.instances).toHaveLength(1); expect(mocks.history).toHaveBeenCalledTimes(2);
  });
  it("keeps history errors and retry actions in the toolbar status panel", async () => {
    mocks.history.mockRejectedValue(new Error("History request failed"));
    await render();
    expect(chartMount.textContent).not.toContain("History request failed");
    const panel = statusHost.querySelector("[aria-label='Chart data status']")!;
    expect(panel.textContent).toContain("History request failed");
    const retry = [...panel.querySelectorAll("button")].find((button) => button.textContent === "Retry chart")!;
    expect(retry).toBeDefined();
    mocks.history.mockResolvedValue({ candles: [candle(3600)], hasMore: true });
    await act(async () => { retry.click(); await new Promise((resolve) => setTimeout(resolve, 10)); });
    expect(statusHost.textContent).not.toContain("History request failed");
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
    mocks.history.mockResolvedValue({ candles: [candle(0), candle(14400)], hasMore: false });
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
    const params = streamParams(FakeEventSource.instances[0]!.url);
    expect(params.get("topics")).toBe("chart,dashboard");
    expect(params.get("coin")).toBe("BTC"); expect(params.get("interval")).toBe("1h");
    const chart = mocks.charts[0]!;
    expect(chart.price.setData).toHaveBeenLastCalledWith([candle(3600), candle(7200)]);
    await act(async () => sendChart({ status: "live", candles: [candle(7200, 14)] }));
    expect(chart.price.update).toHaveBeenLastCalledWith(candle(7200, 14), true);
    expect(container.textContent).toContain("Connected");
    const capture = await handle!.capture(layout, "", false);
    expect(capture).toMatchObject({ layoutId: layout.id, revision: 0, status: "live", candles: [candle(3600), candle(7200, 14)] });
    await act(async () => sendChart({ status: "live", candles: [candle(7200, 12)] }));
    expect(capture.candles.at(-1)!.close).toBe(14);
    await act(async () => stream().onerror?.());
    expect(container.textContent).toContain("Disconnected");
  });
  it("loads older candles and preserves viewport; ignores obsolete history after selection changes", async () => {
    await render();
    mocks.history.mockResolvedValueOnce({ candles: [candle(0)], hasMore: false });
    const chart = mocks.charts[0]!;
    await act(async () => chart.scroll(0));
    expect(mocks.history).toHaveBeenLastCalledWith({ data: { id: "fixture", coin: "BTC", interval: "1h", before: 3_600_000 }, signal: expect.any(AbortSignal) });
    expect(chart.timeScale().setVisibleLogicalRange).toHaveBeenLastCalledWith({ from: 1, to: 121 });
    let resolve!: (value: unknown) => void;
    mocks.history.mockImplementationOnce(() => new Promise((done) => { resolve = done; }));
    layout = { ...layout, document: { ...layout.document, coin: "ETH" } }; await render();
    const obsolete = mocks.charts[1]!;
    layout = { ...layout, document: { ...layout.document, interval: "5m" } }; await render();
    await act(async () => chartSources()[1]!.onerror?.());
    expect(statusHost.querySelector("[aria-label='Chart connection']")!.textContent).toContain("Loading");
    await act(async () => resolve({ candles: [candle(999)], hasMore: false }));
    expect(obsolete.price.setData).not.toHaveBeenCalled();
    expect(obsolete.remove).toHaveBeenCalledOnce();
    expect(chartSources()[1]!.close).toHaveBeenCalledOnce();
  });
  it("streams position and order lines, marks stale sections and removes closed positions and canceled orders", async () => {
    await render();
    const chart = mocks.charts[0]!, current = snapshot();
    expect(streamParams(stream().url).get("topics")).toBe("chart,dashboard");
    await act(async () => sendDashboard(current));
    expect(chart.price.createPriceLine).toHaveBeenCalledTimes(3);
    expect(chart.price.createPriceLine).toHaveBeenCalledWith(expect.objectContaining({ price: 11, title: "Long · 1 BTC · P&L +2.0000 USDC" }));
    expect(chart.price.createPriceLine).toHaveBeenCalledWith(expect.objectContaining({ price: 9, title: expect.stringContaining("Sell Stop Market trigger") }));
    expect(container.textContent).not.toContain("Connecting account");
    const lines = chart.price.createPriceLine.mock.results.map((r) => r.value);
    await act(async () => stream().onerror?.());
    expect(container.textContent).toContain("Position data stale"); expect(container.textContent).toContain("Order data stale");
    expect(lines[0]!.applyOptions).toHaveBeenLastCalledWith(expect.objectContaining({ title: expect.stringContaining("STALE") }));
    await act(async () => sendDashboard({ ...current, positions: [], orders: [] }));
    expect(chart.price.removePriceLine).toHaveBeenCalledTimes(3);
    expect(container.textContent).not.toContain("data stale");
    // Reading live account data never edits the layout or annotations.
    expect(layout.revision).toBe(0); expect(layout.document.drawings).toEqual([]);
  });
  it("loads all fill pages, shows exact hover details and hides overlays independently", async () => {
    const first = { id: "a".repeat(64), time: 3_600_001, side: "Buy", size: "0.10000001", price: "10.12345678" };
    const second = { id: "b".repeat(64), time: 7_200_001, side: "Sell", size: "0.05", price: "12" };
    mocks.fills.mockImplementation(async ({ data }) => ({ fills: data.cursor ? [first] : [second], nextCursor: data.cursor ? null : { time: second.time, id: second.id }, historyLimited: true, sync: { updatedAt: 1, error: null } }));
    await render();
    for (let n = 0; n < 5; n++) await act(async () => { await new Promise((resolve) => setTimeout(resolve, 20)); });
    expect(mocks.fills).toHaveBeenCalledWith(expect.objectContaining({ data: expect.objectContaining({ id: "fixture", coin: "BTC", from: 3_600_000, to: 10_800_000, cursor: { time: second.time, id: second.id } }) }));
    expect(mocks.plugins[0]!.setMarkers).toHaveBeenLastCalledWith([expect.objectContaining({ id: first.id, time: 3600, shape: "arrowUp" }), expect.objectContaining({ id: second.id, time: 7200, shape: "arrowDown" })]);
    expect(container.textContent).toContain("Exchange history is limited");
    await act(async () => mocks.charts[0]!.hover({ hoveredInfo: { type: "marker", objectId: first.id }, point: { x: 5, y: 300 } }));
    expect(container.querySelector("[role=tooltip]")!.textContent).toContain("Buy · 0.10000001 BTC @ 10.12345678");
    expect(container.querySelector("time")!.dateTime).toBe(new Date(first.time).toISOString());
    await act(async () => sendDashboard(snapshot()));
    await render("select", vi.fn(), { trades: false, position: false, orders: true });
    expect(mocks.plugins[0]!.setMarkers).toHaveBeenLastCalledWith([]);
    expect(container.querySelector("[role=tooltip]")).toBeNull();
    expect(mocks.charts[0]!.price.removePriceLine).toHaveBeenCalledTimes(1);
    expect(stream().close).not.toHaveBeenCalled();
    await render("select", vi.fn(), { trades: false, position: false, orders: false });
    expect(FakeEventSource.instances).toHaveLength(2);
    expect(FakeEventSource.instances[0]!.close).toHaveBeenCalledOnce();
    expect(mocks.charts[0]!.price.removePriceLine).toHaveBeenCalledTimes(3);
    // The chart subscription survives the dashboard unsubscribe on the reopened stream.
    await act(async () => sendChart({ status: "live", candles: [candle(7200, 12)] }));
    expect(mocks.charts[0]!.price.update).toHaveBeenCalled();
  });
  it("offers retry on history failure and cleans up subscriptions", async () => {
    mocks.history.mockRejectedValueOnce(new Error("History offline")); await render();
    expect(statusHost.querySelector("[aria-label='Chart data status']")!.textContent).toContain("History offline");
    await act(async () => [...statusHost.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Retry chart")!.click());
    expect(mocks.history).toHaveBeenCalledTimes(2);
    expect(mocks.charts[0]!.remove).toHaveBeenCalledOnce();
    // redraw + account overlays crosshair + read-only generated-object hover.
    expect(mocks.charts[0]!.unsubscribeCrosshairMove).toHaveBeenCalledTimes(3);
    expect(mocks.plugins[0]!.detach).toHaveBeenCalledOnce();
    expect(mocks.plugins[0]!.detach.mock.invocationCallOrder[0]).toBeLessThan(mocks.charts[0]!.remove.mock.invocationCallOrder[0]!);
    expect(container.querySelector("[role=alert]")).toBeNull();
  });
});
