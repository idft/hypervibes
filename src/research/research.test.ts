import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { randomUUID } from "node:crypto";
import { createRuntime, type AppRuntime } from "../runtime.js";
import { testConfig } from "../test-config.js";
import { createModels } from "@earendil-works/pi-ai/models";
import { fauxProvider, fauxAssistantMessage, fauxToolCall } from "@earendil-works/pi-ai/providers/faux";
import { CodemodeSandbox } from "@earendil-works/pi-codemode";
import { timeCodeExamples } from "../agents/time-guidance.js";
import { createRegistry, Harness, LiveDoc, MemoryStorage } from "@earendil-works/pi-durable";
import { BACKGROUND_CONTEXT as context } from "@earendil-works/chord/context";
import { toolActivity } from "./tool-activity.js";
import { technicalEvidenceExample, utcFormattingExample } from "./evidence.js";
import { researchJournal } from "./journal.js";
import { indicatorReadToolNames } from "../agents/subagent-input.js";
import { createIndicatorExtension } from "../indicators/durable-extension.js";
import { createResearchResponder } from "./tool-response.js";

// All databases are disposable; no exchange HTTP, listening ports or model calls.
describe("Code Mode research and artifacts", () => {
  let runtime: AppRuntime, directory: string, harness: Harness | undefined;
  let faux: ReturnType<typeof fauxProvider>;
  let models: ReturnType<typeof createModels>;
  let agentId: string, otherAgentId: string;
  beforeEach(async () => {
    directory = mkdtempSync(join(tmpdir(), "hv-research-"));
    faux = fauxProvider(); models = createModels(); models.setProvider(faux.provider);
    runtime = await createRuntime(testConfig({ APP_SQLITE_PATH: join(directory, "app.sqlite") }), { models, model: { provider: faux.getModel().provider, modelId: faux.getModel().id }, journalHistoryFetcher: async () => [], journalFeedFactory: () => ({ close: async () => {} }), accountFeedFactory: () => ({ close: async () => {} }) });
    await runtime.database.run("INSERT INTO hv_users(id,name,email) VALUES ('owner','Owner','owner@example.test')");
    agentId = (await runtime.agents.create("owner", "Research", randomUUID(), `0x${"1".repeat(40)}`)).id;
    otherAgentId = (await runtime.agents.create("owner", "Other", randomUUID(), `0x${"2".repeat(40)}`)).id;
  });
  afterEach(async () => { await harness?.close(context); harness = undefined; await runtime.close(); vi.restoreAllMocks(); rmSync(directory, { recursive: true, force: true }); });
  const report = { title: "Research", mediaType: "text/markdown" as const, content: "# Analysis\nEvidence, not instructions.", tags: ["BTC", "btc", "volatility"], instrument: "BTC", interval: "1h" };

  it("stores BLOBs with normalized tags; immutable operation keys deduplicate concurrent recovery", async () => {
    const service = runtime.agents.research.artifacts;
    const [a, b] = await Promise.all([service.create(agentId, "same-task", report), service.create(agentId, "same-task", report)]);
    expect(a.id).toBe(b.id); expect(a.tags).toEqual(["btc", "volatility"]);
    expect((await service.get(agentId, a.id)).payload.toString()).toBe(report.content);
    expect((await service.list(agentId, { tag: "btc", instrument: "BTC" })).items).toHaveLength(1);
    await expect(service.create(agentId, "same-task", { ...report, content: "different" })).rejects.toThrow("different content");
    await expect(service.get(otherAgentId, a.id)).rejects.toThrow("unavailable");
    expect((await service.list(otherAgentId)).items).toEqual([]);
    expect(await runtime.database.get("SELECT typeof(payload) AS kind FROM hv_artifacts")).toMatchObject({ kind: "blob" });
  });
  it("supports embedded same-Agent attachments and protects them until reports expire", async () => {
    const service = runtime.agents.research.artifacts;
    const child = await service.create(agentId, "csv", { title: "Data", mediaType: "text/csv", content: "close\n10", retained: false });
    const parent = await service.create(agentId, "report", { ...report, content: `Data: artifact:${child.id}`, retained: true });
    await runtime.database.run("UPDATE hv_artifacts SET expires_at=0 WHERE id=?", child.id);
    await service.cleanup();
    expect((await service.get(agentId, child.id)).id).toBe(child.id);
    expect((await service.get(agentId, parent.id)).attachments).toMatchObject([{ id: child.id }]);
    await expect(service.remove(agentId, child.id)).rejects.toThrow("attached");
    await expect(service.create(otherAgentId, "foreign", { ...report, attachments: [child.id] })).rejects.toThrow("unavailable");
    await expect(service.create(agentId, "nested", { ...report, attachments: [parent.id] })).rejects.toThrow("Nested");
    await service.remove(agentId, parent.id); await service.cleanup();
    await expect(service.get(agentId, child.id)).rejects.toThrow("unavailable");
  });
  it("bounds text reads, validates types/images and expires unretained payloads", async () => {
    const service = runtime.agents.research.artifacts;
    const item = await service.create(agentId, "long", { ...report, content: "x".repeat(20000) });
    expect(await service.read(agentId, item.id)).toMatchObject({ content: "x".repeat(16000), nextOffset: 16000 });
    expect(await service.read(agentId, item.id, 16000)).toMatchObject({ content: "x".repeat(4000), nextOffset: null });
    await expect(service.read(agentId, item.id, -1)).rejects.toThrow("bounds");
    await expect(service.create(agentId, "bad", { title: "Bad", mediaType: "image/png", encoding: "base64", content: Buffer.from("not a png").toString("base64") })).rejects.toThrow("signature");
    await expect(service.create(agentId, "big", { ...report, content: "x".repeat(5 * 1024 * 1024 + 1) })).rejects.toThrow("5 MiB");
    await runtime.database.run("UPDATE hv_artifacts SET expires_at=0 WHERE id=?", item.id);
    expect((await service.list(agentId)).items).toEqual([]);
    await service.cleanup();
    expect(await runtime.database.get("SELECT id FROM hv_artifacts WHERE id=?", item.id)).toBeUndefined();
    // A small receipt survives cleanup; delayed recovery cannot resurrect data.
    await expect(service.create(agentId, "long", { ...report, content: "x".repeat(20000) })).rejects.toThrow("unavailable");
  });
  ;

  async function executeCall(name: string, args: Parameters<typeof fauxToolCall>[1], names: string[]) {
    const extensions = [...runtime.agents.research.extensions(agentId), createIndicatorExtension(agentId, runtime.agents.indicators, runtime.agents.execution, createResearchResponder(runtime.agents.research.artifacts, agentId, true))], registry = createRegistry();
    for (const extension of extensions) registry.install(extension);
    harness = await Harness.open(new MemoryStorage(), { models, registry }, context);
    const root = await harness.root(context, { agent: { model: { provider: faux.getModel().provider, modelId: faux.getModel().id }, tools: extensions.flatMap((extension) => extension.tools ?? []).filter((tool) => names.includes(tool.name)) } });
    faux.setResponses([fauxAssistantMessage(fauxToolCall(name, args), { stopReason: "toolUse" }), fauxAssistantMessage("Research complete")]);
    const settled = await (await root.submit({ type: "input", content: "Research" }, context)).wait(context);
    expect(settled.status).toBe("done");
    return (await root.entries({ order: "ascending" }, 100, undefined, context)).items;
  }
  function execute(code: string, names: string[] = ["codemode", "artifact_create", "artifact_list", "artifact_read"]) {
    return executeCall("codemode", { code }, names);
  }
  it.each([
    { tradingEnabled: false, allowedInstruments: null },
    { tradingEnabled: true, allowedInstruments: null },
    { tradingEnabled: false, allowedInstruments: ["BTC"] },
    { tradingEnabled: true, allowedInstruments: ["BTC"] },
  ])("lists instruments using only the bound Agent's allowlist: %j", async (settings) => {
    vi.spyOn(runtime.hyperliquid.charts, "instrumentDetails").mockResolvedValue([{ symbol: "BTC", sizeDecimals: 5 }, { symbol: "ETH", sizeDecimals: 4 }]);
    const account = vi.spyOn(runtime.hyperliquid, "agentAccountStatus");
    await runtime.agents.risk.save(agentId, { ...settings, maxOrderNotional: "1", maxTotalNotional: "1" });
    await runtime.agents.risk.save(otherAgentId, { ...settings, allowedInstruments: ["ETH"] });
    const entries = await executeCall("list_instruments", {}, ["list_instruments"]);
    const result = entries.flatMap((entry) => entry.model ?? []).find((message) => message.role === "toolResult");
    expect(result).toMatchObject({ toolName: "list_instruments", isError: false, content: [{ type: "text", text: JSON.stringify({ instruments: [
      { symbol: "BTC", allowedForTrading: true, sizeDecimals: 5, sizeStep: "0.00001", priceMaxDecimals: 1, priceMaxSignificantFigures: 5, integerPricesAllowed: true, minOpeningNotional: "10" },
      { symbol: "ETH", allowedForTrading: settings.allowedInstruments === null, sizeDecimals: 4, sizeStep: "0.0001", priceMaxDecimals: 2, priceMaxSignificantFigures: 5, integerPricesAllowed: true, minOpeningNotional: "10" },
    ] }) }] });
    expect(account).not.toHaveBeenCalled();
    expect((await runtime.agents.research.artifacts.list(agentId)).items).toEqual([]);
  });
  it("defaults to unrestricted instruments without saved risk settings and supports Code Mode", async () => {
    vi.spyOn(runtime.hyperliquid.charts, "instrumentDetails").mockResolvedValue([{ symbol: "BTC", sizeDecimals: 5 }, { symbol: "ETH", sizeDecimals: 4 }]);
    const entries = await execute(`text(await tools.list_instruments({}));`, ["codemode", "list_instruments"]);
    const result = entries.flatMap((entry) => entry.model ?? []).find((message) => message.role === "toolResult");
    expect(result).toMatchObject({ isError: false });
    expect(JSON.stringify(result)).toContain('\\"allowedForTrading\\":true');
    expect(JSON.stringify(result)).not.toContain("tradingEnabled");
    expect(toolActivity(entries)).toContainEqual(expect.objectContaining({ name: "↳ list_instruments", status: "ok" }));
  });
  it("rejects model-supplied Agent IDs before looking up the catalog", async () => {
    const catalog = vi.spyOn(runtime.hyperliquid.charts, "instrumentDetails").mockResolvedValue([{ symbol: "BTC", sizeDecimals: 5 }]);
    const entries = await executeCall("list_instruments", { agentId: otherAgentId }, ["list_instruments"]);
    const result = entries.flatMap((entry) => entry.model ?? []).find((message) => message.role === "toolResult");
    expect(result).toMatchObject({ isError: true });
    expect(catalog).not.toHaveBeenCalled();
  });
  it("returns an empty catalog without inventing instruments", async () => {
    vi.spyOn(runtime.hyperliquid.charts, "instrumentDetails").mockResolvedValue([]);
    const entries = await executeCall("list_instruments", {}, ["list_instruments"]);
    const result = entries.flatMap((entry) => entry.model ?? []).find((message) => message.role === "toolResult");
    expect(result).toMatchObject({ isError: false, content: [{ type: "text", text: '{"instruments":[]}' }] });
  });
  it("reports catalog failures instead of treating them as an empty list", async () => {
    vi.spyOn(runtime.hyperliquid.charts, "instrumentDetails").mockRejectedValue(new Error("Market catalog unavailable"));
    const entries = await executeCall("list_instruments", {}, ["list_instruments"]);
    const result = entries.flatMap((entry) => entry.model ?? []).find((message) => message.role === "toolResult");
    expect(result).toMatchObject({ isError: true });
    expect(JSON.stringify(result)).toContain("Market catalog unavailable");
  });
  it("rejects unavailable exchange precision rather than inventing tradable steps", async () => {
    vi.spyOn(runtime.hyperliquid.charts, "instrumentDetails").mockResolvedValue([{ symbol: "BTC", sizeDecimals: NaN }]);
    const entries = await executeCall("list_instruments", {}, ["list_instruments"]);
    const result = entries.flatMap((entry) => entry.model ?? []).find((message) => message.role === "toolResult");
    expect(result).toMatchObject({ isError: true });
    expect(JSON.stringify(result)).toContain("Exchange size precision unavailable");
  });
  it("executes the shared UTC time examples in real Code Mode", async () => {
    const entries = await execute(`${timeCodeExamples}
      if (!Number.isFinite(scriptTimeMs) || scriptTimeMs <= 0) throw new Error('Missing script clock');
      const anchor = fromUtcIso('2026-03-01T06:30:00.000Z');
      const rolling = windowForHours(anchor, 7);
      if (toUtcIso(rolling.start) !== '2026-02-28T23:30:00.000Z' || rolling.end !== anchor) throw new Error('Wrong rolling window');
      const day = previousUtcDay(anchor);
      if (toUtcIso(day.start) !== '2026-02-28T00:00:00.000Z' || toUtcIso(day.end) !== '2026-03-01T00:00:00.000Z') throw new Error('Wrong UTC day');
      const leapDay = previousUtcDay(fromUtcIso('2024-03-01T00:00:00.000Z'));
      if (toUtcIso(leapDay.start) !== '2024-02-29T00:00:00.000Z') throw new Error('Wrong leap day');
      if (Number.isFinite(fromUtcIso('invalid'))) throw new Error('Invalid date accepted');
      store('window', rolling);
      if (load('window').start !== rolling.start) throw new Error('Window changed');
      text('UTC examples passed');
    `, ["codemode"]);
    const result = entries.flatMap((entry) => entry.model ?? []).find((message) => message.role === "toolResult");
    expect(result).toMatchObject({ isError: false });
    expect(JSON.stringify(result)).toContain("UTC examples passed");
  });
  it("runs real QuickJS, shares root-Agent artifacts, and retains nested-call summaries without an env", async () => {
    const entries = await execute(`const report = await tools.artifact_create({title:'Worker report',mediaType:'text/markdown',content:'# BTC',retained:true}); text(report.id); text((await tools.artifact_read({id:report.id})).content);`);
    const result = entries.flatMap((entry) => entry.model ?? []).find((message) => message.role === "toolResult");
    expect(result).toMatchObject({ role: "toolResult", toolName: "codemode", isError: false });
    expect(JSON.stringify(result)).toContain("# BTC");
    expect(toolActivity(entries).filter((call) => call.name.startsWith("↳"))).toMatchObject([{ name: "↳ artifact_create", status: "ok" }, { name: "↳ artifact_read", status: "ok" }]);
    expect((await runtime.agents.research.artifacts.list(agentId)).items).toHaveLength(1);
    expect((await runtime.agents.chat("owner", agentId)).tools).toEqual([]);
  });
  it("documents global discovery, the JSON code envelope, queued fan-out and compact discovery", async () => {
    const code = runtime.agents.research.extensions(agentId).flatMap((extension) => extension.tools ?? []).find((tool) => tool.name === "codemode")!;
    expect(code.description).toContain("{code: string}");
    expect(code.description).toContain("Do not call tools.searchTools");
    expect(code.description).toContain("At most 100 nested tool calls");
    expect(code.description).toContain("queues additional calls FIFO");
    expect(code.description).toContain("limit: 3");
    expect(code.description).toContain(".map(t => t.name)");
    expect(code.description).toContain("If you know a name");
    expect(code.description).toContain("not full candle arrays");
    expect(code.description).toContain("Never manually calculate or guess Unix timestamps");
    expect(code.description).toContain("Reuse authoritative assignment endMs");
    expect(code.description).toContain("Date.parse()");
    expect(code.description).toContain("request start/end are milliseconds, candle time is seconds");
    expect(code.description).toContain("Check that a candle exists before accessing its fields");
    expect(code.description).toContain("Indicator tool results are objects, not arrays");
    expect(code.description).toContain("result.values.at(-1)?.value");
    expect(code.description).toContain("store/load preserve the JSON shape");
    expect(code.description).toContain("Array.isArray(result.values)");
    expect(code.description).toContain("artifact export rather than an inline values array");
    const entries = await execute("text('ready');");
    const system = JSON.stringify(entries.filter((entry) => entry.kind === "pi.system"));
    expect(system).toContain("Discovery helpers are GLOBAL functions");
    expect(system).toContain("queues additional calls FIFO");
    expect(system).toContain("Never print the whole searchTools result");
    expect(system).toContain("Never manually calculate or guess Unix timestamps");
    expect(system).toContain("report missing data rather than assuming a match");
    expect(system).toContain("Indicator tool results are objects, not arrays");
    expect(system).toContain("load('ind').atr.values");
  });
  it("keeps targeted discovery compact while returning the full chosen schema", async () => {
    const entries = await execute(`text((await searchTools('candles',{limit:1})).map(t=>t.name)); text(await describeTool('research_candles'));`, ["codemode", "research_candles", ...indicatorReadToolNames]);
    const result = entries.flatMap((entry) => entry.model ?? []).find((message) => message.role === "toolResult");
    expect(result).toMatchObject({ isError: false });
    expect(JSON.stringify(result)).not.toContain("truncated output");
    expect(JSON.stringify(result)).toContain("firstTime");
    expect(JSON.stringify(result)).toContain("nextStart");
    expect(JSON.stringify(result)).not.toContain("fastPeriod");
  });
  ;
  it("has no Node/network/filesystem capabilities and respects callable tool selection", async () => {
    const entries = await execute(`text([typeof process,typeof require,typeof fetch]); try { await tools.artifact_list({}); } catch(e) { text('blocked'); }`, ["codemode"]);
    const output = JSON.stringify(entries);
    expect(output).toContain("undefined"); expect(output).toContain("blocked");
    expect(output).not.toContain('"name":"bash"');
  });
  it("clamps oversized output even when scripts request a higher budget, without saving spill files", async () => {
    const start = vi.spyOn(CodemodeSandbox.prototype, "start");
    const entries = await execute(`// @options: {"max_output_tokens":100000,"timeout_ms":1000000}\ntext('x'.repeat(30000));`);
    const result = entries.flatMap((entry) => entry.model ?? []).find((message) => message.role === "toolResult");
    expect(JSON.stringify(result)).toContain("truncated output");
    expect(JSON.stringify(result)).toContain("no execution environment");
    expect(JSON.stringify(result).length).toBeLessThan(20000);
    expect(start.mock.calls[0]![1].timeoutMs).toBeLessThanOrEqual(60_000);
    expect((await runtime.agents.research.artifacts.list(agentId)).items).toEqual([]);
  });
  it("formats seconds and milliseconds as exact UTC and leaves missing/invalid timestamps unknown", async () => {
    const entries = await execute(`${utcFormattingExample} text({open:utcSeconds(1792285200),end:utcMilliseconds(1792288800000),epoch:utcSeconds(0),missing:utcSeconds(null),bad:utcSeconds(NaN),infinite:utcMilliseconds(Infinity),outOfRange:utcSeconds(Number.MAX_VALUE)});`);
    const result = entries.flatMap((entry) => entry.model ?? []).find((message) => message.role === "toolResult");
    expect(result).toMatchObject({ isError: false });
    const evidence = JSON.parse(result!.content.flatMap((part) => part.type === "text" ? [part.text] : []).find((text) => text.startsWith("{"))!);
    expect(evidence).toEqual({ open: "2026-10-18T01:00:00.000Z", end: "2026-10-18T02:00:00.000Z", epoch: "1970-01-01T00:00:00.000Z", missing: null, bad: null, infinite: null, outOfRange: null });
  });
  it("runs generic indicators and evidence guidance in Code Mode and exports full results without bulk summaries", async () => {
    const candles = Array.from({ length: 80 }, (_, i) => ({ time: i * 3600, open: i + 10, high: i + 12, low: i + 9, close: i + 11, volume: 20 }));
    vi.spyOn(runtime.hyperliquid.charts, "researchHistory").mockResolvedValue({ coin: "BTC", interval: "1h", start: 0, end: 288000000, pageEnd: 288000000, candles, nextStart: null, coverage: { complete: true, returned: 80, expected: 80, closedOnly: true, sourceLimit: "fixture" } });
    const entries = await execute(`const base={coin:'BTC',interval:'1h',start:0,end:288000000}; const candleResult=await tools.research_candles(base); const [ema20Result,ema50Result]=await Promise.all([tools.indicator_run({...base,indicatorId:'builtin:ema',inputs:{period:20}}),tools.indicator_run({...base,indicatorId:'builtin:ema',inputs:{period:50}})]); const atrResult=await tools.indicator_run({...base,indicatorId:'builtin:atr'}); ${technicalEvidenceExample} const exported=await tools.indicator_run({...base,indicatorId:'builtin:macd',saveAs:'Full MACD'}); if ('values' in exported.summary || 'outputs' in exported.summary || 'markers' in exported.summary) throw new Error('Bulk leaked'); text(exported.summary.exportScope);`, ["codemode", "research_candles", "indicator_run", "artifact_create"]);
    const response = entries.flatMap((entry) => entry.model ?? []).find((message) => message.role === "toolResult");
    expect(response, JSON.stringify(response)).toMatchObject({ isError: false });
    expect(JSON.stringify(entries)).toContain("ema20AboveEma50");
    const item = (await runtime.agents.research.artifacts.list(agentId)).items[0]!;
    const saved = JSON.parse((await runtime.agents.research.artifacts.get(agentId, item.id)).payload.toString());
    expect(saved).toMatchObject({ indicatorId: "builtin:macd", exportScope: "full", version: 1 }); expect(saved.values).toHaveLength(80);
  }, 30000);
  it("discovers only selected indicator tools and requires artifact permission", async () => {
    const entries = await execute(`const namespace=await describeNamespace('indicators'); if(namespace.tools.join(',')!=='indicator_run') throw new Error('Wrong permissions'); text(await describeTool('indicator_run')); try { await tools.indicator_preview({}); throw new Error('UNEXPECTED'); } catch(e) { if(String(e).includes('UNEXPECTED')) throw e; } try { await tools.indicator_run({indicatorId:'builtin:ema',coin:'BTC',interval:'1h',start:0,end:3600000,saveAs:'Denied'}); throw new Error('UNEXPECTED'); } catch(e) { if(String(e).includes('UNEXPECTED')) throw e; text(String(e)); }`, ["codemode", "indicator_run"]);
    expect(entries.flatMap((entry) => entry.model ?? []).find((message) => message.role === "toolResult")).toMatchObject({ isError: false });
    expect(JSON.stringify(entries)).toContain("requires artifact_create");
  });
  it("exports research snapshots server-side without passing datasets through QuickJS", async () => {
    const entries = await execute(`const r = await tools.research_journal({start:0,end:1000,saveAs:'Journal snapshot'}); text({hasTrades:'trades' in r,summary:r.summary,reference:r.reference});`, ["codemode", "research_journal", "artifact_create"]);
    const output = JSON.stringify(entries);
    expect(output).toContain('hasTrades'); expect(output).toContain('artifact:');
    const artifact = (await runtime.agents.research.artifacts.list(agentId)).items[0]!;
    expect(artifact.title).toBe("Journal snapshot");
    expect(JSON.parse((await runtime.agents.research.artifacts.get(agentId, artifact.id)).payload.toString())).toHaveProperty("trades");
  });
  it("requires independent artifact permission for research exports", async () => {
    const entries = await execute(`await tools.research_journal({start:0,end:1000,saveAs:'Denied'});`, ["codemode", "research_journal"]);
    expect(JSON.stringify(entries)).toContain("requires artifact_create permission");
    expect((await runtime.agents.research.artifacts.list(agentId)).items).toEqual([]);
  });
  it("honors a lower deadline and interrupts CPU-bound scripts off the event loop", async () => {
    const entries = await execute(`// @options: {"timeout_ms":50}\nwhile(true){}`);
    const result = entries.flatMap((entry) => entry.model ?? []).find((message) => message.role === "toolResult");
    expect(result).toMatchObject({ isError: true }); expect(JSON.stringify(result)).toContain("timed out");
  });
  it("enforces the QuickJS heap ceiling", async () => {
    const entries = await execute(`const enormous = new Array(50000000).fill(1); text(enormous.length);`);
    const result = entries.flatMap((entry) => entry.model ?? []).find((message) => message.role === "toolResult");
    expect(result).toMatchObject({ isError: true }); expect(JSON.stringify(result)).toMatch(/out of memory|memory limit/i);
  });
  it("rejects excess nested calls with visible failures", async () => {
    const entries = await execute(`for(let i=0;i<101;i++) await tools.artifact_list({});`);
    const result = entries.flatMap((entry) => entry.model ?? []).find((message) => message.role === "toolResult");
    expect(result).toMatchObject({ isError: true });
    expect(JSON.stringify(result)).toContain("limit (100)");
    expect(JSON.stringify(result)).not.toMatch(/different path|resumed after a restart/);
    expect(toolActivity(entries)).toContainEqual(expect.objectContaining({ name: "↳ artifact_list", status: "error", error: expect.stringContaining("limit (100)") }));
  });
  it("closes queued admission immediately when parallel requests exceed the total budget", async () => {
    const list = vi.spyOn(runtime.agents.research.artifacts, "list").mockImplementation(async () => {
      await new Promise((resolve) => setTimeout(resolve, 150));
      return { items: [], nextOffset: null };
    });
    const entries = await execute(`await Promise.all(Array.from({length:101},()=>tools.artifact_list({})));`);
    const result = entries.flatMap((entry) => entry.model ?? []).find((message) => message.role === "toolResult");
    expect(result).toMatchObject({ isError: true }); expect(JSON.stringify(result)).toContain("limit (100)");
    expect(JSON.stringify(result)).not.toContain("resumed after a restart");
    await new Promise((resolve) => setTimeout(resolve, 200));
    expect(list.mock.calls.length).toBeLessThanOrEqual(4);
  });
  it("queues seven parallel calls, exposes live waiters and retains queue timings", async () => {
    let active = 0, peak = 0, sawQueued = false;
    const list = vi.spyOn(runtime.agents.research.artifacts, "list").mockImplementation(async () => {
      active++; peak = Math.max(peak, active);
      try {
        const root = await harness!.root(context);
        const live = await harness!.snapshot(LiveDoc, root.id, context);
        sawQueued ||= toolActivity([], live).some((item) => item.status === "queued");
        await new Promise((resolve) => setTimeout(resolve, 30));
        return { items: [], nextOffset: null };
      } finally { active--; }
    });
    const entries = await execute(`await Promise.all(Array.from({length:7},()=>tools.artifact_list({})));`);
    const result = entries.flatMap((entry) => entry.model ?? []).find((message) => message.role === "toolResult");
    expect(result).toMatchObject({ isError: false });
    expect(JSON.stringify(result)).not.toMatch(/different path|resumed after a restart/);
    const nested = toolActivity(entries).filter((item) => item.name.startsWith("↳"));
    expect(nested).toHaveLength(7); expect(nested.every((item) => item.status === "ok")).toBe(true);
    expect(nested.every((item) => item.queuedMs !== undefined)).toBe(true);
    expect(list).toHaveBeenCalledTimes(7); expect(peak).toBeLessThanOrEqual(4); expect(sawQueued).toBe(true);
  });
  it("never dispatches queued work after the script deadline", async () => {
    const list = vi.spyOn(runtime.agents.research.artifacts, "list").mockImplementation(async () => {
      await new Promise((resolve) => setTimeout(resolve, 200));
      return { items: [], nextOffset: null };
    });
    const entries = await execute(`// @options: {"timeout_ms":80}\nawait Promise.all(Array.from({length:7},()=>tools.artifact_list({})));`);
    const result = entries.flatMap((entry) => entry.model ?? []).find((message) => message.role === "toolResult");
    expect(result).toMatchObject({ isError: true }); expect(JSON.stringify(result)).toContain("timed out");
    await new Promise((resolve) => setTimeout(resolve, 250));
    expect(list.mock.calls.length).toBeLessThanOrEqual(4);
  });
  it("preserves ordinary tool errors and their nested failure details", async () => {
    vi.spyOn(runtime.agents.research.artifacts, "list").mockRejectedValue(new Error("Fixture data unavailable"));
    const entries = await execute(`await tools.artifact_list({});`);
    const result = entries.flatMap((entry) => entry.model ?? []).find((message) => message.role === "toolResult");
    expect(result).toMatchObject({ isError: true });
    expect(JSON.stringify(result)).toContain("Fixture data unavailable");
    expect(JSON.stringify(result)).not.toContain("research_concurrency_limit");
    expect(toolActivity(entries)).toContainEqual(expect.objectContaining({ name: "↳ artifact_list", status: "error", error: expect.stringContaining("Fixture data unavailable") }));
  });
  it("allows seven calls in awaited batches of four then three", async () => {
    const list = vi.spyOn(runtime.agents.research.artifacts, "list").mockImplementation(async () => {
      await new Promise((resolve) => setTimeout(resolve, 10));
      return { items: [], nextOffset: null };
    });
    const entries = await execute(`for(let start=0;start<7;start+=4) await Promise.all(Array.from({length:Math.min(4,7-start)},()=>tools.artifact_list({}))); text('seven completed');`);
    const result = entries.flatMap((entry) => entry.model ?? []).find((message) => message.role === "toolResult");
    expect(result).toMatchObject({ isError: false }); expect(JSON.stringify(result)).toContain("seven completed");
    expect(list).toHaveBeenCalledTimes(7);
    expect(toolActivity(entries).filter((item) => item.name.startsWith("↳"))).toHaveLength(7);
  });
  it("journal summaries preserve decimal arithmetic and incomplete coverage, and isolate accounts", async () => {
    const account = `0x${"1".repeat(40)}`;
    // Account creation immediately starts a journal worker; let its initial
    // sync and projection settle before installing fixture rows.
    await vi.waitFor(async () => {
      const state = await runtime.database.get<{ revision: number; projected_revision: number }>("SELECT revision,projected_revision FROM hv_journal_accounts WHERE account=?", account);
      expect(state).toMatchObject({ revision: 0, projected_revision: 0 });
      const sync = await runtime.database.all<{ updated_at: number | null; error: string | null }>("SELECT updated_at,error FROM hv_journal_sync WHERE account=?", account);
      expect(sync).toHaveLength(3);
      expect(sync.every((row) => row.updated_at !== null && row.error === null)).toBe(true);
    });
    await runtime.database.run("INSERT INTO hv_journal_accounts(account,projected_revision) VALUES (?,0) ON CONFLICT DO NOTHING", account);
    for (const [id, time, net] of [["a", 100, "0.1"], ["b", 101, "0.2"]] as const) {
      const trade = { id, status: "closed", symbol: "BTC", net, grossPnl: net, fees: "0", parts: ["oversized internals"] };
      await runtime.database.run("INSERT INTO hv_journal_entries(account,id,kind,time,data) VALUES (?,?,'trade',?,?)", account, id, time, JSON.stringify(trade));
    }
    for (const stream of ["fill", "funding", "ledger"]) await runtime.database.run("INSERT INTO hv_journal_sync(account,stream,cursor,updated_at) VALUES (?,?,1000,1) ON CONFLICT DO NOTHING", account, stream);
    const result = await researchJournal(runtime.database, agentId, 0, 1000, 100);
    expect(result.performance.knownNetSum).toBe("0.3"); expect(result.coverage.complete).toBe(true);
    expect(JSON.stringify(result)).not.toContain("oversized internals");
    const page = await researchJournal(runtime.database, agentId, 0, 1000, 1);
    expect(page.next).toEqual({ time: 101, id: "b" });
    expect((await researchJournal(runtime.database, agentId, 0, 1000, 1, page.next!)).trades[0]!.id).toBe("a");
    expect((await researchJournal(runtime.database, otherAgentId, 0, 1000, 100)).trades).toEqual([]);
    await runtime.database.run("UPDATE hv_journal_sync SET error='offline' WHERE stream='funding'");
    expect((await researchJournal(runtime.database, agentId, 0, 1000, 100)).performance.knownNetCount).toBe(0);
  });
});
