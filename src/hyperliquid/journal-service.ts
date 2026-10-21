import { enqueueAccountEvent } from "../agents/worker-events.js";
import { InfoClient, SubscriptionClient, WebSocketTransport, type HttpTransport } from "@nktkas/hyperliquid";
import type { OrderUpdatesEvent } from "@nktkas/hyperliquid/api/subscription";
import { randomUUID } from "node:crypto";
import type { SharedDatabase } from "../db/shared-database.js";
import type { FastifyBaseLogger } from "fastify";
import { HyperliquidError } from "./errors.js";
import { amount } from "./journal-decimal.js";
import { chartFillRange, type ChartFillRange, type ChartFillsPage } from "../charts/trading-contracts.js";
import type { BalanceHistory, BalancePoint } from "./agent-dashboard.js";
import { buildTrades, fillEvent, fundingEvent, ledgerEvent, wsFundingEvent, type JournalEvent, type JournalStream, type JournalTrade } from "./journal-model.js";

export type HistoryFetcher = (account: string, stream: JournalStream, start: number, end: number, signal: AbortSignal) => Promise<JournalEvent[]>;

export interface JournalFeedEvents {
  connected(): void;
  disconnected(): void;
  failed(): void;
  push(stream: JournalStream, events: JournalEvent[]): void;
  orders?(events: OrderUpdatesEvent): void;
}
export type JournalFeedFactory = (address: string, events: JournalFeedEvents) => { close(): Promise<void> };

/** Subscribes one account to exchange push events on the shared read-only socket. */
export const openJournalFeed = (address: string, events: JournalFeedEvents, sharedTransport?: WebSocketTransport): ReturnType<JournalFeedFactory> => {
  const transport = sharedTransport ?? new WebSocketTransport();
  const client = new SubscriptionClient({ transport });
  const abort = new AbortController();
  const connected = () => events.connected();
  const disconnected = () => events.disconnected();
  transport.socket.addEventListener("open", connected);
  transport.socket.addEventListener("close", disconnected);
  if (transport.socket.readyState === 1) queueMicrotask(() => { if (!abort.signal.aborted) connected(); });
  const user = address as `0x${string}`;
  const options = { signal: abort.signal, onError: () => events.failed() };
  // A malformed push invalidates the whole batch; HTTP catch-up re-verifies it.
  const emit = <T>(stream: JournalStream, map: (row: T) => JournalEvent, rows: T[]) => {
    if (rows.length === 0) return;
    try { events.push(stream, rows.map(map)); } catch { events.failed(); }
  };
  const tasks = [
    client.orderUpdates({ user }, (updates) => { if (!abort.signal.aborted) events.orders?.(updates); }, options),
    client.userFills({ user, aggregateByTime: false }, (event) => {
      if (event.user.toLowerCase() === address) emit("fill", fillEvent, event.fills);
    }, options),
    client.userFundings({ user }, (event) => {
      if (event.user.toLowerCase() === address) emit("funding", wsFundingEvent, event.fundings);
    }, options),
    client.userNonFundingLedgerUpdates({ user }, (event) => {
      if (event.user.toLowerCase() === address) emit("ledger", (row) => ledgerEvent(address, row), event.nonFundingLedgerUpdates);
    }, options),
  ];
  for (const task of tasks) void task.catch(() => { if (!abort.signal.aborted) events.failed(); });
  return { async close() {
    transport.socket.removeEventListener("open", connected);
    transport.socket.removeEventListener("close", disconnected);
    abort.abort();
    if (!sharedTransport) transport.close();
    const subscriptions = await Promise.allSettled(tasks);
    await Promise.allSettled(subscriptions.flatMap((result) => result.status === "fulfilled" ? [result.value.unsubscribe()] : []));
  } };
};

const FULL_SCAN_MS = 86_400_000;
const SYNC_RETRY_MS = 30_000;
const FEED_RETRY_MS = 5_000;

