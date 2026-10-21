import { RiskSettingsService } from "../hyperliquid/orders/risk-settings.js";
import { createModels } from "@earendil-works/pi-ai/models";
import { fauxAssistantMessage, fauxProvider, fauxToolCall } from "@earendil-works/pi-ai/providers/faux";
import { randomUUID } from "node:crypto";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createRuntime, type AppRuntime } from "../runtime.js";
import { testConfig } from "../test-config.js";
import { fillEvent, type JournalEvent } from "../hyperliquid/journal-model.js";
import { templateDraft } from "./subagent-templates.js";
import type { SubagentInput } from "./subagent-input.js";
import { researchInput } from "../memory/test-fixture.js";

const account = `0x${"ab".repeat(20)}`;
const settings = (patch: Partial<SubagentInput> = {}): SubagentInput => ({ name: "Worker", description: "Evidence", instructions: "Observe only", provider: "faux", modelId: "faux-1", thinkingLevel: "", tools: [], enabled: false, persistent: false, delegatable: true, schedule: "", delaySeconds: 1, timeoutSeconds: 900, completion: "save", ...patch });
const response = (name: string, args: Parameters<typeof fauxToolCall>[1]) => fauxAssistantMessage(fauxToolCall(name, args), { stopReason: "toolUse" });
describe("review evidence and automatic learning cycle", () => {
  let directory: string, runtime: AppRuntime, agentId: string, faux: ReturnType<typeof fauxProvider>, models: ReturnType<typeof createModels>;
  let history: JournalEvent[], sends: number;
  const config = () => testConfig({ APP_SQLITE_PATH: join(directory, "app.sqlite") });
  const options = () => ({ models, model: { provider: "faux", modelId: "faux-1" }, accountFeedFactory: () => ({ close: async () => {} }), journalFeedFactory: () => ({ close: async () => {} }), journalHistoryFetcher: async (_account: string, stream: string, start: number, end: number) => history.filter((event) => event.kind === stream && event.time >= start && event.time <= end) });
  beforeEach(async () => {
    directory = mkdtempSync(join(tmpdir(), "hv-review-")); history = []; sends = 0;
    faux = fauxProvider(); models = createModels(); models.setProvider(faux.provider);
    vi.stubGlobal("fetch", vi.fn(async (_url: unknown, init: RequestInit) => {
      const body = JSON.parse(String(init.body)) as { type?: string; action?: { orders?: unknown[] } };
      if (body.action) { sends++; return Response.json({ status: "ok", response: { type: "order", data: { statuses: [{ filled: { oid: 123, totalSz: "0.0005", avgPx: "100000" } }, "waitingForFill", { error: "SL rejected" }] } } }); }
      if (body.type === "meta") return Response.json({ universe: [{ name: "BTC", szDecimals: 5 }] });
      if (body.type === "allMids") return Response.json({ BTC: "100000" });
      if (body.type === "orderStatus") return Response.json({ status: "unknownOid" });
      throw new Error(`Unexpected simulated exchange call: ${body.type}`);
    }));
    runtime = await createRuntime(config(), options());
    await runtime.database.run("INSERT INTO hv_users(id,name,email) VALUES ('owner','Owner','owner@example.test')");
    agentId = (await runtime.agents.create("owner", "Desk", randomUUID(), account)).id;
    runtime.start(); await runtime.agents.subagents.tick();
  });
  afterEach(async () => { await runtime.close(); vi.restoreAllMocks(); vi.unstubAllGlobals(); rmSync(directory, { recursive: true, force: true }); });
  async function worker(input: SubagentInput) { return runtime.agents.subagents.create("owner", agentId, input, randomUUID()); }
  async function run(id: string, task = "Inspect the explicit assignment") {
    const admitted = await runtime.agents.subagents.run("owner", agentId, id, task, randomUUID());
    await vi.waitFor(async () => expect((await runtime.agents.subagents.detail("owner", agentId, id)).runs.find((run) => run.id === admitted.id)?.status).toBe("completed"));
    return runtime.agents.subagents.runDetail("owner", agentId, id, admitted.id);
  }
  it("exposes generic, paginated frozen run evidence without leaking subsequent persistent assignments", async () => {
    const observer = await worker(settings({ name: "Observer", persistent: true, instructions: "Original instructions " + "x".repeat(12000) }));
    faux.setResponses([fauxAssistantMessage("FIRST result " + "a".repeat(12000))]);
    const first = await run(observer.id, "FIRST task");
    await runtime.agents.subagents.updateSection("owner", agentId, observer.id, "instructions", { instructions: "SECOND instructions" });
    faux.setResponses([fauxAssistantMessage("SECOND result")]);
    const second = await run(observer.id, "SECOND task");
    expect(second.run.conversationId).toBe(first.run.conversationId);
    const instructions = await runtime.agents.subagents.readRunEvidence(agentId, first.run.id, { run_id: first.run.id, section: "instructions", limit: 50 });
    expect(instructions).toMatchObject({ nextOffset: 50, insightStatus: "snapshot" });
    expect(JSON.stringify(instructions)).not.toContain("SECOND instructions");
    const result = await runtime.agents.subagents.readRunEvidence(agentId, first.run.id, { run_id: first.run.id, section: "result", offset: 12000, limit: 100 });
    expect(result).toMatchObject({ text: expect.stringContaining("a"), nextOffset: null });
    let after: number | undefined;
    const pages = [];
    do {
      const page = await runtime.agents.subagents.readRunEvidence(agentId, first.run.id, { run_id: first.run.id, section: "activity", limit: 1, ...(after === undefined ? {} : { after_entry: after }) }) as { items: unknown[]; nextAfterEntry: number | null };
      pages.push(...page.items); after = page.nextAfterEntry ?? undefined;
    } while (after !== undefined);
    expect(JSON.stringify(pages)).toContain("FIRST result");
    expect(JSON.stringify(pages)).not.toContain("SECOND");
    expect(JSON.stringify(pages)).toContain(`admittedAtMs: ${first.run.createdAt}`);
    const jumped = await runtime.agents.subagents.readRunEvidence(agentId, first.run.id, { run_id: first.run.id, section: "activity", after_entry: Number.MAX_SAFE_INTEGER - 1 });
    expect(jumped).toMatchObject({ items: [], nextAfterEntry: null });
    await runtime.agents.subagents.archive("owner", agentId, observer.id);
    const listed = await runtime.agents.subagents.listRunEvidence(agentId, { start: first.run.createdAt, end: Date.now() + 1, limit: 1 });
    expect(listed.items).toHaveLength(1); expect(listed.next).not.toBeNull();
    const next = await runtime.agents.subagents.listRunEvidence(agentId, { start: first.run.createdAt, end: Date.now() + 1, limit: 1, before: listed.next });
    expect(next.items).toHaveLength(1); expect(next.items[0]!.id).not.toBe(listed.items[0]!.id);
    await expect(runtime.agents.subagents.readRunEvidence("foreign-agent", first.run.id, { run_id: first.run.id })).rejects.toThrow("not found");
    expect((await runtime.agents.subagents.listRunEvidence("foreign-agent", { start: Date.now() - 86400_000, end: Date.now() })).items).toEqual([]);
  });
  it.each([false, true])("publishes research directly from Code Mode with computed timestamps (explicit expiry: %s)", async (explicitExpiry) => {
    const analyst = await worker(settings({ name: "Analyst", tools: ["codemode", "research_candles", "publish_research"] }));
    const start = Date.parse("2025-01-01T00:00:00.000Z"), width = 15 * 60 * 1000, end = start + 4 * width;
    const candles = Array.from({ length: 4 }, (_, i) => ({ time: (start + i * width) / 1000, open: 100, high: 102, low: 99, close: 101, volume: 20 }));
    vi.spyOn(runtime.hyperliquid.charts, "researchHistory").mockResolvedValue({ coin: "BTC", interval: "15m", start, end, pageEnd: end, candles, nextStart: null, coverage: { complete: true, returned: 4, expected: 4, closedOnly: true, sourceLimit: "fixture" } });
    const code = `
      const schema = await describeTool('publish_research');
      if (!JSON.stringify(schema).includes('evidenceAsOf')) throw new Error('Missing publication schema');
      const intervalMs = 15 * 60 * 1000;
      const start = Date.parse('2025-01-01T00:00:00.000Z');
      const endMs = start + 4 * intervalMs;
      const result = await tools.research_candles({coin:'BTC',interval:'15m',start,end:endMs});
      const last = result.candles.at(-1);
      if (!last || last.time * 1000 + intervalMs !== endMs) throw new Error('Incomplete evidence');
      const report = {topic:'trend_setup',instrument:'BTC',interval:'15m',summary:'Mixed fixture evidence',content:'Closed-candle fixture analysis; no trading authority.'};
      const receipt = await tools.publish_research({...report,evidenceAsOf:endMs${explicitExpiry ? ",requestedValidUntil:endMs + 6 * 60 * 60 * 1000" : ""}});
      text(receipt);
    `;
    faux.setResponses([response("codemode", { code }), fauxAssistantMessage("Published")]);
    const analysis = await run(analyst.id, "Analyze BTC fixture candles");
    expect(analysis.tools).toContainEqual(expect.objectContaining({ name: "↳ publish_research", status: "ok" }));
    expect(analysis.tools?.some((tool) => tool.status === "error")).toBe(false);
    expect(analysis.publications).toMatchObject([{ ref: "m1", kind: "research" }]);
    expect(await runtime.database.get("SELECT evidence_as_of,valid_until,source_run_id FROM hv_memories WHERE agent_id=? AND number=1", agentId)).toEqual({ evidence_as_of: end, valid_until: end + (explicitExpiry ? 6 * 60 * 60 * 1000 : 2 * width), source_run_id: analysis.run.id });
  });
  it("traces partial execution after restart, lets the Review model publish/curate, and injects revisions into the next run", async () => {
    const analyst = await worker(settings({ name: "Analyst", tools: ["publish_research"] }));
    const boundary = Math.floor(Date.now() / 3600000) * 3600000;
    faux.setResponses([response("publish_research", researchInput(boundary)), fauxAssistantMessage("Published")]);
    const analysis = await run(analyst.id);
    const trader = await worker(settings({ name: "Trader", tools: ["record_trade_decision", "submit_orders"] }));
    vi.spyOn(runtime.hyperliquid, "orderAuthority").mockResolvedValue({ account, privateKey: `0x${"12".repeat(32)}`, builder: { b: `0x${"ef".repeat(20)}`, f: 10 } });
    await new RiskSettingsService(runtime.database).save(agentId, { tradingEnabled: true, allowedInstruments: ["BTC"], maxOrderNotional: "200", maxTotalNotional: null });
    faux.setResponses([
      response("record_trade_decision", { topic: "entry", instrument: "BTC", summary: "Test protected entry", content: "Using m1", payload: { version: 1, action: "enter", direction: "long", reason: "Test hypothesis", researchRefs: ["m1"], plan: "Attached TP/SL" } }),
      response("submit_orders", { request_id: "partial-entry", decision_ref: "m2", orders: [{ symbol: "BTC", side: "buy", order_type: "limit", size: "0.001", price: "100000", take_profits: [{ trigger_price: "110000" }], stop_losses: [{ trigger_price: "90000" }] }] }),
      fauxAssistantMessage("Partial fill; SL rejected. Protection is not verified."),
    ]);
    const trade = await run(trader.id);
    expect(trade.tools?.filter((tool) => tool.status === "error")).toEqual([]);
    expect(sends).toBe(1);
    const item = await runtime.database.get<{ cloid: `0x${string}` }>("SELECT i.cloid FROM hv_order_items i JOIN hv_order_actions a ON a.id=i.action_id JOIN hv_order_requests r ON r.id=a.request_id WHERE r.agent_id=? AND r.request_key='partial-entry' AND i.ordinal=0", agentId);
    history = [fillEvent({ coin: "BTC", hash: "0xtest", tid: 1, oid: 123, cloid: item!.cloid, time: trade.run.createdAt, side: "B", sz: "0.0005", px: "100000", startPosition: "0", fee: "0.01", feeToken: "USDC", closedPnl: "0", dir: "Open Long", crossed: false, twapId: null })];
    await runtime.close(); runtime = await createRuntime(config(), options()); runtime.start(); await runtime.agents.subagents.tick();
    const draft = templateDraft("basic-review", { provider: "faux", modelId: "faux-1", thinkingLevel: "" });
    const reviewer = await runtime.agents.subagents.create("owner", agentId, draft, randomUUID(), "basic-review");
    const start = analysis.run.createdAt - 1, end = Date.now();
    const code = `
      const window = {start:${start},end:${end}};
      const journal = await tools.research_journal({...window,limit:5});
      if (!journal.coverage.complete) throw new Error('Expected synchronized evidence');
      const fills = await tools.research_execution_audit({...window,section:'fills'});
      if (fills.items[0]?.origin?.sourceRunId !== ${JSON.stringify(trade.run.id)}) throw new Error('Wrong run provenance');
      if (fills.items[0]?.decisionRef !== 'm2') throw new Error('Missing decision link');
      const requests = await tools.research_execution_audit({...window,section:'requests'});
      if (!requests.items.some(r => r.orders.some(o => o.error?.includes('SL rejected')))) throw new Error('Missing rejected leg');
      const historical = await tools.get_subagent_run({run_id:${JSON.stringify(trade.run.id)},section:'instructions'});
      const prior = await tools.get_memory({ref:'m2'});
      const research = await tools.get_memory({ref:prior.payload.researchRefs[0]});
      const runs = await tools.list_subagent_runs(window);
      text({run:fills.items[0].origin.sourceRunId,research:research.ref,insightVersions:historical.insightVersions,runs:runs.items.length});`;
    const review = { topic: "execution_review", summary: "Partial execution needs protection verification", content: "Observed partial fill and rejected SL. This is an execution failure, not evidence that research was wrong. Sparse sample; no strategy-performance conclusion.", payload: { version: 1, start, end, coverage: "complete", limitations: ["One simulated trade; no causal profitability claim"], findings: ["SL rejected after partial entry"], recommendations: ["Verify actual protection after every partial fill"], memoryRefs: ["m1", "m2"] } };
    faux.setResponses([
      response("codemode", { code }), response("publish_review", review),
      response("get_insights", { scope: { kind: "worker", subagentId: trader.id } }),
      response("append_insight", { scope: { kind: "worker", subagentId: trader.id }, content: "Verify actual protection after partial fills; never treat waitingForFill children as active coverage.", reason: "Observed execution failure, not a strategy change", evidenceRefs: ["m3", "m2"] }),
      fauxAssistantMessage("Published m3. Updated Trader insights to v1; no change to the research strategy."),
    ]);
    const reviewed = await run(reviewer.id, `Review exact UTC window start=${start}, end=${end}.`);
    expect(reviewed.tools).toContainEqual(expect.objectContaining({ name: "codemode", status: "done" }));
    expect(reviewed.tools?.some((tool) => tool.status === "error")).toBe(false);
    expect(reviewed.publications).toMatchObject([{ ref: "m3", kind: "review" }]);
    expect((await runtime.agents.execution.insights.current(agentId, { kind: "worker", subagentId: trader.id })).version).toBe(1);
    faux.setResponses([fauxAssistantMessage("Applied the new protection guidance")]);
    const next = await run(trader.id);
    expect(next.context?.instructions).toContain("Verify actual protection after partial fills");
    expect(next.context?.insights?.worker.version).toBe(1);
    expect(sends).toBe(1);
    expect((await runtime.agents.execution.memories.list(agentId, { kind: "review" })).items).toHaveLength(1);
  });
  it("preserves a published review when a later insight replacement conflicts", async () => {
    const reviewer = await worker(templateDraft("basic-review", { provider: "faux", modelId: "faux-1", thinkingLevel: "" }));
    const scope = { kind: "agent" as const };
    await runtime.agents.execution.insights.append(runtime.agents.execution.operator(agentId, "owner"), "operator:existing", { scope, content: "Retain this operator guidance", reason: "Initial" });
    const end = Date.now(), start = end - 1000;
    faux.setResponses([
      response("publish_review", { topic: "interim_review", summary: "Insufficient sample", content: "No changes justified", payload: { version: 1, start, end, coverage: "incomplete", limitations: ["No executions"], findings: [], recommendations: [], memoryRefs: [] } }),
      response("replace_insights", { scope, content: "Should not overwrite", expectedVersion: 0, reason: "Stale draft", evidenceRefs: ["m1"] }),
      fauxAssistantMessage("Published m1; insights unchanged because the version conflicted. Partial completion."),
    ]);
    const result = await run(reviewer.id);
    expect(result.publications).toMatchObject([{ ref: "m1", kind: "review" }]);
    expect(result.tools).toContainEqual(expect.objectContaining({ name: "replace_insights", status: "error" }));
    expect(await runtime.agents.execution.insights.current(agentId, scope)).toMatchObject({ version: 1, content: "Retain this operator guidance" });
    expect((await runtime.agents.execution.memories.list(agentId, { kind: "review" })).items).toHaveLength(1);
  });
});
