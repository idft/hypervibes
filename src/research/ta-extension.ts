import type { Context } from "@earendil-works/chord";
import { Type, type TSchema } from "@earendil-works/pi-ai";
import { defineExtension, defineTool, section, type ToolExecutionApi } from "@earendil-works/pi-durable";
import type { MarketCharts } from "../hyperliquid/market-charts.js";
import { describeDataset, technicalEvidenceInstructions } from "./evidence.js";
import { CandleCoverageSchema, DatasetSchema, ExportSchema } from "./contracts.js";
import { technicalAnalysis, validateIndicatorRequest, type IndicatorRequest } from "./technical-analysis.js";
import type { ResearchResponder } from "./tool-response.js";

const strict = { additionalProperties: false } as const;
const period = Type.Integer({ minimum: 2, maximum: 500 });
const source = Type.Union([Type.Literal("open"), Type.Literal("high"), Type.Literal("low"), Type.Literal("close"), Type.Literal("hl2"), Type.Literal("hlc3"), Type.Literal("ohlc4"), Type.Literal("volume")]);
const smoothing = Type.Union([Type.Literal("wilder"), Type.Literal("sma"), Type.Literal("ema"), Type.Literal("wma")]);
const average = Type.Union([Type.Literal("ema"), Type.Literal("dema")]);
const common = {
  coin: Type.String({ minLength: 1, maxLength: 64 }), interval: Type.String({ minLength: 1, maxLength: 8 }),
  start: Type.Integer({ minimum: 0, description: "Inclusive range start, UTC milliseconds; includes warm-up" }),
  end: Type.Integer({ minimum: 1, description: "Exclusive range end, UTC milliseconds; closed candles only" }),
  count: Type.Optional(Type.Integer({ minimum: 1, maximum: 500, description: "Trailing output values; defaults to 1" })),
  saveAs: Type.Optional(Type.String({ minLength: 1, maxLength: 160, description: "Export JSON to an artifact instead of returning values; also requires artifact_create" })),
};
interface CommonInput { coin: string; interval: string; start: number; end: number; count?: number; saveAs?: string }
const priced = { period, source: Type.Optional(source) };
const smoothed = { period, smoothing: Type.Optional(smoothing) };
function resultSchema(name: IndicatorRequest["type"], parameters: TSchema, value: TSchema) {
  return Type.Union([Type.Object({
    coin: Type.String(), interval: Type.String(), start: Type.Integer(), end: Type.Integer(), coverage: CandleCoverageSchema,
    library: Type.Literal("trading-signals@8.3.0"), indicator: Type.Literal(name), parameters,
    dataset: DatasetSchema, outputDataset: DatasetSchema,
    requiredInputs: Type.Integer({ minimum: 0 }), stable: Type.Boolean(),
    values: Type.Array(Type.Object({ time: Type.Number({ description: "Candle open UTC seconds" }), value: Type.Union([Type.Null(), value]) }), { maxItems: 500 }),
  }), ExportSchema]);
}
const priceParameters = Type.Object({ period, source });
const smoothParameters = Type.Object({ period, smoothing });
const seedHistoryGuidance = "Plan the seed window BEFORE fetching. Unless the user explicitly constrains input history, target at least 5 times the longest configured indicator period across the analysis; include MACD fast/slow/signal periods. For count trailing outputs, add count - 1 candles so the first returned point also has the target seed history. EMA50 with count=1 therefore needs at least 250 closed candles; count=5 targets 254, not 119. Library stable=true only means its minimum warm-up was reached, NOT that EMA/Wilder initialization effects have disappeared. Also inspect requiredInputs and never use less than the library minimum. For fixed-width intervals, first align end to a closed-candle boundary, then set start=end-targetCandles*intervalMs. Example for 250 hourly bars: const hour=3600000; const end=Math.floor(Date.now()/hour)*hour; const start=end-250*hour; do not subtract from an unaligned Date.now() and accidentally lose a candle. Reuse the same fixed start/end across indicators. Changing a page limit does not widen the requested time range. research_candles pages cap at 2000; paginate using nextStart and the SAME end if more inspection candles are needed. TA tools fetch their own seed candles server-side, so do not dump raw candle arrays just to warm indicators. Stay within the latest approximately 5000 available candles per instrument/timeframe and the tool's 5000-input cap; pagination cannot recover older retained-out history. If a user constraint or unavailable data prevents the seed target, disclose actual input count, target and initialization sensitivity; never claim full seed coverage just because stable=true. Do not change the requested timeframe to manufacture more history, and never interpolate gaps.";
const commonDescription = " Fetches and calculates server-side over the supplied contiguous closed-candle range (up to 5,000 available candles). Include seed history in start/end: normally target at least 5x the longest configured period plus count-1 candles; stable alone does not prove adequate seeding. Count selects trailing outputs. Every values[] entry is {time, value}, with candle-open time in UTC seconds; read numeric or object fields from entry.value, never from the entry itself. Null value before stable. Source defaults to close where supported. Returns analytical values, not trading/accounting authority.";

