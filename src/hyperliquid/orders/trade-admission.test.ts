import { BACKGROUND_CONTEXT as context } from "@earendil-works/chord/context";
import type { ToolExecutionApi } from "@earendil-works/pi-durable";
import { HttpTransport } from "@nktkas/hyperliquid";
import { eq } from "drizzle-orm";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ExecutionContextService } from "../../agents/execution-context.js";
import { applicationDatabase } from "../../db/shared-database.js";
import { hvSubagents, type SubagentRow } from "../../db/subagent-schema.js";
import { memoryFixture } from "../../memory/test-fixture.js";
import { journalDecisionAudit, windowFillAudit } from "../../memory/decision-audit.js";
import { OrderService } from "./order-service.js";
import { RiskSettingsService } from "./risk-settings.js";
import type { OrderInput } from "./order-contracts.js";
const address = `0x${"ab".repeat(20)}`;
const protectedOrder: OrderInput = { symbol: "BTC", side: "buy", order_type: "limit", size: "0.001", price: "100000", take_profits: [{ trigger_price: "110000", size: "0.001" }], stop_losses: [{ trigger_price: "90000", size: "0.001" }] };
describe("persisted order admission boundary", () => {
  let f: Awaited<ReturnType<typeof memoryFixture>>, execution: ExecutionContextService, service: OrderService, risk: RiskSettingsService;
  let resting: { coin: string; oid: number; sz: string; origSz?: string; limitPx: string; reduceOnly: boolean; cloid?: string }[];
  let root: SubagentRow, positions: { coin: string; szi: string }[], now: number, sends: number, failTransport: boolean;
  beforeEach(async () => {
    f = await memoryFixture();
    await f.db.run("UPDATE hv_agents SET state='ready' WHERE id='agent'");
    now = f.now + 100000; sends = 0; failTransport = false; positions = []; resting = [];
    vi.spyOn(Date, "now").mockImplementation(() => now);
    await f.db.run("INSERT INTO hv_subagents(id,agent_id,kind,name,instructions,provider,model_id,tools,created_at,updated_at) VALUES ('root','agent','orchestrator','Root','Observe','faux','faux',?,0,0)", JSON.stringify(["record_trade_decision", "submit_orders", "cancel_orders", "cancel_all_orders"]));
    root = (await applicationDatabase(f.db).select().from(hvSubagents).where(eq(hvSubagents.id, "root")).get())!;
    execution = new ExecutionContextService(f.db);
    vi.spyOn(execution, "resolve").mockImplementation(async (_agentId, api) => {
      if (api.conversationId === 2) return { actor: { agentId: "agent", userId: null, subagentId: root.id, runId: null, conversationId: 2, toolTaskId: api.taskId }, config: root };
      const run = await f.db.get<{ id: string; subagent_id: string; config: string }>("SELECT id,subagent_id,config FROM hv_subagent_runs WHERE agent_id='agent' AND conversation_id=?", api.conversationId);
      if (!run) throw new Error("Unbound test conversation");
      return { actor: { agentId: "agent", userId: null, subagentId: run.subagent_id, runId: run.id, conversationId: api.conversationId, toolTaskId: api.taskId }, config: JSON.parse(run.config) as SubagentRow };
    });
    risk = new RiskSettingsService(f.db);
    vi.stubGlobal("fetch", vi.fn(async (_url: unknown, init: RequestInit) => {
      const body = JSON.parse(String(init.body)) as { type?: string; action?: { orders?: unknown[] } };
      if (body.action) {
        sends++;
        if (failTransport) throw new Error("Lost after venue acceptance");
        return Response.json({ status: "ok", response: { type: "order", data: { statuses: (body.action.orders ?? []).map((_, index) => index ? "waitingForFill" : { filled: { oid: 123, totalSz: "0.001", avgPx: "100000" } }) } } });
      }
      if (body.type === "meta") return Response.json({ universe: [{ name: "BTC", szDecimals: 5 }, { name: "ETH", szDecimals: 4 }] });
      if (body.type === "allMids") return Response.json({ BTC: "100000", ETH: "2000" });
      if (body.type === "clearinghouseState") return Response.json({ marginSummary: { accountValue: "1000" }, assetPositions: positions.map((position) => ({ position: { ...position, entryPx: "100000" } })) });
      if (body.type === "spotClearinghouseState") return Response.json({ balances: [] });
      if (body.type === "frontendOpenOrders") return Response.json(resting);
      if (body.type === "orderStatus") return Response.json({ status: "unknownOid" });
      throw new Error(`Unexpected request ${body.type}`);
    }));
    service = new OrderService(f.db, new HttpTransport(), { account: async () => address, authority: async () => ({ account: address, privateKey: `0x${"12".repeat(32)}`, builder: { b: `0x${"ef".repeat(20)}`, f: 10 } }) });
    service.installAdmission(execution); await service.recover(); service.start();
  });
  afterEach(async () => { await service.close(); await f.close(); vi.unstubAllGlobals(); vi.restoreAllMocks(); });
  async function authority(config = root, conversation = 2) {
    if (config.kind === "worker") await f.db.run("UPDATE hv_subagent_runs SET config=? WHERE id=? AND agent_id='agent'", JSON.stringify(config), `run-${config.id}`);
    const api = { conversationId: conversation, taskId: 100, getTask: async (id: number) => ({ id, conversationId: conversation }) } as unknown as ToolExecutionApi;
    return { actor: (await execution.resolve("agent", api, context)).actor, handle: await service.executionHandle("agent", api, context) };
  }
  const enabled = (patch: Record<string, unknown> = {}) => risk.save("agent", { tradingEnabled: true, allowedInstruments: null, maxOrderNotional: null, maxTotalNotional: null, ...patch });
  it("rejects forged handles and enforces only risk settings before sending", async () => {
    await expect(service.submit("owner", "agent", { request_id: "forged", orders: [protectedOrder] }, undefined, { mode: "user_directed" })).rejects.toThrow("verified model execution");
    const a = await authority();
    await expect(service.submit("owner", "agent", { request_id: "disabled", orders: [protectedOrder] }, undefined, a.handle)).rejects.toThrow("disabled");
    await enabled({ allowedInstruments: ["ETH"] });
    await expect(service.submit("owner", "agent", { request_id: "disallowed-coin", orders: [protectedOrder] }, undefined, a.handle)).rejects.toThrow("not in this Agent's allowed instruments");
    await enabled({ allowedInstruments: ["BTC"], maxOrderNotional: "50" });
    await expect(service.submit("owner", "agent", { request_id: "over-order-cap", orders: [protectedOrder] }, undefined, a.handle)).rejects.toThrow("maximum order notional");
    await enabled({ maxOrderNotional: "1000", maxTotalNotional: "150" });
    await expect(service.submit("owner", "agent", { request_id: "under-total", orders: [protectedOrder] }, undefined, a.handle)).resolves.toBeDefined();
    positions = [{ coin: "BTC", szi: "0.001" }];
    await expect(service.submit("owner", "agent", { request_id: "over-total", orders: [protectedOrder] }, undefined, a.handle)).rejects.toThrow("maximum total notional");
    expect(sends).toBe(1);
  });
  it("allows entries without attached TP/SL (prompt-level protection) and ungranted tools fail closed", async () => {
    await enabled();
    const a = await authority();
    const bare = { ...protectedOrder, take_profits: undefined, stop_losses: undefined };
    const result = await service.submit("owner", "agent", { request_id: "prompt-protected", orders: [bare] }, undefined, a.handle);
    expect(result.results[0]!.symbol).toBe("BTC");
    const analyst = (await applicationDatabase(f.db).select().from(hvSubagents).where(eq(hvSubagents.id, "analyst")).get())!;
    const b = await authority(analyst, 1);
    await expect(service.submit("owner", "agent", { request_id: "no-tools", orders: [protectedOrder] }, undefined, b.handle)).rejects.toThrow("Order mutation permission not selected");
    expect(sends).toBe(1);
  });
  it("reserves pending/recent local exposure so concurrent workers cannot exceed the total cap", async () => {
    await enabled({ maxTotalNotional: "150" });
    const worker = (await applicationDatabase(f.db).select().from(hvSubagents).where(eq(hvSubagents.id, "reviewer")).get())!;
    worker.tools = ["record_trade_decision", "submit_orders"];
    const a = await authority();
    const b = await authority(worker, 3);
    const results = await Promise.allSettled([
      service.submit("owner", "agent", { request_id: "root-entry", orders: [protectedOrder] }, undefined, a.handle),
      service.submit("owner", "agent", { request_id: "worker-entry", orders: [protectedOrder] }, undefined, b.handle),
    ]);
    expect(results.filter((result) => result.status === "fulfilled")).toHaveLength(1);
    expect(results.filter((result) => result.status === "rejected")).toHaveLength(1);
    expect(sends).toBe(1);
    expect(await f.db.get("SELECT count(*) AS total FROM hv_order_requests")).toMatchObject({ total: 1 });
  });
  it("resolves venue fills to local requests and keeps external/unmatched coverage explicit", async () => {
    await enabled();
    const a = await authority();
    const decision = await f.services().memory.recordDecision(f.analyst, "decision:optional", { topic: "entry", instrument: "BTC", summary: "Entry", content: "Layering rationale", payload: { version: 1, action: "enter", direction: "long", reason: "Strategy", researchRefs: [], plan: "TP/SL" } });
    const result = await service.submit("owner", "agent", { request_id: "attributed-order", decision_ref: decision.ref, orders: [protectedOrder] }, undefined, a.handle);
    await f.db.run("UPDATE hv_agents SET trading_account_address=? WHERE id='agent'", address);
    await f.db.run("UPDATE hv_agents SET trading_account_address=? WHERE id='other'", `0x${"cd".repeat(20)}`);
    await f.db.run("INSERT INTO hv_journal_accounts(account) VALUES (?)", address);
    for (const [id, raw] of [["local-fill", { oid: 123, cloid: result.results[0]!.cloid }], ["external-fill", { oid: 999 }]] as const) await f.db.run("INSERT INTO hv_journal_entries(account,id,kind,time,data) VALUES (?,?,'fill',?,?)", address, id, now, JSON.stringify({ raw }));
    await f.db.run("INSERT INTO hv_journal_entries(account,id,kind,time,data) VALUES (?,'trade','trade',?,?)", address, now, JSON.stringify({ parts: [{ eventId: "local-fill" }, { eventId: "external-fill" }] }));
    const audit = await journalDecisionAudit(f.db, "agent", "trade");
    expect(audit.items).toContainEqual(expect.objectContaining({ fillId: "local-fill", requestId: "attributed-order", decisionRef: decision.ref }));
    expect(audit.items).toContainEqual(expect.objectContaining({ fillId: "external-fill", requestId: null }));
    await expect(journalDecisionAudit(f.db, "other", "trade")).rejects.toThrow("not found");
    const first = await windowFillAudit(f.db, "agent", now, now + 1, 1);
    const second = await windowFillAudit(f.db, "agent", now, now + 1, 1, first.next!);
    expect(first.items).toHaveLength(1); expect(second.items).toHaveLength(1);
    expect(first.items[0]!.fillId).not.toBe(second.items[0]!.fillId);
  });
  it("records verified submit/cancel origins and does not reattribute request retries", async () => {
    await enabled();
    const a = await authority();
    const submit = { request_id: "origin-entry", orders: [protectedOrder] };
    await service.submit("owner", "agent", submit, undefined, a.handle);
    const origin = await f.db.get("SELECT p.* FROM hv_order_request_provenance p JOIN hv_order_requests r ON r.id=p.request_id WHERE r.request_key='origin-entry'");
    expect(origin).toMatchObject({ agent_id: "agent", producer_subagent_id: "root", source_run_id: null, conversation_id: 2, tool_task_id: 100 });
    await service.submit("owner", "agent", submit, undefined, a.handle);
    expect(await f.db.get("SELECT p.* FROM hv_order_request_provenance p JOIN hv_order_requests r ON r.id=p.request_id WHERE r.request_key='origin-entry'")).toEqual(origin);
    // Empty cancel-all still records who captured the empty target set.
    await service.cancelAll("owner", "agent", { request_id: "origin-cancel" }, undefined, a.handle);
    expect(await f.db.get("SELECT p.producer_subagent_id,p.tool_task_id FROM hv_order_request_provenance p JOIN hv_order_requests r ON r.id=p.request_id WHERE r.request_key='origin-cancel'")).toMatchObject({ producer_subagent_id: "root", tool_task_id: 100 });
    await expect(service.cancelAll("owner", "agent", { request_id: "forged-cancel" }, undefined, {})).rejects.toThrow("verified model execution");
    expect(await f.db.get("SELECT id FROM hv_order_requests WHERE request_key='forged-cancel'")).toBeUndefined();
  });
  it("allows layering, shorts, and multiple orders without research, decisions or attached protection", async () => {
    await enabled();
    positions = [{ coin: "BTC", szi: "0.05" }];
    resting = [{ coin: "BTC", oid: 42, sz: "0.01", limitPx: "90000", reduceOnly: false }];
    const a = await authority();
    const bare: OrderInput = { symbol: "BTC", side: "buy", order_type: "limit", size: "0.001", price: "95000" };
    await service.submit("owner", "agent", { request_id: "layer-one", orders: [bare] }, undefined, a.handle);
    await service.submit("owner", "agent", { request_id: "layer-two", orders: [{ ...bare, price: "90000" }] }, undefined, a.handle);
    await service.submit("owner", "agent", { request_id: "multi-short", orders: [{ ...bare, side: "sell" }, { ...bare, price: "85000" }] }, undefined, a.handle);
    expect(sends).toBe(4);
  });
  it("uses each order's own price, without sub-cent rounding or a batch-cap bypass", async () => {
    const a = await authority();
    await enabled({ maxOrderNotional: "50" });
    const small: OrderInput = { symbol: "ETH", side: "buy", order_type: "limit", size: "0.01", price: "2000" };
    await expect(service.submit("owner", "agent", { request_id: "multi-price", orders: [small, protectedOrder] }, undefined, a.handle)).rejects.toThrow("maximum order notional");
    await enabled({ maxOrderNotional: "100" });
    await expect(service.submit("owner", "agent", { request_id: "fraction-over-cap", orders: [{ ...protectedOrder, price: "100001", take_profits: [], stop_losses: [] }] }, undefined, a.handle)).rejects.toThrow("maximum order notional");
    await enabled({ maxTotalNotional: "150" });
    await expect(service.submit("owner", "agent", { request_id: "batch-over-cap", orders: [protectedOrder, protectedOrder] }, undefined, a.handle)).rejects.toThrow("maximum total notional");
    expect(sends).toBe(0);
  });
  it("counts short positions and remaining resting orders, but not reduce-only protection", async () => {
    const a = await authority();
    await enabled({ maxTotalNotional: "150" });
    positions = [{ coin: "BTC", szi: "-0.001" }];
    await expect(service.submit("owner", "agent", { request_id: "short-cap", orders: [protectedOrder] }, undefined, a.handle)).rejects.toThrow("maximum total notional");
    positions = [];
    resting = [{ coin: "BTC", oid: 42, sz: "0.001", origSz: "0.01", limitPx: "90000", reduceOnly: false }];
    await expect(service.submit("owner", "agent", { request_id: "pending-cap", orders: [protectedOrder] }, undefined, a.handle)).rejects.toThrow("maximum total notional");
    resting[0]!.reduceOnly = true;
    await service.submit("owner", "agent", { request_id: "protection-ignored", orders: [protectedOrder] }, undefined, a.handle);
    expect(sends).toBe(1);
  });
  it("does not count a venue resting order twice when it is also stored locally", async () => {
    const a = await authority();
    await enabled({ maxTotalNotional: "250" });
    const result = await service.submit("owner", "agent", { request_id: "first-local", orders: [protectedOrder] }, undefined, a.handle);
    resting = [{ coin: "BTC", oid: 123, sz: "0.001", limitPx: "100000", reduceOnly: false, cloid: result.results[0]!.cloid! }];
    await service.submit("owner", "agent", { request_id: "second-local", orders: [protectedOrder] }, undefined, a.handle);
    expect(sends).toBe(2);
  });
  it("revalidates unsent orders against current limits, but replays committed outcomes after disabling", async () => {
    const a = await authority();
    await enabled();
    const input = { request_id: "prepared-before-change", orders: [protectedOrder] };
    vi.spyOn(service.store, "begin").mockRejectedValueOnce(new Error("Before send"));
    await expect(service.submit("owner", "agent", input, undefined, a.handle)).rejects.toThrow("Before send");
    await risk.save("agent", { tradingEnabled: false, allowedInstruments: null, maxOrderNotional: null, maxTotalNotional: null });
    await expect(service.submit("owner", "agent", input, undefined, a.handle)).rejects.toThrow("disabled");
    expect(sends).toBe(0);
    await enabled();
    const sent = await service.submit("owner", "agent", input, undefined, a.handle);
    await risk.save("agent", { tradingEnabled: false, allowedInstruments: ["ETH"], maxOrderNotional: "1", maxTotalNotional: "1" });
    expect(await service.submit("owner", "agent", input, undefined, a.handle)).toEqual(sent);
    await service.submit("owner", "agent", { request_id: "always-reduce", orders: [{ symbol: "BTC", side: "sell", order_type: "limit", size: "0.001", price: "100000", reduce_only: true }] }, undefined, a.handle);
    expect(sends).toBe(2);
  });
  it("retains uncertain-outcome behavior and request-ID deduplication", async () => {
    await enabled();
    const a = await authority();
    const input = { request_id: "unknown", orders: [protectedOrder] };
    failTransport = true;
    expect((await service.submit("owner", "agent", input, undefined, a.handle)).uncertain).toBe(true);
    now += 900001;
    expect((await service.submit("owner", "agent", input, undefined, a.handle)).uncertain).toBe(true);
    await expect(service.submit("owner", "agent", { ...input, request_id: "blind-retry" }, undefined, a.handle)).rejects.toThrow("unknown outcome");
    expect(sends).toBe(1);
  });
});