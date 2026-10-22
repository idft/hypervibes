import { LineSeries, HistogramSeries, LineStyle, type IChartApi, type ISeriesApi, type IPaneApi, type Time, type UTCTimestamp, type LineWidth } from "lightweight-charts";
import type { ChartCandle } from "../hyperliquid/chart-contracts.js";
import type { ChartIndicatorInstance, IndicatorResult } from "../indicators/contracts.js";
import { IndicatorPrimitive } from "./indicator-primitives.js";
interface Owned { signature: string; series: ISeriesApi<"Line" | "Histogram">[]; anchors: { series: ISeriesApi<"Line">; primitive: IndicatorPrimitive }[]; pane?: IPaneApi<Time> }
/** Owns only indicator resources, never candles/volume/account overlays or annotation records. */
export function createIndicatorRenderer(chart: IChartApi, price: ISeriesApi<"Candlestick">) {
  const owned = new Map<string, Owned>();
  const remove = (id: string) => {
    const current = owned.get(id); if (!current) return;
    for (const { series, primitive } of current.anchors) series.detachPrimitive(primitive);
    for (const series of [...current.series, ...current.anchors.map((a) => a.series)]) chart.removeSeries(series);
    // Empty panes disappear automatically. Never remove an indexed pane owned by someone else.
    owned.delete(id);
  };
  return {
    apply(instance: ChartIndicatorInstance, result: IndicatorResult, candles: ChartCandle[]) {
      const signature = JSON.stringify([instance, result.resultHash, candles[0]?.time, candles.at(-1)?.time, candles.length]);
      if (owned.get(instance.id)?.signature === signature) return;
      const range = chart.timeScale().getVisibleRange();
      remove(instance.id);
      if (instance.hidden) return;
      const state: Owned = { signature, series: [], anchors: [] }; owned.set(instance.id, state);
      try {
        const target = (item: { forceOverlay: boolean; overlay: boolean }) => item.forceOverlay || (instance.pane === "auto" ? item.overlay : instance.pane === "price");
        const paneIndex = (overlay: boolean) => {
          if (overlay) return 0;
          if (!state.pane) { state.pane = chart.addPane(); state.pane.setHeight(150); }
          return state.pane.paneIndex();
        };
        const bars = new Set(candles.map((c) => c.time));
        for (const output of result.outputs) {
          const override = instance.styles[output.id];
          if (output.style.hidden || override?.hidden) continue;
          const pane = paneIndex(target(output));
          const color = override?.color ?? output.style.color ?? "#a78bfa";
          const series = output.kind === "line" ? chart.addSeries(LineSeries, { color, lineWidth: Math.max(1, Math.min(4, override?.width ?? output.style.width ?? 2)) as LineWidth, lineStyle: output.style.lineStyle?.includes("dash") ? LineStyle.Dashed : output.style.lineStyle?.includes("dot") ? LineStyle.Dotted : LineStyle.Solid, lastValueVisible: false, priceLineVisible: false, crosshairMarkerVisible: false }, pane)
            : chart.addSeries(HistogramSeries, { color, base: 0, lastValueVisible: false, priceLineVisible: false }, pane);
          state.series.push(series);
          const points = new Map(output.points.filter((p) => bars.has(p.displayTime)).map((p) => [p.displayTime, p.value === null ? { time: p.displayTime as UTCTimestamp } : { time: p.displayTime as UTCTimestamp, value: p.value, color: override?.color ?? p.color ?? color }]));
          series.setData([...points.values()].sort((a, b) => Number(a.time) - Number(b.time)));
        }
        const panes = new Set<boolean>();
        for (const item of [...result.outputs, ...result.levels, ...result.markers, ...result.drawings]) panes.add(target(item));
        for (const overlay of panes) {
          const pane = paneIndex(overlay);
          const anchor = chart.addSeries(LineSeries, { priceScaleId: overlay ? price.options().priceScaleId : "right", color: "#00000000", lastValueVisible: false, priceLineVisible: false, crosshairMarkerVisible: false }, pane);
          let min = Infinity, max = -Infinity;
          for (const output of result.outputs) if (target(output) === overlay) for (const p of output.points) if (p.value !== null && bars.has(p.time)) { min = Math.min(min, p.value); max = Math.max(max, p.value); }
          if (overlay) { min = candles[0]?.close ?? 1; max = candles.at(-1)?.close ?? min; }
          else if (!Number.isFinite(min)) { min = 0; max = 1; }
          const first = candles[0], last = candles.at(-1);
          if (first && last) anchor.setData(first.time === last.time ? [{ time: first.time as UTCTimestamp, value: min }] : [{ time: first.time as UTCTimestamp, value: min }, { time: last.time as UTCTimestamp, value: max }]);
          const primitive = new IndicatorPrimitive({ instance, result, candles, overlay, paneHeight: () => chart.panes()[pane]?.getHeight() ?? 0 });
          anchor.attachPrimitive(primitive); state.anchors.push({ series: anchor, primitive });
        }
      } catch (error) { remove(instance.id); throw error; }
      finally { if (range) chart.timeScale().setVisibleRange(range); }
    },
    remove,
    describe(id: string) { for (const current of owned.values()) for (const { primitive } of current.anchors) { const text = primitive.describe(id); if (text !== undefined) return text; } return undefined; },
    close() { for (const id of [...owned.keys()]) remove(id); },
  };
}
