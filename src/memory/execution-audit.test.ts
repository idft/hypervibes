import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { memoryFixture } from "./test-fixture.js";
import { executionAudit } from "./execution-audit.js";
import { OrderStore } from "../hyperliquid/orders/order-store.js";
import { newView } from "../hyperliquid/orders/order-prepare.js";
import type { MemoryActor } from "./operations.js";

const account = `0x${"ab".repeat(20)}`;
const cloid = `0x${"12".repeat(16)}`;
describe("bounded read-only execution audit and trusted request provenance", () => {
  let f: Awaited<ReturnType<typeof memoryFixture>>, store: OrderStore;
  beforeEach(async () => {
    f = await memoryFixture();
    vi.spyOn(Date, "now").mockImplementation(() => f.now);
    store = new OrderStore(f.db);
    await f.db.run("UPDATE hv_agents SET trading_account_address=? WHERE id='agent'", account);
    await f.db.run("UPDATE hv_agents SET trading_account_address=? WHERE id='other'", `0x${"cd".repeat(20)}`);
    await f.db.run("INSERT INTO hv_journal_accounts(account) VALUES (?)", account);
  });
  afterEach(async () => { await f.close(); vi.restoreAllMocks(); });
  const bounds = () => ({ start: f.now, end: f.now + 1 });
  async function request(key: string, origin: MemoryActor | undefined = f.analyst, kind = "submit", decisionRef?: string) {
    const view = { ...newView(key, "BTC", kind), exchangeOid: "123", cloid: kind === "submit" ? cloid : null, commandStatus: "rejected" as const, orderStatus: "rejected", error: "Protective leg rejected", size: "0.00000001", price: "100000.0001" };
    const input = { request_id: key, ...(decisionRef ? { decision_ref: decisionRef } : {}) };
    const actions = [{ payload: { kind: "cancel" as const, cancels: [] }, items: [view] }];
    const row = await store.prepare("agent", account, key, kind, input, actions, origin);
    return { row, input, actions };
  }
  async function fill(id: string, oid: number, value?: string) {
    await f.db.run("INSERT INTO hv_journal_entries(account,id,kind,time,data) VALUES (?,?,'fill',?,?)", account, id, f.now, JSON.stringify({ symbol: "BTC", fee: "0.00000001", pnl: "0", fill: { size: "0.00000001", price: "100000.0001" }, raw: { oid, cloid: value } }));
  }
  it("joins exact fills to submit requests/decisions and host provenance, never to cancellation origin", async () => {
    const decision = await f.services().memory.recordDecision(f.analyst, "decision", { topic: "entry", instrument: "BTC", summary: "Entry", content: "Rationale", payload: { version: 1, action: "enter", direction: "long", reason: "Observe", researchRefs: [], plan: "TP/SL" } });
    await request("entry", f.analyst, "submit", decision.ref);
    await request("cancel", f.reviewer, "cancel");
    await fill("local", 123, cloid);
    await fill("external", 999);
    const audit = await executionAudit(f.db, "agent", { ...bounds(), section: "fills" });
    expect(audit.items).toContainEqual(expect.objectContaining({ fillId: "local", requestId: "entry", decisionRef: decision.ref, fee: "0.00000001", origin: { producerSubagentId: "analyst", sourceRunId: "run-analyst", conversationId: 1, toolTaskId: 2 } }));
    expect(audit.items).toContainEqual(expect.objectContaining({ fillId: "external", requestId: null, decisionRef: null, origin: null }));
    const foreign = await executionAudit(f.db, "other", { ...bounds(), section: "fills" });
    expect(foreign.items).toEqual([]);
    const requests = await executionAudit(f.db, "agent", { ...bounds(), section: "requests" });
    expect(requests.items).toContainEqual(expect.objectContaining({ requestId: "cancel", origin: expect.objectContaining({ producerSubagentId: "reviewer" }) }));
    expect(JSON.stringify(requests.items)).toContain("Protective leg rejected");
    expect(JSON.stringify(requests.items)).toContain("100000.0001");
  });
  it("falls back to exact OID if the exchange CLOID has no local match", async () => {
    await request("entry");
    await fill("local", 123, `0x${"ff".repeat(16)}`);
    expect((await executionAudit(f.db, "agent", { ...bounds(), section: "fills" })).items[0]).toMatchObject({ requestId: "entry" });
  });
  it("paginates timestamp ties without loss and preserves unknown provenance", async () => {
    await request("untracked", undefined);
    // Explicitly model a legacy/non-model request, with no provenance row.
    await f.db.run("DELETE FROM hv_order_request_provenance");
    await fill("a", 123); await fill("b", 999); await fill("c", 999);
    const first = await executionAudit(f.db, "agent", { ...bounds(), section: "fills", limit: 2 });
    const second = await executionAudit(f.db, "agent", { ...bounds(), section: "fills", limit: 2, before: first.next });
    expect(first.items).toHaveLength(2); expect(second.items).toHaveLength(1); expect(second.next).toBeNull();
    expect([...first.items, ...second.items].map((item) => "fillId" in item && item.fillId)).toEqual(["c", "b", "a"]);
    expect(second.items[0]).toMatchObject({ requestId: "untracked", origin: null });
  });
  it("retains the original provenance on retries, after terminal runs and reopen", async () => {
    const { row, input, actions } = await request("entry");
    await f.db.run("UPDATE hv_subagent_runs SET status='completed' WHERE id='run-analyst'");
    await f.reopen(); store = new OrderStore(f.db);
    expect((await store.prepare("agent", account, "entry", "submit", input, actions, f.reviewer)).id).toBe(row.id);
    expect(await f.db.get("SELECT producer_subagent_id,source_run_id,conversation_id,tool_task_id FROM hv_order_request_provenance WHERE request_id=?", row.id)).toMatchObject({ producer_subagent_id: "analyst", source_run_id: "run-analyst", conversation_id: 1, tool_task_id: 2 });
  });
  it("rolls back the entire request if trusted provenance is invalid or no longer active", async () => {
    await expect(request("cross-agent", { ...f.analyst, agentId: "other" })).rejects.toThrow("Invalid execution provenance");
    await expect(request("cross-run", { ...f.analyst, runId: "run-reviewer" })).rejects.toThrow("does not match");
    await f.db.run("UPDATE hv_subagent_runs SET status='completed' WHERE id='run-analyst'");
    await expect(request("terminal")).rejects.toThrow("no longer active");
    expect(await f.db.get("SELECT count(*) AS total FROM hv_order_requests")).toMatchObject({ total: 0 });
    expect(await f.db.get("SELECT count(*) AS total FROM hv_order_actions")).toMatchObject({ total: 0 });
  });
  it("returns decisions without inventing requests or absent rationale", async () => {
    await f.services().memory.recordDecision(f.analyst, "hold", { topic: "hold", summary: "Hold", content: "Observe only", payload: { version: 1, action: "hold", direction: null, reason: "Wait", researchRefs: [], plan: "" } });
    const audit = await executionAudit(f.db, "agent", { ...bounds(), section: "decisions" });
    expect(audit.items[0]).toMatchObject({ ref: "m1", linkedRequests: [], payload: { action: "hold" }, sourceRunId: "run-analyst" });
    expect(audit.coverage).toContain("does not prove no action");
  });
  it("rejects invalid bounds, cursors and unscoped queries", async () => {
    for (const patch of [{ end: f.now }, { section: "private_tables" }, { limit: 51 }, { before: { time: -1, id: "x" } }, { agentId: "other" }]) {
      await expect(executionAudit(f.db, "agent", { ...bounds(), section: "fills", ...patch })).rejects.toThrow();
    }
  });
});
