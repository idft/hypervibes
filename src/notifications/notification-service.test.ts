import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { openDatabase } from "../db/database.js";
import { migrateApplication } from "../db/migrations.js";
import { applicationDatabase, SharedDatabase } from "../db/shared-database.js";
import { hvUsers } from "../db/auth-schema.js";
import { hvAgents } from "../db/agent-schema.js";
import { hvSubagents } from "../db/subagent-schema.js";
import { ExecutionContextService } from "../agents/execution-context.js";
import type { MemoryActor } from "../memory/operations.js";
import { JournalService } from "../hyperliquid/journal-service.js";
import { fillEvent, type JournalEvent } from "../hyperliquid/journal-model.js";
import { OrderStore, type PreparedAction } from "../hyperliquid/orders/order-store.js";
import type { OrderView } from "../hyperliquid/orders/order-contracts.js";
import { NotificationService } from "./notification-service.js";
import { NotificationDeliveryError } from "./delivery.js";

const account = `0x${"ab".repeat(20)}`;
const message = { title: "Daily review", body: "A crafted summary. Full review: m1." };
const enabled = { enabled: true, submissions: true, fills: true };
function fill(tid: number, time: number, oid = 123): JournalEvent {
  return fillEvent({ coin: "BTC", px: "100000.12345", sz: "0.001", side: "B", time, startPosition: "0", dir: "Open Long", closedPnl: "0", hash: `0x${tid}`, oid, crossed: true, fee: "0.01", tid, feeToken: "USDC", twapId: null });
}
function view(id: string): OrderView {
  return { id, requestId: "request", symbol: "BTC", side: "buy", kind: "limit", size: "0.001", price: "100000", triggerPrice: null, reduceOnly: false, cloid: null, exchangeOid: "123", commandStatus: "not_sent", orderStatus: "not_sent", filledSize: null, averageFillPrice: null, updatedAt: 1, venueTimestamp: null, source: "intent", error: null };
}
function action(id: string): PreparedAction {
  return { payload: { kind: "submit", orders: [], grouping: "na", builder: { b: account as `0x${string}`, f: 0 } }, items: [view(id)] };
}

