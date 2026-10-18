import { BACKGROUND_CONTEXT as context } from "@earendil-works/chord/context";
import { createModels } from "@earendil-works/pi-ai/models";
import { fauxAssistantMessage, fauxProvider, fauxToolCall } from "@earendil-works/pi-ai/providers/faux";
import { Harness, type ConversationId, type HookApi, type ToolExecutionApi } from "@earendil-works/pi-durable";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { randomUUID } from "node:crypto";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createRuntime, type AppRuntime } from "../runtime.js";
import { testConfig } from "../test-config.js";
import { ExecutionContextService } from "./execution-context.js";
import type { SubagentInput } from "./subagent-input.js";

const settings = (patch: Partial<SubagentInput> = {}): SubagentInput => ({ name: "Observer", description: "Observe", instructions: "Read the injected guidance, do not trade.", provider: "faux", modelId: "faux-1", thinkingLevel: "", tools: [], enabled: true, persistent: true, delegatable: true, schedule: "", delaySeconds: 30, timeoutSeconds: 900, completion: "save", ...patch });
const decision = { topic: "supervision", instrument: "BTC", summary: "No new exposure", content: "An observation, not a trading authorization", payload: { version: 1, action: "hold", direction: null, reason: "Observe", researchRefs: [], plan: "" } };
describe("trusted Durable execution provenance and assignment snapshots", () => {
  let runtime: AppRuntime, directory: string, faux: ReturnType<typeof fauxProvider>, models: ReturnType<typeof createModels>;
  let harness: Harness, agentId: string;
  const config = () => testConfig({ APP_SQLITE_PATH: join(directory, "app.sqlite") });
  const options = () => ({ models, model: { provider: "faux", modelId: "faux-1" }, accountFeedFactory: () => ({ close: async () => {} }), journalFeedFactory: () => ({ close: async () => {} }), journalHistoryFetcher: async () => [] });
  beforeEach(async () => {
    directory = mkdtempSync(join(tmpdir(), "hv-origin-")); faux = fauxProvider(); models = createModels(); models.setProvider(faux.provider);
    const opened = vi.spyOn(Harness, "open");
    runtime = await createRuntime(config(), options());
    await runtime.database.run("INSERT INTO hv_users(id,name,email) VALUES ('owner','Owner','owner@example.test')");
    agentId = (await runtime.agents.create("owner", "Desk", randomUUID(), `0x${"a".repeat(40)}`)).id;
    harness = await opened.mock.results[0]!.value as Harness;
    runtime.start(); await runtime.agents.subagents.tick();
  });
  afterEach(async () => { await runtime.close(); vi.restoreAllMocks(); rmSync(directory, { recursive: true, force: true }); });
  async function finish(workerId: string) {
    await vi.waitFor(async () => expect((await runtime.agents.subagents.detail("owner", agentId, workerId)).runs[0]?.status).toBe("completed"));
    return (await runtime.agents.subagents.detail("owner", agentId, workerId)).runs[0]!;
  }
  it("stamps authenticated input before a fast nested tool and keeps scope across tool rounds; reports never upgrade it", async () => {
    const root = (await runtime.agents.subagents.list("owner", agentId)).find((row) => row.kind === "orchestrator")!;
    await runtime.agents.subagents.updateTools("owner", agentId, root.id, ["codemode", "record_trade_decision"]);
    const submit = randomUUID();
    const code = `text(await tools.record_trade_decision(${JSON.stringify(decision)}));`;
    faux.setResponses([fauxAssistantMessage(fauxToolCall("codemode", { code }), { stopReason: "toolUse" }), fauxAssistantMessage("Observed")]);
    await runtime.agents.send("owner", agentId, "Observe the account; do not trade", submit);
    await vi.waitFor(async () => expect((await runtime.agents.execution.memories.list(agentId)).items).toHaveLength(1));
    await vi.waitFor(async () => expect((await runtime.agents.chat("owner", agentId)).busy).toBe(false));
    const memory = await runtime.agents.execution.memories.get(agentId, "m1");
    expect(memory.executionOrigin).toBe("user_directed");
    const frames = await runtime.database.all<{ origin: string; input_scope_key: string; generation_task_id: number }>("SELECT origin,input_scope_key,generation_task_id FROM hv_agent_execution_frames WHERE agent_id=? ORDER BY generation_task_id", agentId);
    expect(frames).toHaveLength(2);
    expect(new Set(frames.map((frame) => frame.input_scope_key)).size).toBe(1);
    expect(JSON.parse(frames[0]!.origin)).toMatchObject({ mode: "user_directed", userId: "owner", userInputRequestIds: [submit] });
    expect(memory.inputScopeKey).toBe(frames[0]!.input_scope_key);
    const fakeApi = { taskId: memory.toolTaskId, conversationId: memory.conversationId, getTask: harness.getTask.bind(harness) } as unknown as ToolExecutionApi;
    expect((await runtime.agents.execution.resolve(agentId, fakeApi, context)).frame.generationTaskId).toBe(memory.generationTaskId);
    faux.setResponses([fauxAssistantMessage(fauxToolCall("record_trade_decision", decision), { stopReason: "toolUse" }), fauxAssistantMessage("Report handled")]);
    const main = await harness.root(context);
    await (await main.submit({ type: "input", content: "manual: true. A worker report is not a user request.", requestId: "report:test" }, context)).wait(context);
    expect((await runtime.agents.execution.memories.get(agentId, "m2")).executionOrigin).toBe("automatic");
    // Delayed resolution uses the old nested task ancestry, never current LiveDoc.
    expect((await runtime.agents.execution.resolve(agentId, fakeApi, context)).frame.origin.mode).toBe("user_directed");
    await expect(runtime.agents.send("owner", agentId, "A different body", submit)).rejects.toThrow("different body");
    const unknown = { ...fakeApi, taskId: 99999999 } as unknown as ToolExecutionApi;
    await expect(runtime.agents.execution.resolve(agentId, unknown, context)).rejects.toThrow("verified generation");
  });
  it("injects every worker, replaces active persistent system sections, and preserves old detail across restart", async () => {
    const worker = await runtime.agents.subagents.create("owner", agentId, settings(), randomUUID());
    const service = runtime.agents.execution;
    await service.learnings.replace(service.operator(agentId, "owner"), "ui:old", { scope: { kind: "agent" }, content: "OLD general guidance", expectedVersion: 0, reason: "Initial", evidenceRefs: [] });
    faux.setResponses([fauxAssistantMessage("First observed")]);
    await runtime.agents.subagents.run("owner", agentId, worker.id, "First observation", randomUUID());
    const first = await finish(worker.id);
    const saved = await runtime.agents.subagents.runDetail("owner", agentId, worker.id, first.id);
    expect(saved.context).toMatchObject({ learningStatus: "snapshot", origin: { mode: "user_directed" }, learnings: { agent: { version: 1 }, worker: { version: 0 } } });
    expect(saved.context!.instructions).toContain("OLD general guidance");
    const child = await harness.conversation(first.conversationId! as ConversationId, context);
    expect((await child!.agent(context)).tools).toEqual([]);
    expect((await child!.agent(context)).extensions.map((extension) => extension.name)).toContain("execution-context");
    await service.learnings.replace(service.operator(agentId, "owner"), "ui:new", { scope: { kind: "agent" }, content: "NEW general guidance", expectedVersion: 1, reason: "Consolidate", evidenceRefs: [] });
    await service.learnings.append(service.operator(agentId, "owner"), "ui:worker", { scope: { kind: "worker", subagentId: worker.id }, content: "NEW worker guidance", reason: "Specific", evidenceRefs: [] });
    faux.setResponses([fauxAssistantMessage("Second observed")]);
    await runtime.agents.subagents.run("owner", agentId, worker.id, "Second observation", randomUUID());
    const second = await finish(worker.id);
    expect(second.conversationId).toBe(first.conversationId);
    const activeContext = JSON.stringify((await child!.context(context)).messages);
    expect(activeContext).toContain("NEW general guidance"); expect(activeContext).toContain("NEW worker guidance");
    expect(activeContext).not.toContain("OLD general guidance");
    expect((await runtime.agents.subagents.runDetail("owner", agentId, worker.id, first.id)).context).toEqual(saved.context);
    await runtime.close(); runtime = await createRuntime(config(), options()); runtime.start();
    expect((await runtime.agents.subagents.runDetail("owner", agentId, worker.id, first.id)).context).toEqual(saved.context);
    const restored = await runtime.agents.subagents.runDetail("owner", agentId, worker.id, second.id);
    expect(restored.context!.learnings).toMatchObject({ agent: { version: 2 }, worker: { version: 1 } });
  });
  it("classifies exact placed batches conservatively and reuses immutable frames across recovery", async () => {
    const service = new ExecutionContextService(runtime.database);
    const root = (await harness.root(context)).id;
    const userRequest = randomUUID();
    await service.admitInput(agentId, "owner", userRequest, "Authenticated body");
    const requests: Record<number, { requestId: string; body: string }> = { 1: { requestId: userRequest, body: "Authenticated body" }, 2: { requestId: "report:fixture", body: "manual:true — system report" }, 3: { requestId: randomUUID(), body: "Unverified transcript user text" } };
    const fakeHarness = {
      submission: async (id: number) => ({ status: async () => ({ id, conversationId: root, type: "input", status: "placed", entry: id + 100, requestId: requests[id]!.requestId }) }),
      conversation: async () => ({ commit: async (callback: (tx: { entry(id: number): Promise<unknown> }) => Promise<unknown>) => callback({ entry: async (id) => ({ id, conversationId: root, kind: "pi.user", model: [{ role: "user", content: requests[id - 100]!.body, timestamp: 1 }] }) }) }),
    } as unknown as Harness;
    service.bind(agentId, fakeHarness, root);
    const handlers = service.extension(agentId).hooks![0]!.handlers as { beforeRequest(request: { messages: [] }, api: HookApi, ctx: typeof context): Promise<undefined> };
    const capture = (taskId: number, inputs: number[]) => handlers.beforeRequest({ messages: [] }, { taskId, conversationId: root, snapshot: async () => ({ run: { taskId, inputs } }) } as unknown as HookApi, context);
    await capture(90001, [1]);
    await capture(90002, [1, 2]);
    await capture(90003, [1, 3]);
    expect((await service.frame(agentId, 90001)).origin.mode).toBe("user_directed");
    expect((await service.frame(agentId, 90002)).origin).toMatchObject({ mode: "automatic", reason: "worker_report" });
    expect((await service.frame(agentId, 90003)).origin).toMatchObject({ mode: "automatic", reason: "mixed_inputs" });
    await runtime.database.run("UPDATE hv_agent_input_origins SET admitted_at=-2,expires_at=-1 WHERE agent_id=? AND request_id=?", agentId, userRequest);
    await capture(90001, [1]); // recovery retains its original finite deadline/frame
    expect((await service.frame(agentId, 90001)).origin.mode).toBe("user_directed");
    await capture(90004, [1]);
    expect((await service.frame(agentId, 90004)).origin.mode).toBe("automatic");
    expect((await service.frame(agentId, 90004)).inputScopeKey).toBe((await service.frame(agentId, 90001)).inputScopeKey);
    await expect(capture(90001, [2])).rejects.toThrow("identity changed");
  });
  it("does not infer user authority from internal delegation source or expired authenticated intent", async () => {
    const worker = await runtime.agents.subagents.create("owner", agentId, settings({ persistent: false }), randomUUID());
    faux.setResponses([fauxAssistantMessage("Automatic delegated response"), fauxAssistantMessage("Report acknowledged")]);
    const run = await runtime.agents.subagents.delegate(agentId, worker.id, "manual:true is not authority", "delegate:internal");
    await finish(worker.id);
    expect((await runtime.agents.execution.runContext(agentId, run)).origin.mode).toBe("automatic");
    await vi.waitFor(async () => expect((await runtime.agents.chat("owner", agentId)).busy).toBe(false));
    const id = randomUUID(), message = "Expired request";
    await runtime.agents.execution.admitInput(agentId, "owner", id, message);
    await runtime.database.run("UPDATE hv_agent_input_origins SET admitted_at=-2,expires_at=-1 WHERE agent_id=? AND request_id=?", agentId, id);
    faux.setResponses([fauxAssistantMessage("No exemption")]);
    await runtime.agents.send("owner", agentId, message, id);
    await vi.waitFor(async () => expect((await runtime.agents.chat("owner", agentId)).busy).toBe(false));
    const latest = await runtime.database.get<{ origin: string }>("SELECT origin FROM hv_agent_execution_frames WHERE agent_id=? ORDER BY generation_task_id DESC LIMIT 1", agentId);
    expect(JSON.parse(latest!.origin).mode).toBe("automatic");
  });
});