/** A separate capability from Code Mode: explicit tools can also be called directly. */
export function createTaExtension(charts: Pick<MarketCharts, "researchHistory">, respond: ResearchResponder) {
  const run = (args: CommonInput, request: IndicatorRequest, api: ToolExecutionApi, context: Context) => {
    validateIndicatorRequest(request);
    return respond(args.saveAs, api, context, async () => {
      const history = await charts.researchHistory(args.coin, args.interval, args.start, args.end, 5000, context.abortSignal);
      if (!history.coverage.complete || history.nextStart !== null) throw new Error("TA requires a complete contiguous range of at most 5,000 available closed candles. Narrow the range; no silent calculation across gaps.");
      const analysis = technicalAnalysis(history.candles, request, args.count ?? 1);
      return { coin: args.coin, interval: args.interval, start: args.start, end: history.end, coverage: history.coverage, library: "trading-signals@8.3.0", dataset: describeDataset(history.candles), outputDataset: describeDataset(analysis.values), ...analysis };
    }, { instrument: args.coin, interval: args.interval });
  };
  const tools = [
    defineTool({
      name: "ta_sma", description: "Simple moving average. Choose period and input source." + commonDescription,
      parameters: Type.Object({ ...common, ...priced }, strict), replay: "safe",
      structuredOutputSchema: resultSchema("sma", priceParameters, Type.Number()),
      execute: (args, api, context) => run(args, { type: "sma", period: args.period, source: args.source }, api, context),
    }),
    defineTool({
      name: "ta_ema", description: "Exponential moving average. Choose period and input source; trading-signals seeds EMA from the first input and reports stable after the period." + commonDescription,
      parameters: Type.Object({ ...common, ...priced }, strict), replay: "safe",
      structuredOutputSchema: resultSchema("ema", priceParameters, Type.Number()),
      execute: (args, api, context) => run(args, { type: "ema", period: args.period, source: args.source }, api, context),
    }),
    defineTool({
      name: "ta_rsi", description: "Relative strength index. Choose period, input source and smoothing (wilder default, sma, ema, wma). Returns RSI numbers only; apply your own overbought/oversold thresholds in the script." + commonDescription,
      parameters: Type.Object({ ...common, ...priced, smoothing: Type.Optional(smoothing) }, strict), replay: "safe",
      structuredOutputSchema: resultSchema("rsi", Type.Object({ period, source, smoothing }), Type.Number()),
      execute: (args, api, context) => run(args, { type: "rsi", period: args.period, source: args.source, smoothing: args.smoothing }, api, context),
    }),
    defineTool({
      name: "ta_macd", description: "Moving average convergence/divergence. Explicit fastPeriod, slowPeriod and signalPeriod; fast must be smaller than slow. Choose main average and signalAverage independently (ema default or dema), plus input source. Each point.value contains macd, signal and histogram (e.g. result.values.at(-1).value.histogram)." + commonDescription,
      parameters: Type.Object({ ...common, fastPeriod: period, slowPeriod: period, signalPeriod: period, average: Type.Optional(average), signalAverage: Type.Optional(average), source: Type.Optional(source) }, strict), replay: "safe",
      structuredOutputSchema: resultSchema("macd", Type.Object({ fastPeriod: period, slowPeriod: period, signalPeriod: period, average, signalAverage: average, source }), Type.Object({ macd: Type.Number(), signal: Type.Number(), histogram: Type.Number() })),
      execute: (args, api, context) => run(args, { type: "macd", fastPeriod: args.fastPeriod, slowPeriod: args.slowPeriod, signalPeriod: args.signalPeriod, average: args.average, signalAverage: args.signalAverage, source: args.source }, api, context),
    }),
    defineTool({
      name: "ta_bollinger", description: "Bollinger Bands. Explicit period and deviations multiplier (>0 to 10, fractional allowed), plus input source. This library uses an SMA middle band and population standard deviation; no unsupported MA-type parameter is silently accepted. Each point.value contains lower, middle and upper (e.g. result.values.at(-1).value.lower); point.lower is not a field." + commonDescription,
      parameters: Type.Object({ ...common, ...priced, deviations: Type.Number({ exclusiveMinimum: 0, maximum: 10 }) }, strict), replay: "safe",
      structuredOutputSchema: resultSchema("bollinger", Type.Object({ period, deviations: Type.Number(), source }), Type.Object({ lower: Type.Number(), middle: Type.Number(), upper: Type.Number() })),
      execute: (args, api, context) => run(args, { type: "bollinger", period: args.period, deviations: args.deviations, source: args.source }, api, context),
    }),
    defineTool({
      name: "ta_atr", description: "Average true range from candle high/low/close, with explicit period and selectable smoothing (wilder default, sma, ema, wma). No scalar price-source override." + commonDescription,
      parameters: Type.Object({ ...common, ...smoothed }, strict), replay: "safe",
      structuredOutputSchema: resultSchema("atr", smoothParameters, Type.Number()),
      execute: (args, api, context) => run(args, { type: "atr", period: args.period, smoothing: args.smoothing }, api, context),
    }),
    defineTool({
      name: "ta_adx", description: "Average directional index from candle high/low/close, with explicit period and selectable smoothing (wilder default, sma, ema, wma). Returns ADX strength values, not a directional trade recommendation. No scalar price-source override." + commonDescription,
      parameters: Type.Object({ ...common, ...smoothed }, strict), replay: "safe",
      structuredOutputSchema: resultSchema("adx", smoothParameters, Type.Number()),
      execute: (args, api, context) => run(args, { type: "adx", period: args.period, smoothing: args.smoothing }, api, context),
    }),
  ];
  return defineExtension({ name: "ta", tools, sections: [section("technical_analysis", (input) => "Use the selected ta_* tool for each indicator. Periods/deviations are explicit, never a hidden fixed preset. All parameters used are echoed in the result. Use a common fixed start/end to compare indicators on the same closed-candle range; these requests share bounded short-lived candle caching. " + seedHistoryGuidance + " Results follow trading-signals conventions and may differ from other platforms. Code Mode can compose independent calls; there is no multi-indicator tool. " + (input.agent.tools.some((tool) => tool.name === "codemode") ? technicalEvidenceInstructions : "Code Mode is not selected; do not claim to execute scripts. Use only available direct tools. Report each dataset's actual input/output bounds and counts, coverage, parameters and stability separately. Values entries are {time, value}; Bollinger/MACD fields live inside point.value. Preserve raw timestamp units if no UTC conversion was computed; never guess dates or timezone offsets. Check numeric relationships against the returned values before writing prose; describe missing or unstable evidence as unknown."))] });
}
