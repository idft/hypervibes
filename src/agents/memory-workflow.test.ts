import { createModels } from "@earendil-works/pi-ai/models";
import { fauxAssistantMessage, fauxProvider, fauxToolCall, type FauxResponseFactory } from "@earendil-works/pi-ai/providers/faux";
import type { Message } from "@earendil-works/pi-ai";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { randomUUID } from "node:crypto";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createRuntime, type AppRuntime } from "../runtime.js";
import { testConfig } from "../test-config.js";
import { templateDraft } from "./subagent-templates.js";
import type { SubagentRow } from "../db/subagent-schema.js";
import type { JsonObject } from "@earendil-works/pi-durable";

function output(messages: readonly Message[], name: string): JsonObject {
  const message = [...messages].reverse().find((message) => message.role === "toolResult" && message.toolName === name);
  if (!message) return {};
  const content = typeof message.content === "string" ? message.content : message.content.filter((part) => part.type === "text").map((part) => part.text).join("");
  try { return JSON.parse(content) as JsonObject; } catch { return {}; }
}
const call = (name: string, args: JsonObject) => fauxAssistantMessage(fauxToolCall(name, args), { stopReason: "toolUse" });
const selection = { provider: "faux", modelId: "faux-1", thinkingLevel: "" };

describe("fully mocked scheduled analyst/trader/reviewer workflow", () => {
  let runtime: AppRuntime | undefined, directory: string;
  afterEach(async () => { await runtime?.close(); vi.unstubAllGlobals(); vi.restoreAllMocks(); if (directory) rmSync(directory, { recursive: true, force: true }); });
  it("publishes, gates and audits protected entries, reserves setups, honors user delegation and curates next-run learnings", async () => {
    directory = mkdtempSync(join(tmpdir(), "hv-memory-workflow-"));
    const boundary = Math.floor(Date.now() / 3600_000) * 3600_000;
    let now = boundary + 30000, oid = 100, exchanges = 0;
    vi.spyOn(Date, "now").mockImplementation(() => now);
    let positions: { position: { coin: string; szi: string; entryPx: string; unrealizedPnl: string; liquidationPx: null; leverage: { value: number } } }[] = [];
    vi.stubGlobal("fetch", vi.fn(async (_url: unknown, init: RequestInit) => {
      const body = JSON.parse(String(init.body)) as { type?: string; action?: { orders?: { s: string }[] } };
      if (body.action) {
        exchanges++;
        const orders = body.action.orders ?? [];
        return Response.json({ status: "ok", response: { type: "order", data: { statuses: orders.map((order, index) => index === 0 ? { filled: { oid: ++oid, totalSz: order.s, avgPx: "100000" } } : "waitingForFill") } } });
      }
      if (body.type === "meta") return Response.json({ universe: [{ name: "BTC", szDecimals: 5 }, { name: "ETH", szDecimals: 4 }] });
      if (body.type === "allMids") return Response.json({ BTC: "100000", ETH: "2000" });
      if (body.type === "clearinghouseState") return Response.json({ marginSummary: { accountValue: "1000" }, assetPositions: positions });
      if (body.type === "spotClearinghouseState") return Response.json({ balances: [{ coin: "USDC", total: "1000", hold: "0" }] });
      if (body.type === "userAbstraction") return Response.json("default");
      if (body.type === "frontendOpenOrders") return Response.json([]);
      if (body.type === "orderStatus") return Response.json({ status: "unknownOid" });
      throw new Error(`Unexpected mocked exchange request ${body.type}`);
    }));
    const models = createModels(), faux = fauxProvider(); models.setProvider(faux.provider);
    runtime = await createRuntime(testConfig({ APP_SQLITE_PATH: join(directory, "app.sqlite") }), { models, model: { provider: "faux", modelId: "faux-1" }, accountFeedFactory: () => ({ close: async () => {} }), journalFeedFactory: () => ({ close: async () => {} }), journalHistoryFetcher: async () => [] });
    const app = runtime;
    await app.database.run("INSERT INTO hv_users(id,name,email) VALUES ('owner','Owner','owner@example.test')");
    const account = `0x${"a".repeat(40)}`;
    vi.spyOn(app.hyperliquid, "orderAuthority").mockResolvedValue({ account, privateKey: `0x${"12".repeat(32)}`, vaultAddress: account as `0x${string}`, builder: { b: `0x${"ef".repeat(20)}`, f: 10 } });
    vi.spyOn(app.hyperliquid.charts, "instruments").mockResolvedValue(["BTC", "ETH"]);
    app.start(); await app.agents.subagents.tick();
    const agent = await app.agents.create("owner", "Mock desk", randomUUID(), account);
    const analyst = await app.agents.subagents.create("owner", agent.id, { ...templateDraft("basic-analysis", selection), workflow: { version: 1, purpose: "analysis", instruments: ["BTC", "ETH"], interval: "1h", topic: "trend_setup", maxResearchAgeSeconds: 3600, strategy: "Fixture EMA alignment" } }, randomUUID(), "basic-analysis");
    const trader = await app.agents.subagents.create("owner", agent.id, { ...templateDraft("basic-trading", selection), workflow: { version: 1, purpose: "trading", instruments: ["BTC", "ETH"], automaticEntriesEnabled: false, requiredResearch: [{ subagentId: analyst.id, topic: "trend_setup", interval: "1h", maxAgeSeconds: 3600 }], entryStrategy: "Ready bullish only", exitStrategy: "Complete invalidation, not expiry", baseSizeByInstrument: { BTC: "0.001", ETH: "0.01" }, maxEntryNotional: "1000", takeProfitBps: 200, stopLossBps: 100 } }, randomUUID(), "basic-trading");
    const reviewer = await app.agents.subagents.create("owner", agent.id, { ...templateDraft("basic-review", selection), workflow: { version: 1, purpose: "review", lookbackHours: 24, learningTargetSubagentIds: [trader.id] } }, randomUUID(), "basic-review");
    expect([analyst, trader, reviewer].every((worker) => !worker.enabled && worker.nextRunAt === null && !worker.persistent)).toBe(true);
    expect(exchanges).toBe(0);
    let published = false, mode: "enter" | "protect" = "enter", logicalAttempt = 0;
    const factory: FauxResponseFactory = (request) => {
      const text = JSON.stringify(request.messages);
      const tools = request.messages.filter((message) => message.role === "toolResult");
      if (text.includes("You are the HyperVibes worker Analysis")) {
        if (published || tools.length) return fauxAssistantMessage("Analysis saved / no new evidence");
        published = true;
        return call("publish_research", { instrument: "BTC", interval: "1h", topic: "trend_setup", summary: "Fixture ready bullish alignment", content: "Fixture closed-candle observations; trend hypothesis, not numerical verification by the host.", evidenceAsOf: Math.floor(now / 3600_000) * 3600_000, payload: { version: 1, bias: "long", setup: "ready", observations: ["Fixture close > EMA20 > EMA50"], interpretation: "Demonstration only", invalidation: ["Later complete closed-candle loss of alignment"], coverage: "complete" } });
      }
      if (text.includes("You are the HyperVibes worker Trading")) {
        if (text.includes("OBSERVE_NEW_LEARNINGS")) return fauxAssistantMessage("Observed revised guidance; no trade");
        const names = tools.map((tool) => tool.toolName);
        if (mode === "protect" && !text.includes("USER_ETH_TRADE")) {
          if (!names.includes("get_protection_status")) return call("get_protection_status", {});
          if (!names.includes("prepare_position_protection")) return call("prepare_position_protection", { instrument: "BTC" });
          if (!names.includes("submit_orders")) return call("submit_orders", { request_id: "reduce-only-repair", orders: output(request.messages, "prepare_position_protection").orders! });
          return fauxAssistantMessage("Protection legs inspected; no perpetual guarantee");
        }
        const symbol = text.includes("USER_ETH_TRADE") ? "ETH" : "BTC";
        if (!names.includes("get_protection_status")) { logicalAttempt++; return call("get_protection_status", {}); }
        if (!names.includes("get_trading_context")) return call("get_trading_context", { instrument: symbol });
        if (!names.includes("record_trade_decision")) return call("record_trade_decision", { topic: "entry", instrument: symbol, summary: `Attempt ${logicalAttempt}`, content: "An explicitly protected entry plan; backend may reject it.", payload: { version: 1, action: "enter", direction: "long", reason: symbol === "ETH" ? "Actual authenticated user assignment; research absent" : "Fixture ready setup", researchRefs: symbol === "ETH" ? [] : ["m1"], plan: "Use exact host planner with full-size TP and market SL" } });
        if (!names.includes("prepare_protected_entry")) return call("prepare_protected_entry", { instrument: symbol, side: "buy", entry_style: "guarded_ioc" });
        if (!names.includes("submit_orders")) return call("submit_orders", { request_id: `attempt-${logicalAttempt}`, decision_ref: output(request.messages, "record_trade_decision").ref!, orders: output(request.messages, "prepare_protected_entry").orders! });
        return fauxAssistantMessage("Inspected individual outcomes; rejected/unknown is not successful protection");
      }
      if (text.includes("You are the HyperVibes worker Review")) {
        if (tools.length === 0) return call("research_decisions", { start: now - 86400_000, end: now + 1, limit: 50 });
        if (tools.length === 1) return call("publish_review", { topic: "daily_review", summary: "Fixture operational review", content: "Review decisions and coverage; no causal inference from sparse performance.", payload: { version: 1, start: now - 86400_000, end: now, coverage: "incomplete", limitations: ["Mocked exchange; no full journal history"], findings: ["Duplicate and stale openings blocked; protection remains independent"], recommendations: ["Preserve original request IDs"], memoryRefs: ["m1", "m2"] } });
        if (tools.length === 2) return call("get_learnings", { scope: { kind: "agent" } });
        if (tools.length === 3) return call("append_learning", { scope: { kind: "agent" }, content: "Reconcile uncertain orders using their original request ID.", reason: "Operational review", evidenceRefs: [output(request.messages, "publish_review").ref!] });
        if (tools.length === 4) return call("get_learnings", { scope: { kind: "worker", subagentId: trader.id } });
        if (tools.length === 5) return call("append_learning", { scope: { kind: "worker", subagentId: trader.id }, content: "Treat waitingForFill as not verified active protection for partial positions.", reason: "Protection review", evidenceRefs: [output(request.messages, "publish_review").ref!] });
        return fauxAssistantMessage("Review and both learning updates completed separately");
      }
      if (text.includes("[Subagent Trading;")) return fauxAssistantMessage("User trade result acknowledged; report-driven root remains automatic");
      if (tools.length) return fauxAssistantMessage("User assignment delegated; awaiting report");
      return call("delegate_subagent", { subagent_id: trader.id, task: "USER_ETH_TRADE: user requests a protected ETH long of base size 0.01, no research is available" });
    };
    faux.setResponses(Array.from({ length: 150 }, () => factory));
    async function completed(worker: SubagentRow) {
      await vi.waitFor(async () => expect((await app.agents.subagents.detail("owner", agent.id, worker.id)).runs[0]?.status).toBe("completed"), { timeout: 10000 });
      return (await app.agents.subagents.detail("owner", agent.id, worker.id)).runs[0]!;
    }
    async function enable(worker: SubagentRow) { return app.agents.subagents.updateSection("owner", agent.id, worker.id, "enabled", { enabled: true }); }
    let enabled = await enable(analyst); now = enabled.nextRunAt!; await app.agents.subagents.tick(now); await completed(analyst);
    expect((await app.agents.execution.memories.get(agent.id, "m1")).executionOrigin).toBe("automatic");
    const workflow = trader.workflow!;
    if (workflow.purpose !== "trading") throw new Error("Invalid fixture");
    await app.agents.subagents.updateSection("owner", agent.id, trader.id, "workflow", { workflow: { ...workflow, automaticEntriesEnabled: true } });
    enabled = await enable(trader); now = enabled.nextRunAt!; await app.agents.subagents.tick(now);
    const first = await completed(trader);
    const firstDetail = await app.agents.subagents.runDetail("owner", agent.id, trader.id, first.id);
    expect(firstDetail.context!.learnings).toMatchObject({ agent: { version: 0 }, worker: { version: 0 } });
    expect(exchanges).toBe(1);
    expect(await app.orders.request("owner", agent.id, "attempt-1")).toMatchObject({ execution: { origin: "automatic", decisionRef: "m2" }, results: [{ kind: "limit" }, { kind: "take_profit" }, { kind: "stop_loss" }] });
    expect(await app.database.get("SELECT count(*) AS total FROM hv_automatic_entry_claims")).toMatchObject({ total: 1 });
    // Repeat same exact setup under a new model request ID: atomic claim rolls it back.
    enabled = (await app.agents.subagents.list("owner", agent.id)).find((worker) => worker.id === trader.id)!;
    now = enabled.nextRunAt!; await app.agents.subagents.tick(now); const duplicate = await completed(trader);
    expect(JSON.stringify((await app.agents.subagents.runDetail("owner", agent.id, trader.id, duplicate.id)).entries)).toContain("already belongs");
    expect(exchanges).toBe(1);
    expect(await app.database.get("SELECT count(*) AS total FROM hv_order_requests")).toMatchObject({ total: 1 });
    // Let evidence expire while the enabled analyst supplies no new publication.
    now = Math.floor(first.createdAt / 3600_000) * 3600_000 + 3600_000 + 45000;
    // Disposable fixture only: align the trader cursor to this deterministic evaluation.
    await app.database.run("UPDATE hv_subagents SET next_run_at=? WHERE id=?", now, trader.id);
    await app.agents.subagents.tick(now); const stale = await completed(trader);
    expect(JSON.stringify((await app.agents.subagents.runDetail("owner", agent.id, trader.id, stale.id)).entries)).toContain("Automatic entry blocked");
    expect(exchanges).toBe(1);
    positions = [{ position: { coin: "BTC", szi: "0.0005", entryPx: "100000", unrealizedPnl: "0", liquidationPx: null, leverage: { value: 1 } } }];
    mode = "protect";
    now = (await app.agents.subagents.list("owner", agent.id)).find((worker) => worker.id === trader.id)!.nextRunAt!;
    await app.agents.subagents.tick(now); const repaired = await completed(trader);
    expect((await app.agents.execution.runContext(agent.id, repaired)).origin).toMatchObject({ mode: "automatic", reason: "schedule" });
    expect((await app.orders.request("owner", agent.id, "reduce-only-repair")).execution).toMatchObject({ origin: "automatic", decisionRef: null });
    expect(exchanges).toBe(3); // two separate reduce-only actions, no fresh-research requirement
    positions = []; now++; // mocked external close before the separate instrument user assignment
    await app.agents.send("owner", agent.id, "Please execute a protected ETH long of 0.01 base units without waiting for analysis", randomUUID());
    await vi.waitFor(() => expect(exchanges).toBe(4), { timeout: 10000 });
    await completed(trader);
    await vi.waitFor(async () => expect((await app.agents.chat("owner", agent.id)).busy).toBe(false));
    const userOrder = await app.database.get<{ request_key: string; origin: string; decision_number: number }>("SELECT r.request_key,c.origin,c.decision_number FROM hv_order_execution_contexts c JOIN hv_order_requests r ON r.id=c.request_id WHERE c.agent_id=? ORDER BY r.created_at DESC,r.id DESC", agent.id);
    const attributed = await app.database.all<{ origin: string; decision_number: number }>("SELECT origin,decision_number FROM hv_order_execution_contexts WHERE agent_id=?", agent.id);
    expect(attributed.some((row) => JSON.parse(row.origin).mode === "user_directed")).toBe(true);
    expect(userOrder).toBeDefined();
    await app.agents.subagents.updateSection("owner", agent.id, analyst.id, "enabled", { enabled: false });
    await app.agents.subagents.updateSection("owner", agent.id, trader.id, "enabled", { enabled: false });
    enabled = await enable(reviewer); now = enabled.nextRunAt!; await app.agents.subagents.tick(now); const reviewed = await completed(reviewer);
    expect((await app.agents.execution.runContext(agent.id, reviewed)).origin).toMatchObject({ mode: "automatic", reason: "schedule" });
    expect(await app.agents.execution.learnings.current(agent.id, { kind: "agent" })).toMatchObject({ version: 1 });
    expect(await app.agents.execution.learnings.current(agent.id, { kind: "worker", subagentId: trader.id })).toMatchObject({ version: 1 });
    await enable(trader); now++;
    await app.agents.subagents.run("owner", agent.id, trader.id, "OBSERVE_NEW_LEARNINGS, do not trade", randomUUID()); const next = await completed(trader);
    expect((await app.agents.execution.runContext(agent.id, next)).learnings).toMatchObject({ agent: { version: 1 }, worker: { version: 1 } });
    expect((await app.agents.subagents.runDetail("owner", agent.id, trader.id, first.id)).context).toEqual(firstDetail.context);
    expect(exchanges).toBe(4);
    // Audit can be inspected without exchange credentials or transcripts.
    expect((await app.agents.execution.memories.list(agent.id, { kind: "review" })).items).toHaveLength(1);
    expect(await app.database.all("PRAGMA foreign_key_check")).toEqual([]);
  }, 60000);
});
