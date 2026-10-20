import { BACKGROUND_CONTEXT as context } from "@earendil-works/chord/context";
import { createModels } from "@earendil-works/pi-ai/models";
import { fauxAssistantMessage, fauxProvider, fauxToolCall } from "@earendil-works/pi-ai/providers/faux";
import { Harness, type ConversationId } from "@earendil-works/pi-durable";
import { SqliteStorage } from "@earendil-works/pi-durable/storage/sqlite";
import { eq } from "drizzle-orm";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { randomUUID } from "node:crypto";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { hvUsers } from "../db/auth-schema.js";
import { hvSubagents, hvSubagentRuns } from "../db/subagent-schema.js";
import { createRuntime, type AppRuntime } from "../runtime.js";
import { testConfig } from "../test-config.js";
import { nextCandleRun, subagentInput, type SubagentInput } from "./subagent-input.js";
import { subagentTemplates, templateDraft } from "./subagent-templates.js";
import { MarketSnapshotUpgradeDoc } from "./subagent-service.js";

const settings = (patch: Partial<SubagentInput> = {}): SubagentInput => ({ name: "Research", description: "Focused evidence", instructions: "Research the assignment using only available evidence.", provider: "faux", modelId: "faux-1", thinkingLevel: "", tools: [], enabled: true, delegatable: true, persistent: false, schedule: "", delaySeconds: 30, completion: "save", timeoutSeconds: 900, ...patch });
describe("subagent definitions, execution and scheduling", () => {
  let directory: string, runtime: AppRuntime, faux: ReturnType<typeof fauxProvider>;
  let models: ReturnType<typeof createModels>;
  const config = () => testConfig({ APP_SQLITE_PATH: join(directory, "app.sqlite") });
  const options = () => ({ models, model: { provider: "faux", modelId: "faux-1" }, accountFeedFactory: () => ({ close: async () => {} }), journalFeedFactory: () => ({ close: async () => {} }), journalHistoryFetcher: async () => [] });
  const createAgent = () => runtime.agents.create("owner", "Trading desk", randomUUID(), `0x${randomUUID().replaceAll("-", "")}12345678`);
  const createWorker = (agentId: string, patch: Partial<SubagentInput> = {}) => runtime.agents.subagents.create("owner", agentId, settings(patch), randomUUID());
  const detail = (agentId: string, id: string) => runtime.agents.subagents.detail("owner", agentId, id);
  const completed = async (agentId: string, id: string) => {
    await vi.waitFor(async () => expect((await detail(agentId, id)).runs[0]?.status).toBe("completed"));
    return (await detail(agentId, id)).runs[0]!;
  };
  beforeEach(async () => {
    directory = mkdtempSync(join(tmpdir(), "hv-subagents-")); faux = fauxProvider(); models = createModels(); models.setProvider(faux.provider);
    runtime = await createRuntime(config(), options());
    await runtime.db.insert(hvUsers).values([{ id: "owner", name: "Owner", email: "owner@example.test" }, { id: "other", name: "Other", email: "other@example.test" }]).run();
    runtime.start(); await runtime.agents.subagents.tick();
  });
  afterEach(async () => { vi.restoreAllMocks(); await runtime.close(); rmSync(directory, { recursive: true, force: true }); });

  it("upgrades existing orchestrators once without replacing saved selections or instructions", async () => {
    const opened = vi.spyOn(Harness, "open");
    const agent = await createAgent();
    const harness = await opened.mock.results[0]!.value as Harness;
    const root = await harness.root(context);
    const main = (await runtime.agents.subagents.list("owner", agent.id)).find((row) => row.kind === "orchestrator")!;
    expect(main.tools).toContain("get_market_snapshot");
    // Simulate a pre-upgrade root, with customized tools/instructions.
    await runtime.db.update(hvSubagents).set({ tools: ["list_memories"], instructions: "Preserve my custom instructions." }).where(eq(hvSubagents.id, main.id)).run();
    await root.commit(async (tx) => { (await tx.doc(MarketSnapshotUpgradeDoc, root.id)).applied = false; }, context);
    await runtime.close();
    runtime = await createRuntime(config(), options());
    const upgraded = (await runtime.agents.subagents.list("owner", agent.id)).find((row) => row.id === main.id)!;
    expect(upgraded.tools).toEqual(["list_memories", "get_market_snapshot"]);
    expect(upgraded.instructions).toBe("Preserve my custom instructions.");
    expect(upgraded.modelId).toBe(main.modelId);
    const restored = await (await opened.mock.results[1]!.value as Harness).root(context);
    expect((await restored.agent(context)).tools.map((tool) => tool.name)).toEqual(expect.arrayContaining(upgraded.tools));
    expect(faux.state.callCount).toBe(0);
    // A later deliberate deselection must survive another startup.
    await runtime.db.update(hvSubagents).set({ tools: ["list_memories"] }).where(eq(hvSubagents.id, main.id)).run();
    await runtime.close();
    runtime = await createRuntime(config(), options());
    expect((await runtime.agents.subagents.list("owner", agent.id)).find((row) => row.id === main.id)!.tools).toEqual(["list_memories"]);
  });

  it("lists enabled, paused and non-delegatable workers without exposing archived workers or other agents", async () => {
    const agent = await createAgent();
    const enabled = await createWorker(agent.id);
    const paused = await createWorker(agent.id, { enabled: false });
    const unavailable = await createWorker(agent.id, { delegatable: false });
    const archived = await createWorker(agent.id);
    await runtime.agents.subagents.archive("owner", agent.id, archived.id);
    const other = await createAgent();
    await createWorker(other.id);
    expect(await runtime.agents.subagents.listWorkers(agent.id)).toEqual(
      [enabled, paused, unavailable].sort((a, b) => a.createdAt - b.createdAt || a.id.localeCompare(b.id)).map(({ id, name, description, tools, enabled, delegatable }) => ({ id, name, description, tools, enabled, delegatable })),
    );
    for (const worker of [paused, unavailable, archived]) {
      await expect(runtime.agents.subagents.delegate(agent.id, worker.id, "Not available", randomUUID())).rejects.toMatchObject({ status: 409 });
    }
    expect(faux.state.callCount).toBe(0);
  });

  it("summarizes only active workers and their latest bounded result for the dashboard", async () => {
    const agent = await createAgent();
    expect(await runtime.agents.subagents.dashboard("owner", agent.id)).toEqual([]);
    const worker = await createWorker(agent.id, { enabled: false });
    const unused = await createWorker(agent.id, { name: "Unused" });
    const archived = await createWorker(agent.id, { name: "Archived" });
    await runtime.agents.subagents.archive("owner", agent.id, archived.id);
    await runtime.db.insert(hvSubagentRuns).values(Array.from({ length: 7 }, (_, index) => ({
      id: `dashboard-run-${index}`, agentId: agent.id, subagentId: worker.id, requestId: randomUUID(),
      source: "manual" as const, task: "Private task", config: worker, status: "completed" as const,
      result: `${index}${"x".repeat(500)}`, createdAt: index,
    }))).run();
    const summaries = await runtime.agents.subagents.dashboard("owner", agent.id);
    expect(summaries.map((row) => row.id)).toEqual([worker.id, unused.id]);
    expect(summaries[0]?.enabled).toBe(false);
    expect(summaries[0]?.runs.map((run) => run.id)).toEqual(["dashboard-run-6"]);
    expect(summaries[0]?.runs.every((run) => run.result.length === 240 && !("config" in run) && !("task" in run))).toBe(true);
    expect(summaries[1]?.runs).toEqual([]);
    await expect(runtime.agents.subagents.dashboard("other", agent.id)).rejects.toMatchObject({ status: 404 });
  });

  it("unarchives workers paused, preserves history and settings, and safely retries", async () => {
    const agent = await createAgent();
    const worker = await createWorker(agent.id, { schedule: "15m", persistent: true });
    faux.setResponses([fauxAssistantMessage("Preserved result")]);
    await runtime.agents.subagents.run("owner", agent.id, worker.id, "Research once", randomUUID());
    await completed(agent.id, worker.id);
    const before = await detail(agent.id, worker.id);
    await runtime.agents.subagents.archive("owner", agent.id, worker.id);
    const restored = await runtime.agents.subagents.unarchive("owner", agent.id, worker.id);
    expect(restored).toEqual({ ...before.definition, archived: false, enabled: false, nextRunAt: null, updatedAt: expect.any(Number) });
    expect((await detail(agent.id, worker.id)).runs).toEqual(before.runs);
    expect(await runtime.agents.subagents.listWorkers(agent.id)).toEqual([expect.objectContaining({ id: worker.id, enabled: false, delegatable: true })]);
    await runtime.agents.subagents.tick();
    expect((await detail(agent.id, worker.id)).runs).toEqual(before.runs);
    expect(await runtime.agents.subagents.unarchive("owner", agent.id, worker.id)).toEqual(restored);
    const enabled = await runtime.agents.subagents.updateSection("owner", agent.id, worker.id, "enabled", { enabled: true });
    expect(enabled.nextRunAt).not.toBeNull();
    expect(await runtime.agents.subagents.unarchive("owner", agent.id, worker.id)).toEqual(enabled);
  });
  it("restricts unarchiving to owned workers", async () => {
    const agent = await createAgent();
    const worker = await createWorker(agent.id);
    await runtime.agents.subagents.archive("owner", agent.id, worker.id);
    await expect(runtime.agents.subagents.unarchive("other", agent.id, worker.id)).rejects.toMatchObject({ status: 404 });
    const otherAgent = await createAgent();
    await expect(runtime.agents.subagents.unarchive("owner", otherAgent.id, worker.id)).rejects.toMatchObject({ status: 404 });
    const main = (await runtime.agents.subagents.list("owner", agent.id)).find((row) => row.kind === "orchestrator")!;
    await expect(runtime.agents.subagents.unarchive("owner", agent.id, main.id)).rejects.toMatchObject({ status: 409 });
    expect((await detail(agent.id, worker.id)).definition.archived).toBe(true);
  });
  it("creates paused workers from every bundled template with persisted origin and safe retries", async () => {
    const agent = await createAgent();
    for (const template of subagentTemplates) {
      const draft = templateDraft(template.id, { provider: "faux", modelId: "faux-1", thinkingLevel: "" });
      const requestId = randomUUID();
      const worker = await runtime.agents.subagents.create("owner", agent.id, draft, requestId, template.id);
      expect(worker.enabled).toBe(false);
      expect(worker.templateOrigin).toMatchObject({ id: template.id, version: template.version, hash: expect.any(String) });
      expect((await detail(agent.id, worker.id)).definition).toEqual(worker);
      await expect(runtime.agents.subagents.create("owner", agent.id, draft, requestId, template.id)).resolves.toEqual(worker);
    }
  });

  it("allows one manual run while paused without enabling scheduling or delegation", async () => {
    const agent = await createAgent();
    const worker = await createWorker(agent.id, { enabled: false, schedule: "15m" });
    await runtime.agents.subagents.tick();
    expect((await detail(agent.id, worker.id)).runs).toEqual([]);
    expect(await runtime.agents.subagents.listWorkers(agent.id)).toEqual([expect.objectContaining({ id: worker.id, enabled: false, delegatable: true })]);
    await expect(runtime.agents.subagents.delegate(agent.id, worker.id, "Automatic assignment", randomUUID())).rejects.toMatchObject({ status: 409 });

    faux.setResponses([fauxAssistantMessage("Manual assignment complete")]);
    await runtime.agents.subagents.run("owner", agent.id, worker.id, "Run once", randomUUID());
    const run = await completed(agent.id, worker.id);
    expect(run.source).toBe("manual");
    expect(run.result).toContain("Manual assignment complete");
    expect((await detail(agent.id, worker.id)).definition).toMatchObject({ enabled: false, nextRunAt: null });
    await runtime.agents.subagents.tick();
    expect((await detail(agent.id, worker.id)).runs).toHaveLength(1);

    await runtime.agents.subagents.archive("owner", agent.id, worker.id);
    await expect(runtime.agents.subagents.run("owner", agent.id, worker.id, "Run archived worker", randomUUID())).rejects.toMatchObject({ status: 409 });
  });

  it("seeds the existing protected root first without model calls; persists editable instructions/tools/model", async () => {
    const agent = await createAgent(); const worker = await createWorker(agent.id);
    const list = await runtime.agents.subagents.list("owner", agent.id);
    expect(list.map((row) => row.kind)).toEqual(["orchestrator", "worker"]);
    const main = list[0]!; expect(main.conversationId).toBe(agent.conversationId); expect(main.tools).toContain("submit_orders");
    await expect(runtime.agents.subagents.archive("owner", agent.id, main.id)).rejects.toMatchObject({ status: 409 });
    await expect(runtime.agents.subagents.update("owner", agent.id, main.id, { ...main, enabled: false })).rejects.toThrow("cannot");
    await runtime.agents.subagents.update("owner", agent.id, main.id, { ...main, instructions: "Custom coordinator", tools: ["get_account"] });
    await runtime.close(); runtime = await createRuntime(config(), options());
    const restored = await detail(agent.id, main.id);
    expect(restored.definition.instructions).toBe("Custom coordinator"); expect(restored.definition.tools).toEqual(["get_account"]);
    expect((await detail(agent.id, worker.id)).runs).toEqual([]); expect(faux.state.callCount).toBe(0);
  });

  it("enables Code Mode only when selected and shares artifacts across sibling workers", async () => {
    const agent = await createAgent();
    const main = (await runtime.agents.subagents.list("owner", agent.id)).find((row) => row.kind === "orchestrator")!;
    expect(main.tools).not.toContain("codemode");
    const writer = await createWorker(agent.id, { tools: ["codemode", "artifact_create"] });
    faux.setResponses([fauxAssistantMessage(fauxToolCall("codemode", { code: "text(await tools.artifact_create({title:'Shared report',mediaType:'text/markdown',content:'# Research'}));" }), { stopReason: "toolUse" }), fauxAssistantMessage("Saved report")]);
    await runtime.agents.subagents.run("owner", agent.id, writer.id, "Save a report", randomUUID());
    const written = await completed(agent.id, writer.id);
    const artifact = (await runtime.agents.research.artifacts.list(agent.id)).items[0]!;
    expect(artifact.title).toBe("Shared report"); expect(artifact.conversationId).toBe(written.conversationId);
    const detail = await runtime.agents.subagents.runDetail("owner", agent.id, writer.id, written.id);
    expect(detail.tools).toContainEqual(expect.objectContaining({ name: "↳ artifact_create", status: "ok" }));
    const reader = await createWorker(agent.id, { tools: ["codemode", "artifact_read"] });
    faux.setResponses([fauxAssistantMessage(fauxToolCall("codemode", { code: `text((await tools.artifact_read({id:${JSON.stringify(artifact.id)}})).content);` }), { stopReason: "toolUse" }), fauxAssistantMessage("Read sibling report")]);
    await runtime.agents.subagents.run("owner", agent.id, reader.id, "Read the report", randomUUID());
    const read = await completed(agent.id, reader.id);
    const transcript = await runtime.agents.subagents.runDetail("owner", agent.id, reader.id, read.id);
    expect(JSON.stringify(transcript.entries)).toContain("# Research");
    expect(transcript.tools).toContainEqual(expect.objectContaining({ name: "↳ artifact_read", status: "ok" }));
  });

  it("selects the ta extension independently of research and Code Mode", async () => {
    const opened = vi.spyOn(Harness, "open");
    const agent = await createAgent();
    const harness = await opened.mock.results[0]!.value as Harness;
    const main = await harness.root(context);
    expect((await main.agent(context)).extensions.map((extension) => extension.name)).not.toContain("ta");
    expect((await main.agent(context)).tools.some((tool) => tool.name === "codemode")).toBe(false);
    const candles = Array.from({ length: 80 }, (_, i) => ({ time: i * 3600, open: i + 10, high: i + 12, low: i + 9, close: i + 11, volume: 20 }));
    vi.spyOn(runtime.hyperliquid.charts, "researchHistory").mockResolvedValue({ coin: "BTC", interval: "1h", start: 0, end: 80 * 3600000, pageEnd: 80 * 3600000, candles, nextStart: null, coverage: { complete: true, returned: 80, expected: 80, closedOnly: true, sourceLimit: "fixture" } });
    const worker = await createWorker(agent.id, { tools: ["ta_macd"] });
    faux.setResponses([fauxAssistantMessage(fauxToolCall("ta_macd", { coin: "BTC", interval: "1h", start: 0, end: 288000000, fastPeriod: 8, slowPeriod: 21, signalPeriod: 5 }), { stopReason: "toolUse" }), fauxAssistantMessage("MACD complete")]);
    await runtime.agents.subagents.run("owner", agent.id, worker.id, "Calculate MACD 8/21/5", randomUUID());
    const run = await completed(agent.id, worker.id);
    const child = await harness.conversation(run.conversationId! as ConversationId, context);
    const resolved = await child!.agent(context);
    expect(resolved.extensions.map((extension) => extension.name)).toEqual(["hyperliquid", "ta"]);
    expect(resolved.tools.map((tool) => tool.name)).toEqual(["ta_macd"]);
    const transcript = await runtime.agents.subagents.runDetail("owner", agent.id, worker.id, run.id);
    expect(transcript.tools).toContainEqual(expect.objectContaining({ name: "ta_macd", status: "done" }));
    const system = JSON.stringify(transcript.entries.filter((entry) => entry.kind === "pi.system"));
    expect(system).toContain("Code Mode is not selected");
    expect(system).toContain("at least 5 times the longest configured indicator period");
    expect(system).toContain("5000-input cap");
    expect(system).not.toContain("function scalar(result");
    const definition = (await runtime.agents.subagents.list("owner", agent.id)).find((row) => row.kind === "orchestrator")!;
    await runtime.agents.subagents.updateTools("owner", agent.id, definition.id, ["ta_rsi"]);
    const rootAgent = await main.agent(context);
    expect(rootAgent.extensions.map((extension) => extension.name)).toEqual(["hyperliquid", "subagents", "ta", "orchestrator-insights"]);
    expect(rootAgent.tools.map((tool) => tool.name)).toEqual(["ta_rsi"]);
  });

  it("rejects cross-user and cross-Agent access, unknown tools, recursive delegation and malformed schedules", async () => {
    const agent = await createAgent(), other = await createAgent(), worker = await createWorker(agent.id);
    await expect(runtime.agents.subagents.detail("other", agent.id, worker.id)).rejects.toMatchObject({ status: 404 });
    await expect(detail(other.id, worker.id)).rejects.toMatchObject({ status: 404 });
    for (const input of [settings({ tools: ["bash"] }), settings({ tools: ["research_ta"] }), settings({ tools: ["get_subagent_run"] }), settings({ schedule: "bad" }), settings({ timeoutSeconds: 0 }), { ...settings(), enabled: "true" }]) expect(() => subagentInput(input)).toThrow();
    await expect(runtime.agents.subagents.run("other", agent.id, worker.id, "Do work", randomUUID())).rejects.toMatchObject({ status: 404 });
  });

  it("saves tools independently while preserving other settings and schedule state", async () => {
    const agent = await createAgent(), worker = await createWorker(agent.id, { schedule: "1d", persistent: true });
    const nextRunAt = Date.now() + 100000000;
    await runtime.db.update(hvSubagents).set({ nextRunAt }).where(eq(hvSubagents.id, worker.id)).run();
    const updated = await runtime.agents.subagents.updateTools("owner", agent.id, worker.id, ["get_account"]);
    expect(updated).toEqual({ ...worker, tools: ["get_account"], nextRunAt, updatedAt: updated.updatedAt });
    await expect(runtime.agents.subagents.updateTools("other", agent.id, worker.id, [])).rejects.toMatchObject({ status: 404 });
    expect((await runtime.agents.subagents.updateTools("owner", agent.id, worker.id, ["list_subagents", "delegate_subagent"])).tools).toEqual(["list_subagents", "delegate_subagent"]);
    await expect(runtime.agents.subagents.updateTools("owner", agent.id, worker.id, ["get_account", "get_account"])).rejects.toThrow("supported tools");
    await runtime.agents.subagents.archive("owner", agent.id, worker.id);
    await expect(runtime.agents.subagents.updateTools("owner", agent.id, worker.id, [])).rejects.toMatchObject({ status: 409 });
  });

  it("saves identity independently without changing tools, context or schedule state", async () => {
    const agent = await createAgent(), worker = await createWorker(agent.id, { schedule: "1d", persistent: true, tools: ["get_account"] });
    const nextRunAt = Date.now() + 100000000;
    await runtime.db.update(hvSubagents).set({ nextRunAt }).where(eq(hvSubagents.id, worker.id)).run();
    const updated = await runtime.agents.subagents.updateIdentity("owner", agent.id, worker.id, " Updated name ", " Updated description ");
    expect(updated).toEqual({ ...worker, name: "Updated name", description: "Updated description", nextRunAt, updatedAt: updated.updatedAt });
    await expect(runtime.agents.subagents.updateIdentity("other", agent.id, worker.id, "Name", "")).rejects.toMatchObject({ status: 404 });
    const other = await createAgent();
    await expect(runtime.agents.subagents.updateIdentity("owner", other.id, worker.id, "Name", "")).rejects.toMatchObject({ status: 404 });
    await expect(runtime.agents.subagents.updateIdentity("owner", agent.id, worker.id, " ", "")).rejects.toThrow("Name");
    await expect(runtime.agents.subagents.updateIdentity("owner", agent.id, worker.id, "Name", "x".repeat(501))).rejects.toThrow("description");
    await runtime.agents.subagents.archive("owner", agent.id, worker.id);
    await expect(runtime.agents.subagents.updateIdentity("owner", agent.id, worker.id, "Name", "")).rejects.toMatchObject({ status: 409 });
  });

  it("updates assignment settings independently and maintains scheduling and context boundaries", async () => {
    const agent = await createAgent(), worker = await createWorker(agent.id, { schedule: "1d", persistent: true, tools: ["get_account"] });
    const nextRunAt = Date.now() + 100000000;
    await runtime.db.update(hvSubagents).set({ nextRunAt, conversationId: 12345 }).where(eq(hvSubagents.id, worker.id)).run();
    const settings = { delegatable: false, persistent: true, timeoutSeconds: 600 };
    const updated = await runtime.agents.subagents.updateAssignments("owner", agent.id, worker.id, { ...settings, name: "Do not change", tools: [] });
    expect(updated).toEqual({ ...worker, ...settings, nextRunAt, conversationId: 12345, updatedAt: updated.updatedAt });
    const fresh = await runtime.agents.subagents.updateAssignments("owner", agent.id, worker.id, { ...settings, enabled: false, persistent: false });
    expect(fresh.enabled).toBe(true); expect(fresh.nextRunAt).toBe(nextRunAt); expect(fresh.conversationId).toBeNull();
    const persistent = await runtime.agents.subagents.updateAssignments("owner", agent.id, worker.id, settings);
    expect(persistent.nextRunAt).toBe(nextRunAt); expect(persistent.conversationId).toBeNull();
    await expect(runtime.agents.subagents.updateAssignments("other", agent.id, worker.id, settings)).rejects.toMatchObject({ status: 404 });
    const other = await createAgent();
    await expect(runtime.agents.subagents.updateAssignments("owner", other.id, worker.id, settings)).rejects.toMatchObject({ status: 404 });
    for (const data of [null, {}, { ...settings, delegatable: "true" }, { ...settings, persistent: 1 }, { ...settings, timeoutSeconds: 29 }, { ...settings, timeoutSeconds: 3601 }, { ...settings, timeoutSeconds: 30.5 }]) {
      await expect(runtime.agents.subagents.updateAssignments("owner", agent.id, worker.id, data)).rejects.toThrow();
    }
    const main = (await runtime.agents.subagents.list("owner", agent.id)).find((row) => row.kind === "orchestrator")!;
    await expect(runtime.agents.subagents.updateAssignments("owner", agent.id, main.id, settings)).rejects.toMatchObject({ status: 409 });
    await runtime.agents.subagents.archive("owner", agent.id, worker.id);
    await expect(runtime.agents.subagents.updateAssignments("owner", agent.id, worker.id, settings)).rejects.toMatchObject({ status: 409 });
  });

  it("toggles enabled status independently, preserving context and recalculating scheduled runs", async () => {
    const agent = await createAgent(), worker = await createWorker(agent.id, { schedule: "1d", persistent: true, tools: ["get_account"] });
    const nextRunAt = Date.now() + 100000000;
    await runtime.db.update(hvSubagents).set({ nextRunAt, conversationId: 12345 }).where(eq(hvSubagents.id, worker.id)).run();
    const unchanged = await runtime.agents.subagents.updateSection("owner", agent.id, worker.id, "enabled", { enabled: true });
    expect(unchanged.nextRunAt).toBe(nextRunAt);
    const paused = await runtime.agents.subagents.updateSection("owner", agent.id, worker.id, "enabled", { enabled: false, instructions: "Ignored", persistent: false });
    expect(paused).toEqual({ ...worker, enabled: false, nextRunAt: null, conversationId: 12345, updatedAt: paused.updatedAt });
    expect(await runtime.agents.subagents.listWorkers(agent.id)).toEqual([expect.objectContaining({ id: worker.id, enabled: false, delegatable: true })]);
    const assignmentUpdate = await runtime.agents.subagents.updateAssignments("owner", agent.id, worker.id, { delegatable: true, persistent: true, timeoutSeconds: 600, enabled: true });
    expect(assignmentUpdate.enabled).toBe(false); expect(assignmentUpdate.nextRunAt).toBeNull();
    const enabled = await runtime.agents.subagents.updateSection("owner", agent.id, worker.id, "enabled", { enabled: true });
    expect(enabled.nextRunAt).toBe(nextCandleRun(enabled.updatedAt, worker.schedule, worker.delaySeconds));
    expect(enabled.conversationId).toBe(12345); expect(enabled.timeoutSeconds).toBe(600);
    for (const value of ["true", 1, null, undefined]) await expect(runtime.agents.subagents.updateSection("owner", agent.id, worker.id, "enabled", { enabled: value })).rejects.toThrow("enabled");
    await expect(runtime.agents.subagents.updateSection("other", agent.id, worker.id, "enabled", { enabled: false })).rejects.toMatchObject({ status: 404 });
    const other = await createAgent();
    await expect(runtime.agents.subagents.updateSection("owner", other.id, worker.id, "enabled", { enabled: false })).rejects.toMatchObject({ status: 404 });
    const main = (await runtime.agents.subagents.list("owner", agent.id)).find((row) => row.kind === "orchestrator")!;
    await expect(runtime.agents.subagents.updateSection("owner", agent.id, main.id, "enabled", { enabled: false })).rejects.toMatchObject({ status: 409 });
    await runtime.agents.subagents.archive("owner", agent.id, worker.id);
    await expect(runtime.agents.subagents.updateSection("owner", agent.id, worker.id, "enabled", { enabled: true })).rejects.toMatchObject({ status: 409 });
  });

  it("saves instructions, model and schedule sections without overwriting other settings", async () => {
    const agent = await createAgent(), worker = await createWorker(agent.id, { schedule: "1d", persistent: true, tools: ["get_account"] });
    const nextRunAt = Date.now() + 100000000;
    await runtime.db.update(hvSubagents).set({ nextRunAt, conversationId: 12345 }).where(eq(hvSubagents.id, worker.id)).run();
    const instructions = await runtime.agents.subagents.updateSection("owner", agent.id, worker.id, "instructions", { instructions: " New instructions ", name: "Ignored", tools: [] });
    expect(instructions).toEqual({ ...worker, instructions: "New instructions", nextRunAt, conversationId: 12345, updatedAt: instructions.updatedAt });
    const model = await runtime.agents.subagents.updateSection("owner", agent.id, worker.id, "model", { provider: worker.provider, modelId: worker.modelId, thinkingLevel: "" });
    expect(model).toEqual({ ...instructions, updatedAt: model.updatedAt });
    const scheduled = await runtime.agents.subagents.updateSection("owner", agent.id, worker.id, "schedule", { schedule: "1h", delaySeconds: 10, completion: "notify" });
    expect(scheduled).toEqual({ ...model, schedule: "1h", delaySeconds: 10, completion: "notify", nextRunAt: nextCandleRun(scheduled.updatedAt, "1h", 10), updatedAt: scheduled.updatedAt });
    expect((await runtime.agents.subagents.updateSection("owner", agent.id, worker.id, "schedule", { schedule: "", delaySeconds: 10, completion: "save" })).nextRunAt).toBeNull();
    for (const [section, settings] of [
      ["unknown", {}], ["instructions", { instructions: " " }], ["instructions", { instructions: "x".repeat(32001) }],
      ["model", { provider: "missing", modelId: "missing", thinkingLevel: "" }], ["model", { ...worker, thinkingLevel: "invalid" }],
      ["schedule", { schedule: "1m", delaySeconds: 60, completion: "save" }], ["schedule", { schedule: "bad", delaySeconds: 0, completion: "save" }],
    ] as const) await expect(runtime.agents.subagents.updateSection("owner", agent.id, worker.id, section, settings)).rejects.toThrow();
    await expect(runtime.agents.subagents.updateSection("other", agent.id, worker.id, "instructions", { instructions: "Denied" })).rejects.toMatchObject({ status: 404 });
    const other = await createAgent();
    await expect(runtime.agents.subagents.updateSection("owner", other.id, worker.id, "instructions", { instructions: "Denied" })).rejects.toMatchObject({ status: 404 });
    const main = (await runtime.agents.subagents.list("owner", agent.id)).find((row) => row.kind === "orchestrator")!;
    await expect(runtime.agents.subagents.updateSection("owner", agent.id, main.id, "schedule", { schedule: "", delaySeconds: 0, completion: "save" })).rejects.toMatchObject({ status: 409 });
    const mainInstructions = await runtime.agents.subagents.updateSection("owner", agent.id, main.id, "instructions", { instructions: "Coordinate carefully" });
    expect(mainInstructions.instructions).toBe("Coordinate carefully"); expect(mainInstructions.tools).toEqual(main.tools);
    await runtime.agents.subagents.archive("owner", agent.id, worker.id);
    await expect(runtime.agents.subagents.updateSection("owner", agent.id, worker.id, "instructions", { instructions: "Denied" })).rejects.toMatchObject({ status: 409 });
  });

  it("pushes scheduled skips and scheduler errors, and releases history subscriptions on abort", async () => {
    const agent = await createAgent(), worker = await createWorker(agent.id, { schedule: "1h" });
    const source = new AbortController(), publish = vi.fn();
    const watching = runtime.agents.subagents.watchHistory("owner", agent.id, worker.id, 1, source.signal, publish);
    try {
      await vi.waitFor(() => expect(publish).toHaveBeenCalledOnce());
      runtime.agents.subagents.schedulerError = "Scheduler unavailable";
      await vi.waitFor(() => expect(publish.mock.lastCall?.[0].schedulerError).toBe("Scheduler unavailable"));
      const now = Date.now();
      await runtime.db.update(hvSubagents).set({ nextRunAt: now - 120000 }).where(eq(hvSubagents.id, worker.id)).run();
      await runtime.agents.subagents.tick(now);
      await vi.waitFor(() => {
        expect(publish.mock.lastCall?.[0].runs[0]).toMatchObject({ source: "schedule", status: "skipped" });
        expect(publish.mock.lastCall?.[0].schedulerError).toBeNull();
      });
      expect(faux.state.callCount).toBe(0);
    } finally { source.abort(); await watching; }
    const calls = publish.mock.calls.length;
    runtime.agents.subagents.schedulerError = "After disconnect";
    expect(publish).toHaveBeenCalledTimes(calls);
  });

  it("streams live run-detail snapshots until the subscription aborts", async () => {
    const agent = await createAgent(), worker = await createWorker(agent.id);
    faux.setResponses([fauxAssistantMessage("Live worker answer")]);
    const run = await runtime.agents.subagents.run("owner", agent.id, worker.id, "Live task", randomUUID());
    const source = new AbortController(), publish = vi.fn();
    const watching = runtime.agents.subagents.watchRun("owner", agent.id, worker.id, run.id, source.signal, publish);
    try {
      await vi.waitFor(() => expect(publish).toHaveBeenCalled());
      expect(publish.mock.lastCall?.[0].run.id).toBe(run.id);
      await completed(agent.id, worker.id);
      await vi.waitFor(() => expect(publish.mock.lastCall?.[0].run.status).toBe("completed"));
      expect(publish.mock.lastCall?.[0].run.result).toBe("Live worker answer");
      expect(JSON.stringify(publish.mock.lastCall?.[0].entries)).toContain("Live task");
      expect(JSON.stringify(publish.mock.lastCall?.[0].entries)).toContain("Live worker answer");
    } finally { source.abort(); await watching; }
    const calls = publish.mock.calls.length;
    await runtime.agents.subagents.tick();
    expect(publish).toHaveBeenCalledTimes(calls);
    const otherWorker = await createWorker(agent.id);
    await expect(runtime.agents.subagents.watchRun("other", agent.id, worker.id, run.id, source.signal, publish)).rejects.toMatchObject({ status: 404 });
    await expect(runtime.agents.subagents.watchRun("owner", agent.id, otherWorker.id, run.id, source.signal, publish)).rejects.toMatchObject({ status: 404 });
    await expect(runtime.agents.subagents.watchRun("owner", agent.id, worker.id, "missing", source.signal, publish)).rejects.toMatchObject({ status: 404 });
  });

  it("paginates history in stable newest-first pages without large result payloads", async () => {
    const agent = await createAgent(), worker = await createWorker(agent.id);
    expect((await runtime.agents.subagents.history("owner", agent.id, worker.id)).pagination).toEqual({ page: 1, pages: 1, pageSize: 50, total: 0 });
    await runtime.db.insert(hvSubagentRuns).values(Array.from({ length: 105 }, (_, index) => ({ id: `history-${String(index).padStart(3, "0")}`, agentId: agent.id, subagentId: worker.id, requestId: `history:${index}`, source: "schedule" as const, task: "x".repeat(16000), config: worker, status: "skipped" as const, result: "Large result must not be included", createdAt: 1, reportedAt: 1 }))).run();
    const pages = await Promise.all([1, 2, 3].map((page) => runtime.agents.subagents.history("owner", agent.id, worker.id, page)));
    expect(pages.map((page) => page.runs.length)).toEqual([50, 50, 5]);
    expect(pages.map((page) => page.pagination.page)).toEqual([1, 2, 3]);
    expect(pages[0]!.pagination).toEqual({ page: 1, pages: 3, total: 105, pageSize: 50 });
    const ids = pages.flatMap((page) => page.runs.map((run) => run.id));
    expect(new Set(ids).size).toBe(105);
    expect(pages[0]!.runs[0]!.task).toHaveLength(512);
    expect(ids).toEqual([...ids].sort().reverse());
    expect(Object.keys(pages[0]!.runs[0]!).sort()).toEqual(["id", "subagentId", "source", "task", "status", "createdAt"].sort());
    expect((await runtime.agents.subagents.history("owner", agent.id, worker.id, 999)).pagination.page).toBe(3);
    for (const page of [0, -1, 1.5, NaN, Infinity]) await expect(runtime.agents.subagents.history("owner", agent.id, worker.id, page)).rejects.toMatchObject({ status: 400 });
    await expect(runtime.agents.subagents.history("other", agent.id, worker.id)).rejects.toMatchObject({ status: 404 });
    const other = await createAgent();
    await expect(runtime.agents.subagents.history("owner", other.id, worker.id)).rejects.toMatchObject({ status: 404 });
    expect((await detail(agent.id, worker.id)).pagination.total).toBe(105);
  });

  it("runs fresh scoped conversations, saves full results and deduplicates assignment retries", async () => {
    const opened = vi.spyOn(Harness, "open"); const agent = await createAgent(), worker = await createWorker(agent.id, { tools: ["get_account"] });
    const harness = await opened.mock.results[0]!.value as Harness;
    const request = randomUUID(); faux.setResponses([fauxAssistantMessage("Research evidence"), fauxAssistantMessage("Second assignment")]);
    const [one, again] = await Promise.all([runtime.agents.subagents.run("owner", agent.id, worker.id, "First task", request), runtime.agents.subagents.run("owner", agent.id, worker.id, "First task", request)]);
    expect(one.id).toBe(again.id);
    const first = await completed(agent.id, worker.id); expect(first.result).toBe("Research evidence");
    const child = await harness.conversation(first.conversationId! as never, context); const resolved = await child!.agent(context);
    expect(resolved.tools.map((tool) => tool.name)).toEqual(["get_account"]); expect(resolved.extensions.map((extension) => extension.name)).toEqual(["hyperliquid"]);
    await runtime.agents.subagents.run("owner", agent.id, worker.id, "Second task", randomUUID());
    const second = await completed(agent.id, worker.id); expect(second.result).toBe("Second assignment"); expect(second.conversationId).not.toBe(first.conversationId);
    expect((await runtime.agents.chat("owner", agent.id)).messages).toEqual([]);
    await expect(runtime.agents.subagents.run("owner", agent.id, worker.id, "Changed task", request)).rejects.toMatchObject({ status: 409 });
  });

  it("optionally reuses a persistent worker conversation", async () => {
    const agent = await createAgent(), worker = await createWorker(agent.id, { persistent: true });
    faux.setResponses([fauxAssistantMessage("Remember this"), (request) => { expect(JSON.stringify(request.messages)).toContain("Remember this"); return fauxAssistantMessage("I remember"); }]);
    await runtime.agents.subagents.run("owner", agent.id, worker.id, "First", randomUUID()); const first = await completed(agent.id, worker.id);
    await runtime.agents.subagents.run("owner", agent.id, worker.id, "Second", randomUUID()); const second = await completed(agent.id, worker.id);
    expect(second.conversationId).toBe(first.conversationId); expect(second.result).toBe("I remember");
    const firstDetail = await runtime.agents.subagents.runDetail("owner", agent.id, worker.id, first.id);
    expect(firstDetail.prior).toEqual([]);
    expect(JSON.stringify(firstDetail.entries)).toContain("Remember this");
    expect(JSON.stringify(firstDetail.entries)).not.toContain("I remember");
    const secondDetail = await runtime.agents.subagents.runDetail("owner", agent.id, worker.id, second.id);
    expect(JSON.stringify(secondDetail.prior)).toContain("Remember this");
    expect(JSON.stringify(secondDetail.entries)).toContain("I remember");
    expect(JSON.stringify(secondDetail.entries)).not.toContain("Remember this");
    await runtime.close(); runtime = await createRuntime(config(), options());
    expect((await runtime.agents.subagents.runDetail("owner", agent.id, worker.id, first.id)).entries).toEqual(firstDetail.entries);
  });

  it("returns complete transcripts across pages, preserves tool calls/results and enforces ownership", async () => {
    const opened = vi.spyOn(Harness, "open");
    const agent = await createAgent(), worker = await createWorker(agent.id);
    faux.setResponses([fauxAssistantMessage(fauxToolCall("unavailable_tool", { evidence: "test" }), { stopReason: "toolUse" }), fauxAssistantMessage("x".repeat(65000))]);
    await runtime.agents.subagents.run("owner", agent.id, worker.id, "Find evidence", randomUUID());
    const run = await completed(agent.id, worker.id);
    expect(run.result).toHaveLength(65000);
    const harness = await opened.mock.results[0]!.value as Harness;
    const child = (await harness.conversation(run.conversationId! as never, context))!;
    await child.commit(async (tx) => {
      for (let i = 0; i < 205; i++) await tx.appendEntry(child.id, { kind: "test.evidence", data: { index: i } });
    }, context);
    const result = await runtime.agents.subagents.runDetail("owner", agent.id, worker.id, run.id);
    expect(result.entries.filter((entry) => entry.kind === "test.evidence")).toHaveLength(205);
    expect(result.entries.flatMap((entry) => entry.model ?? []).some((message) => message.role === "toolResult")).toBe(true);
    expect(JSON.stringify(result.entries)).toContain("unavailable_tool");
    expect(result.run.config.instructions).toBe(worker.instructions);
    const otherWorker = await createWorker(agent.id), otherAgent = await createAgent();
    for (const [user, agentId, id, runId] of [["other", agent.id, worker.id, run.id], ["owner", otherAgent.id, worker.id, run.id], ["owner", agent.id, otherWorker.id, run.id], ["owner", agent.id, worker.id, "missing"]]) {
      await expect(runtime.agents.subagents.runDetail(user!, agentId!, id!, runId!)).rejects.toMatchObject({ status: 404 });
    }
  });

  it("delivers notification-only results visibly without waking the orchestrator, once across restart", async () => {
    const agent = await createAgent(), worker = await createWorker(agent.id, { completion: "notify" });
    faux.setResponses([fauxAssistantMessage("Notification evidence")]);
    await runtime.agents.subagents.run("owner", agent.id, worker.id, "Research", randomUUID());
    await vi.waitFor(async () => expect((await runtime.agents.chat("owner", agent.id)).messages.filter((message) => message.text.includes("Notification evidence"))).toHaveLength(1));
    expect(faux.state.callCount).toBe(1);
    await runtime.close(); runtime = await createRuntime(config(), options()); runtime.start();
    expect((await runtime.agents.chat("owner", agent.id)).messages.filter((message) => message.text.includes("Notification evidence"))).toHaveLength(1);
    expect(faux.state.callCount).toBe(1);
  });

  it("lets the orchestrator delegate in the background and always receive the result", async () => {
    const agent = await createAgent(), worker = await createWorker(agent.id, { completion: "save" });
    faux.setResponses(Array.from({ length: 8 }, () => (request) => {
      const text = JSON.stringify(request.messages);
      if (text.includes("You are the HyperVibes worker")) return fauxAssistantMessage("Delegated research result");
      if (text.includes("[Subagent Research;")) return fauxAssistantMessage("I have the research result");
      if (request.messages.some((message) => message.role === "toolResult")) return fauxAssistantMessage("Research is working in the background");
      return fauxAssistantMessage(fauxToolCall("delegate_subagent", { subagent_id: worker.id, task: "Find evidence" }), { stopReason: "toolUse" });
    }));
    await runtime.agents.send("owner", agent.id, "Ask Research for evidence", randomUUID());
    await vi.waitFor(async () => expect((await runtime.agents.chat("owner", agent.id)).messages.some((message) => message.text === "I have the research result")).toBe(true));
    expect((await detail(agent.id, worker.id)).runs).toHaveLength(1);
    expect((await detail(agent.id, worker.id)).runs[0]).toMatchObject({ source: "delegation", status: "completed", result: "Delegated research result" });
    const disabled = await runtime.agents.subagents.update("owner", agent.id, worker.id, { ...worker, delegatable: false });
    expect(await runtime.agents.subagents.listWorkers(agent.id)).toEqual([expect.objectContaining({ id: worker.id, enabled: true, delegatable: false })]);
    await expect(runtime.agents.subagents.delegate(agent.id, disabled.id, "No", "delegate:99999")).rejects.toMatchObject({ status: 409 });
  });

  it("prevents overlapping assignments and edits; stopping main response leaves background work alone", async () => {
    faux = fauxProvider({ tokensPerSecond: 1 }); models.setProvider(faux.provider);
    faux.setResponses([fauxAssistantMessage("This worker response stays running long enough to stop explicitly.")]);
    const agent = await createAgent(), worker = await createWorker(agent.id);
    const run = await runtime.agents.subagents.run("owner", agent.id, worker.id, "Long task", randomUUID());
    await vi.waitFor(() => expect(faux.state.callCount).toBe(1));
    await expect(runtime.agents.subagents.run("owner", agent.id, worker.id, "Other task", randomUUID())).rejects.toMatchObject({ status: 409 });
    await expect(runtime.agents.subagents.update("owner", agent.id, worker.id, worker)).rejects.toMatchObject({ status: 409 });
    await expect(runtime.agents.subagents.updateAssignments("owner", agent.id, worker.id, { enabled: true, delegatable: true, persistent: false, timeoutSeconds: 600 })).rejects.toMatchObject({ status: 409 });
    await expect(runtime.agents.subagents.updateSection("owner", agent.id, worker.id, "instructions", { instructions: "Not while running" })).rejects.toMatchObject({ status: 409 });
    await expect(runtime.agents.subagents.updateSection("owner", agent.id, worker.id, "enabled", { enabled: false })).resolves.toMatchObject({ enabled: false });
    await expect(runtime.agents.subagents.archive("owner", agent.id, worker.id)).rejects.toMatchObject({ status: 409 });
    await runtime.agents.stop("owner", agent.id);
    expect((await detail(agent.id, worker.id)).runs[0]?.status).toBe("running");
    await runtime.agents.subagents.stop("owner", agent.id, run.id);
    expect((await detail(agent.id, worker.id)).runs[0]?.status).toBe("cancelled");
    await runtime.agents.subagents.archive("owner", agent.id, worker.id);
    await expect(runtime.agents.subagents.run("owner", agent.id, worker.id, "No", randomUUID())).rejects.toMatchObject({ status: 409 });
  }, 15000);

  it("aligns hourly/daily closes in UTC, deduplicates due runs and skips overlaps and downtime", async () => {
    const at = Date.parse("2026-10-16T10:12:00Z");
    expect(nextCandleRun(at, "1h", 30)).toBe(Date.parse("2026-10-16T11:00:30Z"));
    expect(nextCandleRun(at, "1d", 0)).toBe(Date.parse("2026-10-17T00:00:00Z"));
    expect(nextCandleRun(Date.parse("2026-10-16T11:00:30Z"), "1h", 30)).toBe(Date.parse("2026-10-16T12:00:30Z"));
    const agent = await createAgent(), worker = await createWorker(agent.id, { schedule: "1h" });
    const due = Date.now(); await runtime.db.update(hvSubagents).set({ nextRunAt: due }).where(eq(hvSubagents.id, worker.id)).run();
    faux.setResponses([fauxAssistantMessage("Hourly result")]);
    await Promise.all([runtime.agents.subagents.tick(due), runtime.agents.subagents.tick(due)]);
    const scheduled = await completed(agent.id, worker.id);
    expect(scheduled.task).toContain("Carry out your responsibilities according to your instructions.");
    expect(scheduled.task).toContain(new Date(due - worker.delaySeconds * 1000).toISOString());
    expect(scheduled.task).toContain(`endMs: ${due - worker.delaySeconds * 1000} (UTC milliseconds, exclusive closed-candle end)`);
    expect(scheduled.config.instructions).toBe(worker.instructions);
    expect((await detail(agent.id, worker.id)).runs).toHaveLength(1);
    const nextRunAt = (await detail(agent.id, worker.id)).definition.nextRunAt;
    expect(nextRunAt).toBeGreaterThan(due);
    expect((await runtime.agents.subagents.history("owner", agent.id, worker.id)).schedule).toEqual({ nextRunAt, updatedAt: worker.updatedAt });
    await runtime.db.update(hvSubagents).set({ nextRunAt: due - 3 * 3600000 }).where(eq(hvSubagents.id, worker.id)).run();
    await runtime.close(); runtime = await createRuntime(config(), options()); runtime.start(); await runtime.agents.subagents.tick();
    expect((await detail(agent.id, worker.id)).definition.nextRunAt).toBeGreaterThan(due);
    expect((await detail(agent.id, worker.id)).runs).toHaveLength(1);
  });

  it("records scheduled overlap skips and bounds concurrent workers", async () => {
    faux = fauxProvider({ tokensPerSecond: 1 }); models.setProvider(faux.provider);
    faux.setResponses(Array.from({ length: 4 }, () => fauxAssistantMessage("Long running worker assignment for concurrency checks.")));
    const agent = await createAgent();
    const workers = await Promise.all(Array.from({ length: 5 }, (_, index) => createWorker(agent.id, { name: `Worker ${index}`, schedule: index === 0 ? "1h" : "" })));
    const runs = [];
    for (const worker of workers.slice(0, 4)) runs.push(await runtime.agents.subagents.run("owner", agent.id, worker.id, "Work", randomUUID()));
    await expect(runtime.agents.subagents.run("owner", agent.id, workers[4]!.id, "Work", randomUUID())).rejects.toMatchObject({ status: 429 });
    const now = Date.now(); await runtime.db.update(hvSubagents).set({ nextRunAt: now }).where(eq(hvSubagents.id, workers[0]!.id)).run();
    await runtime.agents.subagents.tick(now);
    expect((await detail(agent.id, workers[0]!.id)).runs.some((run) => run.source === "schedule" && run.status === "skipped" && run.result.includes("active assignment"))).toBe(true);
    for (const run of runs) await runtime.agents.subagents.stop("owner", agent.id, run.id);
  }, 15000);

  it("retains an undispatched intent after a storage failure and retries it once", async () => {
    const agent = await createAgent(), worker = await createWorker(agent.id);
    const fail = vi.spyOn(SqliteStorage.prototype, "commit").mockRejectedValueOnce(new Error("Dispatch storage failure"));
    const id = randomUUID();
    await expect(runtime.agents.subagents.run("owner", agent.id, worker.id, "Recover intent", id)).rejects.toThrow();
    fail.mockRestore();
    await vi.waitFor(async () => expect((await runtime.agents.owned("owner", agent.id)).state).toBe("failed"));
    const admitted = (await detail(agent.id, worker.id)).runs[0]!;
    const frozen = await runtime.agents.execution.runContext(agent.id, admitted);
    await runtime.agents.execution.insights.append(runtime.agents.execution.operator(agent.id, "owner"), "ui:after-intent", { scope: { kind: "agent" }, content: "NEW guidance after admission", reason: "Review completed while dispatch was interrupted", evidenceRefs: [] });
    faux.setResponses([fauxAssistantMessage("Recovered intent")]);
    await runtime.agents.retry("owner", agent.id);
    await runtime.agents.subagents.tick();
    const run = await completed(agent.id, worker.id); expect(run.result).toBe("Recovered intent");
    expect((await detail(agent.id, worker.id)).runs).toHaveLength(1);
    expect(await runtime.agents.execution.runContext(agent.id, run)).toEqual(frozen);
    expect(JSON.stringify((await runtime.agents.subagents.runDetail("owner", agent.id, worker.id, run.id)).entries)).not.toContain("NEW guidance after admission");
  });

  it("records and reports task failures rather than leaving workers permanently active", async () => {
    const agent = await createAgent(), worker = await createWorker(agent.id, { completion: "notify" });
    vi.spyOn(runtime.agents.subagents, "started").mockRejectedValueOnce(new Error("Injected worker failure"));
    await runtime.agents.subagents.run("owner", agent.id, worker.id, "Fail safely", randomUUID());
    await vi.waitFor(async () => {
      await runtime.agents.subagents.tick();
      expect((await runtime.agents.chat("owner", agent.id)).messages.some((entry) => entry.text.includes("stopped unexpectedly"))).toBe(true);
    });
    expect((await detail(agent.id, worker.id)).runs[0]?.status).toBe("failed"); expect(faux.state.callCount).toBe(0);
  });

  it("does not resume an expired worker's model or tool work after downtime", async () => {
    faux = fauxProvider({ tokensPerSecond: 1 }); models.setProvider(faux.provider);
    const agent = await createAgent(), worker = await createWorker(agent.id, { timeoutSeconds: 30 });
    faux.setResponses([fauxAssistantMessage("Work interrupted before its deadline."), fauxAssistantMessage("Must not resume stale work")]);
    const run = await runtime.agents.subagents.run("owner", agent.id, worker.id, "Expire during downtime", randomUUID());
    await vi.waitFor(() => expect(faux.state.callCount).toBe(1));
    await runtime.db.update(hvSubagentRuns).set({ createdAt: Date.now() - 31000 }).where(eq(hvSubagentRuns.id, run.id)).run();
    await runtime.close(AbortSignal.abort()); runtime = await createRuntime(config(), options());
    expect(faux.state.callCount).toBe(1); runtime.start();
    await vi.waitFor(async () => expect((await detail(agent.id, worker.id)).runs[0]?.status).toBe("failed"));
    expect((await detail(agent.id, worker.id)).runs[0]?.result).toContain("expired"); expect(faux.state.callCount).toBe(1);
  }, 15000);

  it("recovers unfinished work only after host start without duplicating the run", async () => {
    faux = fauxProvider({ tokensPerSecond: 5 }); models.setProvider(faux.provider);
    const agent = await createAgent(), worker = await createWorker(agent.id, { completion: "notify" });
    faux.setResponses([fauxAssistantMessage("An interrupted research response that must be resumed after reopening."), fauxAssistantMessage("Recovered evidence")]);
    const run = await runtime.agents.subagents.run("owner", agent.id, worker.id, "Survive restart", randomUUID());
    await vi.waitFor(() => expect(faux.state.callCount).toBe(1));
    await runtime.close(AbortSignal.abort()); runtime = await createRuntime(config(), options());
    expect(faux.state.callCount).toBe(1);
    await runtime.agents.execution.insights.append(runtime.agents.execution.operator(agent.id, "owner"), "ui:while-paused", { scope: { kind: "agent" }, content: "Do not inject this into the recovered assignment", reason: "New revision while a run was suspended", evidenceRefs: [] });
    runtime.start();
    await vi.waitFor(async () => expect((await detail(agent.id, worker.id)).runs[0]?.result).toBe("Recovered evidence"), { timeout: 10000 });
    expect((await detail(agent.id, worker.id)).runs.map((entry) => entry.id)).toEqual([run.id]);
    expect((await runtime.agents.execution.runContext(agent.id, run)).insights!.agent.version).toBe(0);
    expect(JSON.stringify((await runtime.agents.subagents.runDetail("owner", agent.id, worker.id, run.id)).entries)).not.toContain("Do not inject this into the recovered assignment");
  }, 15000);
});
