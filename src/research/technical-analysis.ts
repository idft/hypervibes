import { ADX, ATR, BollingerBands, DEMA, EMA, MACD, RSI, SMA, WMA, WSMA } from "trading-signals";
import type { ChartCandle } from "../hyperliquid/chart-contracts.js";
import { AgentError } from "../agents/agent-input.js";

export type PriceSource = "open" | "high" | "low" | "close" | "hl2" | "hlc3" | "ohlc4" | "volume";
export type Smoothing = "wilder" | "sma" | "ema" | "wma";
export type MacdAverage = "ema" | "dema";
export type IndicatorRequest =
  | { type: "sma" | "ema"; period: number; source?: PriceSource }
  | { type: "rsi"; period: number; source?: PriceSource; smoothing?: Smoothing }
  | { type: "atr" | "adx"; period: number; smoothing?: Smoothing }
  | { type: "bollinger"; period: number; deviations: number; source?: PriceSource }
  | { type: "macd"; fastPeriod: number; slowPeriod: number; signalPeriod: number; average?: MacdAverage; signalAverage?: MacdAverage; source?: PriceSource };

const sources: Record<PriceSource, (candle: ChartCandle) => number> = {
  open: (c) => c.open, high: (c) => c.high, low: (c) => c.low, close: (c) => c.close, volume: (c) => c.volume,
  hl2: (c) => c.high / 2 + c.low / 2,
  hlc3: (c) => c.high / 3 + c.low / 3 + c.close / 3,
  ohlc4: (c) => c.open / 4 + c.high / 4 + c.low / 4 + c.close / 4,
};
const smoothers = { wilder: WSMA, sma: SMA, ema: EMA, wma: WMA };
const macdAverages = { ema: EMA, dema: DEMA };
function period(value: number) {
  if (!Number.isInteger(value) || value < 2 || value > 500) throw new AgentError("Indicator periods must be integers from 2 to 500");
}
export function validateIndicatorRequest(request: IndicatorRequest) {
  if (request.type === "macd") {
    period(request.fastPeriod); period(request.slowPeriod); period(request.signalPeriod);
    if (request.fastPeriod >= request.slowPeriod) throw new AgentError("MACD fastPeriod must be smaller than slowPeriod");
    if (!Object.hasOwn(macdAverages, request.average ?? "ema") || !Object.hasOwn(macdAverages, request.signalAverage ?? "ema")) throw new AgentError("Unsupported MACD average");
  } else period(request.period);
  if ("smoothing" in request && !Object.hasOwn(smoothers, request.smoothing ?? "wilder")) throw new AgentError("Unsupported smoothing method");
  if ("source" in request && !Object.hasOwn(sources, request.source ?? "close")) throw new AgentError("Unsupported input source");
  if (request.type === "bollinger" && (!Number.isFinite(request.deviations) || request.deviations <= 0 || request.deviations > 10)) throw new AgentError("Bollinger deviations must be greater than 0 and at most 10");
}
function collect<Input, Value extends number | object>(candles: ChartCandle[], indicator: { isStable: boolean; getRequiredInputs(): number; update(input: Input, replace: boolean): Value | null }, input: (c: ChartCandle) => Input, count: number) {
  const values = candles.map((candle) => {
    const updated = indicator.update(input(candle), false);
    const value = indicator.isStable ? updated : null;
    if (value !== null && !(typeof value === "number" ? Number.isFinite(value) : Object.values(value).every((number) => typeof number === "number" && Number.isFinite(number)))) throw new AgentError("Indicator produced a non-finite value; inspect the input range");
    return { time: candle.time, value };
  });
  return { requiredInputs: indicator.getRequiredInputs(), stable: indicator.isStable, values: values.slice(-count) };
}

/** Exactly one calculation with explicit parameters; no hidden multi-indicator defaults.
 * Library seed/warm-up conventions apply to this range. Numbers are analytical,
 * never financial authority for order sizing or accounting.
 */
export function technicalAnalysis(candles: ChartCandle[], request: IndicatorRequest, count = 1) {
  if (candles.length > 5000 || !Number.isInteger(count) || count < 1 || count > 500) throw new AgentError("TA allows up to 5,000 input candles and 1–500 output values");
  validateIndicatorRequest(request);
  const smoothing = "smoothing" in request ? request.smoothing ?? "wilder" : "wilder";
  const Smoother = smoothers[smoothing];
  if (request.type === "atr" || request.type === "adx") {
    const indicator = request.type === "atr" ? new ATR(request.period, Smoother) : new ADX(request.period, Smoother);
    return { indicator: request.type, parameters: { period: request.period, smoothing }, ...collect(candles, indicator, (c) => c, count) };
  }
  const source = "source" in request ? request.source ?? "close" : "close";
  const input = (c: ChartCandle) => {
    const value = sources[source](c);
    if (!Number.isFinite(value)) throw new AgentError("Non-finite indicator input");
    return value;
  };
  switch (request.type) {
    case "sma": case "ema": {
      const indicator = request.type === "sma" ? new SMA(request.period) : new EMA(request.period);
      return { indicator: request.type, parameters: { period: request.period, source }, ...collect(candles, indicator, input, count) };
    }
    case "rsi":
      return { indicator: request.type, parameters: { period: request.period, source, smoothing }, ...collect(candles, new RSI(request.period, Smoother), input, count) };
    case "bollinger":
      return { indicator: request.type, parameters: { period: request.period, deviations: request.deviations, source }, ...collect(candles, new BollingerBands(request.period, request.deviations), input, count) };
    case "macd": {
      const average = request.average ?? "ema", signalAverage = request.signalAverage ?? "ema";
      const Average = macdAverages[average], Signal = macdAverages[signalAverage];
      const indicator = new MACD(new Average(request.fastPeriod), new Average(request.slowPeriod), new Signal(request.signalPeriod));
      return { indicator: request.type, parameters: { fastPeriod: request.fastPeriod, slowPeriod: request.slowPeriod, signalPeriod: request.signalPeriod, average, signalAverage, source }, ...collect(candles, indicator, input, count) };
    }
    default: throw new AgentError("Unsupported indicator");
  }
}
