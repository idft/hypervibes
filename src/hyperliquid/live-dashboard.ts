import { HttpTransport, SubscriptionClient, WebSocketTransport } from "@nktkas/hyperliquid";
import { dashboardOrders, dashboardPerps, dashboardSpot, loadAgentDashboard, type AgentDashboard } from "./agent-dashboard.js";

type Section = "perps" | "spot" | "orders";
type Patch = Partial<Pick<AgentDashboard, "perpEquity" | "positions" | "spotUsdc" | "spotUsdcHold" | "orders">>;
export interface AccountFeedEvents {
  connected(): void;
  disconnected(): void;
  update(section: Section, patch: Patch): void;
  failed(): void;
}
export type AccountFeedFactory = (address: string, events: AccountFeedEvents) => { close(): Promise<void> };

/** One read-only connection per linked account, never per browser tab. */
export const openAccountFeed: AccountFeedFactory = (address, events) => {
  const transport = new WebSocketTransport();
  const client = new SubscriptionClient({ transport });
  const abort = new AbortController();
  const connected = () => events.connected();
  const disconnected = () => events.disconnected();
  transport.socket.addEventListener("open", connected);
  transport.socket.addEventListener("close", disconnected);
  const user = address as `0x${string}`;
  const options = { signal: abort.signal, onError: () => events.failed() };
  const task = Promise.all([
    client.clearinghouseState({ user, dex: "" }, (event) => {
      if (event.user.toLowerCase() === address && event.dex === "") events.update("perps", dashboardPerps(event.clearinghouseState));
    }, options),
    client.spotState({ user }, (event) => {
      if (event.user.toLowerCase() === address) events.update("spot", dashboardSpot(event.spotState));
    }, options),
    client.openOrders({ user, dex: "" }, (event) => {
      if (event.user.toLowerCase() === address && event.dex === "") events.update("orders", dashboardOrders(event.orders));
    }, options),
  ]).catch(() => { if (!abort.signal.aborted) events.failed(); });
  return { async close() {
    transport.socket.removeEventListener("open", connected);
    transport.socket.removeEventListener("close", disconnected);
    abort.abort();
    transport.close();
    await task;
  } };
};

const sections: Section[] = ["perps", "spot", "orders"];
interface Account {
  snapshot: AgentDashboard;
  listeners: Set<(snapshot: AgentDashboard) => void>;
  revision: Record<Section, number>;
  generation: number;
  abort: AbortController;
  feed?: ReturnType<AccountFeedFactory>;
  retry?: ReturnType<typeof setTimeout>;
}

/** Ephemeral latest-state projection, not a journal or reliable event queue. */
export class LiveDashboards {
  private readonly accounts = new Map<string, Account>();
  private stopped = false;
  private readonly tasks = new Set<Promise<void>>();
  constructor(
    private readonly transport = new HttpTransport(),
    private readonly factory: AccountFeedFactory = openAccountFeed,
  ) {}

  snapshot(address: string): AgentDashboard {
    return structuredClone(this.ensure(address.toLowerCase()).snapshot);
  }

  watch(address: string, signal: AbortSignal, publish: (snapshot: AgentDashboard) => void): Promise<void> {
    if (signal.aborted) return Promise.resolve();
    const account = this.ensure(address.toLowerCase());
    return new Promise((resolve) => {
      const listener = () => publish(structuredClone(account.snapshot));
      const finish = () => {
        account.listeners.delete(listener);
        signal.removeEventListener("abort", finish);
        account.abort.signal.removeEventListener("abort", finish);
        resolve();
      };
      account.listeners.add(listener);
      signal.addEventListener("abort", finish, { once: true });
      account.abort.signal.addEventListener("abort", finish, { once: true });
      listener();
    });
  }

  async reconcile(addresses: string[]) {
    if (this.stopped) return;
    const wanted = new Set(addresses.map((address) => address.toLowerCase()));
    for (const address of wanted) this.ensure(address);
    for (const [address, account] of this.accounts) {
      if (wanted.has(address)) continue;
      this.accounts.delete(address);
      await this.stop(account);
    }
  }

