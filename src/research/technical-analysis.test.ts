import { describe, expect, it } from "vitest";
import { ADX, ATR, DEMA, EMA, MACD, RSI, SMA, WMA, WSMA } from "trading-signals";
import { technicalAnalysis, type PriceSource } from "./technical-analysis.js";

const candles = Array.from({ length: 160 }, (_, i) => {
  const close = 100 + i * 0.3 + Math.sin(i * 1.7) * 4;
  return { time: i * 3600, open: close + 0.5, high: close + 2 + i % 3, low: close - 2 - i % 2, close, volume: 100 + i % 7 };
});

describe("individual parameterized TA calculations", () => {
  it("uses explicitly chosen MACD periods and independently selected average types", () => {
    const request = { type: "macd", fastPeriod: 8, slowPeriod: 21, signalPeriod: 5, average: "dema", signalAverage: "ema", source: "open" } as const;
    const actual = technicalAnalysis(candles, request, 3);
    const reference = new MACD(new DEMA(8), new DEMA(21), new EMA(5));
    for (const candle of candles) reference.update(candle.open, false);
    expect(actual.parameters).toEqual({ fastPeriod: 8, slowPeriod: 21, signalPeriod: 5, average: "dema", signalAverage: "ema", source: "open" });
    expect(actual.values.at(-1)!.value).toEqual(reference.getResult());
    expect(actual.values).toHaveLength(3);
    const other = technicalAnalysis(candles, { type: "macd", fastPeriod: 12, slowPeriod: 26, signalPeriod: 9 });
    expect(actual.values.at(-1)!.value).not.toEqual(other.values.at(-1)!.value);
    const demaSignal = technicalAnalysis(candles, { ...request, signalAverage: "dema" });
    expect(demaSignal.values.at(-1)!.value).not.toEqual(actual.values.at(-1)!.value);
  });
  it("accepts fractional Bollinger deviations and uses population standard deviation", () => {
    const sample = [1, 2, 3].map((close, time) => ({ time, open: close, high: close, low: close, close, volume: 10 }));
    const result = technicalAnalysis(sample, { type: "bollinger", period: 3, deviations: 3.25 });
    expect(result.parameters).toEqual({ period: 3, deviations: 3.25, source: "close" });
    expect(result.values[0]!.value).toEqual({ middle: 2, lower: 2 - 3.25 * Math.sqrt(2 / 3), upper: 2 + 3.25 * Math.sqrt(2 / 3) });
    expect(result.values[0]!.value).not.toEqual(technicalAnalysis(sample, { type: "bollinger", period: 3, deviations: 2 }).values[0]!.value);
  });
  it.each(["open", "high", "low", "close", "hl2", "hlc3", "ohlc4", "volume"] as const)("honors the %s input source", (source: PriceSource) => {
    const selected = (c: typeof candles[number]) => source === "hl2" ? (c.high + c.low) / 2 : source === "hlc3" ? (c.high + c.low + c.close) / 3 : source === "ohlc4" ? (c.open + c.high + c.low + c.close) / 4 : c[source];
    const expected = candles.slice(-7).reduce((sum, candle) => sum + selected(candle), 0) / 7;
    const result = technicalAnalysis(candles, { type: "sma", period: 7, source });
    expect(result.parameters).toEqual({ period: 7, source });
    expect(result.values[0]!.value).toBeCloseTo(expected);
  });
  it.each(["wilder", "sma", "ema", "wma"] as const)("honors %s smoothing for RSI, ATR and ADX", (smoothing) => {
    const Smoother = { wilder: WSMA, sma: SMA, ema: EMA, wma: WMA }[smoothing];
    const rsi = new RSI(9, Smoother), atr = new ATR(9, Smoother), adx = new ADX(9, Smoother);
    for (const candle of candles) { rsi.update(candle.close, false); atr.update(candle, false); adx.update(candle, false); }
    for (const [type, reference] of [["rsi", rsi], ["atr", atr], ["adx", adx]] as const) {
      const result = technicalAnalysis(candles, { type, period: 9, smoothing });
      expect(result.stable).toBe(true); expect(result.values[0]!.value).toBeCloseTo(reference.getResultOrThrow());
      expect(result.parameters.smoothing).toBe(smoothing);
    }
  });
  it("reports warm-up and keeps scalar/object output shapes bounded", () => {
    expect(technicalAnalysis(candles.slice(0, 1), { type: "rsi", period: 9 })).toMatchObject({ stable: false, values: [{ value: null }] });
    expect(technicalAnalysis(candles.slice(0, 1), { type: "macd", fastPeriod: 8, slowPeriod: 21, signalPeriod: 5 })).toMatchObject({ stable: false, values: [{ value: null }] });
    expect(technicalAnalysis(candles, { type: "ema", period: 11 }, 10).values).toHaveLength(10);
    expect(() => technicalAnalysis(candles, { type: "sma", period: 7 }, 501)).toThrow("output values");
  });
  it("rejects invalid periods, MACD ordering, deviation multipliers and non-finite results", () => {
    for (const period of [0, 1, 2.5, 501, Infinity]) expect(() => technicalAnalysis(candles, { type: "sma", period })).toThrow("periods");
    expect(() => technicalAnalysis(candles, { type: "macd", fastPeriod: 21, slowPeriod: 8, signalPeriod: 5 })).toThrow("smaller");
    for (const deviations of [0, -2, 11, Infinity, NaN]) expect(() => technicalAnalysis(candles, { type: "bollinger", period: 20, deviations })).toThrow("deviations");
    const enormous = candles.map((candle) => ({ ...candle, close: Number.MAX_VALUE }));
    expect(() => technicalAnalysis(enormous, { type: "sma", period: 7 })).toThrow("non-finite");
  });
});
