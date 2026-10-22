import type { CanvasRenderingTarget2D } from "fancy-canvas";
import type { IChartApi, ISeriesApi, ISeriesPrimitive, IPrimitivePaneRenderer, IPrimitivePaneView, SeriesAttachedParameter, UTCTimestamp } from "lightweight-charts";
import type { ChartCandle } from "../hyperliquid/chart-contracts.js";
import type { ChartIndicatorInstance, IndicatorDrawing, IndicatorResult } from "../indicators/contracts.js";
import { lineEnds } from "./geometry.js";

export interface IndicatorPrimitiveData { instance: ChartIndicatorInstance; result: IndicatorResult; candles: ChartCandle[]; overlay: boolean; paneHeight: () => number }
export function indicatorTimeCoordinate(chart: IChartApi, candles: ChartCandle[], time: number, step: number): number | null {
  const scale = chart.timeScale(), direct = scale.timeToCoordinate(time as UTCTimestamp);
  if (direct !== null) return direct;
  if (!candles.length) return null;
  let lo = 0, hi = candles.length - 1;
  while (lo < hi) { const mid = Math.ceil((lo + hi) / 2); if (candles[mid]!.time <= time) lo = mid; else hi = mid - 1; }
  const anchor = candles[lo]!, x = scale.timeToCoordinate(anchor.time as UTCTimestamp);
  return x === null ? null : x + (time - anchor.time) / step * scale.options().barSpacing;
}
/** Chart-native, read-only Pine visuals. Their IDs never enter the annotation editor. */
export class IndicatorPrimitive implements ISeriesPrimitive {
  private chart?: IChartApi;
  private series?: ISeriesApi<"Line">;
  private request?: () => void;
  private readonly hits: { id: string; x: number; y: number; text: string }[] = [];
  private readonly renderer: IPrimitivePaneRenderer = { draw: (target: CanvasRenderingTarget2D) => target.useMediaCoordinateSpace(({ context, mediaSize }) => this.draw(context, mediaSize.width, mediaSize.height)) };
  private readonly views: IPrimitivePaneView[] = [{ zOrder: () => "normal", renderer: () => this.renderer }];
  constructor(private data: IndicatorPrimitiveData) {}
  attached(parameters: SeriesAttachedParameter) { this.chart = parameters.chart as IChartApi; this.series = parameters.series as ISeriesApi<"Line">; this.request = parameters.requestUpdate; }
  detached() { this.chart = undefined; this.series = undefined; this.request = undefined; this.hits.length = 0; }
  update(data: IndicatorPrimitiveData) { this.data = data; this.request?.(); }
  updateAllViews() { this.hits.length = 0; }
  paneViews() { return this.views; }
  hitTest(x: number, y: number) { const hit = this.hits.slice().reverse().find((hit) => Math.hypot(hit.x - x, hit.y - y) < 12); return hit ? { externalId: hit.id, zOrder: "normal" as const, cursorStyle: "default", distance: Math.hypot(hit.x - x, hit.y - y), hitTestPriority: 2 } : null; }
  describe(id: string) { return this.hits.find((hit) => hit.id === id)?.text; }
  autoscaleInfo() {
    const values = this.data.result.drawings.filter((d) => this.owns(d)).flatMap((d) => d.kind === "label" ? d.location === "pr" ? [d.at.price] : [] : [d.from.price, d.to.price]);
    for (const level of this.data.result.levels) if (this.owns(level) && !level.style.hidden) values.push(level.value);
    return values.length ? { priceRange: { minValue: Math.min(...values), maxValue: Math.max(...values) } } : null;
  }
  private owns(item: { overlay: boolean; forceOverlay: boolean }) {
    const target = item.forceOverlay || (this.data.instance.pane === "price" ? true : this.data.instance.pane === "separate" ? false : item.overlay);
    return target === this.data.overlay;
  }
  private draw(ctx: CanvasRenderingContext2D, width: number, height: number) {
    if (!this.chart || !this.series) return;
    const { result, instance, candles } = this.data;
    const step = result.dataset.count > 1 ? (result.dataset.lastOpenTime - result.dataset.firstOpenTime) / (result.dataset.count - 1) : (result.end / 1000 - result.dataset.lastOpenTime);
    const x = (time: number) => indicatorTimeCoordinate(this.chart!, candles, time, step);
    const y = (price: number) => this.series!.priceToCoordinate(price);
    const outputMap = new Map(result.outputs.map((output) => [output.id, new Map(output.points.map((p) => [p.time, p]))]));
    const levels = new Map(result.levels.map((level) => [level.id, level]));
    const point = (id: string, time: number) => {
      const output = result.outputs.find((p) => p.id === id), level = levels.get(id);
      if (output && !this.owns(output) || level && !this.owns(level)) return null;
      const p = outputMap.get(id)?.get(time), value = p?.value ?? level?.value;
      if (value === undefined || value === null) return null;
      const px = x(p?.displayTime ?? time), py = y(value);
      return px === null || py === null ? null : { x: px, y: py };
    };
    this.hits.length = 0;
    ctx.save(); ctx.beginPath(); ctx.rect(0, 0, width, height); ctx.clip();
    for (const fill of result.fills) {
      if (fill.hidden || instance.styles[fill.id]?.hidden) continue;
      for (let i = 1; i < fill.points.length; i++) {
        const a = fill.points[i - 1]!, b = fill.points[i]!, p1 = point(fill.from, a.time), p2 = point(fill.from, b.time), p3 = point(fill.to, b.time), p4 = point(fill.to, a.time);
        if (!p1 || !p2 || !p3 || !p4) continue;
        ctx.fillStyle = instance.styles[fill.id]?.color ?? b.color ?? fill.color ?? "#2962ff30";
        ctx.beginPath(); ctx.moveTo(p1.x, p1.y); for (const p of [p2, p3, p4]) ctx.lineTo(p.x, p.y); ctx.closePath(); ctx.fill();
      }
    }
    // Levels are true horizontal geometry, not thousands of redundant series entries.
    for (const level of result.levels) {
      if (!this.owns(level) || level.style.hidden || instance.styles[level.id]?.hidden) continue;
      const py = y(level.value); if (py === null) continue;
      ctx.strokeStyle = instance.styles[level.id]?.color ?? level.style.color ?? "#787b86";
      ctx.lineWidth = instance.styles[level.id]?.width ?? level.style.width ?? 1; ctx.setLineDash(level.style.lineStyle?.includes("dot") ? [2, 3] : level.style.lineStyle?.includes("dash") ? [6, 4] : []);
      ctx.beginPath(); ctx.moveTo(0, py); ctx.lineTo(width, py); ctx.stroke();
    }
    ctx.setLineDash([]);
    for (const marker of result.markers) {
      if (!this.owns(marker) || instance.styles[marker.outputId]?.hidden) continue;
      const px = x(marker.displayTime); if (px === null || px < -50 || px > width + 50) continue;
      const candle = candles.find((c) => c.time === marker.displayTime) ?? candles.find((c) => c.time === marker.time);
      let py: number | null = marker.location === "Top" ? 15 : marker.location === "Bottom" ? height - 15 : marker.location === "Absolute" && typeof marker.value === "number" ? y(marker.value) : candle ? y(marker.location === "BelowBar" ? candle.low : candle.high) : null;
      if (py === null) continue;
      if (marker.location === "BelowBar") py += 14; else if (marker.location === "AboveBar" || !marker.location) py -= 14;
      const down = marker.kind === "arrow" ? typeof marker.value === "number" && marker.value < 0 : marker.shape?.includes("down");
      ctx.fillStyle = instance.styles[marker.outputId]?.color ?? marker.color ?? "#a78bfa"; ctx.font = "12px sans-serif"; ctx.textAlign = "center";
      // Glyph approximation keeps direction, source/display time and emitted text.
      ctx.fillText(marker.kind === "char" ? marker.character ?? "•" : marker.shape?.includes("circle") ? "●" : marker.shape?.includes("cross") ? "+" : down ? "▼" : "▲", px, py);
      if (marker.text) { ctx.fillStyle = marker.textColor ?? marker.color ?? "#d4d4d8"; ctx.fillText(marker.text, px, py + (down ? 16 : -14)); }
      this.hits.push({ id: `pine:${instance.id}:${result.resultHash}:${marker.id}`, x: px, y: py, text: `${marker.text ?? marker.character ?? marker.kind} · source ${new Date(marker.time * 1000).toISOString()}` });
    }
    for (const drawing of result.drawings) {
      if (!this.owns(drawing) || instance.styles[drawing.id]?.hidden) continue;
      const anchor = drawing.kind === "label" ? drawing.at : drawing.from, px = x(anchor.time); let py = y(anchor.price);
      if (drawing.kind === "label" && drawing.location !== "pr") { const candle = candles.find((c) => c.time === anchor.time); if (candle) py = y(drawing.location === "ab" ? candle.high : candle.low); }
      if (px === null || py === null) continue;
      ctx.strokeStyle = instance.styles[drawing.id]?.color ?? drawing.color ?? "#a78bfa"; ctx.fillStyle = ctx.strokeStyle;
      ctx.lineWidth = instance.styles[drawing.id]?.width ?? drawing.width ?? 1; ctx.setLineDash(drawing.lineStyle?.includes("dash") ? [6, 4] : drawing.lineStyle?.includes("dot") ? [2, 3] : []);
      if (drawing.kind === "label") {
        const textY = py + (drawing.location === "ab" ? -14 : drawing.location === "bl" ? 14 : 0);
        ctx.font = `${drawing.size === "large" ? 16 : 12}px sans-serif`; ctx.textAlign = "center";
        if (drawing.color) { const w = ctx.measureText(drawing.text).width + 8; ctx.fillRect(px - w / 2, textY - 13, w, 18); }
        ctx.fillStyle = drawing.textColor || "#d4d4d8"; ctx.fillText(drawing.text, px, textY);
      } else this.drawObject(ctx, drawing, px, py, x(drawing.to.time), y(drawing.to.price), width);
      this.hits.push({ id: `pine:${instance.id}:${result.resultHash}:${drawing.id}`, x: px, y: py, text: drawing.kind === "line" ? `Line · ${anchor.price}` : drawing.text });
    }
    ctx.restore();
  }
  private drawObject(ctx: CanvasRenderingContext2D, drawing: Exclude<IndicatorDrawing, { kind: "label" }>, x1: number, y1: number, x2: number | null, y2: number | null, width: number) {
    if (x2 === null || y2 === null) return;
    if (drawing.kind === "box") {
      const left = drawing.extend === "left" || drawing.extend === "both" ? 0 : Math.min(x1, x2), right = drawing.extend === "right" || drawing.extend === "both" ? width : Math.max(x1, x2);
      ctx.fillStyle = drawing.background ?? "#2962ff30"; ctx.fillRect(left, Math.min(y1, y2), right - left, Math.abs(y2 - y1)); ctx.strokeRect(left, Math.min(y1, y2), right - left, Math.abs(y2 - y1));
      if (drawing.text) { ctx.fillStyle = drawing.textColor ?? "#d4d4d8"; ctx.font = "12px sans-serif"; ctx.textAlign = "center"; ctx.fillText(drawing.text, (left + right) / 2, (y1 + y2) / 2); }
    } else {
      const a = { x: x1, y: y1 }, b = { x: x2, y: y2 };
      const [from, to] = drawing.extend === "both" ? lineEnds("extended", a, b, width) : drawing.extend === "right" ? lineEnds("ray", a.x <= b.x ? a : b, a.x <= b.x ? b : a, width) : drawing.extend === "left" ? lineEnds("ray", a.x >= b.x ? a : b, a.x >= b.x ? b : a, width) : [a, b];
      ctx.beginPath(); ctx.moveTo(from!.x, from!.y); ctx.lineTo(to!.x, to!.y); ctx.stroke();
    }
  }
}
