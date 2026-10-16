import { HttpTransport, InfoClient } from "@nktkas/hyperliquid";
import type { CandleEvent } from "@nktkas/hyperliquid/api/subscription";
import { HyperliquidError } from "./errors.js";
import { intervalMillis, isChartInterval, mergeChartCandles, type ChartCandle, type ChartHistory, type ChartSelection, type ChartSnapshot } from "./chart-contracts.js";
import type { CandleFeedFactory } from "./shared-feeds.js";

const pageSize = 500;
interface Feed {
  snapshot: ChartSnapshot;
  listeners: Set<(snapshot: ChartSnapshot) => void>;
  abort: AbortController;
  handle?: ReturnType<CandleFeedFactory>;
  retry?: ReturnType<typeof setTimeout>;
  updates: Map<number, ChartCandle>;
  generation: number;
}

function normalize(raw: CandleEvent[], selection: ChartSelection): ChartCandle[] {
  const candles: ChartCandle[] = [];
  for (const row of raw) {
    if (row.s !== selection.coin || row.i !== selection.interval) continue;
    const candle = { time: row.t / 1000, open: Number(row.o), high: Number(row.h), low: Number(row.l), close: Number(row.c), volume: Number(row.v) };
    if (!Number.isSafeInteger(row.t) || row.t < 0 || row.t % intervalMillis[selection.interval] !== 0
      || !Object.values(candle).every(Number.isFinite) || candle.volume < 0 || candle.low <= 0
      || candle.low > Math.min(candle.open, candle.close) || candle.high < Math.max(candle.open, candle.close)) continue;
    candles.push(candle);
  }
  return mergeChartCandles([], candles);
}

