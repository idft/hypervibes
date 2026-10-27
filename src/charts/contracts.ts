import { randomUuid } from "../web/random-uuid.js";
import { AgentError, requestId } from "../agents/agent-input.js";
import { chartIntervals, isChartInterval, type ChartCandle, type ChartInterval, type ChartSelection } from "../hyperliquid/chart-contracts.js";

export const drawingKinds = ["horizontal", "horizontal-ray", "vertical", "trend", "ray", "extended", "rectangle", "text", "callout", "arrow"] as const;
export type DrawingKind = typeof drawingKinds[number];
export const chartToolNames = ["get_chart_snapshot", "get_chart_layout", "edit_chart_annotations"] as const;
export interface ChartPoint { time: number; price: number }
export interface ChartDrawing {
  id: string; coin: string; kind: DrawingKind; name: string; points: ChartPoint[];
  text: string; color: string; width: number; dashed: boolean; fillOpacity: number;
  locked: boolean; hidden: boolean; intervals: ChartInterval[]; group: string; author: "user" | "agent";
}
import { chartIndicatorEvidence, chartIndicatorInstances } from "../indicators/chart-contracts.js";
import type { ChartIndicatorEvidence, ChartIndicatorInstance } from "../indicators/contracts.js";
export interface ChartDocument extends ChartSelection {
  name: string; drawings: ChartDrawing[]; notes: string; hideDrawings: boolean; magnet: boolean; indicators: ChartIndicatorInstance[];
}
export interface ChartLayout { id: string; revision: number; document: ChartDocument; updatedAt: number; canUndo: boolean; canRedo: boolean; lastChange: string }
export interface ChartEdit {
  label: string; settings?: Partial<Omit<ChartDocument, "drawings">>;
  upsert?: ChartDrawing[]; remove?: string[];
}
export interface ChartCapture {
  layoutId: string; revision: number; capturedAt: number; status: "connecting" | "live" | "reconnecting";
  range: { from: number; to: number }; candles: ChartCandle[]; selectedDrawingId?: string; image?: string; indicators?: ChartIndicatorEvidence[]; selectedGeneratedObjectId?: string;
}
export interface ChartAttachment { snapshotId: string; layoutId: string; name: string; coin: string; interval: ChartInterval; capturedAt: number; image: boolean; selectedDrawingId?: string }
export interface ChartContext extends Omit<ChartCapture, "image"> { document: ChartDocument; screenshotIncluded: boolean; formingCandleTime: number | null }

