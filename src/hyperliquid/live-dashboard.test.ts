import { HttpTransport } from "@nktkas/hyperliquid";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { LiveDashboards, type AccountFeedEvents } from "./live-dashboard.js";
import type { AgentDashboard } from "./agent-dashboard.js";

const address = `0x${"ab".repeat(20)}`;
const other = `0x${"cd".repeat(20)}`;
const responses: Record<string, unknown> = {
  clearinghouseState: { marginSummary: { accountValue: "10.5" }, assetPositions: [] },
  spotClearinghouseState: { balances: [{ coin: "USDC", total: "2.5" }] },
  frontendOpenOrders: [],
};

describe("Backend live dashboard projection", () => {
  let live: LiveDashboards;
  let feeds: { address: string; events: AccountFeedEvents; close: ReturnType<typeof vi.fn> }[];
  let fetcher: ReturnType<typeof vi.fn>;
  beforeEach(() => {
    feeds = [];
    fetcher = vi.fn(async (_input: unknown, init: RequestInit) => {
      const { type } = JSON.parse(String(init.body)) as { type: string };
      return Response.json(responses[type]);
    });
    vi.stubGlobal("fetch", fetcher);
    live = new LiveDashboards(new HttpTransport(), (address, events) => {
      const feed = { address, events, close: vi.fn(async () => {}) };
      feeds.push(feed);
      return feed;
    });
  });
  afterEach(async () => { await live.close(); vi.unstubAllGlobals(); vi.useRealTimers(); });

  it("starts independently of browsers, shares account feeds, sends cached snapshots immediately and isolates accounts", async () => {
    expect(feeds).toHaveLength(0);
    await live.reconcile([address, address.toUpperCase(), other]);
    expect(feeds).toHaveLength(2);
    feeds[0]!.events.update("perps", { perpEquity: "9007199254740993.123456", positions: [] });
    const first: AgentDashboard[] = [];
    const second: AgentDashboard[] = [];
    const abort = new AbortController();
    const watchers = [live.watch(address, abort.signal, (snapshot) => first.push(snapshot)), live.watch(address, abort.signal, (snapshot) => second.push(snapshot))];
    expect(first[0]!.perpEquity).toBe("9007199254740993.123456");
    expect(second).toEqual(first);
    feeds[0]!.events.update("spot", { spotUsdc: "7" });
    expect(first.at(-1)!.spotUsdc).toBe("7");
    expect(live.snapshot(other).spotUsdc).toBeNull();
    first.at(-1)!.spotUsdc = "tampered";
    expect(live.snapshot(address).spotUsdc).toBe("7");
    abort.abort(); await Promise.all(watchers);
    expect(feeds[0]!.close).not.toHaveBeenCalled();
    feeds[0]!.events.update("spot", { spotUsdc: "8" });
    expect(live.snapshot(address).spotUsdc).toBe("8");
    expect(first).toHaveLength(2);
    expect(fetcher).not.toHaveBeenCalled();
  });

  it("bootstraps and repairs reconnect gaps with HTTP while retaining stale values on disconnect", async () => {
    live.snapshot(address);
    const events = feeds[0]!.events;
    events.connected();
    await vi.waitFor(() => expect(live.snapshot(address).spotUsdc).toBe("2.5"));
    expect(live.snapshot(address).live!.sections.perps.fresh).toBe(true);
    events.disconnected();
    expect(live.snapshot(address)).toMatchObject({ perpEquity: "10.5", live: { status: "reconnecting", sections: { perps: { fresh: false }, spot: { fresh: false }, orders: { fresh: false } } } });
    events.connected();
    await vi.waitFor(() => expect(fetcher).toHaveBeenCalledTimes(6));
    await vi.waitFor(() => expect(live.snapshot(address).live!.sections.orders.fresh).toBe(true));
  });

  it("never lets delayed HTTP overwrite newer socket state, including empty order/position snapshots", async () => {
    const pending: { type: string; resolve: (response: Response) => void }[] = [];
    fetcher.mockImplementation((_input: unknown, init: RequestInit) => new Promise<Response>((resolve) => {
      pending.push({ type: (JSON.parse(String(init.body)) as { type: string }).type, resolve });
    }));
    live.snapshot(address);
    const events = feeds[0]!.events;
    events.connected();
    await vi.waitFor(() => expect(pending).toHaveLength(3));
    events.update("perps", { perpEquity: "20", positions: [] });
    events.update("orders", { orders: [] });
    for (const request of pending) request.resolve(Response.json(responses[request.type]));
    await vi.waitFor(() => expect(live.snapshot(address).spotUsdc).toBe("2.5"));
    expect(live.snapshot(address).perpEquity).toBe("20");
    expect(live.snapshot(address).positions).toEqual([]);
    expect(live.snapshot(address).orders).toEqual([]);
  });

  it("leaves failed sections unknown and retains last-known values across fatal feed failures", async () => {
    fetcher.mockResolvedValue(new Response("offline", { status: 503 }));
    live.snapshot(address);
    const events = feeds[0]!.events;
    events.connected();
    await vi.waitFor(() => expect(fetcher).toHaveBeenCalledTimes(3));
    expect(live.snapshot(address).perpEquity).toBeNull();
    expect(live.snapshot(address).live!.sections.perps.fresh).toBe(false);
    events.update("spot", { spotUsdc: "42" });
    vi.useFakeTimers();
    events.failed();
    expect(live.snapshot(address).live!.status).toBe("reconnecting");
    expect(live.snapshot(address).spotUsdc).toBe("42");
    await vi.advanceTimersByTimeAsync(5_000);
    expect(feeds).toHaveLength(2);
    expect(feeds[0]!.close).toHaveBeenCalledOnce();
    events.update("spot", { spotUsdc: "obsolete" });
    expect(live.snapshot(address).spotUsdc).toBe("42");
    await live.close();
    await vi.advanceTimersByTimeAsync(30_000);
    expect(feeds).toHaveLength(2);
  });

  it("stops removed accounts and closes owned feeds and watchers on shutdown", async () => {
    await live.reconcile([address, other]);
    const watcher = live.watch(address, new AbortController().signal, () => {});
    await live.reconcile([other]);
    await watcher;
    expect(feeds[0]!.close).toHaveBeenCalledOnce();
    await live.close();
    expect(feeds[1]!.close).toHaveBeenCalledOnce();
    expect(() => live.snapshot(other)).toThrow("closed");
  });
});