const sleep = (ms: number, signal: AbortSignal) => new Promise<void>((resolve) => {
  const finish = () => { clearTimeout(timer); signal.removeEventListener("abort", finish); resolve(); };
  const timer = setTimeout(finish, ms);
  timer.unref();
  signal.addEventListener("abort", finish, { once: true });
});
export interface JournalNote { id: string; body: string; authorId: string; createdAt: number }
export interface JournalPage {
  linked: boolean; view: "trades" | "activity"; page: number; pages: number; total: number;
  trades: (JournalTrade & { noteCount: number })[];
  activity: (Omit<JournalEvent, "raw"> & { balance: string | null; noteCount: number })[];
  sync: { stream: string; cursor: number; updated_at: number | null; error: string | null }[];
  pending: boolean; historyLimited: boolean;
}
interface EntryRow { id: string; kind: string; time: number; data: string; balance: string | null; noteCount: number }
interface AccountRow { revision: number; projected_revision: number; history_limited: number }

export function exchangeHistory(transport: HttpTransport): HistoryFetcher {
  const info = new InfoClient({ transport });
  return async (account, stream, startTime, endTime, cancellation) => {
    const signal = AbortSignal.any([cancellation, AbortSignal.timeout(10_000)]);
    const params = { user: account as `0x${string}`, startTime, endTime };
    if (stream === "fill") return (await info.userFillsByTime({ ...params, aggregateByTime: false, reversed: false }, signal)).map(fillEvent);
    if (stream === "funding") return (await info.userFunding(params, signal)).map(fundingEvent);
    return (await info.userNonFundingLedgerUpdates(params, signal)).map((row) => ledgerEvent(account, row));
  };
}

/** Backend-owned, single-flight account ingestion. No exchange I/O in SQLite transactions. */
export class JournalService {
  private readonly workers = new Map<string, Worker>();
  private stopped = false;
  private readonly shutdown = new AbortController();
  private readonly syncs = new Map<string, Promise<void>>();
  private readonly catchups = new Map<string, Promise<void>>();
  private readonly eventTasks = new Set<Promise<void>>();
  constructor(private readonly db: SharedDatabase, private readonly fetchHistory: HistoryFetcher, private readonly log?: FastifyBaseLogger, private readonly feedFactory?: JournalFeedFactory) {}

  async reconcile(addresses: string[]) {
    if (this.stopped) return;
    const retiring: Promise<unknown>[] = [];
    const desired = new Set(addresses.map((a) => a.toLowerCase()));
    if (desired.size > 0 && this.workers.size === 0) this.log?.info({ accounts: desired.size }, "Synchronizing trade journals");
    for (const [account, worker] of this.workers) if (!desired.has(account)) {
      worker.controller.abort();
      clearTimeout(worker.feedTimer);
      clearTimeout(worker.dailyTimer);
      const feed = worker.feed;
      retiring.push((async () => {
        await Promise.all([feed?.close(), worker.task, worker.chain]);
        worker.feed = undefined;
        if (this.workers.get(account) === worker) this.workers.delete(account);
      })());
    }
    for (const account of desired) {
      if (this.workers.has(account)) continue;
      this.workers.set(account, this.createWorker(account));
    }
    await Promise.all(retiring);
  }

