import { BACKGROUND_CONTEXT as context } from "@earendil-works/chord/context";
import { createModels } from "@earendil-works/pi-ai/models";
import { fauxAssistantMessage, fauxProvider, fauxToolCall } from "@earendil-works/pi-ai/providers/faux";
import { Harness, type ConversationId, type ToolExecutionApi } from "@earendil-works/pi-durable";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { randomUUID } from "node:crypto";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createRuntime, type AppRuntime } from "../runtime.js";
import { testConfig } from "../test-config.js";
import type { SubagentInput } from "./subagent-input.js";
import { researchInput } from "../memory/test-fixture.js";

const settings = (patch: Partial<SubagentInput> = {}): SubagentInput => ({ name: "Observer", description: "Observe", instructions: "Read the injected guidance, do not trade.", provider: "faux", modelId: "faux-1", thinkingLevel: "", tools: [], enabled: true, persistent: true, delegatable: true, schedule: "", delaySeconds: 30, timeoutSeconds: 900, completion: "save", ...patch });
const decision = { topic: "supervision", instrument: "BTC", summary: "No new exposure", content: "An observation, not a trading authorization", payload: { version: 1, action: "hold", direction: null, reason: "Observe", researchRefs: [], plan: "" } };
describe("trusted tool actor resolution and assignment snapshots", () => {
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
  it("resolves trusted actors from the conversation, never from tool arguments, and never infers trading authority", async () => {
    const root = (await runtime.agents.subagents.list("owner", agentId)).find((row) => row.kind === "orchestrator")!;
    await runtime.agents.subagents.updateTools("owner", agentId, root.id, ["codemode", "record_trade_decision"]);
    const code = `text(await tools.record_trade_decision(${JSON.stringify(decision)}));`;
    faux.setResponses([fauxAssistantMessage(fauxToolCall("codemode", { code }), { stopReason: "toolUse" }), fauxAssistantMessage("Observed")]);
    await runtime.agents.send("owner", agentId, "Observe the account; do not trade", randomUUID());
    await vi.waitFor(async () => expect((await runtime.agents.execution.memories.list(agentId)).items).toHaveLength(1));
    await vi.waitFor(async () => expect((await runtime.agents.chat("owner", agentId)).busy).toBe(false));
    const memory = await runtime.agents.execution.memories.get(agentId, "m1");
    expect(memory.producerSubagentId).toBe(root.id);
    expect(memory.sourceRunId).toBeNull();
    // The root conversation resolves to the orchestrator actor.
    const fakeApi = { taskId: memory.toolTaskId, conversationId: memory.conversationId, memo: async (_name: string, candidate: unknown) => typeof candidate === "string" ? candidate : undefined, getTask: harness.getTask.bind(harness) } as unknown as ToolExecutionApi;
    expect((await runtime.agents.execution.resolve(agentId, fakeApi, context)).config.id).toBe(root.id);
    // An unknown conversation has no resolvable application identity.
    const unknown = { ...fakeApi, conversationId: 999999 } as unknown as ToolExecutionApi;
    await expect(runtime.agents.execution.resolve(agentId, unknown, context)).rejects.toThrow("assignment ancestry");
  });
  it("injects every worker, replaces active persistent system sections, and preserves old detail across restart", async () => {
    const worker = await runtime.agents.subagents.create("owner", agentId, settings(), randomUUID());
    const service = runtime.agents.execution;
    await service.learnings.replace(service.operator(agentId, "owner"), "ui:old", { scope: { kind: "agent" }, content: "OLD general guidance", expectedVersion: 0, reason: "Initial", evidenceRefs: [] });
    faux.setResponses([fauxAssistantMessage("First observed")]);
    await runtime.agents.subagents.run("owner", agentId, worker.id, "First observation", randomUUID());
    const first = await finish(worker.id);
    const saved = await runtime.agents.subagents.runDetail("owner", agentId, worker.id, first.id);
    expect(saved.context).toMatchObject({ learningStatus: "snapshot", learnings: { agent: { version: 1 }, worker: { version: 0 } } });
    expect(saved.context!.instructions).toContain("OLD general guidance");
    const child = await harness.conversation(first.conversationId! as ConversationId, context);
    expect((await child!.agent(context)).tools).toEqual([]);
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
  it("lets ordinary tool-authorized workers publish research and revise another worker's strategy learnings", async () => {
    const analyst = await runtime.agents.subagents.create("owner", agentId, settings({ tools: ["publish_research"], persistent: false }), randomUUID());
    const reviewer = await runtime.agents.subagents.create("owner", agentId, settings({ name: "Reviewer", tools: ["codemode", "append_learning"], persistent: false }), randomUUID());
    const boundary = Math.floor(Date.now() / 3600000) * 3600000;
    faux.setResponses([fauxAssistantMessage(fauxToolCall("publish_research", researchInput(boundary)), { stopReason: "toolUse" }), fauxAssistantMessage("Published")]);
    await runtime.agents.subagents.run("owner", agentId, analyst.id, "Publish evidence", randomUUID());
    await finish(analyst.id);
    expect((await runtime.agents.execution.memories.get(agentId, "m1")).producerSubagentId).toBe(analyst.id);
    const finding = { scope: { kind: "worker", subagentId: analyst.id }, content: "Analyze ETH next; use 150bps SL and layered entries when trading is assigned.", reason: "Review of evidence", evidenceRefs: ["m1"] };
    faux.setResponses([fauxAssistantMessage(fauxToolCall("codemode", { code: `text(await tools.append_learning(${JSON.stringify(finding)}));` }), { stopReason: "toolUse" }), fauxAssistantMessage("Updated guidance")]);
    await runtime.agents.subagents.run("owner", agentId, reviewer.id, "Review findings", randomUUID());
    await finish(reviewer.id);
    expect((await runtime.agents.execution.learnings.current(agentId, finding.scope)).content).toContain("Analyze ETH next");
    faux.setResponses([fauxAssistantMessage("New assignment")]);
    const next = await runtime.agents.subagents.run("owner", agentId, analyst.id, "Use the updated guidance", randomUUID());
    expect((await runtime.agents.execution.runContext(agentId, next)).instructions).toContain(finding.content);
    await finish(analyst.id);
  });
  it("resolves worker tool calls to their active run and its frozen configuration", async () => {
    const config = settings({ tools: ["record_trade_decision"], persistent: false });
    const worker = await runtime.agents.subagents.create("owner", agentId, config, randomUUID());
    const frozen = { ...config, id: worker.id, agentId, kind: "worker" as const, templateOrigin: null, nextRunAt: null, createdAt: 1, updatedAt: 1 };
    await runtime.database.run("INSERT INTO hv_subagent_runs(id,agent_id,subagent_id,request_id,source,task,config,status,conversation_id,created_at) VALUES ('resolve-run',? ,?,'resolve-request','manual','Observe',?,'running',555,1)", agentId, worker.id, JSON.stringify(frozen));
    runtime.agents.execution.bind(agentId, {
      conversation: async () => ({ commit: async (callback: (tx: unknown) => Promise<void>) => callback({ submissionByRequest: async (_conversation: number, request: string) => request === "assignment:resolve-run" ? { entry: 10 } : { entry: 30 } }) }),
    } as unknown as Harness, 1 as ConversationId);
    const memos = new Map<string, string>();
    const fakeApi = {
      taskId: 1, conversationId: 555,
      memo: async (name: string, candidate: unknown) => { if (typeof candidate === "string" && !memos.has(name)) memos.set(name, candidate); return memos.get(name); },
      getTask: async () => ({ kind: "pi.tool", conversationId: 555, input: { kind: "model", assistant: 20 } }),
    } as unknown as ToolExecutionApi;
    const resolved = await runtime.agents.execution.resolve(agentId, fakeApi, context);
    expect(resolved.actor).toMatchObject({ agentId, subagentId: worker.id, runId: "resolve-run" });
    expect(resolved.config.tools).toContain("record_trade_decision");
    // A tool permission that was never granted cannot be authorized through actor resolution.
    await expect(runtime.database.transaction((tx) => runtime.agents.execution.authorize(tx, resolved.actor, "publish_research"))).rejects.toThrow();
    await runtime.database.run("UPDATE hv_subagent_runs SET status='completed' WHERE id='resolve-run'");
    await runtime.database.run("INSERT INTO hv_subagent_runs(id,agent_id,subagent_id,request_id,source,task,config,status,conversation_id,created_at) VALUES ('new-run',?,?,'new-request','manual','New',?,'running',555,2)", agentId, worker.id, JSON.stringify(frozen));
    // Replay and even first-time delayed resolution of an older call must not
    // become an effect of the newer assignment in this persistent conversation.
    expect((await runtime.agents.execution.resolve(agentId, fakeApi, context)).actor.runId).toBe("resolve-run");
    memos.clear();
    expect((await runtime.agents.execution.resolve(agentId, fakeApi, context)).actor.runId).toBe("resolve-run");
    await expect(runtime.database.transaction((tx) => runtime.agents.execution.authorize(tx, resolved.actor, "record_trade_decision"))).rejects.toThrow("no longer active");
  });
});