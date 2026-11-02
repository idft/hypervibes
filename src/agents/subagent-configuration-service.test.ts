import type { Message } from "@earendil-works/pi-ai";
import { Type } from "typebox";
import { BACKGROUND_CONTEXT as context } from "@earendil-works/chord/context";
import { createModels } from "@earendil-works/pi-ai/models";
import { fauxAssistantMessage, fauxProvider, fauxToolCall } from "@earendil-works/pi-ai/providers/faux";
import { defineTool, Harness, type Conversation } from "@earendil-works/pi-durable";
import { eq } from "drizzle-orm";
import { randomUUID } from "node:crypto";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { hvUsers } from "../db/auth-schema.js";
import { documentationToolNames } from "../documentation/contracts.js";
import { hvSubagents, hvSubagentRuns } from "../db/subagent-schema.js";
import type { MemoryActor } from "../memory/operations.js";
import { createRuntime, type AppRuntime } from "../runtime.js";
import { testConfig } from "../test-config.js";
import { configurationToolNames } from "./configuration-contracts.js";
import type { SubagentInput } from "./subagent-input.js";
import { DocumentationToolsUpgradeDoc, NotificationToolsUpgradeDoc } from "./subagent-service.js";
import { defaultWorkerTriggers } from "./worker-triggers.js";

const settings = (patch: Partial<SubagentInput> = {}): SubagentInput => ({ name: "Worker", description: "Evidence", instructions: "Keep original instructions.", provider: "faux", modelId: "faux-1", thinkingLevel: "", tools: ["list_memories"], enabled: false, delegatable: true, persistent: false, schedule: "", delaySeconds: 30, completion: "save", timeoutSeconds: 900, ...patch });

