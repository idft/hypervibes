import { beforeEach, describe, expect, it, vi } from "vitest";
import { LineStyle } from "lightweight-charts";
import { createIndicatorRenderer } from "./indicator-renderer.js";
import type { ChartIndicatorInstance, IndicatorResult } from "../indicators/contracts.js";

const mocks = vi.hoisted(() => ({
  panes: [] as { index: number; height: number; series: unknown[] }[],
  series: [] as { kind: string; pane: number; options: unknown; data: unknown[]; primitive?: unknown; detached: boolean }[],
  makeSeries: null as null | ((kind: string, pane: number) => unknown),
}));
vi.mock("lightweight-charts", () => {
  const makeSeries = (kind: string, pane: number) => {
    const series = { kind, pane, options: {}, data: [] as unknown[], detached: false, attachPrimitive: vi.fn(function (this: unknown, primitive: unknown) { (series as { primitive?: unknown }).primitive = primitive; }), detachPrimitive: vi.fn(function (this: unknown) { series.detached = true; }), setData: vi.fn((data: unknown[]) => { series.data = data; }), applyOptions: vi.fn() };
    mocks.series.push(series);
    return series;
  };
  mocks.makeSeries = makeSeries;
  return { LineStyle: { Solid: 0, Dotted: 1, Dashed: 2 }, LineSeries: "line", HistogramSeries: "histogram", createChart: () => ({}), CandlestickSeries: "candles" };
});
function chartDouble() {
  const removed: unknown[] = [];
  const makeSeries = mocks.makeSeries!;
  const chart = {
    addSeries: vi.fn((kind: string, options: unknown, paneIndex = 0) => { const pane = mocks.panes.find((p) => p.index === paneIndex)!; const series = makeSeries(kind, paneIndex) as { options: unknown }; series.options = options; pane.series.push(series); return series; }),
    addPane: vi.fn(() => { const pane = { index: mocks.panes.length, height: 200, series: [] as unknown[] }; mocks.panes.push(pane); return { setHeight: (height: number) => { pane.height = height; }, paneIndex: () => pane.index, getHeight: () => pane.height, getSeries: () => pane.series }; }),
    removeSeries: vi.fn((series: unknown) => { removed.push(series); const pane = mocks.panes.find((p) => p.series.includes(series)); if (pane) pane.series = pane.series.filter((item) => item !== series); }),
    removePane: vi.fn(),
    panes: () => mocks.panes.map((pane) => ({ getHeight: () => pane.height })),
    timeScale: () => ({ getVisibleRange: () => ({ from: 0, to: 10 }), setVisibleRange: vi.fn() }),
  };
  return { chart, removed };
}
const priceSeries = () => ({ options: () => ({ priceScaleId: "right" }) });
const candles = [0, 1, 2, 3].map((index) => ({ time: index * 3600, open: 100 + index, high: 102 + index, low: 99 + index, close: 101 + index, volume: 10 }));
const instance = (overrides: Partial<ChartIndicatorInstance> = {}): ChartIndicatorInstance => ({ id: "instance-1", indicatorId: "builtin:ema", inputs: {}, configuredVersion: 1, hidden: false, pane: "auto", styles: {}, ...overrides });
const point = (time: number, value: number | null, displayTime = time) => ({ time, displayTime, value, color: "#a78bfa" });
const result = (overrides: Partial<IndicatorResult> = {}): IndicatorResult => ({
  schemaVersion: 1, indicatorId: "builtin:ema", builtIn: true, version: 1, sourceHash: "a".repeat(64), engine: { name: "pinets", version: "0.12.1" }, wrapperVersion: 1,
  coin: "BTC", interval: "1h", start: 0, end: 4 * 3600_000, inputSchemaHash: "b".repeat(64), computedAt: 0, candleHash: "c".repeat(64), resultHash: "d".repeat(64),
  dataset: { count: 4, firstOpenTime: 0, lastOpenTime: 10800 }, outputDataset: { count: 4, firstTime: 0, lastTime: 10800 }, coverage: { complete: true, closedOnly: true, expected: 4, returned: 4, seedCoverage: "complete" },
  title: "EMA", overlay: true, inputs: [], resolvedInputs: {},
  outputs: [{ id: "EMA", title: "EMA", kind: "line", overlay: false, forceOverlay: false, style: { color: "#a78bfa", width: 2, hidden: false, offset: 0 }, finiteCount: 2, firstFiniteTime: 7200, points: [point(0, null), point(3600, null), point(7200, 3), point(10800, 4)] }],
  levels: [], fills: [], markers: [], drawings: [], values: candles.map((c, i) => ({ time: c.time, value: { EMA: [null, null, 3, 4][i] ?? null } })), warnings: [], warningsOmitted: 0, ...overrides,
});
beforeEach(() => { mocks.panes.length = 0; mocks.series.length = 0; mocks.panes.push({ index: 0, height: 470, series: [] }); });
describe("owned chart indicator renderer", () => {
  it("renders a separate oscillator pane with null gaps and per-point colors", () => {
    const { chart } = chartDouble();
    const renderer = createIndicatorRenderer(chart as unknown as Parameters<typeof createIndicatorRenderer>[0], priceSeries() as unknown as Parameters<typeof createIndicatorRenderer>[1]);
    renderer.apply(instance(), result(), candles);
    // A separate-pane indicator creates exactly one owned pane after the price pane.
    expect(mocks.panes).toHaveLength(2); expect(mocks.panes[1]).toMatchObject({ index: 1, height: 150 });
    const output = mocks.series.find((series) => series.kind === "line")!;
    expect(output.pane).toBe(1);
    expect(output.data).toEqual([{ time: 0 }, { time: 3600 }, { time: 7200, value: 3, color: "#a78bfa" }, { time: 10800, value: 4, color: "#a78bfa" }]);
    // One invisible anchor series owns the pane primitives; candles are never touched.
    expect(mocks.series.filter((series) => series.primitive)).toHaveLength(1);
    expect(mocks.series.some((series) => series.kind === "candles")).toBe(false);
    renderer.close();
  });
  it("keeps overlays on the price pane and applies permitted style overrides", () => {
    const { chart } = chartDouble();
    const renderer = createIndicatorRenderer(chart as unknown as Parameters<typeof createIndicatorRenderer>[0], priceSeries() as unknown as Parameters<typeof createIndicatorRenderer>[1]);
    renderer.apply(instance({ pane: "price", styles: { EMA: { color: "#ff9900", width: 1, hidden: false } } }), result(), candles);
    const output = mocks.series.find((series) => series.kind === "line")!;
    expect(output.pane).toBe(0); expect(mocks.panes).toHaveLength(1);
    expect(output.options).toMatchObject({ color: "#ff9900", lineWidth: 1, lineStyle: LineStyle.Solid });
    renderer.close();
  });
  it("removes only owned resources on removal, repeated apply and close", () => {
    const { chart, removed } = chartDouble();
    const renderer = createIndicatorRenderer(chart as unknown as Parameters<typeof createIndicatorRenderer>[0], priceSeries() as unknown as Parameters<typeof createIndicatorRenderer>[1]);
    const value = result();
    renderer.apply(instance(), value, candles);
    const owned = mocks.series.length;
    renderer.apply(instance(), value, candles); // Same identity: no duplicate series.
    expect(mocks.series).toHaveLength(owned); expect(removed).toHaveLength(0);
    renderer.apply(instance(), { ...value, resultHash: "e".repeat(64) }, candles);
    expect(removed).toHaveLength(owned); // The old result's owned resources were released.
    renderer.remove("instance-1");
    expect(removed).toHaveLength(mocks.series.length);
    renderer.remove("instance-1"); // Idempotent, and never removes another instance's work.
    expect(removed).toHaveLength(mocks.series.length);
    renderer.apply(instance({ hidden: true }), value, candles); // Hidden: nothing rendered at all.
    expect(removed).toHaveLength(mocks.series.length);
    renderer.close();
    expect(chart.removePane).not.toHaveBeenCalled();
  });
  it("uses histogram series for columns and keeps hidden outputs analytical but unrendered", () => {
    const { chart } = chartDouble();
    const renderer = createIndicatorRenderer(chart as unknown as Parameters<typeof createIndicatorRenderer>[0], priceSeries() as unknown as Parameters<typeof createIndicatorRenderer>[1]);
    const histogram: IndicatorResult = result({ outputs: [{ id: "Histogram", title: "Histogram", kind: "histogram", overlay: false, forceOverlay: false, style: { color: "#34d399", hidden: false, offset: 0 }, finiteCount: 2, firstFiniteTime: 7200, points: [point(7200, 1), point(10800, -1)] }] });
    renderer.apply(instance(), histogram, candles);
    expect(mocks.series.filter((series) => series.kind === "histogram")).toHaveLength(1);
    renderer.close();
    // A fresh chart: the hidden-output case must not reuse the histogram series.
    mocks.series.length = 0; mocks.panes.length = 0; mocks.panes.push({ index: 0, height: 470, series: [] });
    renderer.apply(instance(), result({ outputs: [{ id: "Hidden", title: "Hidden", kind: "line", overlay: false, forceOverlay: false, style: { hidden: true, offset: 0 }, finiteCount: 0, firstFiniteTime: null, points: [] }] }), candles);
    // display.none remains analytical data but renders no plot series: only the
    // pane anchor that hosts primitives carries any data.
    const withData = mocks.series.filter((series) => (series.data as unknown[]).length > 0);
    expect(withData).toHaveLength(1); expect(withData[0]!.primitive).toBeDefined();
    renderer.close();
  });
  it("detaches every primitive it attached", () => {
    const { chart } = chartDouble();
    const renderer = createIndicatorRenderer(chart as unknown as Parameters<typeof createIndicatorRenderer>[0], priceSeries() as unknown as Parameters<typeof createIndicatorRenderer>[1]);
    renderer.apply(instance(), result(), candles);
    const attached = mocks.series.filter((series) => series.primitive);
    expect(attached).toHaveLength(1);
    renderer.close();
    expect(attached.every((series) => series.detached)).toBe(true);
  });
});