  /** One worker per account: an initial and daily full repair scan, exchange
   * push ingestion, and a catch-up scan on every feed (re)connection. */
  private createWorker(account: string): Worker {
    const controller = new AbortController();
    const signal = controller.signal;
    const worker: Worker = { controller, chain: Promise.resolve() };
    const healthy = async (fullScan: boolean): Promise<boolean> => {
      try {
        await this.syncAccount(account, signal, Date.now(), fullScan);
        const rows = await this.db.all<{ error: string | null }>("SELECT error FROM hv_journal_sync WHERE account=?", account);
        return rows.length === 3 && rows.every((row) => row.error === null);
      } catch (error: unknown) {
        if (!signal.aborted) this.log?.error({ err: error, account }, "Unable to synchronize trade journal; retrying");
        return false;
      }
    };
    const sync = (fullScan: boolean) => {
      worker.chain = worker.chain.then(async () => {
        let ok = await healthy(fullScan);
        while (!ok && !signal.aborted) {
          await sleep(SYNC_RETRY_MS, signal);
          if (signal.aborted) return;
          ok = await healthy(false);
        }
      });
      return worker.chain;
    };
    const ingestPush = async (stream: JournalStream, events: JournalEvent[]) => {
      if (signal.aborted || events.length === 0) return;
      try {
        if (await this.ingest(account, stream, events)) {
          await this.project(account);
          this.log?.debug({ account, stream, count: events.length }, "Ingested pushed trade journal events");
        }
      } catch (error: unknown) {
        if (!signal.aborted) this.log?.error({ err: error, account }, "Unable to ingest pushed trade journal events");
      }
    };
    const openFeed = () => {
      if (signal.aborted || worker.feed || !this.feedFactory) return;
      try {
        worker.feed = this.feedFactory(account, {
          connected: () => { if (!signal.aborted) void sync(false); },
          disconnected: () => {},
          failed: reopen,
          push: (stream, events) => { this.trackEvent(ingestPush(stream, events)); },
          orders: (updates) => { if (!signal.aborted) this.trackEvent(this.ingestOrderUpdates(account, updates)); },
        });
      } catch { reopen(); }
    };
    const reopen = () => {
      if (signal.aborted || !this.feedFactory) return;
      clearTimeout(worker.feedTimer);
      const feed = worker.feed;
      worker.feed = undefined;
      void feed?.close();
      worker.feedTimer = setTimeout(() => { worker.feedTimer = undefined; openFeed(); }, FEED_RETRY_MS);
      worker.feedTimer.unref();
    };
    const scheduleDaily = () => {
      if (signal.aborted) return;
      worker.dailyTimer = setTimeout(() => {
        worker.dailyTimer = undefined;
        void sync(true).then(scheduleDaily);
      }, FULL_SCAN_MS);
      worker.dailyTimer.unref();
    };
    worker.task = sync(true).then(() => {
      if (signal.aborted) return;
      openFeed();
      scheduleDaily();
    });
    return worker;
  }

  private trackEvent(task: Promise<unknown>) {
    const tracked = task.then(() => {}, (error: unknown) => { this.log?.error(error, "Unable to persist exchange event"); }).finally(() => this.eventTasks.delete(tracked));
    this.eventTasks.add(tracked);
  }

  async ingestOrderUpdates(account: string, updates: OrderUpdatesEvent) {
    await this.db.transaction(async (tx) => {
      for (const update of updates) {
        // Fills have their own deduplicated journal trigger; routine acknowledgments do not wake a model.
        if (["open", "filled", "triggered"].includes(update.status)) continue;
        await enqueueAccountEvent(tx, account, `order:${update.order.oid}:${update.status}:${update.statusTimestamp}`, "order_status", `Order status change: ${JSON.stringify(update)}. Inspect current account/orders and decide whether action is needed. Do not blindly replace canceled or rejected orders.`, update.statusTimestamp);
      }
    });
  }

  async close() {
    this.stopped = true;
    this.shutdown.abort();
    const workers = [...this.workers.values()];
    for (const worker of workers) {
      worker.controller.abort();
      clearTimeout(worker.feedTimer);
      clearTimeout(worker.dailyTimer);
    }
    await Promise.all(workers.map(async (worker) => {
      await Promise.allSettled([worker.task, worker.chain]);
      const feed = worker.feed;
      worker.feed = undefined;
      if (feed) await feed.close().catch(() => {});
    }));
    this.workers.clear();
    await Promise.allSettled([...this.syncs.values()]);
    await Promise.all(this.eventTasks);
  }

  /** Shared with background repair: never overlap HTTP scans of one account. */
  syncAccount(account: string, signal: AbortSignal, now = Date.now(), fullScan = false): Promise<void> {
    account = account.toLowerCase();
    const key = account;
    const task = (this.syncs.get(key) ?? Promise.resolve()).catch(() => {}).then(async () => {
      signal.throwIfAborted();
      await this.synchronize(key, signal, now, fullScan);
    });
    this.syncs.set(key, task);
    const clean = () => { if (this.syncs.get(key) === task) this.syncs.delete(key); };
    void task.then(clean, clean);
    return task;
  }

