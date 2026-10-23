import { createModels } from "@earendil-works/pi-ai/models";
import { fauxAssistantMessage, fauxProvider } from "@earendil-works/pi-ai/providers/faux";
import { type Harness, type Conversation } from "@earendil-works/pi-durable";
import { eq } from "drizzle-orm";
import { randomUUID } from "node:crypto";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { hvUsers } from "../db/auth-schema.js";
import { hvAgents } from "../db/agent-schema.js";
import { hvSubagentRuns } from "../db/subagent-schema.js";
import { createRuntime, type AppRuntime } from "../runtime.js";
import { testConfig } from "../test-config.js";
import { ParkedRunRecovery, SubagentService } from "./subagent-service.js";
import type { SubagentExtension } from "./subagent-extension.js";
import { BACKGROUND_CONTEXT as context } from "@earendil-works/chord/context";

let runtime: AppRuntime, directory: string, faux: ReturnType<typeof fauxProvider>, harness: Harness, main: Conversation, execution: SubagentExtension;
let bindScheduling: string[];
beforeEach(async () => {
  const bind = SubagentService.prototype.bind;
  bindScheduling = [];
  vi.spyOn(SubagentService.prototype, "bind").mockImplementation(async function (this: SubagentService, ...args) { harness = args[1]; main = args[2]; execution = args[4]; await bind.apply(this, args); bindScheduling.push((await harness.inspect(context)).scheduling); });
  directory = mkdtempSync(join(tmpdir(), "hv-parked-"));
  faux = fauxProvider();
  runtime = await openRuntime();
  await runtime.db.insert(hvUsers).values({ id: "owner", name: "Owner", email: "owner@example.test" }).run();
  runtime.start(); await runtime.agents.subagents.tick();
});
afterEach(async () => { vi.restoreAllMocks(); await runtime.close(AbortSignal.abort()); rmSync(directory, { recursive: true, force: true }); });

async function openRuntime() {
  const models = createModels(); models.setProvider(faux.provider);
  return createRuntime(testConfig({ APP_SQLITE_PATH: join(directory, "app.sqlite") }), {
    models, model: { provider: "faux", modelId: "faux-1" }, accountFeedFactory: () => ({ close: async () => {} }),
    journalFeedFactory: () => ({ close: async () => {} }), journalHistoryFetcher: async () => [],
  });
}
async function setup(persistent = false) {
  const agent = await runtime.agents.create("owner", "Desk", randomUUID(), `0x${randomUUID().replaceAll("-", "")}12345678`);
  await runtime.agents.setEnabled("owner", agent.id, true);
  const worker = await runtime.agents.subagents.create("owner", agent.id, {
    name: "Worker", description: "Research", instructions: "Research evidence", provider: "faux", modelId: "faux-1", thinkingLevel: "", tools: [], enabled: true,
    delegatable: true, persistent, schedule: "", delaySeconds: 30, completion: "save", timeoutSeconds: 900,
  }, randomUUID());
  return { agent, worker };
}
function holdResponse() {
  faux.setResponses([async (_input, options) => {
    await new Promise((resolve) => { options?.signal?.addEventListener("abort", resolve, { once: true }); });
    return fauxAssistantMessage("Interrupted");
  }]);
}
// Inject only the public graph's parking diagnostic. Cancellation, scope traversal,
// task joins and database writes use the real runtime and disposable SQLite file.
function park(predicate: (node: Awaited<ReturnType<Harness["taskGraph"]>>["value"]["tasks"][string]) => boolean) {
  const original = harness.taskGraph.bind(harness);
  return vi.spyOn(harness, "taskGraph").mockImplementation(async function (ctx) {
    const attached = await original(ctx);
    return { ...attached, value: { tasks: Object.fromEntries(Object.entries(attached.value.tasks).map(([key, node]) => [key, predicate(node) ? { ...node, blocked: "restart_limit" as const } : node])) }, dispose: () => attached.dispose() };
  });
}

