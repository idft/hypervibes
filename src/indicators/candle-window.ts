import { intervalMillis, isChartInterval, type ChartCandle } from "../hyperliquid/chart-contracts.js";
import type { MarketCharts } from "../hyperliquid/market-charts.js";
import { indicatorLimits, type PineCandle } from "./contracts.js";
import { IndicatorError } from "./validation.js";

export const pineTimeframes = { "1m": "1", "5m": "5", "15m": "15", "1h": "60", "4h": "240", "1d": "1D" } as const;
export function candleWindow(start: number, end: number, interval: string, now = Date.now()) {
  if (!isChartInterval(interval) || ![start, end].every(Number.isSafeInteger) || start < 0 || end <= start || end > now) throw new IndicatorError("Invalid indicator candle window");
  const width = intervalMillis[interval];
  const effectiveEnd = Math.min(Math.floor(end / width) * width, Math.floor(now / width) * width);
  const alignedStart = Math.ceil(start / width) * width;
  const expected = (effectiveEnd - alignedStart) / width;
  if (expected < 1 || expected > indicatorLimits.candles) throw new IndicatorError("Indicator window must contain 1–5,000 closed candles");
  return { start: alignedStart, end: effectiveEnd, width, expected, pineTimeframe: pineTimeframes[interval] };
}
export function adaptCandles(candles: ChartCandle[], window: ReturnType<typeof candleWindow>, allowLeadingGap = false): { candles: PineCandle[]; seedCoverage: "complete" | "limited" } {
  if (!candles.length || candles.length > window.expected) throw new IndicatorError("Incomplete indicator candle range");
  const first = candles[0]!.time * 1000;
  if ((!allowLeadingGap && first !== window.start) || first < window.start || candles.at(-1)!.time * 1000 + window.width !== window.end) throw new IndicatorError("Incomplete indicator candle range");
  const adapted = candles.map((candle, index) => {
    if (!Object.values(candle).every(Number.isFinite) || candle.time * 1000 !== first + index * window.width || candle.volume < 0 || candle.low > Math.min(candle.open, candle.close) || candle.high < Math.max(candle.open, candle.close)) throw new IndicatorError("Invalid or gapped indicator candles");
    return { open: candle.open, high: candle.high, low: candle.low, close: candle.close, volume: candle.volume, openTime: candle.time * 1000, closeTime: candle.time * 1000 + window.width };
  });
  return { candles: adapted, seedCoverage: first === window.start ? "complete" : "limited" };
}
export async function fetchIndicatorWindow(market: Pick<MarketCharts, "researchHistory">, coin: string, interval: string, start: number, end: number, signal?: AbortSignal, chart = false) {
  const window = candleWindow(start, end, interval);
  const history = await market.researchHistory(coin, interval, window.start, window.end, indicatorLimits.candles, signal);
  signal?.throwIfAborted();
  if (history.nextStart !== null || history.end !== window.end) throw new IndicatorError("Incomplete indicator candle range");
  return { ...window, ...adaptCandles(history.candles, window, chart) };
}
