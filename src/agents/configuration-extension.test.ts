import type { JsonValue } from "@earendil-works/chord";
import { createModels } from "@earendil-works/pi-ai/models";
import { fauxAssistantMessage, fauxProvider, fauxToolCall } from "@earendil-works/pi-ai/providers/faux";
import { Harness, type Registry } from "@earendil-works/pi-durable";
import Fastify, { type FastifyInstance } from "fastify";
import { randomUUID } from "node:crypto";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createRuntime, type AppRuntime } from "../runtime.js";
import { testConfig } from "../test-config.js";
import { configurationToolNames, type ConfigurationChange } from "./configuration-contracts.js";
import { templateDraft } from "./subagent-templates.js";

// Disposable application database and faux model only. No exchange/model network calls.
describe("Agent-scoped configuration tools", () => {
  let directory: string, runtime: AppRuntime, agentId: string, faux: ReturnType<typeof fauxProvider>, lastToolOutput: string;
  let logging: FastifyInstance, logOutput: string;
  let expectedReports: { msg?: string }[];
  // Direct calls keep their normal text/status when structured output is invalid.
  // The host report is the only indication that the output contract is broken.
  const harnessReports = () => logOutput.split("\n").filter(Boolean)
    .map((line) => JSON.parse(line) as { msg?: string })
    .filter((record) => record.msg === "Agent harness report");
  beforeEach(async () => {
    directory = mkdtempSync(join(tmpdir(), "hv-configuration-"));
    logOutput = ""; expectedReports = [];
    logging = Fastify({ logger: { level: "warn", stream: { write(chunk: string) { logOutput += chunk; } } } });
    faux = fauxProvider({ models: [{ id: "faux-1", input: ["text"] }, { id: "faux-2", input: ["text"] }] });
    const models = createModels(); models.setProvider(faux.provider);
    runtime = await createRuntime(testConfig({ APP_SQLITE_PATH: join(directory, "app.sqlite") }), {
      logger: logging.log,
      models, model: { provider: "faux", modelId: "faux-1" }, accountFeedFactory: () => ({ close: async () => {} }), journalFeedFactory: () => ({ close: async () => {} }), journalHistoryFetcher: async () => [],
    });
    await runtime.database.run("INSERT INTO hv_users(id,name,email) VALUES ('owner','Owner','owner@example.test')");
    agentId = (await runtime.agents.create("owner", "Desk", randomUUID(), `0x${"a".repeat(40)}`)).id;
    runtime.start(); await runtime.agents.subagents.tick();
  });
  afterEach(async () => {
    try { await runtime.close(); }
    finally {
      await logging.close();
      vi.restoreAllMocks();
      rmSync(directory, { recursive: true, force: true });
      // Only the negative control permits its exactly verified report. Reports
      // from teardown or later duplicate reports still fail this assertion.
      expect(harnessReports()).toEqual(expectedReports);
    }
  });
  async function call(name: string, args: { [key: string]: JsonValue }, expectedError?: RegExp) {
    lastToolOutput = "";
    let isError: boolean | undefined;
    faux.setResponses([fauxAssistantMessage(fauxToolCall(name, args), { stopReason: "toolUse" }), (context) => {
      const result = [...context.messages].reverse().find((message) => message.role === "toolResult" && message.toolName === name);
      if (result?.role === "toolResult") {
        lastToolOutput = result.content.filter((part) => part.type === "text").map((part) => part.text).join("\n");
        isError = result.isError;
      }
      return fauxAssistantMessage("Finished inspecting the result.");
    }]);
    await runtime.agents.send("owner", agentId, "Apply this explicitly requested configuration change or inspect the configuration", randomUUID());
    await vi.waitFor(async () => expect((await runtime.agents.chat("owner", agentId)).busy).toBe(false));
    expect(isError, `${name} result: ${lastToolOutput}`).toBe(expectedError !== undefined);
    if (expectedError) expect(lastToolOutput).toMatch(expectedError);
    // Expected tool/permission errors are model-facing results, not host reports.
    expect(harnessReports()).toEqual([]);
    return runtime.agents.chat("owner", agentId);
  }
  async function worker() {
    return runtime.agents.subagents.create("owner", agentId, templateDraft("basic-analysis", { provider: "faux", modelId: "faux-1", thinkingLevel: "" }), randomUUID(), "basic-analysis");
  }
  async function definition(id: string) { return (await runtime.agents.subagents.list("owner", agentId)).find((row) => row.id === id)!; }
  it("exposes mandatory configuration tools and read-only docs by default", async () => {
    const main = await definition(`main:${agentId}`);
    expect(main.tools).toEqual(expect.arrayContaining([...configurationToolNames, "list_docs", "read_doc", "search_docs", "send_notification"]));
    const result = await call("get_agent_configuration", {});
    expect(result.messages.some((entry) => entry.tool?.name === "get_agent_configuration" && entry.tool.status === "done")).toBe(true);
    await call("list_agent_tools", {});
    await call("list_docs", {});
  });
  it("detects a missing direct-call output schema despite normal text and done status", async () => {
    const opened = vi.spyOn(Harness, "open");
    agentId = (await runtime.agents.create("owner", "Broken contract control", randomUUID(), `0x${"c".repeat(40)}`)).id;
    // The application owns a mutable Registry; Harness.open receives its read side.
    const registry = opened.mock.calls[0]![1].registry as Registry;
    const extension = registry.snapshot().extension("agent-configuration")!;
    registry.install({ ...extension, tools: extension.tools!.map((tool) => {
      if (tool.name !== "list_agent_tools") return tool;
      const broken = { ...tool };
      delete broken.structuredOutputSchema;
      return broken;
    }) });
    try {
      // The helper must reject the captured host report, not toolResult.isError.
      await expect(call("list_agent_tools", { limit: 1 })).rejects.toMatchObject({
        actual: [expect.objectContaining({ msg: "Agent harness report" })], expected: [],
      });
      const reports = harnessReports();
      expect(reports).toEqual([expect.objectContaining({
        err: expect.objectContaining({ message: "Tool list_agent_tools returned structuredOutput but declares no structuredOutputSchema" }),
        report: expect.objectContaining({ kind: "extension_error", extension: "agent-configuration" }),
      })]);
      expectedReports = reports;
      expect(JSON.parse(lastToolOutput)).toMatchObject({ subagentId: `main:${agentId}`, tools: [expect.objectContaining({ name: expect.any(String) })] });
      const chat = await runtime.agents.chat("owner", agentId);
      expect(chat.messages.some((entry) => entry.tool?.name === "list_agent_tools" && entry.tool.status === "done")).toBe(true);
    }
    finally { registry.install(extension); }
  });
  it("lists template headers and full template details through direct calls", async () => {
    await call("list_worker_templates", {});
    const headers = JSON.parse(lastToolOutput).templates as { id: string; instructions?: string }[];
    expect(headers).toContainEqual(expect.objectContaining({ id: "basic-analysis", tools: expect.any(Array), defaults: expect.objectContaining({ enabled: false }) }));
    expect(headers.every((template) => template.instructions === undefined)).toBe(true);
    await call("list_worker_templates", { template_id: "basic-analysis" });
    expect(JSON.parse(lastToolOutput)).toMatchObject({ template: { id: "basic-analysis", instructions: expect.any(String), tools: expect.any(Array) } });
  });
  it("reads Agent, worker and orchestrator configuration and tool discovery variants", async () => {
    const row = await worker();
    await call("get_agent_configuration", {});
    expect(JSON.parse(lastToolOutput)).toMatchObject({
      agent: { id: agentId, name: "Desk" }, risk: { readOnly: true },
      telegram: { configured: false, linked: false }, orchestrator: { id: `main:${agentId}` },
      workers: [expect.objectContaining({ id: row.id })], workersPage: { offset: 0, total: 1, nextOffset: null },
      models: expect.any(Array), modelsPage: { offset: 0, nextOffset: null },
    });
    await call("get_agent_configuration", { workers_offset: 1, models_offset: 100 });
    expect(JSON.parse(lastToolOutput)).toMatchObject({ workers: [], models: [], workersPage: { offset: 1, total: 1, nextOffset: null }, modelsPage: { offset: 100, nextOffset: null } });
    for (const target of [row, await definition(`main:${agentId}`)]) {
      await call("get_agent_configuration", { subagent_id: target.id });
      expect(JSON.parse(lastToolOutput)).toMatchObject({ definition: { id: target.id, kind: target.kind, updatedAt: target.updatedAt, instructions: target.instructions }, instructionsPage: { offset: 0, total: target.instructions.length, nextOffset: null } });
    }
    await call("list_agent_tools", { subagent_id: row.id, query: "send_notification" });
    const workerTools = JSON.parse(lastToolOutput);
    expect(workerTools).toMatchObject({ subagentId: row.id, tools: [expect.objectContaining({ name: "send_notification", mandatory: false })] });
    expect(workerTools.tools[0]).not.toHaveProperty("exposedNow");
    await call("list_agent_tools", { query: "get_agent_configuration" });
    expect(JSON.parse(lastToolOutput)).toMatchObject({ subagentId: `main:${agentId}`, tools: [expect.objectContaining({ name: "get_agent_configuration", enabled: true, mandatory: true, exposedNow: true })] });
  });
  it("pages tool discovery and saved instructions without requiring Code Mode", async () => {
    await call("list_agent_tools", { limit: 1 });
    const first = JSON.parse(lastToolOutput) as { nextOffset: number; total: number; tools: { name: string }[] };
    expect(first.nextOffset).toBe(1); expect(first.total).toBeGreaterThan(1);
    await call("list_agent_tools", { limit: 1, offset: first.nextOffset });
    expect(JSON.parse(lastToolOutput).tools[0].name).not.toBe(first.tools[0]!.name);
    await call("list_agent_tools", { query: "research_candles" });
    expect(JSON.parse(lastToolOutput).tools).toContainEqual(expect.objectContaining({ name: "research_candles", enabled: false, exposedNow: false }));
    const root = await definition(`main:${agentId}`);
    await call("get_agent_configuration", { subagent_id: root.id, instructions_limit: 16 });
    expect(JSON.parse(lastToolOutput)).toMatchObject({ definition: { instructions: root.instructions.slice(0, 16) }, instructionsPage: { nextOffset: 16 } });
    await call("get_agent_configuration", { subagent_id: root.id, instructions_offset: 16, instructions_limit: 16 });
    expect(JSON.parse(lastToolOutput)).toMatchObject({ definition: { instructions: root.instructions.slice(16, 32) }, instructionsPage: { offset: 16, nextOffset: 32 } });
  });
  it("shows Telegram link status without exposing connection-error secrets", async () => {
    runtime.agents.installTelegramStatus(async () => ({ hasToken: true, connected: true, botUsername: "desk_bot", chatId: 77, chatUsername: "private_name", error: "Failed https://api.telegram.org/bot123456:SECRET/getUpdates" }));
    await call("get_agent_configuration", {});
    expect(JSON.parse(lastToolOutput)).toMatchObject({ telegram: { configured: true, linked: true, botUsername: "desk_bot", error: expect.any(String) } });
    expect(lastToolOutput).not.toContain("SECRET");
    expect(lastToolOutput).not.toContain("private_name");
  });
  it("patches worker tools and preserves unrelated selections and risk settings", async () => {
    const row = await worker(), risk = await runtime.agents.risk.get(agentId);
    const result = await call("update_agent_configuration", { subagent_id: row.id, expected_updated_at: row.updatedAt, change: { section: "tools", add: ["send_notification", "submit_orders"] } });
    const saved = await definition(row.id);
    expect(saved.tools).toEqual(expect.arrayContaining([...row.tools, "send_notification", "submit_orders"]));
    expect(await runtime.agents.risk.get(agentId)).toEqual(risk);
    expect(result.messages.some((entry) => entry.tool?.name === "update_agent_configuration" && entry.tool.status === "done")).toBe(true);
  });
  const updates: { change: ConfigurationChange; after: object }[] = [
    { change: { section: "identity", name: "Renamed analyst", description: "Focused research" }, after: { name: "Renamed analyst", description: "Focused research" } },
    { change: { section: "instructions", instructions: "Inspect evidence without trading." }, after: { instructions: "Inspect evidence without trading." } },
    { change: { section: "model", provider: "faux", modelId: "faux-2", thinkingLevel: "" }, after: { provider: "faux", modelId: "faux-2", thinkingLevel: "" } },
    { change: { section: "assignments", delegatable: false, persistent: true, timeoutSeconds: 120 }, after: { delegatable: false, persistent: true, timeoutSeconds: 120 } },
    { change: { section: "schedule", schedule: "1h", delaySeconds: 15, completion: "notify" }, after: { schedule: "1h", delaySeconds: 15, completion: "notify", nextRunAt: null } },
    { change: { section: "triggers", triggers: { inboxEnabled: true, fills: true, orderChanges: true } }, after: { triggers: { inboxEnabled: true, fills: true, orderChanges: true, workerDelegates: [], researchSources: [] } } },
  ];
  it.each(updates)("returns valid direct output for the $change.section update section", async ({ change, after }) => {
    const row = await worker();
    await call("update_agent_configuration", { subagent_id: row.id, expected_updated_at: row.updatedAt, change });
    expect(JSON.parse(lastToolOutput)).toMatchObject({ subagentId: row.id, section: change.section, before: expect.any(Object), after, updatedAt: expect.any(Number), executionToolsAdded: [] });
    expect(await definition(row.id)).toMatchObject(after);
  });
  it("can change its own tools during a turn without losing configuration management", async () => {
    const main = await definition(`main:${agentId}`);
    await call("update_agent_configuration", { expected_updated_at: main.updatedAt, change: { section: "tools", add: ["research_candles"] } });
    expect((await definition(main.id)).tools).toContain("research_candles");
    const updated = await definition(main.id);
    await call("update_agent_configuration", { expected_updated_at: updated.updatedAt, change: { section: "tools", remove: ["update_agent_configuration"] } }, /Configuration tools must remain enabled/);
    expect((await definition(main.id)).tools).toContain("update_agent_configuration");
  });
  it("rejects stale edits and foreign-Agent targets", async () => {
    const row = await worker();
    await runtime.agents.subagents.updateIdentity("owner", agentId, row.id, "Newer name", row.description);
    await call("update_agent_configuration", { subagent_id: row.id, expected_updated_at: row.updatedAt, change: { section: "identity", name: "Stale name" } }, /Configuration changed; read current settings/);
    expect((await definition(row.id)).name).toBe("Newer name");
    const other = await runtime.agents.create("owner", "Other", randomUUID(), `0x${"b".repeat(40)}`);
    const foreign = await runtime.agents.subagents.create("owner", other.id, templateDraft("basic-analysis", { provider: "faux", modelId: "faux-1", thinkingLevel: "" }), randomUUID(), "basic-analysis");
    await call("update_agent_configuration", { subagent_id: foreign.id, expected_updated_at: foreign.updatedAt, change: { section: "enabled", enabled: true } }, /Subagent not found in this Agent/);
    expect((await runtime.agents.subagents.list("owner", other.id)).find((row) => row.id === foreign.id)?.enabled).toBe(false);
  });
  it("rejects risk-setting sections and extra Agent IDs", async () => {
    const before = await runtime.agents.risk.get(agentId);
    await call("update_agent_configuration", { change: { section: "risk", tradingEnabled: true } }, /section/);
    await call("update_agent_configuration", { agent_id: "another-agent", change: { section: "notifications", enabled: false } }, /agent_id/);
    expect(await runtime.agents.risk.get(agentId)).toEqual(before);
    expect((await runtime.notifications.getSettings(agentId)).enabled).toBe(true);
  });
  it("patches notification preferences without overwriting other choices", async () => {
    await runtime.notifications.saveSettings(agentId, { enabled: true, submissions: false, fills: true });
    await call("update_agent_configuration", { change: { section: "notifications", fills: false } });
    expect(await runtime.notifications.getSettings(agentId)).toEqual({ enabled: true, submissions: false, fills: false });
    expect(JSON.parse(lastToolOutput)).toMatchObject({ section: "notifications", before: { enabled: true, submissions: false, fills: true }, after: { enabled: true, submissions: false, fills: false } });
  });
  it("creates paused template copies, enables, pauses and archives without deleting", async () => {
    await call("create_worker", { template_id: "basic-review", name: "Chat reviewer" });
    const created = (await runtime.agents.subagents.list("owner", agentId)).find((row) => row.name === "Chat reviewer")!;
    expect(created).toMatchObject({ enabled: false, archived: false });
    expect(JSON.parse(lastToolOutput)).toMatchObject({ definition: { id: created.id, enabled: false, archived: false } });
    await call("update_agent_configuration", { subagent_id: created.id, expected_updated_at: created.updatedAt, change: { section: "enabled", enabled: true } });
    expect((await definition(created.id)).enabled).toBe(true);
    expect(JSON.parse(lastToolOutput)).toMatchObject({ section: "enabled", before: { enabled: false }, after: { enabled: true } });
    const enabled = await definition(created.id);
    await call("update_agent_configuration", { subagent_id: created.id, expected_updated_at: enabled.updatedAt, change: { section: "enabled", enabled: false } });
    const paused = await definition(created.id);
    expect(paused.enabled).toBe(false);
    await call("archive_worker", { subagent_id: created.id, expected_updated_at: paused.updatedAt });
    expect(await definition(created.id)).toMatchObject({ archived: true, enabled: false });
    expect(JSON.parse(lastToolOutput)).toMatchObject({ subagentId: created.id, before: { archived: false, enabled: false }, after: { archived: true, enabled: false } });
  });
  it("uses structured configuration values in nested Code Mode calls for all 6 tools", async () => {
    const main = await definition(`main:${agentId}`);
    await runtime.agents.subagents.updateTools("owner", agentId, main.id, [...main.tools, "codemode"]);
    await call("codemode", { code: `
      const catalog = await tools.list_worker_templates({});
      const header = catalog.templates.find(template => template.id === "basic-review");
      const detail = await tools.list_worker_templates({template_id: header.id});
      const agent = await tools.get_agent_configuration({});
      const created = await tools.create_worker({template_id: detail.template.id, name: "Nested reviewer"});
      const saved = await tools.get_agent_configuration({subagent_id: created.definition.id});
      const discovery = await tools.list_agent_tools({subagent_id: saved.definition.id, query: "send_notification"});
      const updated = await tools.update_agent_configuration({
        subagent_id: saved.definition.id, expected_updated_at: saved.definition.updatedAt,
        change: {section: "identity", name: "Nested renamed reviewer"}
      });
      const archived = await tools.archive_worker({subagent_id: updated.subagentId, expected_updated_at: updated.updatedAt});
      text({agentId: agent.agent.id, templateId: detail.template.id,
        instructions: typeof detail.template.instructions, paused: !created.definition.enabled,
        tools: discovery.tools.map(tool => tool.name),
        before: updated.before.name, after: updated.after.name, archived: archived.after.archived});
    ` });
    expect(lastToolOutput).toContain(JSON.stringify({ agentId, templateId: "basic-review", instructions: "string", paused: true, tools: ["send_notification"], before: "Nested reviewer", after: "Nested renamed reviewer", archived: true }));
    const saved = (await runtime.agents.subagents.list("owner", agentId)).find((row) => row.name === "Nested renamed reviewer")!;
    expect(saved).toMatchObject({ enabled: false, archived: true });
    expect((await runtime.agents.subagents.detail("owner", agentId, saved.id)).runs).toEqual([]);
  });
  it("does not allow workers to select configuration-management tools", async () => {
    const row = await worker();
    await expect(runtime.agents.subagents.updateTools("owner", agentId, row.id, ["update_agent_configuration"])).rejects.toThrow("Select supported tools");
  });
});
