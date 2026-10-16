import { InfoClient, type HttpTransport } from "@nktkas/hyperliquid";
import { randomUUID } from "node:crypto";
import type { SharedDatabase } from "../db/shared-database.js";
import { HyperliquidError } from "./errors.js";
import { amount } from "./journal-decimal.js";
import { buildTrades, fillEvent, fundingEvent, ledgerEvent, type JournalEvent, type JournalStream, type JournalTrade } from "./journal-model.js";

export type HistoryFetcher = (account: string, stream: JournalStream, start: number, end: number, signal: AbortSignal) => Promise<JournalEvent[]>;
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
  private readonly workers = new Map<string, { controller: AbortController; task?: Promise<void>; timer?: ReturnType<typeof setTimeout> }>();
  private stopped = false;
  constructor(private readonly db: SharedDatabase, private readonly fetchHistory: HistoryFetcher) {}

  reconcile(addresses: string[]) {
    if (this.stopped) return;
    const desired = new Set(addresses.map((a) => a.toLowerCase()));
    for (const [account, worker] of this.workers) if (!desired.has(account)) {
      worker.controller.abort(); clearTimeout(worker.timer);
      void worker.task?.finally(() => { if (this.workers.get(account) === worker) this.workers.delete(account); });
    }
    for (const account of desired) {
      if (this.workers.has(account)) continue;
      const worker: { controller: AbortController; task?: Promise<void>; timer?: ReturnType<typeof setTimeout> } = { controller: new AbortController() };
      this.workers.set(account, worker);
      let lastFullScan = 0;
      const refresh = () => {
        const now = Date.now();
        const fullScan = now - lastFullScan >= 86_400_000;
        if (fullScan) lastFullScan = now;
        worker.task = this.syncAccount(account, worker.controller.signal, now, fullScan)
          .catch(() => { if (!worker.controller.signal.aborted) console.error("Unable to synchronize trade journal; retrying"); })
          .finally(() => {
            if (worker.controller.signal.aborted) { this.workers.delete(account); return; }
            worker.timer = setTimeout(refresh, 15_000); worker.timer.unref();
          });
      };
      refresh();
    }
  }
  async close() {
    this.stopped = true;
    for (const worker of this.workers.values()) { worker.controller.abort(); clearTimeout(worker.timer); }
    await Promise.all([...this.workers.values()].map((w) => w.task)); this.workers.clear();
  }

  async syncAccount(account: string, signal: AbortSignal, now = Date.now(), fullScan = false) {
    account = account.toLowerCase();
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
    if (!signal.aborted) await this.project(account);
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

  async ingest(account: string, stream: JournalStream, events: JournalEvent[], cursor: number) {
    await this.db.transaction(async (tx) => {
      await tx.run("INSERT INTO hv_journal_accounts(account) VALUES (?) ON CONFLICT DO NOTHING", account);
      let changed = false;
      for (const event of events) {
        const data = JSON.stringify(event);
        const previous = await tx.get<{ data: string }>("SELECT data FROM hv_journal_entries WHERE account=? AND id=?", account, event.id);
        if (previous?.data === data) continue;
        changed = true;
        await tx.run(`INSERT INTO hv_journal_entries(account,id,kind,time,data) VALUES (?,?,?,?,?)
          ON CONFLICT(account,id) DO UPDATE SET data=excluded.data,time=excluded.time`, account, event.id, event.kind, event.time, data);
      }
      if (changed) await tx.run("UPDATE hv_journal_accounts SET revision=revision+1 WHERE account=?", account);
      await tx.run(`INSERT INTO hv_journal_sync(account,stream,cursor) VALUES (?,?,?)
        ON CONFLICT(account,stream) DO UPDATE SET cursor=max(cursor,excluded.cursor)`, account, stream, cursor);
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