it.each(["runner", "descendant", "persistent"] as const)("stops a parked %s scope and retains the run identity", async (kind) => {
  const { agent, worker } = await setup(kind === "persistent");
  if (kind === "persistent") {
    faux.setResponses([fauxAssistantMessage("First assignment")]);
    const first = await runtime.agents.subagents.run("owner", agent.id, worker.id, "First", randomUUID());
    await vi.waitFor(async () => expect((await runtime.agents.subagents.internalRun(agent.id, first.id)).reportedAt).not.toBeNull());
  }
  holdResponse();
  const run = await runtime.agents.subagents.run("owner", agent.id, worker.id, "Do not replay", randomUUID());
  await vi.waitFor(async () => expect((await runtime.agents.subagents.internalRun(agent.id, run.id)).status).toBe("running"));
  const before = await runtime.agents.subagents.internalRun(agent.id, run.id);
  const graphSpy = park((node) => kind === "runner" ? node.id === before.taskId : node.conversationId === before.conversationId);
  await runtime.agents.subagents.tick();
  const after = await runtime.agents.subagents.internalRun(agent.id, run.id);
  expect(after).toMatchObject({ status: "failed", taskId: before.taskId, conversationId: before.conversationId, task: "Do not replay" });
  expect(after.result).toContain("Automatic recovery stopped after repeated interruptions");
  expect(after.result).toContain("Existing exchange orders were not cancelled");
  expect(after.reportedAt).not.toBeNull();
  graphSpy.mockRestore();
  await runtime.agents.subagents.tick();
  expect((await runtime.agents.subagents.detail("owner", agent.id, worker.id)).runs).toHaveLength(kind === "persistent" ? 2 : 1);
  expect((await runtime.agents.subagents.internalRun(agent.id, run.id)).result).toBe(after.result);
});

it("continues cancellation after an interrupted abort without reporting or replaying", async () => {
  const { agent, worker } = await setup(); holdResponse();
  const run = await runtime.agents.subagents.run("owner", agent.id, worker.id, "One assignment", randomUUID());
  await vi.waitFor(async () => expect((await runtime.agents.subagents.internalRun(agent.id, run.id)).status).toBe("running"));
  const taskId = (await runtime.agents.subagents.internalRun(agent.id, run.id)).taskId;
  const graphSpy = park((node) => node.id === taskId);
  const original = harness.abortTask.bind(harness);
  const abort = vi.spyOn(harness, "abortTask").mockRejectedValueOnce(new Error("Interrupted cancellation")).mockImplementation((...args) => original(...args));
  await runtime.agents.subagents.tick();
  expect((await runtime.agents.subagents.internalRun(agent.id, run.id)).status).toBe("failed");
  await expect(runtime.agents.subagents.run("owner", agent.id, worker.id, "Unsafe overlap", randomUUID())).rejects.toMatchObject({ status: 409 });
  graphSpy.mockRestore();
  await runtime.agents.subagents.tick();
  expect(abort.mock.calls.length).toBeGreaterThan(1);
  expect((await runtime.agents.subagents.internalRun(agent.id, run.id)).reportedAt).not.toBeNull();
  expect((await runtime.agents.subagents.internalRun(agent.id, run.id)).taskId).toBe(taskId);
});

it("preserves a completed result when its report task parks", async () => {
  const { agent, worker } = await setup();
  faux.setResponses([fauxAssistantMessage("Original completed evidence")]);
  const run = await runtime.agents.subagents.run("owner", agent.id, worker.id, "Assignment", randomUUID());
  await vi.waitFor(async () => expect((await runtime.agents.subagents.internalRun(agent.id, run.id)).reportedAt).not.toBeNull());
  const completed = await runtime.agents.subagents.internalRun(agent.id, run.id);
  const delivery = vi.spyOn(runtime.agents.subagents, "reportWhenReady").mockImplementation(async (_id, _deliver, signal) => {
    await new Promise((resolve) => signal.addEventListener("abort", resolve, { once: true }));
    return false;
  });
  await runtime.db.update(hvSubagentRuns).set({ config: { ...completed.config, completion: "wake" } }).where(eq(hvSubagentRuns.id, run.id)).run();
  const taskId = await main.commit(async (tx) => tx.createTask(execution.runner, { runId: run.id, reportOnly: true }, { ownership: { kind: "conversation" }, background: true }), context);
  await runtime.db.update(hvSubagentRuns).set({ taskId }).where(eq(hvSubagentRuns.id, run.id)).run();
  harness.resume();
  await vi.waitFor(() => expect(delivery).toHaveBeenCalledTimes(1));
  park((node) => node.id === taskId);
  vi.spyOn(runtime.agents.subagents, "reported").mockRejectedValueOnce(new Error("Interrupted after cleanup marker cleared"));
  await runtime.agents.subagents.tick();
  const after = await runtime.agents.subagents.internalRun(agent.id, run.id);
  expect(after.status).toBe("completed");
  expect(after.result).toBe("Original completed evidence");
  expect(after.reportedAt).toBeNull();
  const recovery = await main.commit(async (tx) => { const doc = await tx.doc(ParkedRunRecovery, main.id, run.id, null); return { ...doc }; }, context);
  expect(recovery).toMatchObject({ pending: false, done: true });
  await runtime.agents.subagents.tick();
  expect((await runtime.agents.subagents.internalRun(agent.id, run.id)).reportedAt).not.toBeNull();
  expect((await runtime.agents.subagents.internalRun(agent.id, run.id)).result).toBe(after.result);
  expect(delivery).toHaveBeenCalledTimes(1);
});