  private ensure(address: string): Account {
    if (this.stopped) throw new Error("Live account service is closed");
    const existing = this.accounts.get(address);
    if (existing) return existing;
    const account: Account = {
      snapshot: {
        address, fetchedAt: Date.now(), perpEquity: null, spotUsdc: null, positions: null, orders: null, errors: [],
        live: { status: "connecting", sections: {
          perps: { updatedAt: null, fresh: false }, spot: { updatedAt: null, fresh: false }, orders: { updatedAt: null, fresh: false },
        } },
      },
      listeners: new Set(), revision: { perps: 0, spot: 0, orders: 0 }, generation: 0, abort: new AbortController(),
    };
    this.accounts.set(address, account);
    this.connect(account);
    return account;
  }

  private publish(account: Account) {
    for (const listener of account.listeners) listener(account.snapshot);
  }

  private connect(account: Account) {
    if (account.abort.signal.aborted) return;
    const generation = ++account.generation;
    let epoch = 0;
    const update = (section: Section, patch: Patch) => {
      Object.assign(account.snapshot, patch);
      account.revision[section]++;
      const now = Date.now();
      account.snapshot.fetchedAt = now;
      account.snapshot.live!.sections[section] = { updatedAt: now, fresh: true };
      this.publish(account);
    };
    const active = () => !account.abort.signal.aborted && generation === account.generation;
    const invalidate = () => {
      ++epoch;
      account.snapshot.live!.status = "reconnecting";
      for (const section of sections) account.snapshot.live!.sections[section].fresh = false;
      this.publish(account);
    };
    const failed = () => {
      if (!active()) return;
      ++account.generation;
      invalidate();
      const feed = account.feed;
      account.feed = undefined;
      if (feed) this.track(feed.close());
      account.retry = setTimeout(() => this.connect(account), 5_000);
      account.retry.unref();
    };
    try {
      account.feed = this.factory(account.snapshot.address!, {
        connected: () => {
          if (!active()) return;
          account.snapshot.live!.status = "connected";
          this.publish(account);
          // HTTP repairs initial/reconnect state. Revisions prevent a slow HTTP
          // response from overwriting a newer WebSocket section.
          const connectionEpoch = ++epoch;
          const revision = { ...account.revision };
          const refresh = loadAgentDashboard(this.transport, account.snapshot.address, account.abort.signal).then((snapshot) => {
            if (!active() || connectionEpoch !== epoch || account.snapshot.live!.status !== "connected") return;
            for (const section of sections) {
              if (account.revision[section] !== revision[section]) continue;
              if (section === "perps" && snapshot.perpEquity !== null && snapshot.positions !== null) update(section, { perpEquity: snapshot.perpEquity, positions: snapshot.positions });
              if (section === "spot" && snapshot.spotUsdc !== null) update(section, { spotUsdc: snapshot.spotUsdc, spotUsdcHold: snapshot.spotUsdcHold ?? null });
              if (section === "orders" && snapshot.orders !== null) update(section, { orders: snapshot.orders });
            }
            this.publish(account);
          }).catch(() => { /* Unknown sections remain explicitly stale. */ });
          this.track(refresh);
        },
        disconnected: () => { if (active()) invalidate(); },
        update: (section, patch) => { if (active()) update(section, patch); },
        failed,
      });
    } catch { failed(); }
  }

  private track(task: Promise<void>) {
    const tracked = task.finally(() => this.tasks.delete(tracked));
    this.tasks.add(tracked);
  }

  private async stop(account: Account) {
    clearTimeout(account.retry);
    account.snapshot.live!.status = "stopped";
    for (const section of sections) account.snapshot.live!.sections[section].fresh = false;
    this.publish(account);
    account.abort.abort();
    await account.feed?.close();
  }

  async close() {
    this.stopped = true;
    await Promise.all([...this.accounts.values()].map((account) => this.stop(account)));
    this.accounts.clear();
    await Promise.all(this.tasks);
  }
}