  /** Bounded, single-flight catch-up. Failure leaves previously ingested evidence
   * readable; the caller still checks actual coverage, never assumes success. */
  async ensureThrough(account: string, end: number, cancellation?: AbortSignal) {
    if (!Number.isSafeInteger(end) || end < 0 || end > Date.now()) return { attempted: false, limitation: "Review boundary is invalid or in the future; coverage is not guaranteed" };
    account = account.toLowerCase();
    if (this.stopped) return { attempted: false, limitation: "Journal is shutting down" };
    const rows = await this.db.all<{ cursor: number; error: string | null; updated_at: number | null }>("SELECT cursor,error,updated_at FROM hv_journal_sync WHERE account=?", account);
    if (rows.length === 3 && rows.every((row) => row.cursor >= end && row.updated_at !== null && !row.error)) {
      await this.project(account);
      return { attempted: false, limitation: null };
    }
    const deadline = AbortSignal.any([this.shutdown.signal, AbortSignal.timeout(20_000)]);
    let task = this.catchups.get(account);
    if (!task) {
      task = this.syncAccount(account, deadline);
      this.catchups.set(account, task);
      const key = account, work = task;
      const clean = () => { if (this.catchups.get(key) === work) this.catchups.delete(key); };
      void work.then(clean, clean);
    }
    const signal = AbortSignal.any([deadline, ...(cancellation ? [cancellation] : [])]);
    let abort: () => void = () => {};
    try {
      await Promise.race([task, new Promise<never>((_resolve, reject) => {
        abort = () => reject(signal.reason);
        signal.addEventListener("abort", abort, { once: true });
        if (signal.aborted) abort();
      })]);
      return { attempted: true, limitation: null };
    } catch {
      cancellation?.throwIfAborted();
      return { attempted: true, limitation: "Journal catch-up did not finish within its deadline; inspect coverage and disclose incomplete evidence" };
    } finally { signal.removeEventListener("abort", abort); }
  }

  private async synchronize(account: string, signal: AbortSignal, now: number, fullScan: boolean) {
    await this.db.run("INSERT INTO hv_journal_accounts(account) VALUES (?) ON CONFLICT DO NOTHING", account);
    for (const stream of ["fill", "funding", "ledger"] as const) {
      if (signal.aborted) return;
      await this.db.run("INSERT INTO hv_journal_sync(account,stream) VALUES (?,?) ON CONFLICT DO NOTHING", account, stream);
      const state = await this.db.get<{ cursor: number; updated_at: number | null }>("SELECT cursor,updated_at FROM hv_journal_sync WHERE account=? AND stream=?", account, stream);
      // Wide funding windows repair sparse hourly results; fills/ledger use a
      // short overlap. Startup and daily full rescans repair older corrections.
      // Per-page checkpoints survive crashes without skipping source data.
      const overlap = stream === "funding" ? 7 * 86_400_000 : 120_000;
      const start = fullScan ? 0 : Math.max(0, (state?.cursor ?? 0) - overlap);
      try {
        await this.window(account, stream, start, now, signal);
        if (stream === "fill" && state?.updated_at === null) {
          const count = await this.db.get<{ total: number }>("SELECT count(*) AS total FROM hv_journal_entries WHERE account=? AND kind='fill'", account);
          // The API exposes only the latest 10,000 fills. Conservatively flag an
          // initial import reaching that boundary; do not assert a full balance.
          if ((count?.total ?? 0) >= 10_000) await this.db.run("UPDATE hv_journal_accounts SET history_limited=1 WHERE account=?", account);
        }
        await this.db.run("UPDATE hv_journal_sync SET updated_at=?,error=NULL WHERE account=? AND stream=?", now, account, stream);
      } catch (error) {
        if (signal.aborted) return;
        const message = error instanceof HistoryCoverageError ? error.message : "History sync failed; retrying. Previously imported data is retained.";
        await this.db.run("UPDATE hv_journal_sync SET error=? WHERE account=? AND stream=?", message, account, stream);
      }
    }
    if (!signal.aborted) {
      this.log?.debug({ account }, "Synchronized trade journal");
      await this.project(account);
    }
  }

