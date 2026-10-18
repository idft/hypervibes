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
import { createRegistry, Harness, LiveDoc, MemoryStorage } from "@earendil-works/pi-durable";
import { BACKGROUND_CONTEXT as context } from "@earendil-works/chord/context";
import { toolActivity } from "./tool-activity.js";
import { technicalAnalysis, type IndicatorRequest } from "./technical-analysis.js";
import { bollingerEvidenceExample, technicalEvidenceExample, utcFormattingExample } from "./evidence.js";
import { researchJournal } from "./journal.js";
import { taToolNames } from "../agents/subagent-input.js";

// All databases are disposable; no exchange HTTP, listening ports or model calls.
describe("Code Mode research and artifacts", () => {
  let runtime: AppRuntime, directory: string, harness: Harness | undefined;
  let faux: ReturnType<typeof fauxProvider>;
  let models: ReturnType<typeof createModels>;
  let agentId: string, otherAgentId: string;
  beforeEach(async () => {
    directory = mkdtempSync(join(tmpdir(), "hv-research-"));
    faux = fauxProvider(); models = createModels(); models.setProvider(faux.provider);
    runtime = await createRuntime(testConfig({ APP_SQLITE_PATH: join(directory, "app.sqlite") }), { models, model: { provider: faux.getModel().provider, modelId: faux.getModel().id }, journalHistoryFetcher: async () => [], accountFeedFactory: () => ({ close: async () => {} }) });
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
  it("calculates fixture indicators with explicit warm-up and bounded output", () => {
    const candles = Array.from({ length: 80 }, (_, i) => ({ time: i * 3600, open: i + 10, high: i + 12, low: i + 9, close: i + 11, volume: 20 }));
    const requests: IndicatorRequest[] = [{ type: "sma", period: 3 }, { type: "ema", period: 3 }, { type: "rsi", period: 14 }, { type: "macd", fastPeriod: 12, slowPeriod: 26, signalPeriod: 9 }, { type: "atr", period: 14 }, { type: "adx", period: 14 }, { type: "bollinger", period: 20, deviations: 2 }];
    const result = requests.map((request) => technicalAnalysis(candles, request, 2));
    expect(result[0]!.values.at(-1)!.value).toBe(89);
    expect(result[2]!.values.at(-1)!.value).toBe(100);
    expect(result[4]!.values.at(-1)!.value).toBeCloseTo(3);
    expect(result.every((item) => item.stable && item.values.length === 2)).toBe(true);
    expect(technicalAnalysis(candles.slice(0, 1), { type: "sma", period: 3 })).toMatchObject({ stable: false, values: [{ value: null }] });
    expect(() => technicalAnalysis(candles, { type: "rsi", period: 0 })).toThrow("period");
  });

  async function execute(code: string, names: string[] = ["codemode", "artifact_create", "artifact_list", "artifact_read"]) {
    const extensions = runtime.agents.research.extensions(agentId), registry = createRegistry();
    for (const extension of extensions) registry.install(extension);
    harness = await Harness.open(new MemoryStorage(), { models, registry }, context);
    const root = await harness.root(context, { agent: { model: { provider: faux.getModel().provider, modelId: faux.getModel().id }, tools: extensions.flatMap((extension) => extension.tools ?? []).filter((tool) => names.includes(tool.name)) } });
    faux.setResponses([fauxAssistantMessage(fauxToolCall("codemode", { code }), { stopReason: "toolUse" }), fauxAssistantMessage("Research complete")]);
    const settled = await (await root.submit({ type: "input", content: "Research" }, context)).wait(context);
    expect(settled.status).toBe("done");
    return (await root.entries({ order: "ascending" }, 100, undefined, context)).items;
  }
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
    const entries = await execute("text('ready');");
    const system = JSON.stringify(entries.filter((entry) => entry.kind === "pi.system"));
    expect(system).toContain("Discovery helpers are GLOBAL functions");
    expect(system).toContain("queues additional calls FIFO");
    expect(system).toContain("Never print the whole searchTools result");
  });
  it("keeps targeted discovery compact while returning the full chosen schema", async () => {
    const entries = await execute(`text((await searchTools('candles',{limit:1})).map(t=>t.name)); text(await describeTool('research_candles'));`, ["codemode", "research_candles", ...taToolNames]);
    const result = entries.flatMap((entry) => entry.model ?? []).find((message) => message.role === "toolResult");
    expect(result).toMatchObject({ isError: false });
    expect(JSON.stringify(result)).not.toContain("truncated output");
    expect(JSON.stringify(result)).toContain("firstTime");
    expect(JSON.stringify(result)).toContain("nextStart");
    expect(JSON.stringify(result)).not.toContain("fastPeriod");
  });
  it("gives explicit seed-history targets, closed-boundary arithmetic and retention caveats", async () => {
    const entries = await execute("text('ready');", ["codemode", "ta_ema"]);
    const system = JSON.stringify(entries.filter((entry) => entry.kind === "pi.system"));
    expect(system).toContain("at least 5 times the longest configured indicator period");
    expect(system).toContain("count=1 therefore needs at least 250 closed candles");
    expect(system).toContain("count=5 targets 254");
    expect(system).toContain("start=end-250*hour");
    expect(system).toContain("Changing a page limit does not widen the requested time range");
    expect(system).toContain("5000-input cap");
    expect(system).toContain("disclose actual input count, target and initialization sensitivity");
    expect(system).toContain("never claim full seed coverage just because stable=true");
    expect(system).toContain("Unless the user explicitly constrains input history");
  });
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
  it("provides validated structured candles and TA results to scripts", async () => {
    const candles = Array.from({ length: 50 }, (_, i) => ({ time: i * 3600, open: i + 10, high: i + 12, low: i + 9, close: i + 11, volume: 20 }));
    vi.spyOn(runtime.hyperliquid.charts, "researchHistory").mockResolvedValue({ coin: "BTC", interval: "1h", start: 0, end: 50 * 3600000, pageEnd: 50 * 3600000, candles, nextStart: null, coverage: { complete: true, returned: 50, expected: 50, closedOnly: true, sourceLimit: "fixture" } });
    const entries = await execute(`const candles = await tools.research_candles({coin:'BTC',interval:'1h',start:0,end:180000000}); const ta = await tools.ta_sma({coin:'BTC',interval:'1h',start:0,end:180000000,period:3}); text({count:candles.candles.length,sma:ta.values[0].value});`, ["codemode", "research_candles", "ta_sma"]);
    const result = entries.flatMap((entry) => entry.model ?? []).find((message) => message.role === "toolResult");
    expect(result).toMatchObject({ isError: false }); expect(JSON.stringify(result)).toContain("59");
    expect(JSON.stringify(result)).not.toContain("invalid_structured_output");
  });
  it.each(["valid", "unstable", "misaligned", "volume", "missing", "zeroClose"] as const)("runs the evidence example against tool datasets (%s)", async (mode) => {
    const candles = Array.from({ length: 179 }, (_, i) => { const close = i === 178 ? 95 : 100 + i * 0.1; return { time: i * 3600, open: close, high: close + 2, low: close - 1, close, volume: 20 }; });
    vi.spyOn(runtime.hyperliquid.charts, "researchHistory").mockImplementation(async (_coin, _interval, start, end) => {
      const rows = candles.filter((candle) => candle.time * 1000 >= start && candle.time * 1000 < end);
      return { coin: "BTC", interval: "1h", start, end, pageEnd: end, candles: rows, nextStart: null, coverage: { complete: true, returned: rows.length, expected: rows.length, closedOnly: true, sourceLimit: "fixture" } };
    });
    const mutations = { valid: "", unstable: "ema20Result.stable=false;", misaligned: "ema20Result.values[0].time-=3600;", volume: "ema20Result.parameters.source='volume';", missing: "ema20Result.values=[];", zeroClose: "candleResult.candles.at(-1).close=0;" };
    const entries = await execute(`const base={coin:'BTC',interval:'1h',start:0,end:644400000,count:1}; const [candleResult,ema20Result,ema50Result,atrResult]=await Promise.all([tools.research_candles({coin:base.coin,interval:base.interval,start:216000000,end:base.end,limit:200}),tools.ta_ema({...base,period:20}),tools.ta_ema({...base,period:50}),tools.ta_atr({...base,period:14})]); ${mutations[mode]} ${technicalEvidenceExample}`, ["codemode", "research_candles", "ta_ema", "ta_atr"]);
    const result = entries.flatMap((entry) => entry.model ?? []).find((message) => message.role === "toolResult");
    expect(result).toMatchObject({ isError: false });
    const evidence = JSON.parse(result!.content.flatMap((part) => part.type === "text" ? [part.text] : []).find((text) => text.startsWith("{"))!);
    expect(evidence.datasets.candles.input).toMatchObject({ count: 119, firstTime: 216000, lastTime: 640800 });
    expect(evidence.datasets.ema20.input).toMatchObject({ count: 179, firstTime: 0, lastTime: 640800 });
    expect(evidence.datasets.ema20.output).toMatchObject({ count: 1, firstTime: 640800, lastTime: 640800 });
    expect(evidence.datasets.ema20.parameters).toMatchObject({ period: 20 });
    if (mode === "valid") {
      expect(evidence.facts).toMatchObject({ priceBelowEma20: true, ema20AboveEma50: true });
      expect(evidence.facts.atrPercent).toBeCloseTo((62.7 / 14) / 95 * 100);
    } else if (mode === "zeroClose") expect(evidence.facts.atrPercent).toBeNull();
    else expect(evidence.facts).toMatchObject({ priceBelowEma20: null, ema20AboveEma50: null });
    expect(JSON.stringify(entries.filter((entry) => entry.kind === "pi.system"))).toContain("Check the final prose against the flags");
  });
  it("formats seconds and milliseconds as exact UTC and leaves missing/invalid timestamps unknown", async () => {
    const entries = await execute(`${utcFormattingExample} text({open:utcSeconds(1792285200),end:utcMilliseconds(1792288800000),epoch:utcSeconds(0),missing:utcSeconds(null),bad:utcSeconds(NaN),infinite:utcMilliseconds(Infinity),outOfRange:utcSeconds(Number.MAX_VALUE)});`);
    const result = entries.flatMap((entry) => entry.model ?? []).find((message) => message.role === "toolResult");
    expect(result).toMatchObject({ isError: false });
    const evidence = JSON.parse(result!.content.flatMap((part) => part.type === "text" ? [part.text] : []).find((text) => text.startsWith("{"))!);
    expect(evidence).toEqual({ open: "2026-10-18T01:00:00.000Z", end: "2026-10-18T02:00:00.000Z", epoch: "1970-01-01T00:00:00.000Z", missing: null, bad: null, infinite: null, outOfRange: null });
  });
  it.each(["valid", "missing", "misaligned", "unstable"] as const)("emits UTC provenance and reads Bollinger fields inside point.value (%s)", async (mode) => {
    const start = 1791208800000, end = 1792288800000;
    const candles = Array.from({ length: 300 }, (_, i) => { const close = i === 299 ? 160 : 100 + i * 0.1; return { time: start / 1000 + i * 3600, open: close, high: close + 2, low: close - 1, close, volume: 20 }; });
    vi.spyOn(runtime.hyperliquid.charts, "researchHistory").mockResolvedValue({ coin: "BTC", interval: "1h", start, end, pageEnd: end, candles, nextStart: null, coverage: { complete: true, returned: 300, expected: 300, closedOnly: true, sourceLimit: "fixture" } });
    const mutations = { valid: "", missing: "delete bollingerResult.values[0].value.lower;", misaligned: "bollingerResult.values[0].time-=3600;", unstable: "bollingerResult.stable=false;" };
    const entries = await execute(`const p={coin:'BTC',interval:'1h',start:${start},end:${end},count:1}; const [candleResult,ema20Result,ema50Result,atrResult,bollingerResult]=await Promise.all([tools.research_candles({coin:p.coin,interval:p.interval,start:p.start,end:p.end,limit:500}),tools.ta_ema({...p,period:20}),tools.ta_ema({...p,period:50}),tools.ta_atr({...p,period:14}),tools.ta_bollinger({...p,period:20,deviations:2})]); ${mutations[mode]} const evidence=[]; {const text=value=>evidence.push(value); ${technicalEvidenceExample} ${bollingerEvidenceExample}} text({evidence});`, ["codemode", "research_candles", "ta_ema", "ta_atr", "ta_bollinger"]);
    const result = entries.flatMap((entry) => entry.model ?? []).find((message) => message.role === "toolResult");
    expect(result).toMatchObject({ isError: false });
    const { evidence } = JSON.parse(result!.content.flatMap((part) => part.type === "text" ? [part.text] : []).find((text) => text.startsWith("{"))!);
    expect(evidence[0].latestClosed.openTimeUtc).toBe("2026-10-18T01:00:00.000Z");
    expect(evidence[0].datasets.candles).toMatchObject({ requestedStartUtc: "2026-10-05T14:00:00.000Z", effectiveEndExclusiveUtc: "2026-10-18T02:00:00.000Z", input: { count: 300, firstTimeUtc: "2026-10-05T14:00:00.000Z", lastTimeUtc: "2026-10-18T01:00:00.000Z" } });
    expect(evidence[0].datasets.ema20.output.lastTimeUtc).toBe("2026-10-18T01:00:00.000Z");
    if (mode === "valid") {
      expect(evidence[1].bollinger.value).toMatchObject({ lower: expect.any(Number), middle: expect.any(Number), upper: expect.any(Number) });
      expect(evidence[1].facts).toEqual({ priceAboveUpperBand: true, priceBelowLowerBand: false });
    } else {
      expect(evidence[1].bollinger.value).toBeNull();
      expect(evidence[1].facts).toEqual({ priceAboveUpperBand: null, priceBelowLowerBand: null });
    }
    expect(toolActivity(entries).filter((item) => item.name === "↳ ta_bollinger")).toHaveLength(1);
    const prompt = JSON.stringify(entries.filter((entry) => entry.kind === "pi.system"));
    expect(prompt).toContain("Quote the emitted ISO UTC strings verbatim");
    expect(prompt).toContain("point.value.lower/middle/upper");
  });
  it("registers a separate ta extension and runs every indicator with non-default parameters", async () => {
    const extensions = runtime.agents.research.extensions(agentId);
    expect(extensions.find((extension) => extension.name === "ta")!.tools!.map((tool) => tool.name)).toEqual([...taToolNames]);
    expect(extensions.find((extension) => extension.name === "research")!.tools!.some((tool) => tool.name.startsWith("ta_") || tool.name === "research_ta")).toBe(false);
    const candles = Array.from({ length: 160 }, (_, i) => { const close = 100 + i * 0.3 + Math.sin(i * 1.7) * 4; return { time: i * 3600, open: close + 0.5, high: close + 2 + i % 3, low: close - 2 - i % 2, close, volume: 100 + i % 7 }; });
    vi.spyOn(runtime.hyperliquid.charts, "researchHistory").mockResolvedValue({ coin: "BTC", interval: "1h", start: 0, end: 160 * 3600000, pageEnd: 160 * 3600000, candles, nextStart: null, coverage: { complete: true, returned: 160, expected: 160, closedOnly: true, sourceLimit: "fixture" } });
    const entries = await execute(`const requests = [
      ['ta_sma',{period:7,source:'hl2'}],['ta_ema',{period:11,source:'open'}],
      ['ta_rsi',{period:9,source:'hlc3',smoothing:'ema'}],
      ['ta_macd',{fastPeriod:8,slowPeriod:21,signalPeriod:5,average:'dema',signalAverage:'ema',source:'ohlc4'}],
      ['ta_bollinger',{period:10,deviations:3.25,source:'high'}],['ta_atr',{period:7,smoothing:'sma'}],['ta_adx',{period:8,smoothing:'wma'}]
    ]; for (const [name,params] of requests) { const result = await tools[name]({coin:'BTC',interval:'1h',start:0,end:576000000,count:2,...params}); if (!result.stable || result.values.length!==2) throw new Error('Unexpected output'); text({name,parameters:result.parameters}); }`, ["codemode", ...taToolNames]);
    const result = entries.flatMap((entry) => entry.model ?? []).find((message) => message.role === "toolResult");
    expect(result).toMatchObject({ isError: false }); expect(JSON.stringify(result)).toContain("3.25");
    expect(toolActivity(entries).filter((item) => item.name.startsWith("↳"))).toEqual(taToolNames.map((name) => expect.objectContaining({ name: `↳ ${name}`, status: "ok" })));
  });
  it("discovers only selected indicators in the ta Code Mode namespace", async () => {
    const entries = await execute(`const found=await searchTools('macd',{namespace:'ta'}); if(found.length!==1 || found[0].name!=='ta_macd') throw new Error('Wrong search permissions'); const ns=await describeNamespace('ta'); if(ns.name!=='ta' || ns.tools.join(',')!=='ta_macd') throw new Error('Wrong namespace permissions'); text(ns); text(await describeTool('ta_macd'));`, ["codemode", "ta_macd"]);
    const result = entries.flatMap((entry) => entry.model ?? []).find((message) => message.role === "toolResult");
    expect(result).toMatchObject({ isError: false }); expect(JSON.stringify(result)).toContain("fastPeriod");
    expect(JSON.stringify(result)).not.toContain("ta_bollinger");
  });
  it("rejects unsupported or missing TA parameters before fetching data", async () => {
    const history = vi.spyOn(runtime.hyperliquid.charts, "researchHistory");
    const entries = await execute(`const invalid = [
      ['ta_sma',{}],['ta_sma',{period:7,deviations:2}],['ta_macd',{fastPeriod:21,slowPeriod:8,signalPeriod:5}],
      ['ta_macd',{fastPeriod:8,slowPeriod:21}],['ta_bollinger',{period:20}],['ta_bollinger',{period:20,deviations:0}],
      ['ta_rsi',{period:14,smoothing:'unsupported'}],['ta_atr',{period:14,source:'close'}]
    ]; for (const [name,params] of invalid) { try { await tools[name]({coin:'BTC',interval:'1h',start:0,end:180000000,...params}); throw new Error('UNEXPECTED success'); } catch (e) { if(String(e).includes('UNEXPECTED')) throw e; text('blocked:'+name); } }`, ["codemode", ...taToolNames]);
    const result = entries.flatMap((entry) => entry.model ?? []).find((message) => message.role === "toolResult");
    expect(result).toMatchObject({ isError: false }); expect(history).not.toHaveBeenCalled();
    expect(toolActivity(entries).filter((item) => item.name.startsWith("↳") && item.status === "error")).toHaveLength(8);
  });
  it.each(["gap", "oversized"] as const)("refuses %s TA ranges rather than calculating partial data", async (kind) => {
    vi.spyOn(runtime.hyperliquid.charts, "researchHistory").mockResolvedValue({ coin: "BTC", interval: "1h", start: 0, end: 180000000, pageEnd: 180000000, candles: [], nextStart: kind === "oversized" ? 180000000 : null, coverage: { complete: kind !== "gap", returned: 0, expected: 50, closedOnly: true, sourceLimit: "fixture" } });
    const entries = await execute(`await tools.ta_rsi({coin:'BTC',interval:'1h',start:0,end:180000000,period:14});`, ["codemode", "ta_rsi"]);
    const result = entries.flatMap((entry) => entry.model ?? []).find((message) => message.role === "toolResult");
    expect(result).toMatchObject({ isError: true }); expect(JSON.stringify(result)).toContain("complete contiguous range");
  });
  it("exports a single indicator's values with its exact parameters and no values in the script result", async () => {
    const candles = Array.from({ length: 80 }, (_, i) => ({ time: i * 3600, open: i + 10, high: i + 12, low: i + 9, close: i + 11, volume: 20 }));
    vi.spyOn(runtime.hyperliquid.charts, "researchHistory").mockResolvedValue({ coin: "BTC", interval: "1h", start: 0, end: 80 * 3600000, pageEnd: 80 * 3600000, candles, nextStart: null, coverage: { complete: true, returned: 80, expected: 80, closedOnly: true, sourceLimit: "fixture" } });
    const entries = await execute(`const result = await tools.ta_macd({coin:'BTC',interval:'1h',start:0,end:288000000,fastPeriod:8,slowPeriod:21,signalPeriod:5,count:20,saveAs:'MACD 8-21-5'}); if('values' in result || 'values' in result.summary) throw new Error('Dataset leaked'); text(result.summary.parameters);`, ["codemode", "ta_macd", "artifact_create"]);
    expect(entries.flatMap((entry) => entry.model ?? []).find((message) => message.role === "toolResult")).toMatchObject({ isError: false });
    const artifact = (await runtime.agents.research.artifacts.list(agentId)).items[0]!;
    const snapshot = JSON.parse((await runtime.agents.research.artifacts.get(agentId, artifact.id)).payload.toString());
    expect(snapshot).toMatchObject({ indicator: "macd", parameters: { fastPeriod: 8, slowPeriod: 21, signalPeriod: 5 } });
    expect(snapshot.values).toHaveLength(20); expect(snapshot.indicators).toBeUndefined();
    expect(snapshot.dataset).toEqual({ count: 80, firstTime: 0, lastTime: 79 * 3600 });
    expect(snapshot.outputDataset).toEqual({ count: 20, firstTime: 60 * 3600, lastTime: 79 * 3600 });
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
    await runtime.database.run("INSERT INTO hv_journal_accounts(account,projected_revision) VALUES (?,0)", account);
    for (const [id, time, net] of [["a", 100, "0.1"], ["b", 101, "0.2"]] as const) {
      const trade = { id, status: "closed", symbol: "BTC", net, grossPnl: net, fees: "0", parts: ["oversized internals"] };
      await runtime.database.run("INSERT INTO hv_journal_entries(account,id,kind,time,data) VALUES (?,?,'trade',?,?)", account, id, time, JSON.stringify(trade));
    }
    for (const stream of ["fill", "funding", "ledger"]) await runtime.database.run("INSERT INTO hv_journal_sync(account,stream,cursor,updated_at) VALUES (?,?,1000,1)", account, stream);
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