it("keeps disabled startup paused and recovers a durable cancellation after restart", async () => {
  const { agent, worker } = await setup(); holdResponse();
  const run = await runtime.agents.subagents.run("owner", agent.id, worker.id, "Interrupted", randomUUID());
  await vi.waitFor(async () => expect((await runtime.agents.subagents.internalRun(agent.id, run.id)).status).toBe("running"));
  const current = await runtime.agents.subagents.internalRun(agent.id, run.id);
  await main.commit(async (tx) => { const doc = await tx.doc(ParkedRunRecovery, main.id, run.id, null); doc.pending = true; doc.conversationId = current.conversationId; }, context);
  await runtime.agents.subagents.finish(run.id, "failed", "Automatic recovery stopped after repeated interruptions. Existing exchange orders were not cancelled.");
  // Simulate persisted disablement at an interruption boundary, not the normal
  // disable API, which deliberately finishes cancellation before it returns.
  await runtime.db.update(hvAgents).set({ enabled: false }).where(eq(hvAgents.id, agent.id)).run();
  await runtime.close(AbortSignal.abort());
  runtime = await openRuntime();
  await runtime.agents.chat("owner", agent.id);
  // AgentService can then resume cancellation through its disableWork path.
  // The subagent bind itself must leave scheduling paused.
  expect(bindScheduling.at(-1)).toBe("paused");
  expect((await runtime.agents.subagents.internalRun(agent.id, run.id)).reportedAt).toBeNull();
  runtime.start(); await runtime.agents.subagents.tick();
  expect((await runtime.agents.subagents.internalRun(agent.id, run.id)).reportedAt).toBeNull();
  await runtime.agents.setEnabled("owner", agent.id, true);
  await runtime.agents.subagents.tick();
  const recovered = await runtime.agents.subagents.internalRun(agent.id, run.id);
  expect(recovered).toMatchObject({ status: "failed", taskId: current.taskId, conversationId: current.conversationId });
  expect(recovered.reportedAt).not.toBeNull();
  expect((await runtime.agents.subagents.detail("owner", agent.id, worker.id)).runs).toHaveLength(1);
});

it.each(["error", "timeout"] as const)("isolates recovery %s without normal reporting and continues another Agent", async (failure) => {
  const first = await setup(); holdResponse();
  const run = await runtime.agents.subagents.run("owner", first.agent.id, first.worker.id, "Parked", randomUUID());
  await vi.waitFor(async () => expect((await runtime.agents.subagents.internalRun(first.agent.id, run.id)).status).toBe("running"));
  const firstHarness = harness;
  const taskId = (await runtime.agents.subagents.internalRun(first.agent.id, run.id)).taskId;
  park((node) => node.id === taskId);
  if (failure === "timeout") {
    const timeout = AbortSignal.timeout.bind(AbortSignal);
    vi.spyOn(AbortSignal, "timeout").mockImplementation(() => timeout(5));
    vi.spyOn(firstHarness, "abortTask").mockImplementation(async (_id, ctx) => new Promise((_resolve, reject) => {
      const signal = ctx.abortSignal;
      if (!signal) throw new Error("Cancellation must have a bounded abort signal");
      const aborted = () => reject(signal.reason);
      signal.addEventListener("abort", aborted, { once: true });
      if (signal.aborted) aborted();
    }));
  } else vi.spyOn(firstHarness, "abortTask").mockRejectedValue(new Error("Cancellation unavailable"));
  const second = await setup();
  faux.setResponses([fauxAssistantMessage("Other Agent completed")]);
  const otherRun = await runtime.agents.subagents.run("owner", second.agent.id, second.worker.id, "Independent", randomUUID());
  await runtime.agents.subagents.tick();
  expect((await runtime.agents.subagents.internalRun(first.agent.id, run.id)).reportedAt).toBeNull();
  expect(runtime.agents.subagents.schedulerError).toContain("cleanup is pending");
  await vi.waitFor(async () => expect((await runtime.agents.subagents.internalRun(second.agent.id, otherRun.id)).status).toBe("completed"));
});
