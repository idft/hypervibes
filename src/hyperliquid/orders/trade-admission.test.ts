import { BACKGROUND_CONTEXT as context } from "@earendil-works/chord/context";
import type { ToolExecutionApi } from "@earendil-works/pi-durable";
import { HttpTransport } from "@nktkas/hyperliquid";
import { eq } from "drizzle-orm";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ExecutionContextService } from "../../agents/execution-context.js";
import { applicationDatabase } from "../../db/shared-database.js";
import { hvSubagents, type SubagentRow } from "../../db/subagent-schema.js";
import { memoryFixture, researchInput, researchRequirement } from "../../memory/test-fixture.js";
import { journalDecisionAudit, windowFillAudit } from "../../memory/decision-audit.js";
import { OrderService } from "./order-service.js";
import type { OrderInput } from "./order-contracts.js";
const address = `0x${"ab".repeat(20)}`;
const protectedOrder: OrderInput = { symbol: "BTC", side: "buy", order_type: "limit", size: "0.001", price: "100000", take_profits: [{ trigger_price: "110000", size: "0.001" }], stop_losses: [{ trigger_price: "90000", size: "0.001" }] };
const rationale = (refs: string[] = []) => ({ topic: "entry", instrument: "BTC", summary: "Entry", content: "Explicit exact protection plan", payload: { version: 1, action: "enter", direction: "long", reason: "Host-authorized assignment", researchRefs: refs, plan: "Full-size attached TP and market SL; verify each leg" } });
describe("persisted order admission boundary", () => {
  let f: Awaited<ReturnType<typeof memoryFixture>>, execution: ExecutionContextService, service: OrderService;
  let root: SubagentRow, now: number, sends: number, failTransport: boolean, unavailableMode: boolean;
  beforeEach(async () => {
    f = await memoryFixture(); now = f.now + 100000; sends = 0; failTransport = false; unavailableMode = false;
    vi.spyOn(Date, "now").mockImplementation(() => now);
    await f.db.run("INSERT INTO hv_subagents(id,agent_id,kind,name,instructions,provider,model_id,tools,created_at,updated_at) VALUES ('root','agent','orchestrator','Root','Observe','faux','faux',?,0,0)", JSON.stringify(["record_trade_decision", "submit_orders"]));
    root = (await applicationDatabase(f.db).select().from(hvSubagents).where(eq(hvSubagents.id, "root")).get())!;
    execution = new ExecutionContextService(f.db);
    vi.stubGlobal("fetch", vi.fn(async (_url: unknown, init: RequestInit) => {
      const body = JSON.parse(String(init.body)) as { type?: string; action?: { orders?: unknown[] } };
      if (body.action) {
        sends++;
        expect(await f.db.get("SELECT request_id FROM hv_order_execution_contexts")).toBeDefined();
        expect(await f.db.get("SELECT request_id FROM hv_automatic_entry_claims")).toBeDefined();
        if (failTransport) throw new Error("Lost after venue acceptance");
        return Response.json({ status: "ok", response: { type: "order", data: { statuses: (body.action.orders ?? []).map((_, index) => index ? "waitingForFill" : { filled: { oid: 123, totalSz: "0.001", avgPx: "100000" } }) } } });
      }
      if (body.type === "meta") return Response.json({ universe: [{ name: "BTC", szDecimals: 5 }] });
      if (body.type === "allMids") return Response.json({ BTC: "100000" });
      if (body.type === "clearinghouseState") return Response.json({ marginSummary: { accountValue: "1000" }, assetPositions: [] });
      if (body.type === "spotClearinghouseState") return Response.json({ balances: [] });
      if (body.type === "userAbstraction") { if (unavailableMode) throw new Error("Unknown mode"); return Response.json("default"); }
      if (body.type === "frontendOpenOrders") return Response.json([]);
      if (body.type === "orderStatus") return Response.json({ status: "unknownOid" });
      throw new Error(`Unexpected request ${body.type}`);
    }));
    service = new OrderService(f.db, new HttpTransport(), { account: async () => address, authority: async () => ({ account: address, privateKey: `0x${"12".repeat(32)}`, builder: { b: `0x${"ef".repeat(20)}`, f: 10 } }) });
    service.installAdmission(execution); await service.recover(); service.start();
  });
  afterEach(async () => { await service.close(); await f.close(); vi.unstubAllGlobals(); vi.restoreAllMocks(); });
  async function authority(config = root, options: { scope?: string; automatic?: boolean; expiresAt?: number; generation?: number } = {}) {
    const generation = options.generation ?? 10, runId = config.kind === "worker" ? `run-${config.id}` : null;
    const api = { conversationId: 1, taskId: generation + 100, getTask: async (id: number) => ({ id, conversationId: 1, kind: id === generation ? "pi.generation" : "pi.tool", ...(id === generation ? {} : { owner: generation }) }) } as unknown as ToolExecutionApi;
    await f.db.run("INSERT INTO hv_agent_execution_frames(agent_id,generation_task_id,conversation_id,input_scope_key,inputs,run_id,origin,config,created_at,expires_at) VALUES ('agent',?,1,?,'[1]',?,?,?,?,?)", generation, options.scope ?? "input-batch", runId, JSON.stringify(options.automatic ? { mode: "automatic", reason: "schedule" } : { mode: "user_directed", userId: "owner", userInputRequestIds: ["trusted-request"], expiresAt: options.expiresAt ?? now + 900000 }), JSON.stringify(config), now, options.expiresAt ?? now + 900000);
    const resolved = await execution.resolve("agent", api, context);
    return { ...resolved, handle: await service.executionHandle("agent", api, context) };
  }
  it("rejects forged handles, absent/cross-input decisions and incomplete or partial-size TP/SL before sending", async () => {
    await expect(service.submit("owner", "agent", { request_id: "forged", decision_ref: "m1", orders: [protectedOrder] }, undefined, { mode: "user_directed" })).rejects.toThrow("verified model execution");
    const a = await authority();
    const result = await execution.memories.recordDecision(a.actor, "decision", rationale());
    await expect(service.submit("owner", "agent", { request_id: "missing-ref", orders: [protectedOrder] }, undefined, a.handle)).rejects.toThrow("decision_ref");
    await expect(service.submit("owner", "agent", { request_id: "missing-sl", decision_ref: result.ref, orders: [{ ...protectedOrder, stop_losses: [] }] }, undefined, a.handle)).rejects.toThrow("full-size");
    await expect(service.submit("owner", "agent", { request_id: "partial-sl", decision_ref: result.ref, orders: [{ ...protectedOrder, stop_losses: [{ trigger_price: "90000", size: "0.0005" }] }] }, undefined, a.handle)).rejects.toThrow("full-size");
    const other = await authority(root, { generation: 11, scope: "different-input-batch" });
    await expect(service.submit("owner", "agent", { request_id: "cross-input", decision_ref: result.ref, orders: [protectedOrder] }, undefined, other.handle)).rejects.toThrow("authorized run/root input scope");
    expect(sends).toBe(0);
    expect(await f.db.get("SELECT count(*) AS total FROM hv_order_requests")).toMatchObject({ total: 0 });
  });
  it("allows root decisions across successive generations of the same input scope and preserves exact replay after expiry", async () => {
    const first = await authority();
    const decision = await execution.memories.recordDecision(first.actor, "decision", rationale());
    const next = await authority(root, { generation: 11 });
    const input = { request_id: "user-trade", decision_ref: decision.ref, orders: [protectedOrder] };
    const original = await service.submit("owner", "agent", input, undefined, next.handle);
    expect(original.execution).toMatchObject({ origin: "user_directed", decisionRef: decision.ref });
    now += 900001;
    expect(await service.submit("owner", "agent", input, undefined, next.handle)).toEqual(original);
    expect(sends).toBe(1);
    await expect(service.submit("owner", "agent", { ...input, request_id: "new-id" }, undefined, next.handle)).rejects.toThrow("expired");
  });
  it("revalidates a persisted never-sent automatic action after a crash and never sends stale research", async () => {
    const db = applicationDatabase(f.db);
    const trader = (await db.select().from(hvSubagents).where(eq(hvSubagents.id, "reviewer")).get())!;
    trader.tools = ["record_trade_decision", "submit_orders"];
    trader.workflow = { version: 1, purpose: "trading", instruments: ["BTC"], automaticEntriesEnabled: true, requiredResearch: researchRequirement, entryStrategy: "Ready bullish", exitStrategy: "Invalidation", baseSizeByInstrument: { BTC: "0.001" }, maxEntryNotional: "101", takeProfitBps: 200, stopLossBps: 100 };
    now = f.now + 3599000;
    await f.services().memory.publishResearch(f.analyst, "analysis", researchInput(f.now));
    const a = await authority(trader, { automatic: true });
    const decision = await execution.memories.recordDecision(a.actor, "decision", rationale(["m1"]));
    const input = { request_id: "prepared", decision_ref: decision.ref, orders: [{ ...protectedOrder, take_profits: [{ trigger_price: "102000" }], stop_losses: [{ trigger_price: "99000" }] }] };
    vi.spyOn(service.store, "begin").mockRejectedValueOnce(new Error("Crash immediately before send"));
    await expect(service.submit("owner", "agent", input, undefined, a.handle)).rejects.toThrow("Crash immediately");
    expect(await f.db.get("SELECT state FROM hv_order_actions")).toMatchObject({ state: "prepared" });
    expect(await f.db.get("SELECT count(*) AS total FROM hv_automatic_entry_claims")).toMatchObject({ total: 1 });
    now += 2000; // still inside order TTL and assignment deadline, but research expired
    await expect(service.submit("owner", "agent", input, undefined, a.handle)).rejects.toThrow("Automatic entry blocked");
    expect(sends).toBe(0);
  });
  it("serializes root and worker entries against the same account despite a persistently flat HTTP fixture", async () => {
    const a = await authority();
    const worker = (await applicationDatabase(f.db).select().from(hvSubagents).where(eq(hvSubagents.id, "reviewer")).get())!;
    worker.tools = ["record_trade_decision", "submit_orders"];
    const b = await authority(worker, { generation: 11 });
    const first = await execution.memories.recordDecision(a.actor, "root-decision", rationale());
    const second = await execution.memories.recordDecision(b.actor, "worker-decision", rationale());
    const results = await Promise.allSettled([
      service.submit("owner", "agent", { request_id: "root-entry", decision_ref: first.ref, orders: [protectedOrder] }, undefined, a.handle),
      service.submit("owner", "agent", { request_id: "worker-entry", decision_ref: second.ref, orders: [protectedOrder] }, undefined, b.handle),
    ]);
    expect(results.filter((result) => result.status === "fulfilled")).toHaveLength(1);
    expect(results.filter((result) => result.status === "rejected")).toHaveLength(1);
    expect(sends).toBe(1);
    expect(await f.db.get("SELECT count(*) AS total FROM hv_order_requests")).toMatchObject({ total: 1 });
  });
  it("resolves venue fills to scoped decisions and keeps external/unmatched coverage explicit", async () => {
    const a = await authority();
    const decision = await execution.memories.recordDecision(a.actor, "decision", rationale());
    const result = await service.submit("owner", "agent", { request_id: "attributed-order", decision_ref: decision.ref, orders: [protectedOrder] }, undefined, a.handle);
    await f.db.run("UPDATE hv_agents SET trading_account_address=? WHERE id='agent'", address);
    await f.db.run("UPDATE hv_agents SET trading_account_address=? WHERE id='other'", `0x${"cd".repeat(20)}`);
    await f.db.run("INSERT INTO hv_journal_accounts(account) VALUES (?)", address);
    for (const [id, raw] of [["local-fill", { oid: 123, cloid: result.results[0]!.cloid }], ["external-fill", { oid: 999 }]] as const) await f.db.run("INSERT INTO hv_journal_entries(account,id,kind,time,data) VALUES (?,?,'fill',?,?)", address, id, now, JSON.stringify({ raw }));
    await f.db.run("INSERT INTO hv_journal_entries(account,id,kind,time,data) VALUES (?,'trade','trade',?,?)", address, now, JSON.stringify({ parts: [{ eventId: "local-fill" }, { eventId: "external-fill" }] }));
    const audit = await journalDecisionAudit(f.db, "agent", "trade");
    expect(audit.items).toContainEqual(expect.objectContaining({ fillId: "local-fill", requestId: "attributed-order", decisionRef: "m1", origin: "user_directed" }));
    expect(audit.items).toContainEqual(expect.objectContaining({ fillId: "external-fill", requestId: null, decisionRef: null, origin: "legacy/unattributed" }));
    await expect(journalDecisionAudit(f.db, "other", "trade")).rejects.toThrow("not found");
    const first = await windowFillAudit(f.db, "agent", now, now + 1, 1);
    const second = await windowFillAudit(f.db, "agent", now, now + 1, 1, first.next!);
    expect(first.items).toHaveLength(1); expect(second.items).toHaveLength(1);
    expect(first.items[0]!.fillId).not.toBe(second.items[0]!.fillId);
  });
  it("fails closed for automatic policy absence/current-data uncertainty, and retains uncertain-outcome behavior", async () => {
    const automatic = await authority(root, { automatic: true });
    const decision = await execution.memories.recordDecision(automatic.actor, "automatic-decision", rationale());
    await expect(service.submit("owner", "agent", { request_id: "automatic-no-policy", decision_ref: decision.ref, orders: [protectedOrder] }, undefined, automatic.handle)).rejects.toThrow("explicitly enabled");
    const manual = await authority(root, { generation: 11 });
    const userDecision = await execution.memories.recordDecision(manual.actor, "user-decision", rationale());
    const input = { request_id: "unknown", decision_ref: userDecision.ref, orders: [protectedOrder] };
    unavailableMode = true;
    await expect(service.submit("owner", "agent", input, undefined, manual.handle)).rejects.toThrow("health is incomplete");
    unavailableMode = false; failTransport = true;
    expect((await service.submit("owner", "agent", input, undefined, manual.handle)).uncertain).toBe(true);
    now += 900001;
    expect((await service.submit("owner", "agent", input, undefined, manual.handle)).uncertain).toBe(true);
    await expect(service.submit("owner", "agent", { ...input, request_id: "blind-retry" }, undefined, manual.handle)).rejects.toThrow("unknown outcome");
    expect(sends).toBe(1);
  });
});