describe("orchestrator configuration service", () => {
  let directory: string, runtime: AppRuntime, faux: ReturnType<typeof fauxProvider>;
  let models: ReturnType<typeof createModels>;
  const spyHarness = () => vi.spyOn(Harness, "open");
  let opened: ReturnType<typeof spyHarness>;
  let invoke: (actor: MemoryActor) => Promise<void>;
  let toolFailure: unknown;
  let invocations: number;
  const config = () => testConfig({ APP_SQLITE_PATH: join(directory, "app.sqlite") });
  const options = () => ({ models, model: { provider: "faux", modelId: "faux-1" }, accountFeedFactory: () => ({ close: async () => {} }), journalFeedFactory: () => ({ close: async () => {} }), journalHistoryFetcher: async () => [] });
  const createAgent = () => runtime.agents.create("owner", "Desk", randomUUID(), `0x${randomUUID().replaceAll("-", "")}12345678`);
  const createWorker = (agentId: string, patch: Partial<SubagentInput> = {}) => runtime.agents.subagents.create("owner", agentId, settings(patch), randomUUID());
  const definition = async (agentId: string, id = `main:${agentId}`) => (await runtime.agents.subagents.detail("owner", agentId, id)).definition;
  const root = async (index = 0): Promise<Conversation> => (await opened.mock.results[index]!.value as Harness).root(context);
  const actor = async (agentId: string): Promise<MemoryActor> => ({ agentId, userId: null, subagentId: `main:${agentId}`, runId: null, conversationId: (await root()).id, toolTaskId: 999 });
  beforeEach(async () => {
    toolFailure = undefined; invocations = 0;
    directory = mkdtempSync(join(tmpdir(), "hv-configuration-service-"));
    faux = fauxProvider({ models: [{ id: "faux-1", input: ["text", "image"] }, { id: "faux-2", input: ["text"] }] });
    models = createModels(); models.setProvider(faux.provider);
    opened = spyHarness();
    runtime = await createRuntime(config(), options());
    await runtime.db.insert(hvUsers).values({ id: "owner", name: "Owner", email: "owner@example.test" }).run();
    // A real Durable tool round calls only the service under test. This keeps
    // these tests independent of the configuration extension's input/output API.
    const extension = runtime.agents.subagents.extension.bind(runtime.agents.subagents);
    vi.spyOn(runtime.agents.subagents, "extension").mockImplementation((...args) => {
      const result = extension(...args);
      const replacement = defineTool({ name: "delegate_subagent", description: "Exercise configuration service", parameters: Type.Object({}), replay: "unsafe", execute: async (_input, api, callContext) => {
        const identity = await runtime.agents.execution.resolve(args[0], api, callContext);
        invocations++;
        try { await invoke(identity.actor); }
        catch (error) { toolFailure = error; throw error; }
        return { output: [{ type: "text", text: "Configuration saved" }] };
      } });
      return { ...result, extension: { ...result.extension, tools: [...result.extension.tools!.filter((tool) => tool.name !== replacement.name), replacement] } };
    });
    runtime.start(); await runtime.agents.subagents.tick();
  });
  afterEach(async () => { await runtime.close(); vi.restoreAllMocks(); rmSync(directory, { recursive: true, force: true }); });

  it("applies root edits at the next request without replaying the current generation", async () => {
    const agent = await createAgent(), main = await root();
    const requests: { model: string; messages: Message[] }[] = [];
    invoke = async (caller) => {
      await expect(runtime.agents.subagents.updateSection("owner", agent.id, `main:${agent.id}`, "instructions", { instructions: "UI must not edit busy roots." })).rejects.toMatchObject({ status: 409 });
      await runtime.agents.subagents.updateSection("owner", agent.id, `main:${agent.id}`, "instructions", { instructions: "Use the newly saved instructions." }, caller);
      await runtime.agents.subagents.updateSection("owner", agent.id, `main:${agent.id}`, "model", { provider: "faux", modelId: "faux-2", thinkingLevel: "" }, caller);
      await runtime.agents.subagents.updateTools("owner", agent.id, `main:${agent.id}`, ["delegate_subagent"], caller);
    };
    faux.setResponses([
      (request, _options, _state, model) => { requests.push({ model: model.id, messages: structuredClone(request.messages) }); return fauxAssistantMessage(fauxToolCall("delegate_subagent", {}), { stopReason: "toolUse" }); },
      (request, _options, _state, model) => { requests.push({ model: model.id, messages: structuredClone(request.messages) }); return fauxAssistantMessage("Finished"); },
    ]);
    await runtime.agents.send("owner", agent.id, "Change the configuration", randomUUID());
    await vi.waitFor(async () => expect((await runtime.agents.chat("owner", agent.id)).messages.some((message) => message.text === "Finished")).toBe(true));
    expect(invocations).toBe(1); expect(toolFailure).toBeUndefined();
    expect(requests.map((request) => request.model)).toEqual(["faux-1", "faux-2"]);
    expect(JSON.stringify(requests[0]!.messages)).not.toContain("Use the newly saved instructions.");
    expect(JSON.stringify(requests[1]!.messages)).toContain("Use the newly saved instructions.");
    expect(requests[1]!.messages.some((message) => message.role === "toolResult")).toBe(true);
    const saved = await definition(agent.id);
    expect(saved.tools).toEqual(["delegate_subagent", ...configurationToolNames]);
    expect((await main.agent(context)).tools.some((tool) => tool.name === "get_account")).toBe(false);
    expect(faux.state.callCount).toBe(2);
  });

  it("keeps management selections through UI section and full edits", async () => {
    const agent = await createAgent();
    const saved = await runtime.agents.subagents.updateTools("owner", agent.id, `main:${agent.id}`, ["list_memories"]);
    expect(saved.tools).toEqual(["list_memories", ...configurationToolNames]);
    const updated = await runtime.agents.subagents.update("owner", agent.id, saved.id, { ...saved, tools: [] });
    expect(updated.tools).toEqual(configurationToolNames);
    expect(updated.tools).not.toContain("send_notification");
  });

  it("checks root identity, ownership, enablement and saved capability", async () => {
    const agent = await createAgent(), worker = await createWorker(agent.id), caller = await actor(agent.id);
    const update = (value: MemoryActor) => runtime.agents.subagents.updateIdentity("owner", agent.id, worker.id, "Changed", "Description", value);
    for (const invalid of [{ ...caller, agentId: "another-agent" }, { ...caller, subagentId: worker.id, runId: "worker-run" }, { ...caller, subagentId: null }, { ...caller, conversationId: 99999 }, { ...caller, toolTaskId: null }, { ...caller, userId: "another-user" }]) await expect(update(invalid)).rejects.toMatchObject({ status: expect.any(Number) });
    await runtime.db.update(hvSubagents).set({ tools: [] }).where(eq(hvSubagents.id, `main:${agent.id}`)).run();
    await expect(update(caller)).rejects.toMatchObject({ status: 403 });
    await runtime.db.update(hvSubagents).set({ tools: [...configurationToolNames] }).where(eq(hvSubagents.id, `main:${agent.id}`)).run();
    await runtime.agents.setEnabled("owner", agent.id, false);
    await expect(update(caller)).rejects.toMatchObject({ status: 409 });
    expect((await definition(agent.id, worker.id)).name).toBe("Worker");
  });

  it("rejects cross-Agent targets and root assignment, schedule, inbox and risk changes", async () => {
    const agent = await createAgent(), caller = await actor(agent.id), other = await createAgent(), worker = await createWorker(other.id);
    await expect(runtime.agents.subagents.updateIdentity("owner", agent.id, worker.id, "Outside", "", caller)).rejects.toMatchObject({ status: 404 });
    await expect(runtime.agents.subagents.archive("owner", agent.id, worker.id, caller)).rejects.toMatchObject({ status: 404 });
    const mainId = `main:${agent.id}`;
    await expect(runtime.agents.subagents.updateAssignments("owner", agent.id, mainId, { persistent: true, delegatable: true, timeoutSeconds: 900 }, caller)).rejects.toMatchObject({ status: 409 });
    await expect(runtime.agents.subagents.updateSection("owner", agent.id, mainId, "schedule", { schedule: "1h", delaySeconds: 30, completion: "save" }, caller)).rejects.toMatchObject({ status: 409 });
    await expect(runtime.agents.subagents.updateSection("owner", agent.id, mainId, "enabled", { enabled: false }, caller)).rejects.toMatchObject({ status: 409 });
    await expect(runtime.agents.subagents.updateSection("owner", agent.id, mainId, "triggers", { triggers: { ...defaultWorkerTriggers, inboxEnabled: true } }, caller)).rejects.toThrow("Only workers");
    await expect(runtime.agents.subagents.updateSection("owner", agent.id, mainId, "risk", { tradingEnabled: true }, caller)).rejects.toThrow("Unknown settings section");
  });

  it("authorizes create and archive separately and never starts newly created workers", async () => {
    const agent = await createAgent(), caller = await actor(agent.id), mainId = `main:${agent.id}`;
    await runtime.db.update(hvSubagents).set({ tools: ["update_agent_configuration"] }).where(eq(hvSubagents.id, mainId)).run();
    await expect(runtime.agents.subagents.create("owner", agent.id, settings(), randomUUID(), "", caller)).rejects.toMatchObject({ status: 403 });
    await runtime.db.update(hvSubagents).set({ tools: ["create_worker"] }).where(eq(hvSubagents.id, mainId)).run();
    const worker = await runtime.agents.subagents.create("owner", agent.id, settings(), randomUUID(), "", caller);
    expect(worker.enabled).toBe(false);
    expect((await runtime.agents.subagents.detail("owner", agent.id, worker.id)).runs).toEqual([]);
    await expect(runtime.agents.subagents.archive("owner", agent.id, worker.id, caller, worker.updatedAt)).rejects.toMatchObject({ status: 403 });
    await runtime.db.update(hvSubagents).set({ tools: ["archive_worker"] }).where(eq(hvSubagents.id, mainId)).run();
    await runtime.agents.subagents.archive("owner", agent.id, worker.id, caller, worker.updatedAt);
    expect((await definition(agent.id, worker.id)).archived).toBe(true);
    expect(faux.state.callCount).toBe(0);
  });

  it("checks expected timestamps inside serialization, including archive", async () => {
    const agent = await createAgent(), worker = await createWorker(agent.id), caller = await actor(agent.id);
    const results = await Promise.allSettled([
      runtime.agents.subagents.updateIdentity("owner", agent.id, worker.id, "Winner", worker.description, caller, worker.updatedAt),
      runtime.agents.subagents.updateTools("owner", agent.id, worker.id, [], caller, worker.updatedAt),
    ]);
    expect(results.map((result) => result.status)).toEqual(["fulfilled", "rejected"]);
    expect(results[1]).toMatchObject({ reason: { status: 409 } });
    await expect(runtime.agents.subagents.archive("owner", agent.id, worker.id, caller, worker.updatedAt)).rejects.toMatchObject({ status: 409 });
    const saved = await definition(agent.id, worker.id);
    await runtime.agents.subagents.archive("owner", agent.id, worker.id, caller, saved.updatedAt);
    expect((await definition(agent.id, worker.id)).archived).toBe(true);
  });

  it("changes future worker configuration but preserves an active frozen assignment", async () => {
    const agent = await createAgent(), worker = await createWorker(agent.id, { persistent: true }), caller = await actor(agent.id);
    const runId = randomUUID();
    // A queued admission needs no provider or exchange call. The scheduler is
    // stopped so the saved snapshot stays available for deterministic checks.
    await runtime.agents.subagents.drain();
    // Use a fresh runtime scheduler only after all fixture writes are complete.
    await runtime.close(); runtime = await createRuntime(config(), options());
    await runtime.db.insert(hvSubagentRuns).values({ id: runId, agentId: agent.id, subagentId: worker.id, requestId: randomUUID(), source: "manual", task: "Original", config: worker, status: "running", createdAt: Date.now() }).run();
    await expect(runtime.agents.subagents.updateTools("owner", agent.id, worker.id, [])).rejects.toMatchObject({ status: 409 });
    await runtime.agents.subagents.updateTools("owner", agent.id, worker.id, ["get_account"], caller);
    await runtime.agents.subagents.updateSection("owner", agent.id, worker.id, "model", { provider: "faux", modelId: "faux-2", thinkingLevel: "" }, caller);
    await runtime.agents.subagents.updateSection("owner", agent.id, worker.id, "schedule", { schedule: "1h", delaySeconds: 15, completion: "notify" }, caller);
    await runtime.agents.subagents.updateSection("owner", agent.id, worker.id, "triggers", { triggers: { ...defaultWorkerTriggers, inboxEnabled: true, fills: true } }, caller);
    await runtime.agents.subagents.updateAssignments("owner", agent.id, worker.id, { persistent: true, delegatable: false, timeoutSeconds: 300 }, caller);
    await expect(runtime.agents.subagents.updateAssignments("owner", agent.id, worker.id, { persistent: false, delegatable: false, timeoutSeconds: 300 }, caller)).rejects.toMatchObject({ status: 409 });
    await expect(runtime.agents.subagents.archive("owner", agent.id, worker.id, caller)).rejects.toMatchObject({ status: 409 });
    expect((await runtime.agents.subagents.internalRun(agent.id, runId)).config).toEqual(worker);
    const saved = await definition(agent.id, worker.id);
    expect(saved).toMatchObject({ tools: ["get_account"], modelId: "faux-2", schedule: "1h", persistent: true, timeoutSeconds: 300, triggers: { inboxEnabled: true, fills: true } });
    await runtime.db.update(hvSubagentRuns).set({ status: "cancelled", reportedAt: Date.now() }).where(eq(hvSubagentRuns.id, runId)).run();
  });

  it("uses frozen source delegation permission and current recipient approval", async () => {
    const agent = await createAgent(), rootActor = await actor(agent.id);
    const source = await createWorker(agent.id, { enabled: true, tools: ["delegate_subagent"] });
    const recipient = await createWorker(agent.id, { enabled: true, triggers: { ...defaultWorkerTriggers, inboxEnabled: true, workerDelegates: [source.id] } });
    let receipt: { status: string } | undefined;
    invoke = async (caller) => {
      expect(caller.subagentId).toBe(source.id); expect(caller.runId).not.toBeNull();
      await runtime.agents.subagents.updateTools("owner", agent.id, source.id, [], rootActor);
      await runtime.agents.subagents.updateSection("owner", agent.id, source.id, "triggers", { triggers: { ...defaultWorkerTriggers, inboxEnabled: true } }, rootActor);
      await runtime.agents.subagents.updateSection("owner", agent.id, source.id, "enabled", { enabled: false }, rootActor);
      receipt = await runtime.agents.subagents.delegate(agent.id, recipient.id, "Frozen assignment delegation", randomUUID(), undefined, "evaluation", caller);
      await expect(runtime.agents.subagents.delegate(agent.id, recipient.id, "Wrong conversation", randomUUID(), undefined, "evaluation", { ...caller, conversationId: 99999 })).rejects.toMatchObject({ status: 409 });
      await expect(runtime.agents.subagents.delegate(agent.id, recipient.id, "Wrong run", randomUUID(), undefined, "evaluation", { ...caller, runId: randomUUID() })).rejects.toMatchObject({ status: 409 });
      await expect(runtime.agents.subagents.delegate(agent.id, recipient.id, "Wrong source", randomUUID(), undefined, "evaluation", { ...caller, subagentId: recipient.id })).rejects.toMatchObject({ status: 403 });
      await runtime.agents.subagents.updateSection("owner", agent.id, recipient.id, "triggers", { triggers: { ...defaultWorkerTriggers, inboxEnabled: true } }, rootActor);
      await expect(runtime.agents.subagents.delegate(agent.id, recipient.id, "Approval removed", randomUUID(), undefined, "evaluation", caller)).rejects.toMatchObject({ status: 403 });
    };
    faux.setResponses([fauxAssistantMessage(fauxToolCall("delegate_subagent", {}), { stopReason: "toolUse" }), fauxAssistantMessage("Delegation finished")]);
    const run = await runtime.agents.subagents.run("owner", agent.id, source.id, "Delegate after a configuration edit", randomUUID());
    await vi.waitFor(async () => expect((await runtime.agents.subagents.internalRun(agent.id, run.id)).status).toBe("completed"));
    expect(invocations).toBe(1); expect(toolFailure).toBeUndefined();
    expect(receipt?.status).toBe("pending");
    expect((await runtime.agents.subagents.internalRun(agent.id, run.id)).config).toMatchObject({ tools: ["delegate_subagent"], enabled: true, triggers: { inboxEnabled: false } });
    expect(await definition(agent.id, source.id)).toMatchObject({ tools: [], enabled: false, triggers: { inboxEnabled: true } });
  });

  it("preserves image and model checks for root actors", async () => {
    const agent = await createAgent(), caller = await actor(agent.id), main = await root();
    // This check runs during a real tool round, after the image is in context.
    invoke = async (identity) => {
      await expect(runtime.agents.subagents.updateSection("owner", agent.id, `main:${agent.id}`, "model", { provider: "faux", modelId: "faux-2", thinkingLevel: "" }, identity)).rejects.toThrow("contains images");
    };
    faux.setResponses([fauxAssistantMessage(fauxToolCall("delegate_subagent", {}), { stopReason: "toolUse" }), fauxAssistantMessage("Image retained")]);
    await main.submit({ type: "input", content: [{ type: "image", mimeType: "image/png", data: "aGVsbG8=" }], requestId: randomUUID() }, context);
    await vi.waitFor(async () => expect((await runtime.agents.chat("owner", agent.id)).messages.some((message) => message.text === "Image retained")).toBe(true));
    expect(invocations).toBe(1); expect(toolFailure).toBeUndefined();
    await expect(runtime.agents.subagents.updateSection("owner", agent.id, `main:${agent.id}`, "model", { provider: "faux", modelId: "missing", thinkingLevel: "" }, caller)).rejects.toThrow("unavailable");
    await expect(runtime.agents.subagents.updateSection("owner", agent.id, `main:${agent.id}`, "model", { provider: "faux", modelId: "faux-1", thinkingLevel: "high" }, caller)).rejects.toThrow("supported thinking");
    expect((await definition(agent.id)).modelId).toBe("faux-1");
  });

  it("upgrades notifications once only for existing roots and basic-review workers", async () => {
    const agent = await createAgent(), main = await root();
    const review = await createWorker(agent.id), ordinary = await createWorker(agent.id);
    await runtime.db.update(hvSubagents).set({ tools: ["list_memories"] }).where(eq(hvSubagents.id, `main:${agent.id}`)).run();
    await runtime.db.update(hvSubagents).set({ templateOrigin: { id: "basic-review", version: 1, hash: "original" } }).where(eq(hvSubagents.id, review.id)).run();
    await main.commit(async (tx) => { (await tx.doc(NotificationToolsUpgradeDoc, main.id)).applied = false; (await tx.doc(DocumentationToolsUpgradeDoc, main.id)).applied = false; }, context);
    await runtime.close(); runtime = await createRuntime(config(), options());
    expect((await definition(agent.id)).tools).toEqual(["list_memories", ...configurationToolNames, ...documentationToolNames, "send_notification"]);
    expect((await definition(agent.id, review.id)).tools).toEqual(["list_memories", "send_notification"]);
    expect((await definition(agent.id, ordinary.id)).tools).toEqual(["list_memories"]);
    await runtime.agents.subagents.updateTools("owner", agent.id, `main:${agent.id}`, ["list_memories"]);
    await runtime.agents.subagents.updateTools("owner", agent.id, review.id, ["list_memories"]);
    await runtime.close(); runtime = await createRuntime(config(), options());
    expect((await definition(agent.id)).tools).toEqual(["list_memories", ...configurationToolNames]);
    expect((await definition(agent.id, review.id)).tools).toEqual(["list_memories"]);
  });
});
