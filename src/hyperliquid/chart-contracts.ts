export const chartIntervals = ["1m", "5m", "15m", "1h", "4h", "1d"] as const;
export type ChartInterval = typeof chartIntervals[number];
export const intervalMillis: Record<ChartInterval, number> = { "1m": 60_000, "5m": 300_000, "15m": 900_000, "1h": 3_600_000, "4h": 14_400_000, "1d": 86_400_000 };
export interface ChartSelection { coin: string; interval: ChartInterval }
/** Display-only numbers, never an authority for order sizing or execution. */
export interface ChartCandle { time: number; open: number; high: number; low: number; close: number; volume: number }
export interface ChartSnapshot { candles: ChartCandle[]; status: "connecting" | "live" | "reconnecting" }
export interface ChartHistory { candles: ChartCandle[]; hasMore: boolean }
export function isChartInterval(value: string): value is ChartInterval { return chartIntervals.some((interval) => interval === value); }
export function mergeChartCandles(previous: ChartCandle[], incoming: ChartCandle[]): ChartCandle[] {
  const values = new Map(previous.map((candle) => [candle.time, candle]));
  for (const candle of incoming) values.set(candle.time, candle);
  return [...values.values()].sort((a, b) => a.time - b.time);
}
