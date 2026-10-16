import { BACKGROUND_CONTEXT as context } from "@earendil-works/chord/context";
import { createModels } from "@earendil-works/pi-ai/models";
import { fauxAssistantMessage, fauxProvider, fauxToolCall } from "@earendil-works/pi-ai/providers/faux";
import { Harness } from "@earendil-works/pi-durable";
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

const settings = (patch: Partial<SubagentInput> = {}): SubagentInput => ({ name: "Research", description: "Focused evidence", instructions: "Research the assignment using only available evidence.", provider: "faux", modelId: "faux-1", thinkingLevel: "", tools: [], enabled: true, delegatable: true, persistent: false, schedule: "", delaySeconds: 30, completion: "save", timeoutSeconds: 900, ...patch });
describe("subagent definitions, execution and scheduling", () => {
  let directory: string, runtime: AppRuntime, faux: ReturnType<typeof fauxProvider>;
  let models: ReturnType<typeof createModels>;
  const config = () => testConfig({ APP_SQLITE_PATH: join(directory, "app.sqlite") });
  const options = () => ({ models, model: { provider: "faux", modelId: "faux-1" }, accountFeedFactory: () => ({ close: async () => {} }), journalHistoryFetcher: async () => [] });
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

  it("rejects cross-user and cross-Agent access, unknown tools, recursive delegation and malformed schedules", async () => {
    const agent = await createAgent(), other = await createAgent(), worker = await createWorker(agent.id);
    await expect(runtime.agents.subagents.detail("other", agent.id, worker.id)).rejects.toMatchObject({ status: 404 });
    await expect(detail(other.id, worker.id)).rejects.toMatchObject({ status: 404 });
    for (const input of [settings({ tools: ["bash"] }), settings({ tools: ["delegate_subagent"] }), settings({ schedule: "bad" }), settings({ timeoutSeconds: 0 }), { ...settings(), enabled: "true" }]) expect(() => subagentInput(input)).toThrow();
    await expect(runtime.agents.subagents.run("other", agent.id, worker.id, "Do work", randomUUID())).rejects.toMatchObject({ status: 404 });
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
    expect(await runtime.agents.subagents.delegatable(agent.id)).toEqual([]);
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
    expect(scheduled.config.instructions).toBe(worker.instructions);
    expect((await detail(agent.id, worker.id)).runs).toHaveLength(1);
    expect((await detail(agent.id, worker.id)).definition.nextRunAt).toBeGreaterThan(due);
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
    faux.setResponses([fauxAssistantMessage("Recovered intent")]);
    await runtime.agents.retry("owner", agent.id);
    await runtime.agents.subagents.tick();
    const run = await completed(agent.id, worker.id); expect(run.result).toBe("Recovered intent");
    expect((await detail(agent.id, worker.id)).runs).toHaveLength(1);
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
    await runtime.close(); runtime = await createRuntime(config(), options());
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
    await runtime.close(); runtime = await createRuntime(config(), options());
    expect(faux.state.callCount).toBe(1); runtime.start();
    await vi.waitFor(async () => expect((await detail(agent.id, worker.id)).runs[0]?.result).toBe("Recovered evidence"), { timeout: 10000 });
    expect((await detail(agent.id, worker.id)).runs.map((entry) => entry.id)).toEqual([run.id]);
  }, 15000);
});
