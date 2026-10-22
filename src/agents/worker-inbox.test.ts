import { BACKGROUND_CONTEXT as context } from "@earendil-works/chord/context";
import { createModels } from "@earendil-works/pi-ai/models";
import { fauxAssistantMessage, fauxProvider, fauxToolCall } from "@earendil-works/pi-ai/providers/faux";
import { Harness } from "@earendil-works/pi-durable";
import { SqliteStorage } from "@earendil-works/pi-durable/storage/sqlite";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { randomUUID } from "node:crypto";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createRuntime, type AppRuntime } from "../runtime.js";
import { testConfig } from "../test-config.js";
import { researchInput } from "../memory/test-fixture.js";
import { JournalService } from "../hyperliquid/journal-service.js";
import { fillEvent } from "../hyperliquid/journal-model.js";
import type { SubagentInput } from "./subagent-input.js";
import { defaultWorkerTriggers, type WorkerTriggers } from "./worker-triggers.js";
import { enqueueWorkerEvent } from "./worker-events.js";
import { RunReceipt } from "./subagent-extension.js";

const settings = (patch: Partial<SubagentInput> = {}): SubagentInput => ({ name: "Worker", description: "Test worker", instructions: "Complete the assignment; do not call exchange APIs.", provider: "faux", modelId: "faux-1", thinkingLevel: "", tools: [], enabled: true, delegatable: true, persistent: false, schedule: "", delaySeconds: 1, completion: "save", timeoutSeconds: 900, ...patch });
const inbox = (patch: Partial<WorkerTriggers> = {}): WorkerTriggers => ({ ...defaultWorkerTriggers, inboxEnabled: true, ...patch });

