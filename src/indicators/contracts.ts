/** Browser-safe indicator contracts. Analytical numbers are never order authority. */
export interface IndicatorEngineInfo { readonly name: "pinets"; readonly version: string }
/** The pin this build was developed and tested against. Startup derives the recorded engine version from the installed package and warns (never fails) on drift. */
export const indicatorEngine: IndicatorEngineInfo = { name: "pinets", version: "0.12.1" };
export const indicatorWrapperVersion = 1;
export const indicatorLimits = {
  sourceBytes: 64 * 1024, inputs: 32, numericOutputs: 32, markerOutputs: 32,
  markerEvents: 10_000, drawings: 500, candles: 5000, chartInstances: 10,
  memoryLimitBytes: 128 * 1024 * 1024, timeoutMs: 15_000, maxLoops: 10_000,
  workers: 8, queuedCalls: 64, resultBytes: 8 * 1024 * 1024,
  inlineBytes: 24 * 1024, count: 1, maxCount: 5000, inlineCount: 500,
  markerLimit: 20, maxMarkerLimit: 100, warnings: 64, listLimit: 20, maxListLimit: 50,
  cacheEntries: 64, cacheBytes: 64 * 1024 * 1024, cacheTtlMs: 30_000,
  customPerAgent: 100, versions: 200,
} as const;
export type IndicatorLimits = { [K in keyof typeof indicatorLimits]: number };
export const builtinIndicatorNames = ["sma", "ema", "rsi", "macd", "bollinger", "atr", "adx"] as const;
export type BuiltinIndicatorName = typeof builtinIndicatorNames[number];
export type IndicatorInputValue = number | boolean | string;
export type IndicatorInputs = Record<string, IndicatorInputValue>;
export interface IndicatorInput {
  key: string; runtimeId: string; variable?: string; title: string;
  type: "int" | "float" | "bool" | "string" | "source" | "color";
  default: IndicatorInputValue; min?: number; max?: number; step?: number;
  options?: IndicatorInputValue[]; group?: string; tooltip?: string;
}
export interface IndicatorDescriptor {
  id: string; builtIn: boolean; name: string; description: string; interpretation: string;
  version: number; sourceHash: string; inputs: IndicatorInput[]; overlay: boolean;
}
export interface IndicatorRunRequest {
  indicatorId: string; version?: number; coin: string; interval: string;
  start: number; end: number; inputs?: IndicatorInputs; count?: number; markerLimit?: number; saveAs?: string;
}
export interface ChartIndicatorInstance {
  id: string; indicatorId: string; inputs: IndicatorInputs; configuredVersion: number;
  hidden: boolean; pane: "auto" | "price" | "separate";
  styles: Record<string, { hidden?: boolean; color?: string; width?: number }>;
}
export interface ChartIndicatorEvidence {
  instanceId: string; indicatorId: string; status: "loading" | "current" | "stale" | "error"; error?: string;
  configuration: ChartIndicatorInstance;
  result?: { version: number; sourceHash: string; engine: typeof indicatorEngine; wrapperVersion: number;
    resultHash: string; resolvedInputs: IndicatorInputs; inputSchemaHash: string; computedAt: number; end: number;
    dataset: IndicatorResult["dataset"]; coverage: IndicatorResult["coverage"];
    sample: IndicatorValuePoint[]; objects: { id: string; kind: string; text?: string; time: number }[]; sampleScope: "partial-visible" };
}
export interface IndicatorSaveRequest {
  indicatorId?: string; expectedVersion?: number; name: string; description: string; interpretation: string;
  source: string; validation?: { coin: string; interval: string; start: number; end: number };
}
export interface IndicatorActor {
  kind: "user" | "agent"; userId?: string | null; subagentId?: string | null; runId?: string | null;
  conversationId?: string | null; taskId?: string | null;
}
export interface IndicatorValuePoint { time: number; value: Record<string, number | null> }
export interface IndicatorVisualStyle {
  color?: string; width?: number; lineStyle?: string; hidden: boolean;
  offset: number; showLast?: number;
}
export interface IndicatorOutput {
  id: string; title: string; kind: "line" | "histogram" | "columns";
  overlay: boolean; forceOverlay: boolean; style: IndicatorVisualStyle;
  finiteCount: number; firstFiniteTime: number | null;
  points: { time: number; displayTime: number; value: number | null; color?: string }[];
}
export interface IndicatorLevel {
  id: string; title: string; value: number; overlay: boolean; forceOverlay: boolean; style: IndicatorVisualStyle;
}
export interface IndicatorFill {
  id: string; title: string; from: string; to: string; color?: string; hidden: boolean;
  points: { time: number; color?: string }[];
}
export interface IndicatorMarker {
  id: string; outputId: string; kind: "shape" | "char" | "arrow";
  time: number; displayTime: number; value: number | boolean; offset: number;
  overlay: boolean; forceOverlay: boolean; location: string; shape?: string;
  text?: string; character?: string; color?: string; textColor?: string; size?: string;
}
export interface IndicatorAnchor { time: number; price: number; barIndex?: number }
interface DrawingBase {
  id: string; overlay: boolean; forceOverlay: boolean; color?: string; width?: number;
  lineStyle?: string; extend: "none" | "left" | "right" | "both";
}
export type IndicatorDrawing =
  | (DrawingBase & { kind: "line"; from: IndicatorAnchor; to: IndicatorAnchor })
  | (DrawingBase & { kind: "box"; from: IndicatorAnchor; to: IndicatorAnchor; background?: string; text: string; textColor?: string; size?: string })
  | (DrawingBase & { kind: "label"; at: IndicatorAnchor; text: string; textColor?: string; size?: string; shape?: string; location: string });
export interface NormalizedIndicator {
  title: string; overlay: boolean; inputs: IndicatorInput[]; resolvedInputs: IndicatorInputs;
  outputs: IndicatorOutput[]; levels: IndicatorLevel[]; fills: IndicatorFill[];
  markers: IndicatorMarker[]; drawings: IndicatorDrawing[]; values: IndicatorValuePoint[];
  warnings: string[]; warningsOmitted: number;
}
export interface IndicatorResult extends NormalizedIndicator {
  schemaVersion: 1; indicatorId: string; builtIn: boolean; version: number; sourceHash: string;
  engine: typeof indicatorEngine; wrapperVersion: number; coin: string; interval: string; start: number; end: number;
  inputSchemaHash: string; computedAt: number; candleHash: string; resultHash: string;
  dataset: { count: number; firstOpenTime: number; lastOpenTime: number };
  outputDataset: { count: number; firstTime: number | null; lastTime: number | null };
  coverage: { complete: boolean; closedOnly: true; expected: number; returned: number; seedCoverage: "complete" | "limited" };
}
export interface IndicatorPreviewRequest {
  source: string; coin: string; interval: string; start: number; end: number;
  inputs?: IndicatorInputs; count?: number; markerLimit?: number; saveAs?: string;
}
export interface PineCandle {
  open: number; high: number; low: number; close: number; volume: number;
  openTime: number; closeTime: number;
}
export interface IndicatorVmRequest {
  source: string; coin: string; pineTimeframe: string; intervalMs: number;
  candles: PineCandle[]; inputs: IndicatorInputs; limits: IndicatorLimits;
}