  private async window(account: string, stream: JournalStream, start: number, end: number, signal: AbortSignal): Promise<void> {
    signal.throwIfAborted();
    const rows = await this.fetchHistory(account, stream, start, end, signal);
    if (rows.some((e) => e.kind !== stream || !Number.isSafeInteger(e.time) || e.time < start || e.time > end)) throw new Error("Invalid history response");
    // Time endpoints cap either elements or distinct blocks. Conservatively split
    // at 500, even for fills (whose element cap is 2000). Never skip timestamp ties.
    if (rows.length >= 500) {
      if (start === end) {
        await this.ingest(account, stream, rows, start);
        throw new HistoryCoverageError("Exchange history is saturated at one timestamp; coverage is incomplete.");
      }
      const middle = start + Math.floor((end - start) / 2);
      await this.window(account, stream, start, middle, signal);
      await this.window(account, stream, middle + 1, end, signal);
      return;
    }
    signal.throwIfAborted();
    await this.ingest(account, stream, rows, end);
  }

  async ingest(account: string, stream: JournalStream, events: JournalEvent[], cursor?: number): Promise<boolean> {
    account = account.toLowerCase();
    return this.db.transaction(async (tx) => {
      await tx.run("INSERT INTO hv_journal_accounts(account) VALUES (?) ON CONFLICT DO NOTHING", account);
      let changed = false;
      for (const event of events) {
        const data = JSON.stringify(event);
        const previous = await tx.get<{ data: string }>("SELECT data FROM hv_journal_entries WHERE account=? AND id=?", account, event.id);
        if (previous?.data === data) continue;
        changed = true;
        await tx.run(`INSERT INTO hv_journal_entries(account,id,kind,time,data) VALUES (?,?,?,?,?)
          ON CONFLICT(account,id) DO UPDATE SET data=excluded.data,time=excluded.time`, account, event.id, event.kind, event.time, data);
        if (!previous && event.kind === "fill") await enqueueAccountEvent(tx, account, `fill:${event.id}`, "fill", `Exchange fill ${event.id}: ${JSON.stringify({ instrument: event.symbol, time: event.time, fill: event.fill })}. Refresh account and orders, inspect actual position size and TP/SL coverage. A fill is evidence, not a new-entry instruction.`, event.time);
      }
      if (changed) await tx.run("UPDATE hv_journal_accounts SET revision=revision+1 WHERE account=?", account);
      // Pushed events never advance cursors; only fetched windows checkpoint progress.
      if (cursor !== undefined) await tx.run(`INSERT INTO hv_journal_sync(account,stream,cursor) VALUES (?,?,?)
        ON CONFLICT(account,stream) DO UPDATE SET cursor=max(cursor,excluded.cursor)`, account, stream, cursor);
      return changed;
    });
  }

  async project(account: string) {
    const snapshot = await this.db.transaction(async (tx) => {
      const state = await tx.get<AccountRow>("SELECT revision,projected_revision FROM hv_journal_accounts WHERE account=?", account);
      if (!state || state.revision === state.projected_revision) return null;
      return { revision: state.revision, rows: await tx.all<EntryRow>("SELECT id,kind,time,data,balance FROM hv_journal_entries WHERE account=? AND kind<>'trade' ORDER BY time,id", account) };
    });
    if (!snapshot) return;
    const events = snapshot.rows.map((row) => JSON.parse(row.data) as JournalEvent);
    const trades = buildTrades(account, events);
    let balance: ReturnType<typeof amount> | null = amount("0");
    const balances = events.map((event) => {
      balance = balance === null || event.delta === null ? null : balance.add(amount(event.delta));
      return { id: event.id, balance: balance?.toString() ?? null };
    });
    await this.db.transaction(async (tx) => {
      const current = await tx.get<AccountRow>("SELECT revision,projected_revision FROM hv_journal_accounts WHERE account=?", account);
      if (current?.revision !== snapshot.revision) return; // Leave durable dirty revision for retry.
      await tx.run("UPDATE hv_journal_entries SET active=0 WHERE account=? AND kind='trade'", account);
      for (const trade of trades) await tx.run(`INSERT INTO hv_journal_entries(account,id,kind,time,data) VALUES (?,?,'trade',?,?)
        ON CONFLICT(account,id) DO UPDATE SET time=excluded.time,data=excluded.data,active=1`, account, trade.id, trade.lastActivityAt, JSON.stringify(trade));
      for (const row of balances) await tx.run("UPDATE hv_journal_entries SET balance=? WHERE account=? AND id=?", row.balance, account, row.id);
      await tx.run("UPDATE hv_journal_accounts SET projected_revision=? WHERE account=?", snapshot.revision, account);
    });
  }

