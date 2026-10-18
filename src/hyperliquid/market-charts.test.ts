import { HttpTransport } from "@nktkas/hyperliquid";
import type { CandleEvent } from "@nktkas/hyperliquid/api/subscription";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { MarketCharts } from "./market-charts.js";
import type { CandleFeedEvents } from "./shared-feeds.js";
import type { ChartSnapshot } from "./chart-contracts.js";

const selection = { coin: "BTC", interval: "1h" } as const;
const width = 3_600_000;
const raw = (t: number, close = "11"): CandleEvent => ({ t, T: t + width - 1, s: "BTC", i: "1h", o: "10", h: "15", l: "9", c: close, v: "20", n: 1 });

describe("Shared candle market data", () => {
  let charts: MarketCharts;
  let fetcher: ReturnType<typeof vi.fn>;
  let feeds: { events: CandleFeedEvents; close: ReturnType<typeof vi.fn> }[];
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(10_000 * width);
    feeds = [];
    fetcher = vi.fn(async (_url: unknown, init: RequestInit) => {
      const body = JSON.parse(String(init.body));
      if (body.type === "meta") return Response.json({ universe: [{ name: "BTC" }, { name: "ETH" }, { name: "OLD", isDelisted: true }] });
      const { startTime, endTime } = body.req;
      return Response.json(Array.from({ length: 500 }, (_, i) => raw(startTime + i * width)).filter((candle) => candle.t <= endTime));
    });
    vi.stubGlobal("fetch", fetcher);
    charts = new MarketCharts(new HttpTransport(), (_selection, events) => {
      const feed = { events, close: vi.fn(async () => {}) };
      feeds.push(feed);
      return feed;
    });
  });
  afterEach(async () => { await charts.close(); vi.unstubAllGlobals(); vi.useRealTimers(); });

  it("validates perps and intervals using a shared cached universe", async () => {
    expect(await charts.instruments()).toEqual(["BTC", "ETH"]);
    expect(await charts.selection("BTC", "1h")).toEqual(selection);
    await expect(charts.selection("@1", "1h")).rejects.toThrow("perpetual");
    await expect(charts.selection("BTC", "3m")).rejects.toThrow("selection");
    expect(fetcher).toHaveBeenCalledOnce();
  });
  it("deduplicates history requests, uses exclusive cursors, sorts and validates display data", async () => {
    const [one, two] = await Promise.all([charts.history(selection), charts.history(selection)]);
    expect(one).toEqual(two);
    expect(one.candles).toHaveLength(500);
    expect(one.hasMore).toBe(true);
    expect(fetcher).toHaveBeenCalledOnce();
    const before = one.candles[0]!.time * 1000;
    const older = await charts.history(selection, before);
    expect(older.candles.at(-1)!.time * 1000).toBe(before - width);
    expect(older.candles.every((candle) => candle.time < one.candles[0]!.time)).toBe(true);
    fetcher.mockResolvedValueOnce(Response.json([raw(width), raw(0), raw(width, "12"), { ...raw(2 * width), c: "NaN" }, { ...raw(3 * width), s: "ETH" }]));
    const normalized = await charts.history(selection, 5 * width);
    expect(normalized.candles.map((candle) => [candle.time, candle.close])).toEqual([[0, 11], [3600, 12]]);
    await expect(charts.history(selection, NaN)).rejects.toThrow("cursor");
    await expect(charts.history(selection, Date.now() + width * 2)).rejects.toThrow("cursor");
  });
  it("recognizes retention exhaustion and retries failed requests instead of caching errors", async () => {
    fetcher.mockRejectedValueOnce(new Error("offline"));
    await expect(charts.history(selection)).rejects.toThrow();
    fetcher.mockResolvedValueOnce(Response.json([]));
    expect(await charts.history(selection)).toEqual({ candles: [], hasMore: false });
    expect(fetcher).toHaveBeenCalledTimes(2);
  });
  it("research pages use fixed end boundaries, exclude forming candles, and expose gaps", async () => {
    const first = await charts.researchHistory("BTC", "1h", 9995 * width, Date.now(), 2);
    expect(first.candles.map((row) => row.time * 1000)).toEqual([9995 * width, 9996 * width]);
    expect(first.nextStart).toBe(9997 * width); expect(first.coverage.complete).toBe(true);
    const second = await charts.researchHistory("BTC", "1h", first.nextStart!, first.end, 10);
    expect(second.candles.at(-1)!.time * 1000).toBe(9999 * width);
    expect(second.nextStart).toBeNull();
    fetcher.mockResolvedValueOnce(Response.json([raw(9995 * width), raw(9997 * width)]));
    const gap = await charts.researchHistory("BTC", "1h", 9995 * width, 9998 * width, 10);
    expect(gap.coverage).toMatchObject({ complete: false, returned: 2, expected: 3, closedOnly: true });
    await expect(charts.researchHistory("BTC", "1h", 0, Date.now() + width, 10)).rejects.toThrow("range");
    await expect(charts.researchHistory("BTC", "1h", 0, Date.now(), 5001)).rejects.toThrow("range");
  });
  it("shares research fetches across indicators, isolates returned objects and refreshes expired snapshots", async () => {
    const start = 9990 * width, end = 10000 * width;
    const [one, two] = await Promise.all([charts.researchHistory("BTC", "1h", start, end, 5000), charts.researchHistory("BTC", "1h", start, end, 2000)]);
    expect(fetcher).toHaveBeenCalledTimes(2); // one meta + one identical closed window
    one.candles[0]!.close = 999;
    expect(two.candles[0]!.close).toBe(11);
    expect((await charts.researchHistory("BTC", "1h", start, end, 5000)).candles[0]!.close).toBe(11);
    expect(fetcher).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(60_001);
    await charts.researchHistory("BTC", "1h", start, end, 5000);
    const snapshots = fetcher.mock.calls.filter((call) => JSON.parse(String(call[1].body)).type === "candleSnapshot");
    expect(snapshots).toHaveLength(2);
  });
  it("cancels a research waiter without aborting another consumer's shared fetch", async () => {
    await charts.selection("BTC", "1h");
    let finish!: (response: Response) => void;
    fetcher.mockImplementationOnce(async () => new Promise<Response>((resolve) => { finish = resolve; }));
    const abort = new AbortController();
    const first = charts.researchHistory("BTC", "1h", 9990 * width, 9992 * width, 5000, abort.signal);
    const second = charts.researchHistory("BTC", "1h", 9990 * width, 9992 * width, 5000);
    await vi.waitFor(() => expect(fetcher).toHaveBeenCalledTimes(2));
    abort.abort(new Error("Stop this waiter"));
    await expect(first).rejects.toThrow("Stop this waiter");
    expect(fetcher.mock.calls[1]![1].signal?.aborted).toBe(false);
    finish(Response.json([raw(9990 * width), raw(9991 * width)]));
    expect((await second).coverage.complete).toBe(true);
    expect((await charts.researchHistory("BTC", "1h", 9990 * width, 9992 * width, 5000)).candles).toHaveLength(2);
    expect(fetcher).toHaveBeenCalledTimes(2);
  });
  it("evicts failed and excess research cache entries", async () => {
    await charts.selection("BTC", "1h");
    fetcher.mockRejectedValueOnce(new Error("offline"));
    await expect(charts.researchHistory("BTC", "1h", 9000 * width, 9001 * width, 1)).rejects.toThrow();
    expect((await charts.researchHistory("BTC", "1h", 9000 * width, 9001 * width, 1)).coverage.complete).toBe(true);
    for (let i = 1; i <= 16; i++) await charts.researchHistory("BTC", "1h", (9000 + i) * width, (9001 + i) * width, 1);
    const calls = fetcher.mock.calls.length;
    await charts.researchHistory("BTC", "1h", 9000 * width, 9001 * width, 1);
    expect(fetcher.mock.calls.length).toBe(calls + 1);
  });
  it("shares one subscription per pair, publishes isolated snapshots, and releases only after the final viewer", async () => {
    const first: ChartSnapshot[] = [];
    const second: ChartSnapshot[] = [];
    const a = new AbortController();
    const b = new AbortController();
    const tasks = [charts.watch(selection, a.signal, (snapshot) => first.push(snapshot)), charts.watch(selection, b.signal, (snapshot) => second.push(snapshot))];
    expect(feeds).toHaveLength(1);
    feeds[0]!.events.connected();
    await vi.waitFor(() => expect(first.at(-1)?.status).toBe("live"));
    feeds[0]!.events.update(raw(Math.floor(Date.now() / width) * width, "13"));
    expect(first.at(-1)!.candles.at(-1)!.close).toBe(13);
    first.at(-1)!.candles.at(-1)!.close = 999;
    expect(second.at(-1)!.candles.at(-1)!.close).toBe(13);
    a.abort();
    expect(feeds[0]!.close).not.toHaveBeenCalled();
    b.abort(); await Promise.all(tasks);
    expect(feeds[0]!.close).toHaveBeenCalledOnce();
  });
  it("keeps WebSocket updates newer than an in-flight snapshot and repairs reconnect gaps", async () => {
    let resolve!: (response: Response) => void;
    fetcher.mockImplementationOnce(() => new Promise<Response>((done) => { resolve = done; }));
    const snapshots: ChartSnapshot[] = [];
    const abort = new AbortController();
    const task = charts.watch(selection, abort.signal, (snapshot) => snapshots.push(snapshot));
    feeds[0]!.events.connected();
    feeds[0]!.events.update(raw(Date.now(), "14"));
    resolve(Response.json([raw(Date.now(), "10")]));
    await vi.waitFor(() => expect(snapshots.at(-1)?.status).toBe("live"));
    expect(snapshots.at(-1)!.candles.at(-1)!.close).toBe(14);
    feeds[0]!.events.disconnected();
    expect(snapshots.at(-1)!.status).toBe("reconnecting");
    await vi.advanceTimersByTimeAsync(3_000);
    expect(feeds[0]!.close).toHaveBeenCalledOnce();
    expect(feeds).toHaveLength(2);
    feeds[1]!.events.connected();
    await vi.waitFor(() => expect(snapshots.at(-1)?.status).toBe("live"));
    // Repaired HTTP candles replace old values, not the previous connection's stale projection.
    expect(snapshots.at(-1)!.candles.at(-1)!.close).toBe(11);
    feeds[0]!.events.update(raw(Date.now() - 3_000, "15"));
    expect(snapshots.at(-1)!.candles.at(-1)!.close).toBe(11);
    abort.abort(); await task;
  });
  it("aborts owned work and resolves viewers on shutdown", async () => {
    const task = charts.watch(selection, new AbortController().signal, () => {});
    await charts.close();
    await task;
    expect(feeds[0]!.close).toHaveBeenCalledOnce();
    expect(() => charts.instruments()).toThrow("closed");
  });
});
