import { drizzle } from "drizzle-orm/node-sqlite";
import { migrate } from "drizzle-orm/node-sqlite/migrator";
import { randomUUID } from "node:crypto";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { openDatabase } from "../db/database.js";
import { applicationDatabase, SharedDatabase } from "../db/shared-database.js";
import { JournalService, type HistoryFetcher, type JournalFeedEvents } from "./journal-service.js";
import { fillEvent, type JournalEvent } from "./journal-model.js";
import { HyperliquidService } from "./service.js";

const account = `0x${"12".repeat(20)}`;
const otherAccount = `0x${"34".repeat(20)}`;
const signal = () => new AbortController().signal;
function deposit(id: number, delta = "1"): JournalEvent {
  return { id: `deposit-${id}`, time: id, kind: "ledger", type: "deposit", symbol: "USDC", fee: null, pnl: null, delta, fill: null, raw: { id } };
}
function fill(tid: number, startPosition = "0", side: "B" | "A" = "B") {
  return fillEvent({ coin: "BTC", hash: `0x${tid}`, tid, oid: tid, time: tid, side, sz: "1", px: "100", startPosition, fee: "0.1", feeToken: "USDC", closedPnl: side === "A" ? "10" : "0", dir: "", crossed: false, twapId: null });
}

describe("durable account journal", () => {
  let directory: string;
  let db: SharedDatabase;
  let journal: JournalService;
  let rows: JournalEvent[];
  let fetcher: ReturnType<typeof vi.fn<HistoryFetcher>>;
  beforeEach(async () => {
    directory = mkdtempSync(join(tmpdir(), "hv-journal-"));
    const connection = openDatabase(join(directory, "app.sqlite"));
    migrate(drizzle({ client: connection }), { migrationsFolder: fileURLToPath(new URL("../db/migrations/", import.meta.url)), migrationsTable: "hv_schema_migrations" });
    db = new SharedDatabase(connection);
    await db.run("INSERT INTO hv_users(id,name,email) VALUES ('owner','Owner','owner@example.test'), ('other','Other','other@example.test')");
    rows = [];
    fetcher = vi.fn(async (_account, stream, start, end) => rows.filter((e) => e.kind === stream && e.time >= start && e.time <= end).slice(0, 500));
    journal = new JournalService(db, fetcher);
  });
  afterEach(async () => { await journal.close(); await db.close(); vi.useRealTimers(); vi.restoreAllMocks(); rmSync(directory, { recursive: true, force: true }); });

  it("paginates chart fills by stable time/identity, scoped to account, instrument and candle range", async () => {
    const fills = Array.from({ length: 505 }, (_, index) => ({ ...fill(index + 1), time: 100 }));
    await journal.ingest(account, "fill", [...fills, { ...fill(600), time: 99 }, { ...fill(601), time: 200 }, { ...fill(602), time: 150, symbol: "ETH" }]);
    await journal.ingest(otherAccount, "fill", [fill(700)]);
    const first = await journal.chartFills(account.toUpperCase(), "BTC", { from: 100, to: 200 });
    expect(first.fills).toHaveLength(500);
    expect(first.fills.every((row) => row.time === 100 && row.side === "Buy" && row.price === "100")).toBe(true);
    expect(first.nextCursor).not.toBeNull();
    expect(first.sync).toBeNull();
    const second = await journal.chartFills(account, "BTC", { from: 100, to: 200, cursor: first.nextCursor! });
    expect(second.fills).toHaveLength(5); expect(second.nextCursor).toBeNull();
    expect(new Set([...first.fills, ...second.fills].map((row) => row.id)).size).toBe(505);
    expect(Object.keys(first.fills[0]!).sort()).toEqual(["id", "price", "side", "size", "time"]);
    expect((await journal.chartFills(account, "ETH", { from: 100, to: 200 })).fills).toHaveLength(1);
    await expect(journal.chartFills(account, "BTC", { from: 200, to: 100 })).rejects.toThrow("range");
    await db.run("UPDATE hv_journal_accounts SET history_limited=1 WHERE account=?", account);
    expect((await journal.chartFills(account, "BTC", { from: 100, to: 200 })).historyLimited).toBe(true);
  });
  it("returns one historical marker for a flip, preserves decimal strings and reflects subsequent journal ingestion", async () => {
    const sell = { ...fill(2, "1", "A"), fill: { ...fill(2, "1", "A").fill!, price: "100.00000001", size: "2.00000001" } };
    rows = [fill(1), sell];
    await journal.syncAccount(account, signal(), 10);
    const page = await journal.chartFills(account, "BTC", { from: 0, to: 20 });
    expect(page.fills).toHaveLength(2);
    expect(page.fills[0]).toMatchObject({ side: "Sell", price: "100.00000001", size: "2.00000001" });
    expect(page.sync).toEqual({ updatedAt: 10, error: null });
    await journal.ingest(account, "fill", [fill(11)]);
    expect((await journal.chartFills(account, "BTC", { from: 0, to: 20 })).fills).toHaveLength(3);
    fetcher.mockRejectedValue(new Error("offline"));
    await journal.syncAccount(account, signal(), 20);
    expect((await journal.chartFills(account, "BTC", { from: 0, to: 20 })).sync?.error).toBeTruthy();
  });

  it("samples rolling balance windows with a carried baseline, exact changes and quiet periods", async () => {
    const day = 86_400_000;
    const now = 40 * day;
    rows = [deposit(1, "1000.00000001"), deposit(now - 2 * day, "-10"), deposit(now - 3_600_000, "0.00000002")];
    await journal.syncAccount(account, signal(), now);
    const history = (await journal.balanceHistory(account.toUpperCase(), now))!;
    expect(history.day).toHaveLength(25);
    expect(history.month).toHaveLength(31);
    expect(history.month[0]).toEqual({ time: now - 30 * day, balance: "1000.00000001" });
    expect(history.day[0]?.balance).toBe("990.00000001");
    expect(history.day.at(-1)).toEqual({ time: now, balance: "990.00000003" });
    expect(history.day.at(-2)?.balance).toBe("990.00000003");
    expect(history.month.at(-1)?.balance).toBe("990.00000003");
    expect(await journal.balanceHistory(otherAccount, now)).toBeNull();
  });
  it("charts complete empty history as flat and withholds incomplete or unknown balances", async () => {
    expect(await journal.balanceHistory(account, 100)).toBeNull();
    await journal.syncAccount(account, signal(), 100);
    expect((await journal.balanceHistory(account, 100))?.day.every((p) => p.balance === "0")).toBe(true);
    await journal.ingest(account, "ledger", [deposit(1)]);
    expect(await journal.balanceHistory(account, 100)).toBeNull();
    await journal.project(account);
    expect(await journal.balanceHistory(account, 100)).not.toBeNull();
    await db.run("UPDATE hv_journal_accounts SET history_limited=1 WHERE account=?", account);
    expect(await journal.balanceHistory(account, 100)).toBeNull();
    await db.run("UPDATE hv_journal_accounts SET history_limited=0 WHERE account=?", account);
    await journal.ingest(account, "ledger", [{ ...deposit(2), delta: null }]);
    await journal.project(account);
    expect(await journal.balanceHistory(account, 100)).toBeNull();
    fetcher.mockRejectedValue(new Error("offline"));
    await journal.syncAccount(otherAccount, signal(), 100);
    expect(await journal.balanceHistory(otherAccount, 100)).toBeNull();
  });

  it("catches up stale HTTP cursors once for concurrent reviews without advancing them on pushes", async () => {
    await journal.syncAccount(account, signal(), 1000);
    await journal.ingest(account, "fill", [fill(1500)]);
    await journal.project(account);
    expect(await db.all("SELECT cursor FROM hv_journal_sync WHERE account=?", account)).toEqual([{ cursor: 1000 }, { cursor: 1000 }, { cursor: 1000 }]);
    fetcher.mockClear(); rows = [fill(1500)];
    const [first, second] = await Promise.all([journal.ensureThrough(account, 1999), journal.ensureThrough(account, 1999)]);
    expect(first).toMatchObject({ attempted: true, limitation: null });
    expect(second).toMatchObject({ attempted: true, limitation: null });
    expect(fetcher).toHaveBeenCalledTimes(3);
    expect((await db.all<{ cursor: number }>("SELECT cursor FROM hv_journal_sync WHERE account=?", account)).every((row) => row.cursor >= 1999)).toBe(true);
    expect(await journal.ensureThrough(account, 1999)).toMatchObject({ attempted: false });
    expect(fetcher).toHaveBeenCalledTimes(3);
  });
  it("retains readable evidence and explicit sync errors after failed catch-up", async () => {
    await journal.syncAccount(account, signal(), 1000);
    rows = [deposit(1)]; await journal.ingest(account, "ledger", rows);
    fetcher.mockRejectedValue(new Error("offline"));
    await journal.ensureThrough(account, 1999);
    expect((await journal.page(account, "activity", 1)).activity).toHaveLength(1);
    expect((await db.all<{ cursor: number; error: string }>("SELECT cursor,error FROM hv_journal_sync WHERE account=?", account)).every((row) => row.cursor === 1000 && row.error.includes("failed"))).toBe(true);
  });
  it("bounds catch-up to twenty seconds and rejects future review boundaries", async () => {
    vi.useFakeTimers();
    vi.spyOn(AbortSignal, "timeout").mockImplementation((ms) => {
      expect(ms).toBe(20_000);
      const controller = new AbortController();
      setTimeout(() => controller.abort(new Error("deadline")), ms);
      return controller.signal;
    });
    expect(await journal.ensureThrough(account, Date.now() + 1)).toMatchObject({ attempted: false, limitation: expect.stringContaining("future") });
    fetcher.mockImplementation(async (_account, _stream, _start, _end, signal) => new Promise((_resolve, reject) => {
      signal.addEventListener("abort", () => reject(signal.reason), { once: true });
      if (signal.aborted) reject(signal.reason);
    }));
    const pending = journal.ensureThrough(account, Date.now());
    await vi.waitFor(() => expect(fetcher).toHaveBeenCalledOnce());
    await vi.advanceTimersByTimeAsync(20_000);
    expect(await pending).toMatchObject({ attempted: true, limitation: expect.stringContaining("deadline") });
  });

  it("backfills every page, checkpoints empty streams and deduplicates overlapping imports", async () => {
    rows = Array.from({ length: 1100 }, (_, n) => deposit(n + 1));
    await journal.syncAccount(account, signal(), 1200);
    const page = await journal.page(account, "activity", 2);
    expect(page).toMatchObject({ total: 1100, pages: 44, page: 2, pending: false });
    expect(page.activity[0]?.balance).toBe("1075");
    expect(page.sync).toHaveLength(3);
    expect(page.sync.every((s) => s.cursor === 1200 && s.updated_at === 1200 && s.error === null)).toBe(true);
    expect(fetcher.mock.calls.length).toBeGreaterThan(3);
    await journal.syncAccount(account, signal(), 1300);
    expect((await journal.page(account, "activity", 1)).total).toBe(1100);
  });
  it("rebuilds corrected and late history, retaining notes and stable trade identities", async () => {
    rows = [deposit(1, "1000"), fill(2), fill(3, "1", "A")];
    await journal.syncAccount(account, signal(), 10);
    const trade = (await journal.page(account, "trades", 1)).trades[0]!;
    expect(trade).toMatchObject({ status: "closed", grossPnl: "10", fees: "0.2", net: "9.8" });
    await journal.addNote(account, trade.id, "owner", "Entry rationale");
    await journal.addNote(account, rows[0]!.id, "owner", "Deposit");
    rows[0] = deposit(1, "2000");
    rows.push({ ...deposit(4, "-0.25"), id: "funding-4", kind: "funding", symbol: "BTC", time: 2, type: "funding" });
    await journal.syncAccount(account, signal(), 20);
    expect((await journal.page(account, "trades", 1)).trades[0]).toMatchObject({ id: trade.id, funding: "-0.25", net: "9.55", noteCount: 1 });
    expect((await journal.notes(account, trade.id))[0]?.body).toBe("Entry rationale");
    expect((await journal.page(account, "activity", 1)).activity[0]?.balance).toBe("2009.55");
    // An earlier opening changes the cycle identity. Old note targets survive.
    rows.push({ ...fill(0), time: 0 });
    rows[1] = fill(2, "1"); rows[2] = fill(3, "2", "A");
    await journal.syncAccount(account, signal(), 30);
    expect((await journal.page(account, "trades", 1)).trades.some((t) => t.id === trade.id)).toBe(false);
    expect((await journal.notes(account, trade.id))[0]?.body).toBe("Entry rationale");
  });
  it("persists a conservative coverage warning when the initial import reaches the exchange retention limit", async () => {
    rows = Array.from({ length: 10_000 }, (_, n) => ({ ...deposit(n + 1), kind: "fill" as const }));
    await journal.syncAccount(account, signal(), 10_001);
    expect(await journal.page(account, "activity", 1)).toMatchObject({ total: 10_000, historyLimited: true });
    expect((await journal.page(account, "activity", 1)).activity.every((e) => e.balance === null)).toBe(true);
    await journal.close(); journal = new JournalService(db, fetcher);
    expect((await journal.page(account, "activity", 1)).historyLimited).toBe(true);
  });
  it("propagates unknown cash impact across full-history pagination", async () => {
    rows = Array.from({ length: 55 }, (_, n) => deposit(n + 1));
    rows[4] = { ...deposit(5), delta: null, type: "unknown" };
    await journal.syncAccount(account, signal(), 100);
    const newest = await journal.page(account, "activity", 1);
    expect(newest.activity.every((e) => e.balance === null)).toBe(true);
    const oldest = await journal.page(account, "activity", 3);
    expect(oldest.activity.map((e) => e.balance)).toEqual([null, "4", "3", "2", "1"]);
  });
  it("retries stream failures without erasing history or advancing failed cursors", async () => {
    rows = [deposit(1), fill(2)];
    fetcher.mockImplementation(async (_a, stream) => { if (stream === "ledger") throw new Error("offline"); return rows.filter((e) => e.kind === stream); });
    await journal.syncAccount(account, signal(), 10);
    let page = await journal.page(account, "trades", 1);
    expect(page.sync.find((s) => s.stream === "ledger")).toMatchObject({ cursor: 0, updated_at: null, error: expect.stringContaining("failed") });
    expect(page.trades[0]?.net).toBeNull();
    fetcher.mockImplementation(async (_a, stream) => rows.filter((e) => e.kind === stream));
    await journal.syncAccount(account, signal(), 20);
    page = await journal.page(account, "trades", 1);
    expect(page.sync.every((s) => s.error === null)).toBe(true);
    expect(page.trades[0]?.net).toBe("-0.1");
  });
  it("refuses to skip saturated timestamp ties and keeps available source records", async () => {
    rows = Array.from({ length: 500 }, (_, n) => ({ ...deposit(n), time: 5 }));
    await journal.syncAccount(account, signal(), 10);
    const page = await journal.page(account, "activity", 1);
    expect(page.total).toBe(500);
    expect(page.sync.find((s) => s.stream === "ledger")?.error).toContain("one timestamp");
    expect(page.activity[0]?.balance).toBeNull();
  });
  it("does not publish an old projection over a concurrent source revision", async () => {
    await journal.ingest(account, "fill", [fill(1)], 1);
    const projection = journal.project(account);
    await journal.ingest(account, "fill", [fill(2, "1", "A")], 2);
    await projection;
    expect((await journal.page(account, "trades", 1)).pending).toBe(true);
    await journal.project(account);
    expect((await journal.page(account, "trades", 1))).toMatchObject({ pending: false, total: 1 });
    expect((await journal.page(account, "trades", 1)).trades[0]?.status).toBe("closed");
  });
  it("pages the timeline through an ordered account index rather than sorting full history", async () => {
    const plan = await db.all<{ detail: string }>("EXPLAIN QUERY PLAN SELECT id,data FROM hv_journal_entries WHERE account=? AND kind<>'trade' ORDER BY time DESC,id DESC LIMIT 25 OFFSET 25", account);
    expect(plan.some((row) => row.detail.includes("hv_journal_entries_timeline"))).toBe(true);
    expect(plan.some((row) => row.detail.includes("TEMP B-TREE"))).toBe(false);
  });
  it("recovers a committed import whose projection was interrupted", async () => {
    await journal.ingest(account, "fill", [fill(2)], 10);
    expect((await journal.page(account, "trades", 1)).pending).toBe(true);
    await journal.close();
    journal = new JournalService(db, fetcher);
    await journal.syncAccount(account, signal(), 20);
    expect((await journal.page(account, "trades", 1))).toMatchObject({ total: 1, pending: false });
  });
  it("scopes reads and notes to the owned Agent and linked account", async () => {
    rows = [deposit(1)]; await journal.syncAccount(account, signal(), 10);
    const id = randomUUID(); const otherId = randomUUID(); const unlinked = randomUUID();
    for (const [agent, owner, address] of [[id, "owner", account], [otherId, "other", otherAccount], [unlinked, "owner", null]]) {
      await db.run("INSERT INTO hv_agents(id,user_id,name,creation_request_id,trading_account_address,created_at,updated_at) VALUES (?,?,'A',?,?,0,0)", agent!, owner!, agent!, address ?? null);
    }
    const service = new HyperliquidService(applicationDatabase(db), undefined, undefined, undefined, journal);
    const chartFills = vi.spyOn(journal, "chartFills");
    await expect(service.chartFills("other", id, "BTC", { from: 0, to: 100 })).rejects.toMatchObject({ status: 404 });
    expect(chartFills).not.toHaveBeenCalled();
    await expect(service.chartFills("owner", unlinked, "BTC", { from: 0, to: 100 })).rejects.toMatchObject({ status: 409 });
    expect(chartFills).not.toHaveBeenCalled();
    await service.chartFills("owner", id, "BTC", { from: 0, to: 100 });
    expect(chartFills).toHaveBeenCalledWith(account, "BTC", { from: 0, to: 100 });
    await expect(service.agentJournal("other", id, "activity", 1)).rejects.toMatchObject({ status: 404 });
    await expect(service.journalNotes("other", otherId, "deposit-1")).rejects.toThrow("not found");
    await expect(service.addJournalNote("other", otherId, "deposit-1", "cross-account")).rejects.toThrow("not found");
    await service.addJournalNote("owner", id, "deposit-1", " scoped ");
    expect((await service.journalNotes("owner", id, "deposit-1"))[0]).toMatchObject({ body: "scoped", authorId: "owner" });
    await expect(service.agentJournal("owner", unlinked, "activity", 1)).rejects.toMatchObject({ status: 409 });
    await expect(service.addJournalNote("owner", id, "deposit-1", "   ")).rejects.toThrow("1–4000");
    await expect(service.addJournalNote("owner", id, "deposit-1", "x".repeat(4001))).rejects.toThrow("1–4000");
  });
  it("runs one backend worker per account without UI subscribers, and aborts on removal/close", async () => {
    rows = [deposit(1)];
    journal.reconcile([account, account]);
    await vi.waitFor(async () => expect((await journal.page(account, "activity", 1)).total).toBe(1));
    expect(fetcher.mock.calls.filter((c) => c[1] === "ledger")).toHaveLength(1);
    journal.reconcile([]);
    await journal.close();
    expect(fetcher.mock.calls.every((c) => c[4].aborted)).toBe(true);
  });
  it("ingests pushed events and catch-up scans without any idle polling", async () => {
    rows = [deposit(1)];
    const feeds = new Map<string, JournalFeedEvents>();
    journal = new JournalService(db, fetcher, undefined, (address, events) => {
      feeds.set(address, events);
      return { close: async () => { feeds.delete(address); } };
    });
    journal.reconcile([account]);
    await vi.waitFor(async () => expect((await journal.page(account, "activity", 1)).total).toBe(1));
    await vi.waitFor(() => expect(feeds.size).toBe(1));
    const events = feeds.get(account)!;
    // Pushed events are ingested and projected without any HTTP fetch.
    const calls = fetcher.mock.calls.length;
    events.push("ledger", [deposit(2), deposit(3)]);
    await vi.waitFor(async () => expect((await journal.page(account, "activity", 1)).total).toBe(3));
    expect(fetcher.mock.calls.length).toBe(calls);
    // A reconnect runs one incremental catch-up per stream, then stays idle.
    events.connected();
    await vi.waitFor(() => expect(fetcher.mock.calls.length).toBeGreaterThanOrEqual(calls + 3));
    const afterCatchUp = fetcher.mock.calls.length;
    await new Promise((resolve) => setTimeout(resolve, 60));
    expect(fetcher.mock.calls.length).toBe(afterCatchUp);
  });
  it("never resynchronizes while idle; only the daily repair scan refetches", async () => {
    vi.useFakeTimers();
    try {
      rows = [deposit(1)];
      journal.reconcile([account]);
      await vi.waitFor(async () => expect((await journal.page(account, "activity", 1)).total).toBe(1));
      const calls = fetcher.mock.calls.length;
      await vi.advanceTimersByTimeAsync(3600_000);
      expect(fetcher.mock.calls.length).toBe(calls);
      await vi.advanceTimersByTimeAsync(24 * 3600_000);
      expect(fetcher.mock.calls.length).toBeGreaterThan(calls);
    } finally { vi.useRealTimers(); }
  });
  it("retries a failing initial backfill until it succeeds, then subscribes", async () => {
    vi.useFakeTimers();
    try {
      rows = [deposit(1)];
      const feeds = new Map<string, JournalFeedEvents>();
      journal = new JournalService(db, fetcher, undefined, (address, events) => {
        feeds.set(address, events);
        return { close: async () => { feeds.delete(address); } };
      });
      let offline = true;
      fetcher.mockImplementation(async (_a, stream) => {
        if (offline && stream === "ledger") throw new Error("offline");
        return rows.filter((e) => e.kind === stream);
      });
      journal.reconcile([account]);
      await vi.waitFor(async () => expect((await journal.page(account, "activity", 1)).sync.find((s) => s.stream === "ledger")?.error).toContain("failed"));
      expect(feeds.size).toBe(0); // No subscription until the backfill is healthy.
      offline = false;
      await vi.advanceTimersByTimeAsync(30_000);
      await vi.waitFor(async () => expect((await journal.page(account, "activity", 1)).total).toBe(1));
      await vi.waitFor(() => expect(feeds.size).toBe(1));
    } finally { vi.useRealTimers(); }
  });
});