  /** Realized USDC balance, including transfers, fees and funding; not equity.
   * Carry the preceding balance into each window and sample hourly/daily closes. */
  async balanceHistory(account: string, now = Date.now()): Promise<BalanceHistory | null> {
    account = account.toLowerCase();
    return this.db.transaction(async (tx) => {
      const state = await tx.get<AccountRow>("SELECT revision,projected_revision,history_limited FROM hv_journal_accounts WHERE account=?", account);
      const sync = await tx.all<{ updated_at: number | null; error: string | null }>("SELECT updated_at,error FROM hv_journal_sync WHERE account=?", account);
      if (!state || state.revision !== state.projected_revision || state.history_limited || sync.length !== 3 || sync.some((s) => s.updated_at === null || s.error)) return null;
      const start = now - 30 * 86_400_000;
      const previous = await tx.get<{ balance: string | null }>("SELECT balance FROM hv_journal_entries WHERE account=? AND kind<>'trade' AND time<=? ORDER BY time DESC,id DESC LIMIT 1", account, start);
      const rows = await tx.all<{ time: number; balance: string | null }>("SELECT time,balance FROM hv_journal_entries WHERE account=? AND kind<>'trade' AND time>? AND time<=? ORDER BY time,id", account, start, now);
      if (previous?.balance === null || rows.some((row) => row.balance === null)) return null;
      const sample = (duration: number, bucket: number): BalancePoint[] => {
        let balance = previous?.balance ?? "0";
        let index = 0;
        const points: BalancePoint[] = [];
        for (let time = now - duration; time <= now; time += bucket) {
          while (index < rows.length && rows[index]!.time <= time) balance = rows[index++]!.balance!;
          points.push({ time, balance });
        }
        return points;
      };
      return { day: sample(86_400_000, 3_600_000), month: sample(30 * 86_400_000, 86_400_000) };
    });
  }