const fail = (label: string): never => { throw new AgentError(`Invalid chart ${label}`); };
export function record(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) return fail("object");
  return value as Record<string, unknown>;
}
function text(value: unknown, max: number, empty = true): string {
  if (typeof value !== "string" || value.length > max || (!empty && !value.trim())) return fail("text");
  return value;
}
function number(value: unknown, min: number, max: number): number {
  if (typeof value !== "number" || !Number.isFinite(value) || value < min || value > max) return fail("number");
  return value;
}
export function revision(value: unknown): number {
  const result = number(value, 0, Number.MAX_SAFE_INTEGER - 1);
  if (!Number.isInteger(result)) return fail("revision");
  return result;
}
function bool(value: unknown): boolean { return typeof value === "boolean" ? value : fail("boolean"); }
export function chartCoin(value: unknown): string {
  const coin = text(value, 32, false);
  if (!/^[A-Za-z0-9][A-Za-z0-9_-]*$/.test(coin)) return fail("instrument");
  return coin;
}
function interval(value: unknown): ChartInterval { return typeof value === "string" && isChartInterval(value) ? value : fail("timeframe"); }
function list<T>(value: unknown, max: number, parse: (value: unknown) => T): T[] {
  if (!Array.isArray(value) || value.length > max) return fail("list");
  return value.map(parse);
}
export function point(value: unknown): ChartPoint {
  const p = record(value);
  return { time: number(p.time, 0, 32_503_680_000), price: number(p.price, 0.000000000001, 1e15) };
}
export function drawing(value: unknown): ChartDrawing {
  const d = record(value);
  const kind = drawingKinds.find((kind) => kind === d.kind) ?? fail("drawing type");
  const points = list(d.points, 2, point);
  if (points.length !== (["horizontal", "horizontal-ray", "vertical", "text"].includes(kind) ? 1 : 2)) return fail("anchor count");
  if (points.length === 2 && points[0]!.time === points[1]!.time && !["arrow", "callout"].includes(kind)) return fail("anchors (use different times)");
  const color = d.color;
  if (typeof color !== "string" || !/^#[0-9a-f]{6}$/i.test(color)) return fail("color");
  return { id: requestId(text(d.id, 36)), coin: chartCoin(d.coin), kind, name: text(d.name, 100, false), points, text: text(d.text, 1000), color,
    width: number(d.width, 1, 5), dashed: bool(d.dashed), fillOpacity: number(d.fillOpacity, 0, 0.6), locked: bool(d.locked), hidden: bool(d.hidden),
    intervals: [...new Set(list(d.intervals, chartIntervals.length, interval))], group: text(d.group, 80), author: d.author === "agent" ? "agent" : d.author === "user" ? "user" : fail("author") };
}
export function chartSettings(value: unknown): Partial<Omit<ChartDocument, "drawings">> {
  const s = record(value), result: Partial<Omit<ChartDocument, "drawings">> = {};
  for (const key of Object.keys(s)) {
    if (key === "coin") result.coin = chartCoin(s.coin);
    else if (key === "interval") result.interval = interval(s.interval);
    else if (key === "name") result.name = text(s.name, 80, false);
    else if (key === "notes") result.notes = text(s.notes, 8000);
    else if (key === "hideDrawings") result.hideDrawings = bool(s.hideDrawings);
    else if (key === "magnet") result.magnet = bool(s.magnet);
    else if (key === "indicators") result.indicators = chartIndicatorInstances(s.indicators);
    else return fail("setting");
  }
  return result;
}
export function chartEdit(value: unknown): ChartEdit {
  const e = record(value);
  if (Object.keys(e).some((key) => !["label", "settings", "upsert", "remove"].includes(key))) return fail("edit");
  const upsert = e.upsert === undefined ? [] : list(e.upsert, 300, drawing);
  const remove = e.remove === undefined ? [] : list(e.remove, 300, (id) => requestId(text(id, 36)));
  const ids = [...upsert.map((d) => d.id), ...remove];
  if (new Set(ids).size !== ids.length) return fail("duplicate drawing IDs");
  return { label: text(e.label, 100, false), ...(e.settings === undefined ? {} : { settings: chartSettings(e.settings) }), upsert, remove };
}
export function chartCapture(value: unknown): ChartCapture {
  const c = record(value), range = record(c.range);
  const from = number(range.from, 0, 32_503_680_000), to = number(range.to, from, 32_503_680_000);
  const candles = list(c.candles, 1500, (value) => {
    const row = record(value);
    const time = number(row.time, 0, 32_503_680_000), open = number(row.open, 1e-12, 1e15), high = number(row.high, 1e-12, 1e15), low = number(row.low, 1e-12, 1e15), close = number(row.close, 1e-12, 1e15), volume = number(row.volume, 0, 1e25);
    if (high < Math.max(open, close, low) || low > Math.min(open, close)) return fail("OHLC candle");
    return { time, open, high, low, close, volume };
  });
  if (candles.some((candle, index) => index > 0 && candle.time <= candles[index - 1]!.time)) return fail("candle ordering");
  if (!["live", "connecting", "reconnecting"].includes(String(c.status))) return fail("connection status");
  return { layoutId: requestId(text(c.layoutId, 36)), revision: revision(c.revision), capturedAt: number(c.capturedAt, 0, 32_503_680_000_000), status: c.status as ChartCapture["status"], range: { from, to }, candles, indicators: chartIndicatorEvidence(c.indicators),
    ...(c.selectedGeneratedObjectId === undefined ? {} : { selectedGeneratedObjectId: text(c.selectedGeneratedObjectId, 200) }),
    ...(c.selectedDrawingId === undefined ? {} : { selectedDrawingId: requestId(text(c.selectedDrawingId, 36)) }),
    ...(c.image === undefined ? {} : { image: text(c.image, 700_000, false) }) };
}
export function newDrawing(kind: DrawingKind, coin: string, points: ChartPoint[], id = randomUuid()): ChartDrawing {
  return { id, coin, kind, points, name: kind.replaceAll("-", " "), text: "", color: "#a78bfa", width: 2, dashed: false, fillOpacity: 0.15, locked: false, hidden: false, intervals: [], group: "", author: "user" };
}
export function visibleDrawing(d: ChartDrawing, document: ChartDocument): boolean {
  return !document.hideDrawings && !d.hidden && d.coin === document.coin && (!d.intervals.length || d.intervals.includes(document.interval));
}
