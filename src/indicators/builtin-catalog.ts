import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import { type BuiltinIndicatorName, type IndicatorDescriptor, type IndicatorInputs, type PineCandle } from "./contracts.js";
import type { IndicatorRuntime } from "./runtime.js";
import { IndicatorError, resolveInputs } from "./validation.js";

const manifest = [
  { name: "sma", title: "SMA", overlay: true, description: "Simple moving average of the selected source.", interpretation: "Equal-weight trend smoothing; lags price." },
  { name: "ema", title: "EMA", overlay: true, description: "Exponential moving average of the selected source.", interpretation: "Recent observations receive greater weight. Seed-window initialization affects history." },
  { name: "rsi", title: "RSI", overlay: false, description: "Relative strength with selectable smoothing.", interpretation: "Momentum oscillator, usually 0–100. Levels 30/70 are context, not trade instructions." },
  { name: "macd", title: "MACD", overlay: false, description: "Moving-average convergence/divergence with EMA or DEMA.", interpretation: "MACD minus signal is the histogram; crossings do not establish profitability." },
  { name: "bollinger", title: "Bollinger Bands", overlay: true, description: "SMA with standard-deviation bands.", interpretation: "Bands describe dispersion, not guaranteed support or resistance." },
  { name: "atr", title: "ATR", overlay: false, description: "Smoothed true range.", interpretation: "Volatility in price units, not direction or an order-sizing authority." },
  { name: "adx", title: "ADX", overlay: false, description: "Directional-movement trend strength with selectable smoothing.", interpretation: "Nonnegative trend strength, not trend direction." },
] satisfies { name: BuiltinIndicatorName; title: string; overlay: boolean; description: string; interpretation: string }[];
export interface BundledIndicator { descriptor: IndicatorDescriptor; source: string }
// Trusted bundled sources are immutable for a process/build. Never cache custom metadata here.
let completedCatalog: Map<string, BundledIndicator> | undefined;

/** Explicit server-side file manifest. No filesystem discovery or import-time I/O. */
export class BuiltinIndicatorCatalog {
  private readonly entries = new Map<string, BundledIndicator>();
  async initialize(runtime: IndicatorRuntime) {
    if (completedCatalog) { for (const [id, entry] of completedCatalog) this.entries.set(id, structuredClone(entry)); return; }
    const loaded = new Map<string, BundledIndicator>();
    const candles: PineCandle[] = Array.from({ length: 320 }, (_, index) => {
      const close = 100 + index / 10 + Math.sin(index / 3);
      return { open: close - 0.2, close, high: close + 1, low: close - 1, volume: 100 + index, openTime: index * 3600_000, closeTime: (index + 1) * 3600_000 };
    });
    const sources = await Promise.all(manifest.map((entry) => readFile(new URL(`./builtins/${entry.name}.pine`, import.meta.url), "utf8")));
    const results = await runtime.executeBatch("builtin-metadata", sources.map((source) => ({ source, coin: "BTC", pineTimeframe: "60", intervalMs: 3600_000, candles, inputs: {} })));
    for (const [index, entry] of manifest.entries()) {
      const result = results[index]!, source = sources[index]!;
      if (result.overlay !== entry.overlay) throw new Error(`Bundled ${entry.name} declaration does not match manifest`);
      const descriptor: IndicatorDescriptor = { id: `builtin:${entry.name}`, builtIn: true, name: entry.title, description: entry.description,
        interpretation: `${entry.interpretation} Approximately 5× the longest period plus output history is a seed recommendation, not a stability guarantee. PineTS semantics apply.`, version: 1,
        sourceHash: createHash("sha256").update(source).digest("hex"), inputs: result.inputs, overlay: result.overlay };
      loaded.set(descriptor.id, { descriptor, source });
    }
    completedCatalog = loaded;
    this.entries.clear();
    for (const [id, entry] of loaded) this.entries.set(id, entry);
  }
  list(): IndicatorDescriptor[] { return [...this.entries.values()].map((entry) => structuredClone(entry.descriptor)); }
  get(id: string, version?: number): BundledIndicator {
    const entry = this.entries.get(id);
    if (!entry || (version !== undefined && version !== entry.descriptor.version)) throw new IndicatorError("Bundled indicator version is unavailable", 404);
    return structuredClone(entry);
  }
  resolve(id: string, overrides: IndicatorInputs): IndicatorInputs {
    const inputs = resolveInputs(this.get(id).descriptor.inputs, overrides);
    if (id === "builtin:macd" && (inputs.fastPeriod as number) >= (inputs.slowPeriod as number)) throw new IndicatorError("MACD fastPeriod must be smaller than slowPeriod");
    return inputs;
  }
}