  async page(account: string | null, view: "trades" | "activity", requestedPage: number): Promise<JournalPage> {
    const empty: JournalPage = { linked: account !== null, view, page: 1, pages: 1, total: 0, trades: [], activity: [], sync: [], pending: true, historyLimited: false };
    if (!account) return { ...empty, pending: false };
    account = account.toLowerCase();
    return this.db.transaction(async (tx) => {
      const filter = view === "trades" ? "kind='trade' AND active=1" : "kind<>'trade'";
      const count = await tx.get<{ total: number }>(`SELECT count(*) AS total FROM hv_journal_entries WHERE account=? AND ${filter}`, account);
      const total = count?.total ?? 0; const pages = Math.max(1, Math.ceil(total / 25)); const page = Math.max(1, Math.min(pages, requestedPage));
      const rows = await tx.all<EntryRow>(`SELECT e.*, (SELECT count(*) FROM hv_journal_notes n WHERE n.account=e.account AND n.target=e.id) AS noteCount
        FROM hv_journal_entries e WHERE account=? AND ${filter} ORDER BY time DESC,id DESC LIMIT 25 OFFSET ?`, account, (page - 1) * 25);
      const sync = await tx.all<JournalPage["sync"][number]>("SELECT stream,cursor,updated_at,error FROM hv_journal_sync WHERE account=? ORDER BY stream", account);
      const state = await tx.get<AccountRow>("SELECT revision,projected_revision,history_limited FROM hv_journal_accounts WHERE account=?", account);
      const result = { ...empty, page, pages, total, sync, pending: !state || state.revision !== state.projected_revision, historyLimited: state?.history_limited === 1 };
      if (view === "trades") result.trades = rows.map((row) => ({ ...JSON.parse(row.data) as JournalTrade, noteCount: row.noteCount }));
      else result.activity = rows.map((row) => {
        const { raw, ...event } = JSON.parse(row.data) as JournalEvent;
        void raw; // Exchange payloads stay local; only normalized fields cross the UI boundary.
        return { ...event, balance: row.balance, noteCount: row.noteCount };
      });
      // Funding may be numerically zero but is not authoritative until its stream
      // has finished backfill. Avoid reporting a partial import as final net P&L.
      if (sync.length < 3 || sync.some((s) => !s.updated_at || s.error) || result.pending) {
        result.trades = result.trades.map((trade) => ({ ...trade, funding: null, net: null }));
        result.activity = result.activity.map((event) => ({ ...event, balance: null }));
      }
      if (result.historyLimited) result.activity = result.activity.map((event) => ({ ...event, balance: null }));
      return result;
    });
  }
  /** Raw fills, not reconstructed trade segments (a position flip is one fill).
   * Reads ingested journal evidence only; never initiates exchange requests. */
  async chartFills(account: string, coin: string, range: ChartFillRange): Promise<ChartFillsPage> {
    const { from, to, cursor } = chartFillRange({ ...range });
    account = account.toLowerCase();
    return this.db.transaction(async (tx) => {
      const rows = await tx.all<{ id: string; time: number; side: string; size: string; price: string }>(`SELECT id,time,
        json_extract(data,'$.fill.side') AS side, json_extract(data,'$.fill.size') AS size,
        json_extract(data,'$.fill.price') AS price FROM hv_journal_entries
        WHERE account=? AND kind='fill' AND time>=? AND time<? AND json_extract(data,'$.symbol')=?
          AND json_extract(data,'$.fill.side') IN ('B','A','buy','sell')
          AND (? IS NULL OR time<? OR (time=? AND id<?))
        ORDER BY time DESC,id DESC LIMIT 501`, account, from, to, coin, cursor?.time ?? null, cursor?.time ?? null, cursor?.time ?? null, cursor?.id ?? null);
      const state = await tx.get<{ history_limited: number }>("SELECT history_limited FROM hv_journal_accounts WHERE account=?", account);
      const sync = await tx.get<{ updatedAt: number | null; error: string | null }>("SELECT updated_at AS updatedAt,error FROM hv_journal_sync WHERE account=? AND stream='fill'", account);
      const page = rows.slice(0, 500), last = page.at(-1);
      return {
        fills: page.map((row) => ({ ...row, side: row.side === "B" || row.side === "buy" ? "Buy" as const : "Sell" as const })),
        nextCursor: rows.length > 500 && last ? { time: last.time, id: last.id } : null,
        historyLimited: state?.history_limited === 1,
        sync: sync ?? null,
      };
    });
  }

  async notes(account: string, target: string) {
    account = account.toLowerCase();
    const found = await this.db.get("SELECT id FROM hv_journal_entries WHERE account=? AND id=?", account, target);
    if (!found) throw new HyperliquidError("Journal entry not found", 404);
    return this.db.all<JournalNote>("SELECT id,body,author_id AS authorId,created_at AS createdAt FROM hv_journal_notes WHERE account=? AND target=? ORDER BY created_at,id", account, target);
  }
  async addNote(account: string, target: string, authorId: string, body: string) {
    account = account.toLowerCase();
    body = body.trim();
    if (!body || body.length > 4000) throw new HyperliquidError("Notes must contain 1–4000 characters.");
    await this.db.transaction(async (tx) => {
      const found = await tx.get("SELECT id FROM hv_journal_entries WHERE account=? AND id=?", account, target);
      if (!found) throw new HyperliquidError("Journal entry not found", 404);
      await tx.run("INSERT INTO hv_journal_notes(id,account,target,author_id,body,created_at) VALUES (?,?,?,?,?,?)", randomUUID(), account, target, authorId, body, Date.now());
    });
  }
}
class HistoryCoverageError extends Error {}
interface Worker {
  controller: AbortController;
  /** Serialized HTTP sync attempts; also drains retry backoff while unhealthy. */
  chain: Promise<void>;
  feed?: { close(): Promise<void> };
  feedTimer?: ReturnType<typeof setTimeout>;
  dailyTimer?: ReturnType<typeof setTimeout>;
  task?: Promise<void>;
}
