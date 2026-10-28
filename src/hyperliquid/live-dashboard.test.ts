import { HttpTransport } from "@nktkas/hyperliquid";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { LiveDashboards, type AccountFeedEvents } from "./live-dashboard.js";
import type { AgentDashboard } from "./agent-dashboard.js";
import { dashboardTotalEquity } from "./dashboard-equity.js";

const address = `0x${"ab".repeat(20)}`;
const other = `0x${"cd".repeat(20)}`;
const responses: Record<string, unknown> = {
  clearinghouseState: { marginSummary: { accountValue: "10.5" }, assetPositions: [] },
  spotClearinghouseState: { balances: [{ coin: "USDC", total: "2.5" }] },
  frontendOpenOrders: [], userAbstraction: "disabled",
};
const staging: Record<string, unknown> = {
  clearinghouseState: { marginSummary: { accountValue: "20.298475" }, assetPositions: [] },
  spotClearinghouseState: { balances: [{ coin: "USDC", total: "186.061799" }] },
  frontendOpenOrders: [], userAbstraction: "unifiedAccount",
};
interface PendingRequest { type: string; resolve: (response: Response) => void }

function resolveRequests(pending: PendingRequest[], values: Record<string, unknown>, failed?: string) {
  for (const request of pending.splice(0)) {
    request.resolve(request.type === failed ? new Response("offline", { status: 503 }) : Response.json(values[request.type]));
  }
}