describe("event-driven worker inboxes", () => {
  let runtime: AppRuntime, directory: string, faux: ReturnType<typeof fauxProvider>, models: ReturnType<typeof createModels>;
  let agentId: string;
  let release: (() => void) | undefined;
  const address = `0x${"a".repeat(40)}`;
  const config = () => testConfig({ APP_SQLITE_PATH: join(directory, "app.sqlite") });
  const options = () => ({ models, model: { provider: "faux", modelId: "faux-1" }, accountFeedFactory: () => ({ close: async () => {} }), journalFeedFactory: () => ({ close: async () => {} }), journalHistoryFetcher: async () => [] });
  const createWorker = (patch: Partial<SubagentInput> = {}) => runtime.agents.subagents.create("owner", agentId, settings(patch), randomUUID());
  const requests = () => runtime.database.all<{ id: string; subagent_id: string; run_id: string | null; payload: string; discarded_reason: string | null }>("SELECT id,subagent_id,run_id,payload,discarded_reason FROM hv_worker_requests WHERE agent_id=? ORDER BY created_at,id", agentId);
  const runs = async (id: string) => (await runtime.agents.subagents.detail("owner", agentId, id)).runs;
  const waitCompleted = async (id: string, total = 1) => {
    await vi.waitFor(async () => {
      const values = await runs(id);
      expect(values).toHaveLength(total);
      expect(values.every((run) => run.status === "completed")).toBe(true);
    });
    return (await runs(id))[0]!;
  };
  const event = (sourceId: string, key: string, reason: "research" | "fill" = "research") => runtime.database.transaction((tx) => enqueueWorkerEvent(tx, agentId, key, { intent: reason === "fill" ? "account_event" : "evaluation", reason, task: `Inspect ${key}`, callerSubagentId: sourceId, callerRunId: null, researchRefs: reason === "research" ? ["m1"] : [] }));
  beforeEach(async () => {
    directory = mkdtempSync(join(tmpdir(), "hv-inbox-")); faux = fauxProvider(); models = createModels(); models.setProvider(faux.provider);
    runtime = await createRuntime(config(), options());
    await runtime.database.run("INSERT INTO hv_users(id,name,email) VALUES ('owner','Owner','owner@example.test')");
    agentId = (await runtime.agents.create("owner", "Desk", randomUUID(), address)).id;
    faux.setResponses(Array.from({ length: 40 }, () => fauxAssistantMessage("Inspected; no order necessary")));
    runtime.start(); await runtime.agents.subagents.tick();
  });
  afterEach(async () => { release?.(); release = undefined; await runtime.close(); vi.restoreAllMocks(); rmSync(directory, { recursive: true, force: true }); });

  it("discards queued work on agent disable and does not collect events while disabled", async () => {
    const trader = await createWorker({ triggers: inbox({ fills: true }) });
    const queued = await runtime.agents.subagents.delegate(agentId, trader.id, "Queued operator trade", randomUUID(), undefined, "operator_trade");
    expect(queued.status).toBe("pending");
    await runtime.agents.setEnabled("owner", agentId, false);
    expect((await requests())[0]?.discarded_reason).toContain("Agent disabled");
    await event("", "fill-while-disabled", "fill");
    expect(await requests()).toHaveLength(1);
    await expect(runtime.agents.subagents.delegate(agentId, trader.id, "Blocked", randomUUID(), undefined, "operator_trade")).rejects.toMatchObject({ status: 409 });
    await runtime.agents.subagents.tick();
    expect(await runs(trader.id)).toEqual([]);
    await runtime.agents.setEnabled("owner", agentId, true);
    await runtime.agents.subagents.tick();
    expect(await runs(trader.id)).toEqual([]);
    await event("", "new-fill-after-enable", "fill");
    await runtime.agents.subagents.tick();
    await waitCompleted(trader.id);
    expect(await requests()).toHaveLength(2);
  });

  it("allows multiple inbox subscribers and independent schedules while checking same-Agent sources", async () => {
    const trader = await createWorker({ triggers: inbox() });
    const second = await createWorker({ triggers: inbox(), schedule: "5m" });
    expect(second.triggers.inboxEnabled).toBe(true);
    expect(second.schedule).toBe("5m");
    expect(trader.schedule).toBe("");
    await expect(runtime.agents.subagents.updateTools("owner", agentId, trader.id, ["delegate_subagent"])).rejects.toThrow("cannot delegate recursively");
    await expect(runtime.agents.subagents.updateSection("owner", agentId, trader.id, "triggers", { triggers: inbox({ researchSources: [trader.id] }) })).rejects.toThrow("another");
    const other = await runtime.agents.create("owner", "Other", randomUUID(), `0x${"b".repeat(40)}`);
    const foreign = await runtime.agents.subagents.create("owner", other.id, settings(), randomUUID());
    await expect(runtime.agents.subagents.updateSection("owner", agentId, trader.id, "triggers", { triggers: inbox({ researchSources: [foreign.id] }) })).rejects.toThrow("not found");
    expect(await runs(trader.id)).toEqual([]);
  });

  it("keeps traders editable after a subscribed source is archived", async () => {
    const analyst = await createWorker();
    const trader = await createWorker({ triggers: inbox({ researchSources: [analyst.id], workerDelegates: [analyst.id] }) });
    await runtime.agents.subagents.archive("owner", agentId, analyst.id);
    await runtime.agents.subagents.updateSection("owner", agentId, trader.id, "enabled", { enabled: false });
    const renamed = await runtime.agents.subagents.updateIdentity("owner", agentId, trader.id, "Renamed trader", "Still editable");
    expect(renamed.enabled).toBe(false);
    const detail = await runtime.agents.subagents.detail("owner", agentId, trader.id);
    expect(detail.workers).toContainEqual({ id: analyst.id, name: analyst.name, archived: true, tools: analyst.tools });
    const cleared = await runtime.agents.subagents.updateSection("owner", agentId, trader.id, "triggers", { triggers: inbox() });
    expect(cleared.triggers.researchSources).toEqual([]);
    await expect(runtime.agents.subagents.updateSection("owner", agentId, trader.id, "triggers", { triggers: inbox({ researchSources: [analyst.id] }) })).rejects.toThrow("non-archived");
  });

  it("can pause an actively running trader without stopping its assignment", async () => {
    const trader = await createWorker({ triggers: inbox() });
    const gate = new Promise<void>((resolve) => { release = resolve; });
    faux.setResponses([async () => { await gate; return fauxAssistantMessage("Finished"); }]);
    await runtime.agents.subagents.run("owner", agentId, trader.id, "Inspect account", randomUUID());
    await vi.waitFor(() => expect(faux.state.callCount).toBe(1));
    const paused = await runtime.agents.subagents.updateSection("owner", agentId, trader.id, "enabled", { enabled: false });
    expect(paused.enabled).toBe(false);
    expect((await runs(trader.id))[0]!.status).toBe("running");
    release!();
    await waitCompleted(trader.id);
  });

  it("preserves subscription activation across unrelated edits and timestamps newly enabled triggers", async () => {
    const analyst = await createWorker();
    const trader = await createWorker({ triggers: inbox({ fills: true }) });
    const activation = await runtime.database.get<{ activated_at: number }>("SELECT activated_at FROM hv_worker_subscriptions WHERE subagent_id=? AND event_type='fill'", trader.id);
    const occurredAt = activation!.activated_at + 1;
    await vi.waitFor(() => expect(Date.now()).toBeGreaterThan(occurredAt + 1));
    await runtime.agents.subagents.updateIdentity("owner", agentId, trader.id, "Renamed", "");
    await runtime.agents.subagents.updateSection("owner", agentId, trader.id, "triggers", { triggers: inbox({ fills: true, orderChanges: true, researchSources: [analyst.id] }) });
    const emit = (key: string, reason: "fill" | "order_status", time: number) => runtime.database.transaction((tx) => enqueueWorkerEvent(tx, agentId, key, { intent: "account_event", reason, task: key, callerSubagentId: null, callerRunId: null, researchRefs: [] }, time));
    await emit("delayed-fill", "fill", occurredAt);
    await emit("historic-fill", "fill", activation!.activated_at - 1);
    await emit("historic-order", "order_status", occurredAt);
    await emit("current-order", "order_status", Date.now());
    expect((await requests()).map((row) => JSON.parse(row.payload).task).sort()).toEqual(["current-order", "delayed-fill"]);
    const unchanged = await runtime.database.get<{ activated_at: number }>("SELECT activated_at FROM hv_worker_subscriptions WHERE subagent_id=? AND event_type='fill'", trader.id);
    expect(unchanged).toEqual(activation);
    await runtime.agents.subagents.updateSection("owner", agentId, trader.id, "triggers", { triggers: inbox() });
    await runtime.agents.subagents.updateSection("owner", agentId, trader.id, "triggers", { triggers: inbox({ fills: true }) });
    await emit("before-resubscription", "fill", occurredAt);
    expect(await requests()).toHaveLength(2);
  });

  it("prioritizes account events and operator trades before limiting a large evaluation backlog", async () => {
    const analyst = await createWorker();
    const trader = await createWorker({ triggers: inbox({ researchSources: [analyst.id], fills: true }) });
    await runtime.database.transaction(async (tx) => {
      for (let i = 0; i < 256; i++) await enqueueWorkerEvent(tx, agentId, `backlog:${i}`, { intent: "evaluation", reason: "research", task: `Evaluate ${i}`, callerSubagentId: analyst.id, callerRunId: null, researchRefs: [] });
    });
    await runtime.agents.subagents.delegate(agentId, trader.id, "Explicit operator request", "priority:operator", undefined, "operator_trade");
    await event(analyst.id, "priority:fill", "fill");
    await runtime.agents.subagents.tick();
    expect((await waitCompleted(trader.id)).task).toContain("priority:fill");
    await runtime.agents.subagents.tick();
    expect((await waitCompleted(trader.id, 2)).task).toContain("Explicit operator request");
  });

  it("fans research, fill and order-status events out independently to every opted-in worker", async () => {
    const analyst = await createWorker();
    const first = await createWorker({ name: "First receiver", triggers: inbox({ researchSources: [analyst.id], fills: true, orderChanges: true }) });
    const second = await createWorker({ name: "Second receiver", triggers: inbox({ researchSources: [analyst.id], fills: true, orderChanges: true }) });
    const unsubscribed = await createWorker({ triggers: inbox() });
    await event(analyst.id, "research:fanout");
    await event(analyst.id, "research:fanout");
    await event(analyst.id, "fill:fanout", "fill");
    const journal = new JournalService(runtime.database, async () => []);
    const now = Date.now() + 1;
    const update = { order: { coin: "BTC", oid: 42, side: "B" as const, limitPx: "60000", sz: "0.01", timestamp: now, origSz: "0.01" }, status: "rejected" as const, statusTimestamp: now };
    await journal.ingestOrderUpdates(address, [update]);
    await journal.ingestOrderUpdates(address, [update]);
    const rows = await requests();
    expect(rows).toHaveLength(6);
    expect(rows.filter((row) => row.subagent_id === first.id)).toHaveLength(3);
    expect(rows.filter((row) => row.subagent_id === second.id)).toHaveLength(3);
    expect(rows.some((row) => row.subagent_id === unsubscribed.id)).toBe(false);
    await runtime.agents.subagents.tick();
    const firstRun = await waitCompleted(first.id), secondRun = await waitCompleted(second.id);
    for (const run of [firstRun, secondRun]) {
      expect(run.task).toContain("reason=fill");
      expect(run.task).toContain("reason=order_status");
    }
    expect(firstRun.id).not.toBe(secondRun.id);
    await runtime.agents.subagents.tick();
    await waitCompleted(first.id, 2); await waitCompleted(second.id, 2);
    expect(await runs(unsubscribed.id)).toHaveLength(0);
    await journal.close();
  });

  it("routes explicit chat intent to one selected trader without a research requirement", async () => {
    const trader = await createWorker({ triggers: inbox() });
    const otherTrader = await createWorker({ triggers: inbox() });
    faux.setResponses([
      fauxAssistantMessage(fauxToolCall("delegate_subagent", { subagent_id: trader.id, intent: "operator_trade", task: "Open a BTC long using the configured size and TP/SL policy." }), { stopReason: "toolUse" }),
      ...Array.from({ length: 12 }, () => fauxAssistantMessage("Delegated or inspected")),
    ]);
    await runtime.agents.send("owner", agentId, "Open a BTC long using my configured size and protection", randomUUID());
    await vi.waitFor(async () => expect(await requests()).toHaveLength(1));
    await runtime.agents.subagents.tick();
    const run = await waitCompleted(trader.id);
    expect(run.task).toContain("[Host assignment intent: operator_trade]");
    expect(run.task).toContain("Missing or contrary research alone is not a reason to refuse");
    expect(run.task).toContain("Open a BTC long");
    expect((await runtime.agents.execution.memories.list(agentId)).items).toHaveLength(0);
    expect((await requests())[0]!.run_id).toBe(run.id);
    expect(await runs(otherTrader.id)).toHaveLength(0);
    expect(await runtime.agents.subagents.assignmentStatus(agentId, (await requests())[0]!.id)).toMatchObject({ run: { id: run.id } });
    await expect(runtime.agents.subagents.stop("owner", agentId, (await requests())[0]!.id)).rejects.toThrow("already dispatched");
  });

  it("retains busy-worker requests, coalesces fills, and prioritizes account management", async () => {
    const analyst = await createWorker();
    const trader = await createWorker({ triggers: inbox({ researchSources: [analyst.id], fills: true }) });
    const gate = new Promise<void>((resolve) => { release = resolve; });
    faux.setResponses([async () => { await gate; return fauxAssistantMessage("First finished"); }, ...Array.from({ length: 12 }, () => fauxAssistantMessage("Managed"))]);
    await runtime.agents.subagents.run("owner", agentId, trader.id, "Initial account inspection", randomUUID());
    await vi.waitFor(() => expect(faux.state.callCount).toBe(1));
    await event(analyst.id, "research:1");
    await event(analyst.id, "fill:1", "fill");
    await event(analyst.id, "fill:2", "fill");
    await event(analyst.id, "fill:2", "fill");
    await runtime.agents.subagents.tick();
    expect(await requests()).toHaveLength(3);
    expect((await requests()).every((request) => request.run_id === null)).toBe(true);
    const history = await runtime.agents.subagents.history("owner", agentId, trader.id);
    expect(history.inbox?.map((request) => request.id).sort()).toEqual((await requests()).map((request) => request.id).sort());
    release!();
    await waitCompleted(trader.id);
    await runtime.agents.subagents.tick();
    const second = await waitCompleted(trader.id, 2);
    expect(second.task).toContain("account_event");
    expect(second.task).toContain("fill:1"); expect(second.task).toContain("fill:2");
    expect(second.task).not.toContain("Inspect research:1");
    await runtime.agents.subagents.tick();
    const third = await waitCompleted(trader.id, 3);
    expect(third.task).toContain("evaluation");
    expect(third.task).toContain("Inspect research:1");
    expect(new Set((await requests()).map((request) => request.run_id)).size).toBe(2);
  });

  it("recovers a persisted request across restart without executing it twice", async () => {
    const analyst = await createWorker();
    const trader = await createWorker({ triggers: inbox({ researchSources: [analyst.id] }) });
    await event(analyst.id, "research:restart");
    await runtime.close();
    runtime = await createRuntime(config(), options());
    runtime.start(); await runtime.agents.subagents.tick();
    await waitCompleted(trader.id);
    await event(analyst.id, "research:restart");
    await runtime.agents.subagents.tick();
    expect(await requests()).toHaveLength(1);
    expect(await runs(trader.id)).toHaveLength(1);
  });

  it.each(["before", "after"] as const)("recovers interrupted dispatch %s the Pi task commit without losing provenance or duplicating execution", async (phase) => {
    const analyst = await createWorker();
    const trader = await createWorker({ triggers: inbox({ researchSources: [analyst.id] }) });
    await event(analyst.id, "research:commit-gap");
    const commit = SqliteStorage.prototype.commit;
    const interrupted = vi.spyOn(SqliteStorage.prototype, "commit").mockImplementationOnce(async function (this: SqliteStorage, ...args) {
      if (phase === "after") await commit.apply(this, args);
      throw new Error("Simulated interruption after app admission");
    });
    await runtime.agents.subagents.tick();
    const admitted = (await runs(trader.id))[0]!;
    expect((await requests())[0]!.run_id).toBe(admitted.id);
    expect(admitted.taskId).toBeNull();
    interrupted.mockRestore();
    await runtime.close();
    const opened = vi.spyOn(Harness, "open");
    runtime = await createRuntime(config(), options());
    const restored = await opened.mock.results.at(-1)!.value as Harness;
    const root = await restored.root(context);
    const receipt = () => root.commit(async (tx) => (await tx.doc(RunReceipt, root.id, admitted.id, null)).taskId, context);
    const taskBeforeRecovery = await receipt();
    if (phase === "before") expect(taskBeforeRecovery).toBeNull();
    else expect(taskBeforeRecovery).not.toBeNull();
    runtime.start(); await runtime.agents.subagents.tick();
    const completed = await waitCompleted(trader.id);
    expect(completed.id).toBe(admitted.id);
    expect(await receipt()).toBe(completed.taskId);
    if (taskBeforeRecovery !== null) expect(completed.taskId).toBe(taskBeforeRecovery);
    await event(analyst.id, "research:commit-gap");
    await runtime.agents.subagents.tick();
    expect(await requests()).toHaveLength(1);
    expect(await runs(trader.id)).toHaveLength(1);
    expect(faux.state.callCount).toBe(1);
  });

  it("keeps paused work pending and supports discarding without canceling exchange orders", async () => {
    const trader = await createWorker({ triggers: inbox() });
    const first = await runtime.agents.subagents.delegate(agentId, trader.id, "Inspect account", "test:one");
    expect((await runtime.agents.subagents.delegate(agentId, trader.id, "Inspect account", "test:one")).id).toBe(first.id);
    await expect(runtime.agents.subagents.delegate(agentId, trader.id, "Different", "test:one")).rejects.toThrow("already used");
    await runtime.agents.subagents.updateSection("owner", agentId, trader.id, "enabled", { enabled: false });
    await runtime.agents.subagents.tick();
    expect(await runs(trader.id)).toHaveLength(0);
    expect((await requests())[0]!.discarded_reason).toBeNull();
    await runtime.agents.subagents.stop("owner", agentId, first.id);
    await runtime.agents.subagents.updateSection("owner", agentId, trader.id, "enabled", { enabled: true });
    await runtime.agents.subagents.tick();
    expect(await runs(trader.id)).toHaveLength(0);
    expect((await requests())[0]!.discarded_reason).toContain("Discarded by operator");
  });

  it("expires old requests instead of replaying delayed trade intent", async () => {
    const trader = await createWorker({ triggers: inbox(), timeoutSeconds: 30 });
    await runtime.agents.subagents.delegate(agentId, trader.id, "Open a long", "old:request", undefined, "operator_trade");
    await runtime.agents.subagents.tick(Date.now() + 31000);
    expect(await runs(trader.id)).toHaveLength(0);
    expect((await requests())[0]!.discarded_reason).toContain("expired");
  });

  it("publishes research and its handoff atomically, even for no-opportunity reports", async () => {
    const analyst = await createWorker({ tools: ["publish_research"] });
    const trader = await createWorker({ triggers: inbox({ researchSources: [analyst.id] }) });
    const boundary = Math.floor(Date.now() / 3600000) * 3600000;
    faux.setResponses([fauxAssistantMessage(fauxToolCall("publish_research", researchInput(boundary, { summary: "No opportunity" })), { stopReason: "toolUse" }), ...Array.from({ length: 12 }, () => fauxAssistantMessage("Done"))]);
    await runtime.agents.subagents.run("owner", agentId, analyst.id, "Publish findings", randomUUID());
    const publicationRun = await waitCompleted(analyst.id);
    await vi.waitFor(async () => expect(await requests()).toHaveLength(1));
    expect(JSON.parse((await requests())[0]!.payload)).toMatchObject({ intent: "evaluation", researchRefs: ["m1"], callerRunId: publicationRun.id });
    await runtime.agents.subagents.tick();
    expect((await waitCompleted(trader.id)).task).toContain("research=m1");
    await expect(runtime.database.transaction(async (tx) => { await enqueueWorkerEvent(tx, agentId, "research:rollback", { intent: "evaluation", reason: "research", task: "Do not commit", callerSubagentId: analyst.id, callerRunId: null, researchRefs: [] }); throw new Error("rollback"); })).rejects.toThrow("rollback");
    expect(await requests()).toHaveLength(1);
  });

  it("hands newer superseding research to the trader once, without enqueueing rejected corrections", async () => {
    const analyst = await createWorker({ tools: ["publish_research"] });
    const trader = await createWorker({ triggers: inbox({ researchSources: [analyst.id] }) });
    const boundary = Math.floor(Date.now() / 900_000) * 900_000;
    faux.setResponses([
      fauxAssistantMessage(fauxToolCall("publish_research", researchInput(boundary - 900_000, { interval: "15m", requestedValidUntil: boundary - 600_000 })), { stopReason: "toolUse" }),
      ...Array.from({ length: 12 }, () => fauxAssistantMessage("Done")),
    ]);
    await runtime.agents.subagents.run("owner", agentId, analyst.id, "Publish the earlier report", randomUUID());
    await waitCompleted(analyst.id);
    await runtime.agents.subagents.tick();
    await waitCompleted(trader.id);
    const next = researchInput(boundary, { interval: "15m", links: [{ ref: "m1", relation: "supersedes" }] });
    faux.setResponses([
      fauxAssistantMessage(fauxToolCall("publish_research", { ...next, links: [{ ref: "m1", relation: "corrects" }] }), { stopReason: "toolUse" }),
      fauxAssistantMessage(fauxToolCall("publish_research", next), { stopReason: "toolUse" }),
      ...Array.from({ length: 12 }, () => fauxAssistantMessage("Done")),
    ]);
    await runtime.agents.subagents.run("owner", agentId, analyst.id, "Publish the new candle, replacing the expired view", randomUUID());
    await waitCompleted(analyst.id, 2);
    await runtime.agents.subagents.tick();
    await waitCompleted(trader.id, 2);
    const rows = await requests();
    expect(rows).toHaveLength(2);
    expect(rows.map((row) => JSON.parse(row.payload).researchRefs)).toEqual([["m1"], ["m2"]]);
    expect(rows.every((row) => row.run_id !== null)).toBe(true);
    expect(await runtime.agents.execution.memories.get(agentId, "m1")).toMatchObject({ status: "stale", supersededBy: "m2", correctedBy: null });
    expect(await runtime.agents.execution.memories.get(agentId, "m2")).toMatchObject({ status: "fresh", validUntil: boundary + 1800_000, links: [{ ref: "m1", relation: "supersedes" }] });
  });

  it.each([false, true])("allows approved targeted research delegation; publication deduplication=%s", async (subscribed) => {
    const analyst = await createWorker({ tools: ["publish_research", "delegate_subagent"] });
    const trader = await createWorker({ triggers: inbox({ workerDelegates: [analyst.id], researchSources: subscribed ? [analyst.id] : [] }) });
    const boundary = Math.floor(Date.now() / 3600000) * 3600000;
    faux.setResponses([
      fauxAssistantMessage(fauxToolCall("publish_research", researchInput(boundary)), { stopReason: "toolUse" }),
      fauxAssistantMessage(fauxToolCall("delegate_subagent", { subagent_id: trader.id, task: "Evaluate this report independently", research_refs: ["m1"] }), { stopReason: "toolUse" }),
      ...Array.from({ length: 12 }, () => fauxAssistantMessage("Done")),
    ]);
    await runtime.agents.subagents.run("owner", agentId, analyst.id, "Publish and request an evaluation", randomUUID());
    const run = await waitCompleted(analyst.id);
    const detail = await runtime.agents.subagents.runDetail("owner", agentId, analyst.id, run.id);
    expect(detail.tools).toContainEqual(expect.objectContaining({ name: "delegate_subagent", status: "done" }));
    expect(await requests()).toHaveLength(1);
    expect(JSON.parse((await requests())[0]!.payload)).toMatchObject({ intent: "evaluation", callerSubagentId: analyst.id });
    await runtime.agents.subagents.tick();
    await waitCompleted(trader.id);
  });

  it("discovers multiple approved targets but delegates only to the explicitly selected worker", async () => {
    const analyst = await createWorker({ tools: ["list_subagents", "publish_research", "delegate_subagent"] });
    const first = await createWorker({ name: "First trader", triggers: inbox({ workerDelegates: [analyst.id] }) });
    const second = await createWorker({ name: "Second recipient", triggers: inbox({ workerDelegates: [analyst.id] }) });
    const hidden = [
      await createWorker({ triggers: inbox() }),
      await createWorker({ enabled: false, triggers: inbox({ workerDelegates: [analyst.id] }) }),
      await createWorker({ delegatable: false, triggers: inbox({ workerDelegates: [analyst.id] }) }),
      await createWorker(),
    ];
    const boundary = Math.floor(Date.now() / 3600000) * 3600000;
    faux.setResponses([
      fauxAssistantMessage(fauxToolCall("list_subagents", {}), { stopReason: "toolUse" }),
      fauxAssistantMessage(fauxToolCall("publish_research", researchInput(boundary)), { stopReason: "toolUse" }),
      fauxAssistantMessage(fauxToolCall("delegate_subagent", { subagent_id: second.id, task: "Evaluate independently", research_refs: ["m1"] }), { stopReason: "toolUse" }),
      ...Array.from({ length: 12 }, () => fauxAssistantMessage("Done")),
    ]);
    await runtime.agents.subagents.run("owner", agentId, analyst.id, "Choose the second trader", randomUUID());
    const run = await waitCompleted(analyst.id);
    const detail = await runtime.agents.subagents.runDetail("owner", agentId, analyst.id, run.id);
    expect(detail.tools).toContainEqual(expect.objectContaining({ name: "list_subagents", status: "done", output: expect.stringContaining(first.id) }));
    expect(detail.tools).toContainEqual(expect.objectContaining({ name: "list_subagents", output: expect.stringContaining(second.id) }));
    const discovered = detail.tools!.find((tool) => tool.name === "list_subagents")!.output;
    for (const worker of [...hidden, analyst]) expect(discovered).not.toContain(worker.id);
    expect(await requests()).toHaveLength(1);
    expect((await requests())[0]!.subagent_id).toBe(second.id);
    await runtime.agents.subagents.tick();
    await waitCompleted(second.id);
    expect(await runs(first.id)).toHaveLength(0);
  });

  it.each([undefined, []])("delegates a self-contained task without research refs (%s)", async (refs) => {
    const caller = await createWorker({ tools: ["delegate_subagent"] });
    const recipient = await createWorker({ triggers: inbox({ workerDelegates: [caller.id], researchSources: [caller.id] }) });
    faux.setResponses([
      fauxAssistantMessage(fauxToolCall("delegate_subagent", { subagent_id: recipient.id, task: "Summarize the configured strategy", ...(refs ? { research_refs: refs } : {}) }), { stopReason: "toolUse" }),
      ...Array.from({ length: 12 }, () => fauxAssistantMessage("Done")),
    ]);
    await runtime.agents.subagents.run("owner", agentId, caller.id, "Delegate a non-research task", randomUUID());
    const completed = await waitCompleted(caller.id);
    const detail = await runtime.agents.subagents.runDetail("owner", agentId, caller.id, completed.id);
    expect(detail.tools).toContainEqual(expect.objectContaining({ name: "delegate_subagent", status: "done" }));
    expect(await requests()).toHaveLength(1);
    expect(JSON.parse((await requests())[0]!.payload)).toMatchObject({ intent: "evaluation", researchRefs: [], callerSubagentId: caller.id, callerRunId: completed.id });
    await runtime.agents.subagents.tick();
    const run = await waitCompleted(recipient.id);
    expect(run.task).toContain("Summarize the configured strategy");
    expect(run.task).toContain("published research is not required");
    expect(run.task).toContain(`Worker inbox request ${(await requests())[0]!.id};`);
    expect(run.task).toContain("not exchange order requests. Do not pass them to get_order_request");
    expect(run.task).toContain("discover local submissions with list_orders(source=local)");
  });

  it("accepts another worker's research and rejects missing or foreign references", async () => {
    const publisher = await createWorker({ tools: ["publish_research"] });
    const boundary = Math.floor(Date.now() / 3600000) * 3600000;
    faux.setResponses([
      fauxAssistantMessage(fauxToolCall("publish_research", researchInput(boundary)), { stopReason: "toolUse" }),
      fauxAssistantMessage("Published"),
    ]);
    await runtime.agents.subagents.run("owner", agentId, publisher.id, "Publish research", randomUUID());
    await waitCompleted(publisher.id);
    const other = await runtime.agents.create("owner", "Other", randomUUID(), `0x${"b".repeat(40)}`);
    // This reference exists only in another Agent, never in the caller's scope.
    await runtime.agents.execution.memories.publishResearch({ agentId: other.id, userId: "owner", subagentId: null, runId: null, conversationId: null, toolTaskId: null }, "foreign:one", researchInput(boundary));
    await runtime.agents.execution.memories.publishResearch({ agentId: other.id, userId: "owner", subagentId: null, runId: null, conversationId: null, toolTaskId: null }, "foreign:two", researchInput(boundary, { topic: "other_topic" }));
    const caller = await createWorker({ tools: ["delegate_subagent"] });
    const recipient = await createWorker({ triggers: inbox({ workerDelegates: [caller.id] }) });
    faux.setResponses([
      fauxAssistantMessage(fauxToolCall("delegate_subagent", { subagent_id: recipient.id, task: "Use another worker's report", research_refs: ["m1"] }), { stopReason: "toolUse" }),
      fauxAssistantMessage(fauxToolCall("delegate_subagent", { subagent_id: recipient.id, task: "Invalid foreign report", research_refs: ["m2"] }), { stopReason: "toolUse" }),
      fauxAssistantMessage(fauxToolCall("delegate_subagent", { subagent_id: recipient.id, task: "Missing report", research_refs: ["m999"] }), { stopReason: "toolUse" }),
      ...Array.from({ length: 12 }, () => fauxAssistantMessage("Done")),
    ]);
    await runtime.agents.subagents.run("owner", agentId, caller.id, "Delegate with references", randomUUID());
    const completed = await waitCompleted(caller.id);
    const detail = await runtime.agents.subagents.runDetail("owner", agentId, caller.id, completed.id);
    const calls = detail.tools!.filter((tool) => tool.name === "delegate_subagent");
    expect(calls.map((tool) => tool.status)).toEqual(["done", "error", "error"]);
    expect(calls[1]!.output).toContain("Research reference not found in this Agent");
    expect(await requests()).toHaveLength(1);
    expect(JSON.parse((await requests())[0]!.payload)).toMatchObject({ callerSubagentId: caller.id, researchRefs: ["m1"] });
    await runtime.agents.subagents.tick();
    expect((await waitCompleted(recipient.id)).task).toContain("research=m1");
  });

  it("upgrades saved delegation selections without adding capabilities or changing instructions", async () => {
    const caller = await createWorker({ tools: ["get_memory"], instructions: "Keep my custom instructions." });
    await runtime.database.run("UPDATE hv_subagents SET tools=? WHERE id=?", JSON.stringify(["get_memory", "list_trader_targets", "request_trader_evaluation"]), caller.id);
    await runtime.close();
    runtime = await createRuntime(config(), options());
    const upgraded = (await runtime.agents.subagents.detail("owner", agentId, caller.id)).definition;
    expect(upgraded.tools).toEqual(["get_memory", "list_subagents", "delegate_subagent"]);
    expect(upgraded.instructions).toBe(caller.instructions);
    const recipient = await createWorker({ triggers: inbox({ workerDelegates: [caller.id] }) });
    faux.setResponses([
      fauxAssistantMessage(fauxToolCall("delegate_subagent", { subagent_id: recipient.id, task: "Use the upgraded tool" }), { stopReason: "toolUse" }),
      ...Array.from({ length: 12 }, () => fauxAssistantMessage("Done")),
    ]);
    await runtime.agents.subagents.start(); runtime.start();
    await runtime.agents.subagents.run("owner", agentId, caller.id, "Delegate", randomUUID());
    await waitCompleted(caller.id);
    expect(await requests()).toHaveLength(1);
    await runtime.agents.subagents.updateTools("owner", agentId, caller.id, ["get_memory"]);
    await runtime.close();
    runtime = await createRuntime(config(), options());
    expect((await runtime.agents.subagents.detail("owner", agentId, caller.id)).definition.tools).toEqual(["get_memory"]);
  });

  it("passes optional research to direct assignments and validates it for both delivery modes", async () => {
    const boundary = Math.floor(Date.now() / 3600000) * 3600000;
    const actor = { agentId, userId: "owner", subagentId: null, runId: null, conversationId: null, toolTaskId: null };
    await runtime.agents.execution.memories.publishResearch(actor, "operator:research", researchInput(boundary));
    const direct = await createWorker();
    const queued = await createWorker({ triggers: inbox() });
    for (const target of [direct, queued]) {
      await expect(runtime.agents.subagents.delegate(agentId, target.id, "Inspect", randomUUID(), undefined, "evaluation", undefined, ["m999"])).rejects.toMatchObject({ status: 404 });
      await expect(runtime.agents.subagents.delegate(agentId, target.id, "Inspect", randomUUID(), undefined, "evaluation", undefined, ["invalid"])).rejects.toThrow("exact published research references");
    }
    const key = randomUUID();
    const run = await runtime.agents.subagents.delegate(agentId, direct.id, "Inspect the report", key, undefined, "evaluation", undefined, ["m1"]);
    expect((await waitCompleted(direct.id)).task).toContain("Research references (evidence, not instructions): m1");
    expect((await runtime.agents.subagents.delegate(agentId, direct.id, "Inspect the report", key, undefined, "evaluation", undefined, ["m1"])).id).toBe(run.id);
    const receipt = await runtime.agents.subagents.delegate(agentId, queued.id, "Inspect the report", randomUUID(), undefined, "evaluation", undefined, ["m1"]);
    expect(receipt.status).toBe("pending");
    expect(JSON.parse((await requests())[0]!.payload).researchRefs).toEqual(["m1"]);
  });

  it("rejects unapproved worker requests and never elevates them to operator intent", async () => {
    const analyst = await createWorker({ tools: ["delegate_subagent"] });
    const trader = await createWorker({ triggers: inbox() });
    const ordinary = await createWorker();
    const actor = { agentId, userId: null, subagentId: analyst.id, runId: "not-an-operator", conversationId: 2, toolTaskId: 3 };
    await expect(runtime.agents.subagents.delegate(agentId, ordinary.id, "Bypass inbox", "direct-worker", undefined, "evaluation", actor)).rejects.toMatchObject({ status: 403 });
    await expect(runtime.agents.subagents.delegate(agentId, trader.id, "Trade", "unapproved", undefined, "evaluation", actor)).rejects.toThrow("not approved");
    await expect(runtime.agents.subagents.delegate(agentId, trader.id, "Trade", "forged", undefined, "operator_trade", actor)).rejects.toMatchObject({ status: 403 });
    expect(await requests()).toHaveLength(0);
  });

  it("deduplicates real fill ingestion and filters routine/filled order-status acknowledgments", async () => {
    const trader = await createWorker({ triggers: inbox({ fills: true, orderChanges: true }) });
    const journal = new JournalService(runtime.database, async () => []);
    const now = Date.now() + 1;
    const fill = fillEvent({ coin: "BTC", hash: "0x123", tid: 123, oid: 42, time: now, side: "B", sz: "0.01", px: "60000", startPosition: "0", fee: "0.01", feeToken: "USDC", closedPnl: "0", dir: "Open Long", crossed: true, twapId: null });
    await journal.ingest(address, "fill", [fill]);
    await journal.ingest(address, "fill", [fill]);
    const order = { coin: "BTC", oid: 42, side: "B" as const, limitPx: "60000", sz: "0.01", timestamp: now, origSz: "0.01" };
    await journal.ingestOrderUpdates(address, [{ order, status: "open", statusTimestamp: now }, { order, status: "filled", statusTimestamp: now }]);
    await journal.ingestOrderUpdates(address, [{ order, status: "rejected", statusTimestamp: now }]);
    await journal.ingestOrderUpdates(address, [{ order, status: "rejected", statusTimestamp: now }]);
    expect(await requests()).toHaveLength(2);
    expect((await requests()).map((request) => (JSON.parse(request.payload) as { reason: string }).reason).sort()).toEqual(["fill", "order_status"]);
    await journal.ingest(address, "fill", [{ ...fill, id: "historic-fill", time: now - 86400000 }]);
    expect(await requests()).toHaveLength(2);
    await runtime.agents.subagents.tick();
    const run = await waitCompleted(trader.id);
    expect(run.source).toBe("event");
    expect(run.task).toContain("actual positions");
    await journal.close();
  });
});