describe("Agent notification inbox and Telegram outbox", () => {
  let directory: string, db: SharedDatabase, service: NotificationService, journal: JournalService, execution: ExecutionContextService, now: number;
  const actor: MemoryActor = { agentId: "agent", userId: null, subagentId: "root", runId: null, conversationId: 1, toolTaskId: 10 };
  beforeEach(async () => {
    directory = mkdtempSync(join(tmpdir(), "hv-notifications-"));
    const connection = openDatabase(join(directory, "app.sqlite")); migrateApplication(connection);
    db = new SharedDatabase(connection); now = 10000;
    const app = applicationDatabase(db);
    await app.insert(hvUsers).values({ id: "owner", name: "Owner", email: "owner@example.test" }).run();
    await app.insert(hvAgents).values([
      { id: "agent", userId: "owner", name: "Agent", tradingAccountAddress: account, creationRequestId: "a", createdAt: 1000, updatedAt: 1000 },
      { id: "other", userId: "owner", name: "Other", creationRequestId: "b", createdAt: 1000, updatedAt: 1000 },
    ]).run();
    await app.insert(hvSubagents).values({ id: "root", agentId: "agent", kind: "orchestrator", name: "Orchestrator", instructions: "Test", provider: "faux", modelId: "faux", tools: ["send_notification"], createdAt: 1, updatedAt: 1 }).run();
    execution = new ExecutionContextService(db);
    service = new NotificationService(db, "http://localhost:3003", undefined, () => now);
    journal = new JournalService(db, async () => []); journal.notifications = service;
  });
  afterEach(async () => { await journal.close(); await service.close(); await db.close(); rmSync(directory, { recursive: true, force: true }); });
  async function link(chat = 7, hash = "token-hash") {
    await db.run(`INSERT INTO hv_gateways(agent_id,type,bot_token_ciphertext,bot_token_hash,key_id,bot_username,chat_id,updated_at) VALUES ('agent','telegram',X'41',?,'key','test_bot',?,1)
      ON CONFLICT(agent_id,type) DO UPDATE SET chat_id=excluded.chat_id,bot_token_hash=excluded.bot_token_hash`, hash, chat);
  }
  const rows = () => service.list("agent");

  it("has only three default-on settings, validates input, and keeps history scoped and paginated", async () => {
    expect(await service.getSettings("agent")).toEqual(enabled);
    await expect(service.saveSettings("agent", { ...enabled, fills: "yes" })).rejects.toThrow("Invalid");
    await expect(service.send("agent", "bad", { ...message, title: " " })).rejects.toThrow("title");
    await expect(service.send("agent", "bad", { ...message, body: "x".repeat(12001) })).rejects.toThrow("body");
    await service.send("other", "other", message);
    for (let i = 0; i < 51; i++) { now++; await service.send("agent", `event-${i}`, message); }
    expect(await rows()).toMatchObject({ total: 51, pages: 2, page: 1, items: expect.any(Array) });
    expect((await rows()).items).toHaveLength(50);
    expect((await service.list("agent", 2)).items).toHaveLength(1);
    const first = (await rows()).items[0]!;
    await expect(service.get("other", first.id)).rejects.toMatchObject({ status: 404 });
    expect(first).not.toHaveProperty("botTokenHash"); expect(first).not.toHaveProperty("readAt");
    await expect(service.list("agent", 0)).rejects.toThrow("page");
  });

  it("keeps unlinked notifications web-only and never sends an old backlog when linked", async () => {
    const sender = vi.fn(async () => 42); service.installSender(sender);
    const result = await service.send("agent", "event", message);
    expect(await service.send("agent", "event", message)).toEqual(result);
    expect((await rows()).items[0]!.delivery).toBe("web_only");
    await link(); await service.flush(); expect(sender).not.toHaveBeenCalled();
    await service.send("agent", "new", message); await service.flush();
    expect(sender).toHaveBeenCalledOnce();
    expect(sender.mock.calls[0]).toBeDefined();
    expect((await rows()).items.some((row) => row.delivery === "sent")).toBe(true);
  });

  it("suppresses pending delivery and all new producers when disabled, retaining child selections and history", async () => {
    await link(); await service.send("agent", "pending", message);
    await service.saveSettings("agent", { enabled: false, submissions: false, fills: true });
    expect((await rows()).items[0]!.delivery).toBe("suppressed");
    expect(await service.send("agent", "disabled", message)).toEqual({ status: "disabled" });
    expect(await service.sendFromTool(actor, message, execution)).toEqual({ status: "disabled" });
    const sender = vi.fn(async () => 42); service.installSender(sender); await service.flush(); expect(sender).not.toHaveBeenCalled();
    now += 1000; await service.saveSettings("agent", { enabled: true, submissions: false, fills: true });
    expect(await service.getSettings("agent")).toEqual({ enabled: true, submissions: false, fills: true });
    expect(await service.sendFromTool(actor, message, execution)).toEqual({ status: "disabled" });
    expect(await service.send("agent", "disabled", message)).toEqual({ status: "disabled" });
    expect((await rows()).total).toBe(1);
  });

  it("enforces trusted tool permissions, returns one receipt across restarts, and rejects changed replay arguments", async () => {
    const result = await service.sendFromTool(actor, message, execution);
    await service.close(); service = new NotificationService(db, "http://localhost:3003", undefined, () => now);
    expect(await service.sendFromTool(actor, message, execution)).toEqual(result);
    await expect(service.sendFromTool(actor, { ...message, body: "different" }, execution)).rejects.toMatchObject({ status: 409 });
    await db.run("UPDATE hv_subagents SET tools='[]' WHERE id='root'");
    await expect(service.sendFromTool({ ...actor, toolTaskId: 11 }, message, execution)).rejects.toMatchObject({ status: 403 });
    await expect(service.sendFromTool({ ...actor, subagentId: null, toolTaskId: 12 }, message, execution)).rejects.toMatchObject({ status: 403 });
    expect((await rows()).total).toBe(1);
  });

  it("persists Telegram message IDs and resumes long messages after a chunk failure and restart", async () => {
    await link();
    const sender = vi.fn(async () => 42).mockResolvedValueOnce(41).mockRejectedValueOnce(new NotificationDeliveryError("Temporary"));
    service.installSender(sender);
    await service.send("agent", "long", { ...message, body: "😀".repeat(5500) });
    await service.flush(); now += 1100; await service.flush();
    expect(sender).toHaveBeenCalledTimes(2);
    expect(await db.get("SELECT message_ids,delivery FROM hv_notifications")).toMatchObject({ message_ids: "[41]", delivery: "pending" });
    await service.close(); now += 10000;
    service = new NotificationService(db, "http://localhost:3003", undefined, () => now);
    const resumed = vi.fn(async () => 43); service.installSender(resumed);
    await service.flush();
    expect(resumed).toHaveBeenCalledOnce();
    expect(await db.get("SELECT message_ids,delivery FROM hv_notifications")).toMatchObject({ message_ids: "[41,43]", delivery: "sent" });
  });

  it("honors Telegram retry_after and gives permanent failures an explicit status", async () => {
    await link();
    const sender = vi.fn(async () => 42).mockRejectedValueOnce(new NotificationDeliveryError("Rate limited", false, 60000)); service.installSender(sender);
    await service.send("agent", "limited", message); await service.flush(); now += 59999; await service.flush();
    expect(sender).toHaveBeenCalledOnce(); now++; await service.flush(); expect(sender).toHaveBeenCalledTimes(2);
    now += 1100; sender.mockRejectedValueOnce(new NotificationDeliveryError("Bot blocked", true));
    await service.send("agent", "blocked", message); await service.flush();
    expect((await rows()).items.find((row) => row.title === message.title && row.delivery === "failed")?.error).toBe("Bot blocked");
  });

  it("never redirects queued messages to a different bot or linked chat", async () => {
    await link(); await service.send("agent", "event", message); await link(8, "new-token");
    const sender = vi.fn(async () => 42); service.installSender(sender); await service.flush();
    expect(sender).not.toHaveBeenCalled(); expect((await rows()).items[0]!.delivery).toBe("suppressed");
  });

  it("aborts in-flight delivery and joins it before closing", async () => {
    await link();
    let started!: () => void; const sending = new Promise<void>((resolve) => { started = resolve; });
    service.installSender(async (_target, _text, signal) => { started(); return new Promise<number>((_resolve, reject) => { signal.addEventListener("abort", () => reject(new Error("Aborted")), { once: true }); }); });
    await service.send("agent", "event", message);
    const flushed = service.flush(); await sending; await service.close(); await flushed;
    expect((await rows()).items[0]!.delivery).toBe("pending");
  });

  it("notifies all newly observed account fills, including external activity, but not old imports, corrections or duplicates", async () => {
    const old = fill(1, 500), current = fill(2, now);
    await journal.ingest(account, "fill", [old, current]);
    await journal.ingest(account, "fill", [old, current]);
    await journal.ingest(account, "fill", [{ ...current, fee: "0.02" }]);
    expect((await rows()).total).toBe(1);
    expect((await rows()).items[0]).toMatchObject({ category: "fill", source: "Account fill · external/unknown" });
    expect((await rows()).items[0]!.body).toContain("0.001 BTC @ 100000.12345");
    await service.saveSettings("agent", { ...enabled, fills: false });
    await journal.ingest(account, "fill", [fill(3, now)]); now += 1000;
    await service.saveSettings("agent", enabled);
    await journal.ingest(account, "fill", [fill(4, now - 1), fill(5, now)]);
    expect((await rows()).total).toBe(2);
  });

  it("groups execution fragments observed together by order, without mixing different orders", async () => {
    await journal.ingest(account, "fill", [fill(20, now, 123), fill(21, now, 123), fill(22, now, 456)]);
    expect((await rows()).total).toBe(2);
    expect((await rows()).items.some((row) => row.body.includes("2 executions observed together"))).toBe(true);
    await journal.ingest(account, "fill", [fill(20, now, 123), fill(21, now, 123), fill(22, now, 456)]);
    expect((await rows()).total).toBe(2);
  });

  it("queues order outcomes atomically and aggregates legs without duplicating normal reconciliation", async () => {
    const store = new OrderStore(db); store.notifications = service;
    const request = await store.prepare("agent", account, "request", "submit", {}, [action("entry"), action("stop")]);
    const actions = await store.actions(request.id);
    await store.outcome(actions[0]!.id, "done", [{ ...view("entry"), commandStatus: "accepted", orderStatus: "open" }], "exchange_response");
    expect((await rows()).total).toBe(0);
    await store.outcome(actions[1]!.id, "done", [{ ...view("stop"), commandStatus: "rejected", error: "Rejected protection" }], "exchange_response");
    expect((await rows()).total).toBe(1);
    expect((await rows()).items[0]).toMatchObject({ title: "Order submission partially accepted", severity: "warning" });
    await store.outcome(actions[0]!.id, "done", [{ ...view("entry"), commandStatus: "reconciled", orderStatus: "filled" }], "reconciliation");
    expect((await rows()).total).toBe(1);
    await journal.ingest(account, "fill", [fill(6, now)]);
    expect((await rows()).items.some((row) => row.category === "fill" && row.orderRequestId === request.id)).toBe(true);
  });

  it("reports unknown submissions honestly, then sends exactly one resolution notification", async () => {
    const store = new OrderStore(db); store.notifications = service;
    const request = await store.prepare("agent", account, "request", "submit", {}, [action("entry")]);
    const saved = (await store.actions(request.id))[0]!;
    await store.outcome(saved.id, "unknown", [{ ...view("entry"), commandStatus: "unknown", orderStatus: "unknown" }], "exchange_response");
    expect((await rows()).items[0]!.title).toContain("unknown");
    await store.outcome(saved.id, "done", [{ ...view("entry"), commandStatus: "reconciled", orderStatus: "open" }], "reconciliation");
    await store.outcome(saved.id, "done", [{ ...view("entry"), commandStatus: "reconciled", orderStatus: "filled" }], "reconciliation");
    expect((await rows()).total).toBe(2);
    expect((await rows()).items.some((row) => row.title === "Order submission outcome resolved")).toBe(true);
  });

  it("warns after a crash during one leg even when later legs remain unattempted", async () => {
    const store = new OrderStore(db); store.notifications = service;
    const request = await store.prepare("agent", account, "interrupted", "submit", {}, [action("entry"), action("unattempted")]);
    const first = (await store.actions(request.id))[0]!;
    await store.begin(first);
    await store.recover();
    expect((await rows()).items).toMatchObject([{ title: "Order submission outcome unknown", severity: "warning" }]);
    expect((await rows()).items[0]!.body).toContain("not_sent");
    expect((await store.actions(request.id))[1]!.state).toBe("prepared");
    await store.recover(); expect((await rows()).total).toBe(1);
    await store.outcome(first.id, "done", [{ ...view("entry"), commandStatus: "reconciled", orderStatus: "open" }], "reconciliation");
    expect((await rows()).total).toBe(2);
  });

  it("rolls back notification creation with its source transaction and cascades Agent-owned records", async () => {
    const event = fill(7, now);
    await expect(db.transaction(async (tx) => { await service.fillsTx(tx, account, [event]); throw new Error("Rollback"); })).rejects.toThrow("Rollback");
    expect((await rows()).total).toBe(0);
    await service.sendFromTool(actor, message, execution);
    await db.run("DELETE FROM hv_agents WHERE id='agent'");
    expect(await db.all("SELECT * FROM hv_notifications")).toEqual([]);
    expect(await db.all("SELECT * FROM hv_notification_operations")).toEqual([]);
    expect(await db.all("SELECT * FROM hv_notification_settings")).toEqual([]);
  });

  it("pushes invalidations to an Agent watcher and releases it on abort", async () => {
    const controller = new AbortController(), publish = vi.fn();
    const watching = service.watch("agent", controller.signal, publish);
    expect(publish).toHaveBeenCalledOnce();
    await service.send("other", "other", message); expect(publish).toHaveBeenCalledOnce();
    await service.send("agent", "event", message);
    await vi.waitFor(() => expect(publish).toHaveBeenCalledTimes(2));
    controller.abort(); await watching;
  });
});