describe("Backend live dashboard projection", () => {
  let live: LiveDashboards;
  let feeds: { address: string; events: AccountFeedEvents; close: ReturnType<typeof vi.fn> }[];
  let fetcher: ReturnType<typeof vi.fn>;
  let server: Record<string, unknown>;
  beforeEach(() => {
    feeds = [];
    server = { ...responses };
    fetcher = vi.fn(async (_input: unknown, init: RequestInit) => {
      const { type } = JSON.parse(String(init.body)) as { type: string };
      return Response.json(server[type]);
    });
    vi.stubGlobal("fetch", fetcher);
    live = new LiveDashboards(new HttpTransport(), (address, events) => {
      const feed = { address, events, close: vi.fn(async () => {}) };
      feeds.push(feed);
      return feed;
    });
  });
  afterEach(async () => { await live.close(); vi.unstubAllGlobals(); vi.useRealTimers(); });

  function deferRequests() {
    const pending: PendingRequest[] = [];
    fetcher.mockImplementation((_input: unknown, init: RequestInit) => new Promise<Response>((resolve) => {
      pending.push({ type: (JSON.parse(String(init.body)) as { type: string }).type, resolve });
    }));
    return pending;
  }
  async function bootstrap() {
    const initial = live.snapshot(address);
    expect(initial.accountAbstraction).toBeNull();
    expect(initial.live!.sections.mode).toEqual({ fresh: false, updatedAt: null });
    const events = feeds[0]!.events;
    events.connected();
    await vi.waitFor(() => expect(dashboardTotalEquity(live.snapshot(address))).toBe(server.userAbstraction === "unifiedAccount" ? "2.5" : "13"));
    expect(fetcher).toHaveBeenCalledTimes(8);
    return events;
  }

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
    const events = await bootstrap();
    const modeUpdatedAt = live.snapshot(address).live!.sections.mode.updatedAt;
    events.disconnected();
    expect(live.snapshot(address)).toMatchObject({ accountAbstraction: "disabled", perpEquity: "10.5", live: { status: "reconnecting", sections: { perps: { fresh: false }, spot: { fresh: false }, orders: { fresh: false }, mode: { fresh: false, updatedAt: modeUpdatedAt } } } });
    expect(dashboardTotalEquity(live.snapshot(address))).toBeNull();
    events.connected();
    await vi.waitFor(() => expect(dashboardTotalEquity(live.snapshot(address))).toBe("13"));
    expect(fetcher).toHaveBeenCalledTimes(12);
  });

  it("invalidates balances on initial mode lookup and obtains post-boundary balances in one follow-up repair", async () => {
    const pending = deferRequests();
    live.snapshot(address);
    const events = feeds[0]!.events;
    events.connected();
    await vi.waitFor(() => expect(pending).toHaveLength(4));
    resolveRequests(pending, responses);
    await vi.waitFor(() => expect(pending).toHaveLength(4));
    const boundary = live.snapshot(address);
    expect(boundary.accountAbstraction).toBe("disabled");
    expect(boundary.spotUsdc).toBeNull();
    expect(boundary.perpEquity).toBeNull();
    expect(boundary.live!.sections.spot.fresh).toBe(false);
    expect(boundary.live!.sections.perps.fresh).toBe(false);
    expect(dashboardTotalEquity(boundary)).toBeNull();
    resolveRequests(pending, responses);
    await vi.waitFor(() => expect(dashboardTotalEquity(live.snapshot(address))).toBe("13"));
    expect(fetcher).toHaveBeenCalledTimes(8);
  });

  it.each([
    { from: "disabled", to: "unifiedAccount", order: "balances-first" },
    { from: "disabled", to: "unifiedAccount", order: "mode-first" },
    { from: "unifiedAccount", to: "disabled", order: "balances-first" },
    { from: "unifiedAccount", to: "disabled", order: "mode-first" },
  ] as const)("repairs $from -> $to with $order updates without extra socket balances", async ({ from, to, order }) => {
    server.userAbstraction = from;
    const events = await bootstrap();
    const pending = deferRequests();
    if (order === "balances-first") {
      events.update("spot", { spotUsdc: "186.061799" });
      events.update("perps", { perpEquity: "20.298475", positions: [] });
    }
    events.update("mode", { accountAbstraction: to });
    expect(live.snapshot(address).live!.sections.spot.fresh).toBe(false);
    expect(live.snapshot(address).live!.sections.perps.fresh).toBe(false);
    expect(dashboardTotalEquity(live.snapshot(address))).toBeNull();
    if (order === "mode-first") {
      events.update("spot", { spotUsdc: "186.061799" });
      expect(dashboardTotalEquity(live.snapshot(address))).toBe(to === "unifiedAccount" ? "186.061799" : null);
      events.update("perps", { perpEquity: "20.298475", positions: [] });
    }
    await vi.waitFor(() => expect(pending).toHaveLength(4));
    resolveRequests(pending, { ...staging, userAbstraction: to });
    await vi.waitFor(() => expect(dashboardTotalEquity(live.snapshot(address))).toBe(to === "unifiedAccount" ? "186.061799" : "206.360274"));
    expect(live.snapshot(address).live!.sections.spot.fresh).toBe(true);
    expect(live.snapshot(address).live!.sections.perps.fresh).toBe(true);
    expect(fetcher).toHaveBeenCalledTimes(12);
    events.update("mode", { accountAbstraction: to });
    expect(live.snapshot(address).live!.sections.spot.fresh).toBe(true);
  });

  it("does not combine cached disabled mode with unified balances when reconnect mode HTTP fails", async () => {
    const events = await bootstrap();
    events.disconnected();
    fetcher.mockImplementation(async (_input: unknown, init: RequestInit) => {
      const { type } = JSON.parse(String(init.body)) as { type: string };
      return type === "userAbstraction" ? new Response("offline", { status: 503 }) : Response.json(staging[type]);
    });
    events.connected();
    await vi.waitFor(() => expect(live.snapshot(address).spotUsdc).toBe("186.061799"));
    const repaired = live.snapshot(address);
    expect(repaired.accountAbstraction).toBe("disabled");
    expect(repaired.perpEquity).toBe("20.298475");
    expect(repaired.live!.sections.mode.fresh).toBe(false);
    expect(repaired.live!.sections.spot.fresh).toBe(true);
    expect(repaired.live!.sections.perps.fresh).toBe(true);
    expect(dashboardTotalEquity(repaired)).toBeNull();
    const pending = deferRequests();
    events.update("mode", { accountAbstraction: "unifiedAccount" });
    expect(dashboardTotalEquity(live.snapshot(address))).toBeNull();
    await vi.waitFor(() => expect(pending).toHaveLength(4));
    resolveRequests(pending, staging);
    await vi.waitFor(() => expect(dashboardTotalEquity(live.snapshot(address))).toBe("186.061799"));
  });

  it("applies a changed HTTP mode before balances and repairs them in a follow-up request", async () => {
    const events = await bootstrap();
    events.disconnected();
    const pending = deferRequests();
    events.connected();
    await vi.waitFor(() => expect(pending).toHaveLength(4));
    resolveRequests(pending, staging);
    await vi.waitFor(() => expect(pending).toHaveLength(4));
    const boundary = live.snapshot(address);
    expect(boundary.accountAbstraction).toBe("unifiedAccount");
    expect(boundary.spotUsdc).toBe("2.5");
    expect(boundary.perpEquity).toBe("10.5");
    expect(boundary.live!.sections.spot.fresh).toBe(false);
    expect(boundary.live!.sections.perps.fresh).toBe(false);
    expect(dashboardTotalEquity(boundary)).toBeNull();
    resolveRequests(pending, staging);
    await vi.waitFor(() => expect(dashboardTotalEquity(live.snapshot(address))).toBe("186.061799"));
    expect(fetcher).toHaveBeenCalledTimes(16);
  });

  it("coalesces mode repairs and rejects delayed initial HTTP balances across socket mode transitions", async () => {
    const pending = deferRequests();
    live.snapshot(address);
    const events = feeds[0]!.events;
    events.connected();
    await vi.waitFor(() => expect(pending).toHaveLength(4));
    events.update("mode", { accountAbstraction: "disabled" });
    events.update("spot", { spotUsdc: "7" });
    events.update("perps", { perpEquity: "8", positions: [] });
    events.update("mode", { accountAbstraction: "unifiedAccount" });
    expect(fetcher).toHaveBeenCalledTimes(4);
    resolveRequests(pending, responses);
    await vi.waitFor(() => expect(pending).toHaveLength(4));
    expect(live.snapshot(address).accountAbstraction).toBe("unifiedAccount");
    expect(live.snapshot(address).spotUsdc).toBe("7");
    expect(live.snapshot(address).perpEquity).toBe("8");
    expect(live.snapshot(address).live!.sections.spot.fresh).toBe(false);
    expect(live.snapshot(address).live!.sections.perps.fresh).toBe(false);
    expect(dashboardTotalEquity(live.snapshot(address))).toBeNull();
    resolveRequests(pending, staging);
    await vi.waitFor(() => expect(dashboardTotalEquity(live.snapshot(address))).toBe("186.061799"));
    expect(fetcher).toHaveBeenCalledTimes(8);
  });

  it.each(["disabled", "default"] as const)("repairs an initial null socket mode that supersedes delayed HTTP %s mode", async (mode) => {
    const pending = deferRequests();
    live.snapshot(address);
    const events = feeds[0]!.events;
    events.connected();
    await vi.waitFor(() => expect(pending).toHaveLength(4));
    events.update("mode", { accountAbstraction: null });
    resolveRequests(pending, { ...responses, userAbstraction: mode });
    await vi.waitFor(() => expect(pending).toHaveLength(4));
    expect(live.snapshot(address).accountAbstraction).toBeNull();
    expect(dashboardTotalEquity(live.snapshot(address))).toBeNull();
    resolveRequests(pending, { ...responses, userAbstraction: mode });
    await vi.waitFor(() => expect(pending).toHaveLength(4));
    expect(live.snapshot(address).accountAbstraction).toBe(mode);
    expect(live.snapshot(address).live!.sections.spot.fresh).toBe(false);
    expect(live.snapshot(address).live!.sections.perps.fresh).toBe(false);
    expect(dashboardTotalEquity(live.snapshot(address))).toBeNull();
    resolveRequests(pending, { ...responses, userAbstraction: mode });
    await vi.waitFor(() => expect(dashboardTotalEquity(live.snapshot(address))).toBe("13"));
    expect(fetcher).toHaveBeenCalledTimes(12);
  });

  it("protects newer perps and empty orders while independently repairing spot and unchanged mode", async () => {
    const events = await bootstrap();
    events.disconnected();
    const pending = deferRequests();
    events.connected();
    await vi.waitFor(() => expect(pending).toHaveLength(4));
    events.update("perps", { perpEquity: "20", positions: [] });
    events.update("orders", { orders: [] });
    resolveRequests(pending, responses);
    await vi.waitFor(() => expect(live.snapshot(address).live!.sections.spot.fresh).toBe(true));
    expect(live.snapshot(address).perpEquity).toBe("20");
    expect(live.snapshot(address).positions).toEqual([]);
    expect(live.snapshot(address).orders).toEqual([]);
    expect(dashboardTotalEquity(live.snapshot(address))).toBe("22.5");
  });

  it("recovers known -> null -> known mode with HTTP repairs and no extra socket balances", async () => {
    const events = await bootstrap();
    const pending = deferRequests();
    events.update("mode", { accountAbstraction: null });
    expect(live.snapshot(address).live!.sections.mode.updatedAt).not.toBeNull();
    expect(live.snapshot(address).live!.sections.spot.fresh).toBe(false);
    expect(live.snapshot(address).live!.sections.perps.fresh).toBe(false);
    expect(dashboardTotalEquity(live.snapshot(address))).toBeNull();
    await vi.waitFor(() => expect(pending).toHaveLength(4));
    resolveRequests(pending, staging, "userAbstraction");
    await vi.waitFor(() => expect(live.snapshot(address).live!.sections.spot.fresh).toBe(true));
    expect(live.snapshot(address).accountAbstraction).toBeNull();
    expect(dashboardTotalEquity(live.snapshot(address))).toBeNull();
    events.update("mode", { accountAbstraction: "unifiedAccount" });
    expect(live.snapshot(address).live!.sections.spot.fresh).toBe(false);
    expect(live.snapshot(address).live!.sections.perps.fresh).toBe(false);
    expect(dashboardTotalEquity(live.snapshot(address))).toBeNull();
    await vi.waitFor(() => expect(pending).toHaveLength(4));
    resolveRequests(pending, staging);
    await vi.waitFor(() => expect(dashboardTotalEquity(live.snapshot(address))).toBe("186.061799"));
    expect(fetcher).toHaveBeenCalledTimes(16);
  });

  it("leaves failed sections unknown and retains last-known values across fatal feed failures", async () => {
    fetcher.mockResolvedValue(new Response("offline", { status: 503 }));
    live.snapshot(address);
    const events = feeds[0]!.events;
    events.connected();
    await vi.waitFor(() => expect(fetcher).toHaveBeenCalledTimes(4));
    expect(live.snapshot(address).accountAbstraction).toBeNull();
    expect(live.snapshot(address).perpEquity).toBeNull();
    expect(live.snapshot(address).live!.sections.mode).toEqual({ fresh: false, updatedAt: null });
    events.update("spot", { spotUsdc: "42" });
    vi.useFakeTimers();
    events.failed();
    expect(live.snapshot(address).live!.status).toBe("reconnecting");
    expect(live.snapshot(address).spotUsdc).toBe("42");
    await vi.advanceTimersByTimeAsync(5_000);
    expect(feeds).toHaveLength(2);
    expect(feeds[0]!.close).toHaveBeenCalledOnce();
    events.update("spot", { spotUsdc: "obsolete" });
    events.update("mode", { accountAbstraction: "unifiedAccount" });
    expect(live.snapshot(address).spotUsdc).toBe("42");
    expect(live.snapshot(address).accountAbstraction).toBeNull();
    await live.close();
    await vi.advanceTimersByTimeAsync(30_000);
    expect(feeds).toHaveLength(2);
  });

  it("stops removed accounts and closes owned feeds and watchers on shutdown", async () => {
    const events = await bootstrap();
    await live.reconcile([address, other]);
    const snapshots: AgentDashboard[] = [];
    const watcher = live.watch(address, new AbortController().signal, (snapshot) => snapshots.push(snapshot));
    await live.reconcile([other]);
    await watcher;
    expect(snapshots.at(-1)!.live!.status).toBe("stopped");
    expect(dashboardTotalEquity(snapshots.at(-1)!)).toBeNull();
    expect(feeds[0]!.close).toHaveBeenCalledOnce();
    events.update("spot", { spotUsdc: "obsolete" });
    await live.close();
    expect(feeds[1]!.close).toHaveBeenCalledOnce();
    expect(() => live.snapshot(other)).toThrow("closed");
  });
});