/** Bounded ephemeral market data; history is shared across viewers, never persisted. */
export class MarketCharts {
  private readonly info: InfoClient;
  private readonly abort = new AbortController();
  private readonly feeds = new Map<string, Feed>();
  private readonly tasks = new Set<Promise<unknown>>();
  private readonly cache = new Map<string, { expires: number; task: Promise<ChartHistory> }>();
  private universe?: { expires: number; task: Promise<string[]> };
  constructor(transport: HttpTransport, private readonly factory: CandleFeedFactory) { this.info = new InfoClient({ transport }); }
  private admit() { if (this.abort.signal.aborted) throw new HyperliquidError("Market data service is closed", 503); }
  private track<T>(task: Promise<T>): Promise<T> {
    this.tasks.add(task);
    void task.finally(() => this.tasks.delete(task)).catch(() => {});
    return task;
  }
  instruments(): Promise<string[]> {
    this.admit();
    if (this.universe && this.universe.expires > Date.now()) return this.universe.task;
    const task = this.track(this.info.meta({}, this.abort.signal).then((meta) => meta.universe.filter((asset) => !asset.isDelisted).map((asset) => asset.name).sort()));
    this.universe = { expires: Date.now() + 60_000, task };
    void task.catch(() => { if (this.universe?.task === task) this.universe = undefined; });
    return task;
  }
  async selection(coin: string, interval: string): Promise<ChartSelection> {
    this.admit();
    if (!isChartInterval(interval) || !coin || coin.length > 64) throw new HyperliquidError("Invalid chart selection");
    if (!(await this.instruments()).includes(coin)) throw new HyperliquidError("Select an available perpetual instrument");
    return { coin, interval };
  }
  async history(selection: ChartSelection, before?: number): Promise<ChartHistory> {
    this.admit();
    if (before !== undefined && (!Number.isSafeInteger(before) || before <= 0 || before > Date.now() + intervalMillis[selection.interval])) throw new HyperliquidError("Invalid history cursor");
    const width = intervalMillis[selection.interval];
    // The cursor is an exclusive candle-open timestamp, in milliseconds.
    const endTime = before === undefined ? Math.floor(Date.now() / width) * width : Math.floor((before - 1) / width) * width;
    const startTime = Math.max(0, endTime - (pageSize - 1) * width);
    const key = `${selection.coin}/${selection.interval}/${endTime}`;
    const cached = this.cache.get(key);
    if (cached && cached.expires > Date.now()) return cached.task;
    if (this.tasks.size >= 12) throw new HyperliquidError("Market data is busy. Please retry.", 429);
    const task = this.track(this.info.candleSnapshot({ ...selection, startTime, endTime }, this.abort.signal).then((raw) => {
      const candles = normalize(raw, selection).filter((candle) => candle.time * 1000 >= startTime && candle.time * 1000 <= endTime).slice(-pageSize);
      return { candles, hasMore: candles.length > 0 && startTime > 0 };
    }));
    if (this.cache.size >= 64) this.cache.delete(this.cache.keys().next().value!);
    this.cache.set(key, { expires: Date.now() + (before === undefined ? 2_000 : 60_000), task });
    void task.catch(() => { if (this.cache.get(key)?.task === task) this.cache.delete(key); });
    return task;
  }
  watch(selection: ChartSelection, signal: AbortSignal, publish: (snapshot: ChartSnapshot) => void): Promise<void> {
    this.admit();
    if (signal.aborted) return Promise.resolve();
    const key = `${selection.coin}/${selection.interval}`;
    let feed = this.feeds.get(key);
    if (!feed) {
      if (this.feeds.size >= 40) throw new HyperliquidError("Too many active charts. Please retry.", 429);
      feed = { snapshot: { candles: [], status: "connecting" }, listeners: new Set(), abort: new AbortController(), updates: new Map(), generation: 0 };
      this.feeds.set(key, feed);
    }
    const current = feed;
    const task = new Promise<void>((resolve) => {
      const listener = (snapshot: ChartSnapshot) => publish(structuredClone(snapshot));
      const finish = () => {
        current.listeners.delete(listener);
        signal.removeEventListener("abort", finish);
        current.abort.signal.removeEventListener("abort", finish);
        if (!current.listeners.size && this.feeds.get(key) === current) {
          this.feeds.delete(key);
          void this.stop(current);
        }
        resolve();
      };
      current.listeners.add(listener);
      signal.addEventListener("abort", finish, { once: true });
      current.abort.signal.addEventListener("abort", finish, { once: true });
      listener(current.snapshot);
      if (!current.handle) this.open(current, selection);
    });
    return task;
  }
  private publish(feed: Feed) { for (const listener of feed.listeners) listener(feed.snapshot); }
  private open(feed: Feed, selection: ChartSelection) {
    const generation = ++feed.generation;
    feed.updates.clear();
    const active = () => !feed.abort.signal.aborted && feed.generation === generation;
    const repair = async () => {
      const history = await this.history(selection);
      if (!active()) return;
      // HTTP must never overwrite newer websocket values for the forming candle.
      feed.snapshot.candles = mergeChartCandles(history.candles, [...feed.updates.values()]).slice(-pageSize);
      feed.snapshot.status = "live";
      this.publish(feed);
    };
    const disconnected = () => { if (active()) { ++feed.generation; feed.snapshot.status = "reconnecting"; this.publish(feed); this.retry(feed, selection); } };
    try {
      feed.handle = this.factory(selection, {
        connected: () => { if (active()) void this.track(repair()).catch(disconnected); },
        disconnected,
        failed: disconnected,
        update: (raw) => {
          if (!active()) return;
          const candles = normalize([raw], selection);
          if (!candles.length) return;
          for (const candle of candles) feed.updates.set(candle.time, candle);
          if (feed.updates.size > pageSize) feed.updates.delete(feed.updates.keys().next().value!);
          feed.snapshot.candles = mergeChartCandles(feed.snapshot.candles, candles).slice(-pageSize);
          this.publish(feed);
        },
      });
    } catch { disconnected(); }
  }
  private retry(feed: Feed, selection: ChartSelection) {
    if (feed.retry || feed.abort.signal.aborted) return;
    feed.retry = setTimeout(() => {
      feed.retry = undefined;
      const old = feed.handle;
      feed.handle = undefined;
      void this.track((async () => {
        await old?.close();
        if (!feed.abort.signal.aborted) this.open(feed, selection);
      })()).catch(() => this.retry(feed, selection));
    }, 3_000);
    feed.retry.unref();
  }
  private stop(feed: Feed) {
    feed.abort.abort();
    clearTimeout(feed.retry);
    return this.track(feed.handle?.close() ?? Promise.resolve());
  }
  async close() {
    this.abort.abort();
    const feeds = [...this.feeds.values()];
    this.feeds.clear();
    for (const feed of feeds) void this.stop(feed);
    await Promise.allSettled(this.tasks);
    this.cache.clear();
  }
}
