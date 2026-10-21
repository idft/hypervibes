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

/** Cancelling a consumer must not cancel a shared public-market-data request. */
function waitForSignal<T>(task: Promise<T>, signal?: AbortSignal): Promise<T> {
  if (!signal) return task;
  if (signal.aborted) {
    void task.catch(() => {}); // The abandoned caller must still observe rejection.
    return Promise.reject(signal.reason);
  }
  return new Promise<T>((resolve, reject) => {
    const abort = () => { signal.removeEventListener("abort", abort); reject(signal.reason); };
    signal.addEventListener("abort", abort, { once: true });
    void task.then((value) => { signal.removeEventListener("abort", abort); resolve(value); }, (error: unknown) => { signal.removeEventListener("abort", abort); reject(error); });
  });
}

/** Bounded ephemeral market data; history is shared across viewers, never persisted. */
export class MarketCharts {
  private readonly info: InfoClient;
  private readonly abort = new AbortController();
  private readonly feeds = new Map<string, Feed>();
  private readonly tasks = new Set<Promise<unknown>>();
  private readonly cache = new Map<string, { expires: number; task: Promise<ChartHistory> }>();
  private readonly researchCache = new Map<string, { expires: number; task: Promise<ChartCandle[]> }>();
  private universe?: { expires: number; task: Promise<{ symbol: string; sizeDecimals: number }[]> };
  constructor(transport: HttpTransport, private readonly factory: CandleFeedFactory) { this.info = new InfoClient({ transport }); }
  private admit() { if (this.abort.signal.aborted) throw new HyperliquidError("Market data service is closed", 503); }
  private track<T>(task: Promise<T>): Promise<T> {
    this.tasks.add(task);
    void task.finally(() => this.tasks.delete(task)).catch(() => {});
    return task;
  }
  instruments(): Promise<string[]> {
    return this.instrumentDetails().then((items) => items.map((item) => item.symbol));
  }
  instrumentDetails(): Promise<{ symbol: string; sizeDecimals: number }[]> {
    this.admit();
    if (this.universe && this.universe.expires > Date.now()) return this.universe.task;
    const task = this.track(this.info.meta({ dex: "" }, this.abort.signal).then((meta) => meta.universe.filter((asset) => !asset.isDelisted)
      .map((asset) => ({ symbol: asset.name, sizeDecimals: asset.szDecimals })).sort((a, b) => a.symbol.localeCompare(b.symbol))));
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
  /** Research pages have a fixed exclusive end and never include forming candles.
   * Hyperliquid retains only the most recent ~5,000 candles per instrument/interval.
   */
  async researchHistory(coin: string, interval: string, start: number, end: number, limit: number, cancellation?: AbortSignal) {
    cancellation?.throwIfAborted();
    const selection = await waitForSignal(this.selection(coin, interval), cancellation);
    const width = intervalMillis[selection.interval];
    if (![start, end, limit].every(Number.isSafeInteger) || start < 0 || end <= start || end > Date.now() || limit < 1 || limit > 5000) throw new HyperliquidError("Invalid research candle range");
    const boundary = Math.min(Math.floor(end / width) * width, Math.floor(Date.now() / width) * width);
    const pageStart = Math.ceil(start / width) * width;
    const pageEnd = Math.min(boundary, pageStart + limit * width);
    const key = JSON.stringify([selection.coin, selection.interval, pageStart, pageEnd]);
    let cached = this.researchCache.get(key);
    if (!cached || cached.expires <= Date.now()) {
      if (this.tasks.size >= 12) throw new HyperliquidError("Market data is busy", 429);
      const task = pageEnd <= pageStart ? Promise.resolve([]) : this.track(this.info.candleSnapshot({ ...selection, startTime: pageStart, endTime: pageEnd - 1 }, AbortSignal.any([this.abort.signal, AbortSignal.timeout(15_000)]))
        .then((raw) => normalize(raw, selection).filter((row) => row.time * 1000 >= pageStart && row.time * 1000 + width <= pageEnd)));
      if (this.researchCache.size >= 16) this.researchCache.delete(this.researchCache.keys().next().value!);
      cached = { expires: Date.now() + 60_000, task };
      this.researchCache.set(key, cached);
      void task.catch(() => { if (this.researchCache.get(key)?.task === task) this.researchCache.delete(key); });
    }
    const items = (await waitForSignal(cached.task, cancellation)).slice(0, limit).map((candle) => ({ ...candle }));
    const expected = Math.max(0, (pageEnd - pageStart) / width);
    return { coin, interval, start, end: boundary, pageEnd, candles: items, nextStart: pageEnd < boundary ? pageEnd : null,
      coverage: { complete: items.length === expected, returned: items.length, expected, sourceLimit: "Hyperliquid retains approximately the latest 5,000 candles; empty/gapped data is not proof of no trading.", closedOnly: true } };
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
    this.researchCache.clear();
  }
}
