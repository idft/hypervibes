import { BACKGROUND_CONTEXT as context } from "@earendil-works/chord/context";
import type { Message } from "@earendil-works/pi-ai";
import { createModels } from "@earendil-works/pi-ai/models";
import { fauxAssistantMessage, fauxProvider, fauxToolCall } from "@earendil-works/pi-ai/providers/faux";
import { Harness, type ConversationId } from "@earendil-works/pi-durable";
import { randomUUID } from "node:crypto";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { hvUsers } from "../db/auth-schema.js";
import { hvSubagentRuns } from "../db/subagent-schema.js";
import { createRuntime, type AppRuntime } from "../runtime.js";
import { testConfig } from "../test-config.js";
import { RunDiscussionLink } from "./run-discussion.js";
import type { SubagentInput } from "./subagent-input.js";
import type { RunDetail } from "./subagent-service.js";

const settings = (patch: Partial<SubagentInput> = {}): SubagentInput => ({
  name: "Research", description: "Frozen research", instructions: "ORIGINAL instructions: explain only the admitted evidence.",
  provider: "faux", modelId: "original", thinkingLevel: "medium", tools: [], enabled: true, delegatable: true,
  persistent: false, schedule: "", delaySeconds: 30, completion: "save", timeoutSeconds: 900, ...patch,
});
const waitOptions = { timeout: 10000 };
const offeredTools = (messages: Message[]) => {
  const names = new Set<string>();
  for (const message of messages) if (message.role === "system") {
    for (const tool of message.toolsRemoved ?? []) names.delete(tool.name);
    for (const tool of message.toolsAdded ?? []) names.add(tool.name);
  }
  return [...names];
};
const gate = () => {
  let resolve!: () => void;
  const promise = new Promise<void>((done) => { resolve = done; });
  return { promise, resolve };
};

describe("read-only branched run discussions", () => {
  let directory: string, runtime: AppRuntime, faux: ReturnType<typeof fauxProvider>;
  let models: ReturnType<typeof createModels>;
  const config = () => testConfig({ APP_SQLITE_PATH: join(directory, "app.sqlite") });
  const options = () => ({
    models, model: { provider: "faux", modelId: "original" },
    accountFeedFactory: () => ({ close: async () => {} }), journalFeedFactory: () => ({ close: async () => {} }),
    journalHistoryFetcher: async () => [],
  });
  const createAgent = () => runtime.agents.create("owner", "Trading desk", randomUUID(), `0x${randomUUID().replaceAll("-", "")}12345678`);
  const createWorker = (agentId: string, patch: Partial<SubagentInput> = {}) => runtime.agents.subagents.create("owner", agentId, settings(patch), randomUUID());
  const runDetail = (agentId: string, workerId: string, runId: string) => runtime.agents.subagents.runDetail("owner", agentId, workerId, runId);
  const completedRun = async (agentId: string, workerId: string, assignment = "ORIGINAL assignment: review the closed candle") => {
    const run = await runtime.agents.subagents.run("owner", agentId, workerId, assignment, randomUUID());
    await vi.waitFor(async () => expect((await runtime.agents.subagents.internalRun(agentId, run.id)).status).toBe("completed"), waitOptions);
    return runDetail(agentId, workerId, run.id);
  };
  const answered = async (agentId: string, workerId: string, runId: string, answer: string) => {
    await vi.waitFor(async () => {
      const detail = await runDetail(agentId, workerId, runId);
      expect(detail.discussion?.busy).toBe(false);
      expect(JSON.stringify(detail.discussion?.entries)).toContain(answer);
    }, waitOptions);
    return (await runDetail(agentId, workerId, runId)).discussion!;
  };
  const followup = (agentId: string, workerId: string, runId: string, message: string, requestId = randomUUID()) =>
    runtime.agents.subagents.sendRunFollowup("owner", agentId, workerId, runId, message, requestId);

  beforeEach(async () => {
    directory = mkdtempSync(join(tmpdir(), "hv-run-discussion-"));
    faux = fauxProvider({ models: [{ id: "original", reasoning: true }, { id: "changed", reasoning: true }], tokenSize: { min: 1, max: 1 }, tokensPerSecond: 100 });
    models = createModels(); models.setProvider(faux.provider);
    runtime = await createRuntime(config(), options());
    await runtime.db.insert(hvUsers).values([
      { id: "owner", name: "Owner", email: "owner@example.test" },
      { id: "other", name: "Other", email: "other@example.test" },
    ]).run();
    runtime.start(); await runtime.agents.subagents.tick();
  });
  afterEach(async () => {
    vi.restoreAllMocks();
    await runtime.close();
    rmSync(directory, { recursive: true, force: true });
  });

  it("forks at the settled boundary and preserves frozen context without contaminating a persistent worker", async () => {
    const opened = vi.spyOn(Harness, "open");
    const agent = await createAgent(), worker = await createWorker(agent.id, { persistent: true, tools: ["artifact_create"] });
    faux.setResponses([fauxAssistantMessage("ORIGINAL answer: no trade"), fauxAssistantMessage("LATER answer: private new evidence")]);
    const first = await completedRun(agent.id, worker.id);
    expect(first.discussion).toBeUndefined();
    const laterRun = await completedRun(agent.id, worker.id, "LATER assignment: do not include this in the discussion");
    expect(laterRun.run.conversationId).toBe(first.run.conversationId);
    await runtime.agents.subagents.updateSection("owner", agent.id, worker.id, "instructions", { instructions: "CHANGED instructions: unrelated future behavior" });
    await runtime.agents.subagents.updateSection("owner", agent.id, worker.id, "model", { provider: "faux", modelId: "changed", thinkingLevel: "low" });
    // A later assignment can append its normal context-retirement entry to the
    // prior run. Freeze the baseline after that assignment, before discussing it.
    const original = await runDetail(agent.id, worker.id, first.run.id);
    const later = await runDetail(agent.id, worker.id, laterRun.run.id);
    const requests: { messages: Message[]; model: string; reasoning: string | undefined }[] = [];
    faux.setResponses([(request, options, _state, model) => {
      requests.push({ messages: structuredClone(request.messages), model: model.id, reasoning: options?.reasoning });
      return fauxAssistantMessage("DISCUSSION answer: the original evidence was insufficient");
    }, fauxAssistantMessage("NEXT persistent answer")]);
    await followup(agent.id, worker.id, original.run.id, "DISCUSSION question: why did you reject the trade?");
    const discussion = await answered(agent.id, worker.id, original.run.id, "DISCUSSION answer");
    expect(discussion.conversationId).not.toBe(original.run.conversationId);
    expect(requests).toHaveLength(1);
    expect(requests[0]).toMatchObject({ model: "original", reasoning: "medium" });
    const prompt = JSON.stringify(requests[0]!.messages);
    for (const expected of [worker.instructions, original.run.task, "ORIGINAL answer: no trade", "DISCUSSION question"]) expect(prompt).toContain(expected);
    for (const excluded of ["LATER assignment", "LATER answer", "CHANGED instructions"]) expect(prompt).not.toContain(excluded);
    expect(offeredTools(requests[0]!.messages)).toEqual([]);
    const harness = await opened.mock.results[0]!.value as Harness;
    const main = await harness.root(context);
    const link = await main.commit(async (tx) => {
      const saved = await tx.doc(RunDiscussionLink, main.id, original.run.id, null);
      return { conversationId: saved.conversationId, forkEntryId: saved.forkEntryId };
    }, context);
    expect(link).toEqual({
      conversationId: discussion.conversationId,
      forkEntryId: original.entries.filter((entry) => entry.model?.some((message) => message.role === "assistant")).at(-1)!.id,
    });
    const branch = (await harness.conversation(discussion.conversationId as ConversationId, context))!;
    const resolved = await branch.agent(context);
    expect(resolved.tools).toEqual([]);
    expect(resolved.callable).toEqual([]);
    expect(resolved.deferred).toEqual([]);
    expect(resolved.extensions).toEqual([]);
    expect(resolved.model).toEqual({ provider: "faux", modelId: "original" });
    expect(resolved.thinkingLevel).toBe("medium");
    const after = await runDetail(agent.id, worker.id, original.run.id);
    expect(after.run).toEqual(original.run);
    expect(after.entries).toEqual(original.entries);
    expect(after.prior).toEqual(original.prior);
    const afterLater = await runDetail(agent.id, worker.id, later.run.id);
    expect(afterLater.run).toEqual(later.run);
    expect(afterLater.entries).toEqual(later.entries);
    expect(afterLater.prior).toEqual(later.prior);
    const next = await completedRun(agent.id, worker.id, "NEXT persistent assignment");
    expect(next.run.conversationId).toBe(original.run.conversationId);
    expect(JSON.stringify([...next.prior, ...next.entries])).not.toContain("DISCUSSION question");
    expect(JSON.stringify([...next.prior, ...next.entries])).not.toContain("DISCUSSION answer");
    expect((await runtime.agents.chat("owner", agent.id)).messages).toEqual([]);
  });

  it.each(["failed", "cancelled"] as const)("forks an unanswered %s run before the next persistent assignment", async (status) => {
    const opened = vi.spyOn(Harness, "open");
    const agent = await createAgent(), worker = await createWorker(agent.id, { persistent: true });
    const response = gate();
    faux.setResponses([async (_request, options) => {
      if (status === "failed") throw new Error("Original assignment provider failure");
      const signal = options?.signal, release = () => response.resolve();
      signal?.addEventListener("abort", release, { once: true });
      try {
        if (signal?.aborted) release();
        await response.promise;
        return fauxAssistantMessage("Cancelled assignment must not receive an answer");
      } finally { signal?.removeEventListener("abort", release); }
    }]);
    try {
      const admitted = await runtime.agents.subagents.run("owner", agent.id, worker.id, `ORIGINAL ${status} assignment: review the recorded evidence`, randomUUID());
      await vi.waitFor(() => expect(faux.state.callCount).toBe(1), waitOptions);
      if (status === "cancelled") await runtime.agents.subagents.stop("owner", agent.id, admitted.id);
      await vi.waitFor(async () => expect((await runtime.agents.subagents.internalRun(agent.id, admitted.id)).status).toBe(status), waitOptions);
      const first = await runDetail(agent.id, worker.id, admitted.id);
      const harness = await opened.mock.results[0]!.value as Harness;
      const child = (await harness.conversation(first.run.conversationId! as ConversationId, context))!;
      const submission = await child.commit((tx) => tx.submissionByRequest(child.id, `assignment:${admitted.id}`), context);
      expect(submission).toMatchObject({ type: "input", status: "unanswered", entry: expect.any(Number) });
      expect(submission?.answer).toBeUndefined();
      await runtime.agents.subagents.updateSection("owner", agent.id, worker.id, "instructions", { instructions: "LATER instructions: use only the next assignment" });
      faux.setResponses([fauxAssistantMessage("LATER persistent answer: unrelated new evidence")]);
      const later = await completedRun(agent.id, worker.id, "LATER persistent assignment: keep this outside the discussion");
      expect(later.run.conversationId).toBe(first.run.conversationId);
      const original = await runDetail(agent.id, worker.id, first.run.id);
      const retirementIndex = original.entries.findIndex((entry) => entry.kind === "hv.assignment-instructions-retired");
      expect(retirementIndex).toBeGreaterThan(0);
      const prompts: Message[][] = [];
      faux.setResponses([(request) => { prompts.push(structuredClone(request.messages)); return fauxAssistantMessage("Explanation of the unanswered original assignment"); }]);
      await followup(agent.id, worker.id, original.run.id, "Explain why this assignment did not finish");
      const discussion = await answered(agent.id, worker.id, original.run.id, "Explanation of the unanswered original assignment");
      expect(prompts).toHaveLength(1);
      const prompt = JSON.stringify(prompts[0]);
      expect(prompt).toContain(original.run.task);
      expect(prompt).toContain(worker.instructions);
      for (const excluded of ["LATER persistent assignment", "LATER persistent answer", "LATER instructions"]) expect(prompt).not.toContain(excluded);
      const main = await harness.root(context);
      const link = await main.commit(async (tx) => {
        const saved = await tx.doc(RunDiscussionLink, main.id, original.run.id, null);
        return { conversationId: saved.conversationId, forkEntryId: saved.forkEntryId };
      }, context);
      expect(link).toEqual({ conversationId: discussion.conversationId, forkEntryId: original.entries[retirementIndex - 1]!.id });
      const after = await runDetail(agent.id, worker.id, original.run.id);
      expect(after.run).toEqual(original.run);
      expect(after.entries).toEqual(original.entries);
      const afterLater = await runDetail(agent.id, worker.id, later.run.id);
      expect(afterLater.run).toEqual(later.run);
      expect(afterLater.entries).toEqual(later.entries);
    } finally { response.resolve(); }
  }, 15000);

  it("blocks a still-stopping run until its placed submission becomes unanswered", async () => {
    const opened = vi.spyOn(Harness, "open");
    const agent = await createAgent(), worker = await createWorker(agent.id, { persistent: true });
    const response = gate();
    faux.setResponses([async (_request, options) => {
      const signal = options?.signal, release = () => response.resolve();
      signal?.addEventListener("abort", release, { once: true });
      try {
        if (signal?.aborted) release();
        await response.promise;
        return fauxAssistantMessage("Stopping assignment must not receive an answer");
      } finally { signal?.removeEventListener("abort", release); }
    }]);
    try {
      const admitted = await runtime.agents.subagents.run("owner", agent.id, worker.id, "ORIGINAL still-stopping assignment", randomUUID());
      await vi.waitFor(() => expect(faux.state.callCount).toBe(1), waitOptions);
      const harness = await opened.mock.results[0]!.value as Harness;
      const running = await runtime.agents.subagents.internalRun(agent.id, admitted.id);
      const child = (await harness.conversation(running.conversationId! as ConversationId, context))!;
      const submission = () => child.commit((tx) => tx.submissionByRequest(child.id, `assignment:${admitted.id}`), context);
      expect(await submission()).toMatchObject({ type: "input", status: "placed", entry: expect.any(Number) });
      // Reproduce the application/Durable settlement window through the public
      // application service. Keep the actual model submission placed and running.
      await runtime.agents.subagents.finish(admitted.id, "cancelled", "Application cancellation recorded before Durable settles");
      const stopping = await runDetail(agent.id, worker.id, admitted.id);
      expect(stopping.run.status).toBe("cancelled");
      expect(stopping.discussion).toBeUndefined();
      expect(stopping.discussionUnavailable).toContain("still stopping");
      await expect(followup(agent.id, worker.id, admitted.id, "Too early")).rejects.toMatchObject({ status: 409 });
      const main = await harness.root(context);
      expect(await main.commit(async (tx) => (await tx.doc(RunDiscussionLink, main.id, admitted.id, null)).conversationId, context)).toBeNull();
      expect(faux.state.callCount).toBe(1);
      await child.abort(context, { background: true });
      expect(await submission()).toMatchObject({ type: "input", status: "unanswered", entry: expect.any(Number) });
      await vi.waitFor(async () => expect((await runtime.agents.subagents.internalRun(agent.id, admitted.id)).reportedAt).not.toBeNull(), waitOptions);
      const settled = await runDetail(agent.id, worker.id, admitted.id);
      expect(settled.discussionUnavailable).toBeUndefined();
      const prompts: Message[][] = [];
      faux.setResponses([(request) => { prompts.push(structuredClone(request.messages)); return fauxAssistantMessage("Discussion starts after settlement"); }]);
      await followup(agent.id, worker.id, admitted.id, "Explain after cancellation settles");
      await answered(agent.id, worker.id, admitted.id, "Discussion starts after settlement");
      expect(JSON.stringify(prompts[0])).toContain(running.task);
      const after = await runDetail(agent.id, worker.id, admitted.id);
      expect(after.run).toEqual(settled.run);
      expect(after.entries).toEqual(settled.entries);
    } finally { response.resolve(); }
  }, 15000);

  it("offers no tools and cannot execute fabricated direct or nested write calls", async () => {
    const agent = await createAgent(), worker = await createWorker(agent.id, { tools: ["codemode", "artifact_create"] });
    faux.setResponses([fauxAssistantMessage("Original result")]);
    const original = await completedRun(agent.id, worker.id);
    const requests: Message[][] = [];
    faux.setResponses([
      (request) => {
        requests.push(structuredClone(request.messages));
        return fauxAssistantMessage([
          fauxToolCall("artifact_create", { title: "Forbidden artifact", mediaType: "text/markdown", content: "Must not be saved" }),
          fauxToolCall("codemode", { code: "text(await tools.artifact_create({title:'Nested forbidden artifact',mediaType:'text/markdown',content:'Must not be saved'}));" }),
        ], { stopReason: "toolUse" });
      },
      (request) => { requests.push(structuredClone(request.messages)); return fauxAssistantMessage("Read-only explanation complete"); },
    ]);
    const artifacts = await runtime.agents.research.artifacts.list(agent.id);
    await followup(agent.id, worker.id, original.run.id, "Try to save the explanation");
    const discussion = await answered(agent.id, worker.id, original.run.id, "Read-only explanation complete");
    expect(requests).toHaveLength(2);
    for (const messages of requests) expect(offeredTools(messages)).toEqual([]);
    const results = requests[1]!.filter((message) => message.role === "toolResult");
    expect(results).toHaveLength(2);
    expect(results.every((message) => message.isError)).toBe(true);
    expect(JSON.stringify(discussion.entries)).toContain("artifact_create");
    expect(await runtime.agents.research.artifacts.list(agent.id)).toEqual(artifacts);
    expect((await runDetail(agent.id, worker.id, original.run.id)).run).toEqual(original.run);
    expect((await runDetail(agent.id, worker.id, original.run.id)).entries).toEqual(original.entries);
  });

  it("deduplicates concurrent retries and reuses the saved branch after restart", async () => {
    const agent = await createAgent(), worker = await createWorker(agent.id);
    faux.setResponses([fauxAssistantMessage("Original result")]);
    const original = await completedRun(agent.id, worker.id);
    const requestId = randomUUID(), message = "FIRST discussion question", response = gate();
    faux.setResponses([async () => { await response.promise; return fauxAssistantMessage("FIRST discussion answer"); }]);
    try {
      await Promise.all([
        followup(agent.id, worker.id, original.run.id, message, requestId),
        followup(agent.id, worker.id, original.run.id, message, requestId),
      ]);
      await vi.waitFor(() => expect(faux.state.callCount).toBe(2), waitOptions);
    } finally { response.resolve(); }
    const first = await answered(agent.id, worker.id, original.run.id, "FIRST discussion answer");
    const userEntries = first.entries.filter((entry) => entry.kind === "pi.user" && JSON.stringify(entry).includes(message));
    expect(userEntries).toHaveLength(1);
    await followup(agent.id, worker.id, original.run.id, message, requestId);
    expect(faux.state.callCount).toBe(2);
    await runtime.close(); runtime = await createRuntime(config(), options()); runtime.start();
    const restored = (await runDetail(agent.id, worker.id, original.run.id)).discussion!;
    expect(restored.conversationId).toBe(first.conversationId);
    expect(restored.entries).toEqual(first.entries);
    expect(restored.busy).toBe(false);
    await followup(agent.id, worker.id, original.run.id, message, requestId);
    expect(faux.state.callCount).toBe(2);
    const prompts: Message[][] = [];
    faux.setResponses([(request) => { prompts.push(structuredClone(request.messages)); return fauxAssistantMessage("SECOND discussion answer"); }]);
    await followup(agent.id, worker.id, original.run.id, "SECOND discussion question");
    const second = await answered(agent.id, worker.id, original.run.id, "SECOND discussion answer");
    expect(second.conversationId).toBe(first.conversationId);
    expect(JSON.stringify(prompts[0])).toContain(message);
    expect(JSON.stringify(prompts[0])).toContain("FIRST discussion answer");
    expect(faux.state.callCount).toBe(3);
    expect((await runDetail(agent.id, worker.id, original.run.id)).run).toEqual(original.run);
    expect((await runDetail(agent.id, worker.id, original.run.id)).entries).toEqual(original.entries);
  });

  it("rejects changed content for an existing discussion request ID", async () => {
    const agent = await createAgent(), worker = await createWorker(agent.id);
    faux.setResponses([fauxAssistantMessage("Original result")]);
    const original = await completedRun(agent.id, worker.id);
    const requestId = randomUUID(), response = gate();
    faux.setResponses([async () => { await response.promise; return fauxAssistantMessage("Discussion answer"); }]);
    try {
      await followup(agent.id, worker.id, original.run.id, "Original question", requestId);
      await vi.waitFor(() => expect(faux.state.callCount).toBe(2), waitOptions);
      await expect(followup(agent.id, worker.id, original.run.id, "Changed question", requestId)).rejects.toMatchObject({ status: 409 });
    } finally { response.resolve(); }
    await answered(agent.id, worker.id, original.run.id, "Discussion answer");
    await runtime.close(); runtime = await createRuntime(config(), options()); runtime.start();
    await expect(followup(agent.id, worker.id, original.run.id, "Changed after restart", requestId)).rejects.toMatchObject({ status: 409 });
    await followup(agent.id, worker.id, original.run.id, "Original question", requestId);
    expect(faux.state.callCount).toBe(2);
  });

  it("rejects the wrong owner, Agent, worker, missing run, and settled runs without a started conversation", async () => {
    const agent = await createAgent(), otherAgent = await createAgent();
    const worker = await createWorker(agent.id), otherWorker = await createWorker(agent.id);
    faux.setResponses([fauxAssistantMessage("Original result")]);
    const original = await completedRun(agent.id, worker.id);
    for (const [userId, agentId, workerId, runId] of [
      ["other", agent.id, worker.id, original.run.id], ["owner", otherAgent.id, worker.id, original.run.id],
      ["owner", agent.id, otherWorker.id, original.run.id], ["owner", agent.id, worker.id, "missing"],
    ] as const) await expect(runtime.agents.subagents.sendRunFollowup(userId, agentId, workerId, runId, "Denied", randomUUID())).rejects.toMatchObject({ status: 404 });
    for (const status of ["skipped", "failed", "cancelled"] as const) {
      const id = randomUUID();
      await runtime.db.insert(hvSubagentRuns).values({
        id, agentId: agent.id, subagentId: worker.id, requestId: randomUUID(), source: "manual", task: "Never started",
        config: worker, status, result: "No conversation was started", createdAt: Date.now(),
      }).run();
      const unavailable = await runDetail(agent.id, worker.id, id);
      expect(unavailable.discussion).toBeUndefined();
      expect(unavailable.discussionUnavailable).toEqual(expect.any(String));
      await expect(followup(agent.id, worker.id, id, "Denied")).rejects.toMatchObject({ status: 409 });
    }
    expect(faux.state.callCount).toBe(1);
    expect((await runDetail(agent.id, worker.id, original.run.id)).discussion).toBeUndefined();
  });

  it.each(["queued", "running"] as const)("rejects a %s assignment before it settles", async (status) => {
    const agent = await createAgent(), worker = await createWorker(agent.id);
    const beforeStart = gate(), response = gate();
    const service = runtime.agents.subagents, started = service.started.bind(service);
    const starting = vi.spyOn(service, "started").mockImplementation(async (...args) => {
      if (status === "queued") await beforeStart.promise;
      return started(...args);
    });
    faux.setResponses([async () => { await response.promise; return fauxAssistantMessage("Settled later"); }]);
    let runId = "";
    try {
      const run = await service.run("owner", agent.id, worker.id, "Active assignment", randomUUID()); runId = run.id;
      await vi.waitFor(() => expect(starting).toHaveBeenCalledOnce(), waitOptions);
      if (status === "running") await vi.waitFor(() => expect(faux.state.callCount).toBe(1), waitOptions);
      expect((await service.internalRun(agent.id, run.id)).status).toBe(status);
      await expect(followup(agent.id, worker.id, run.id, "Too soon")).rejects.toMatchObject({ status: 409 });
      expect((await runDetail(agent.id, worker.id, run.id)).discussion).toBeUndefined();
    } finally { beforeStart.resolve(); response.resolve(); }
    await vi.waitFor(async () => expect((await service.internalRun(agent.id, runId)).status).toBe("completed"), waitOptions);
  });

  it("pushes branch messages, streamed partials, and completion through watchRun without client polling", async () => {
    const agent = await createAgent(), worker = await createWorker(agent.id);
    faux.setResponses([fauxAssistantMessage("Original result")]);
    const original = await completedRun(agent.id, worker.id);
    const explanation = "Streamed discussion explanation with several visible partial updates.";
    faux.setResponses([fauxAssistantMessage(explanation)]);
    const source = new AbortController(), snapshots: RunDetail[] = [];
    const watching = runtime.agents.subagents.watchRun("owner", agent.id, worker.id, original.run.id, source.signal, (snapshot) => { snapshots.push(structuredClone(snapshot)); });
    try {
      await vi.waitFor(() => expect(snapshots.length).toBeGreaterThan(0), waitOptions);
      expect(snapshots[0]?.discussion).toBeUndefined();
      await followup(agent.id, worker.id, original.run.id, "Explain through the live discussion");
      // Observe only published snapshots. No runDetail refresh or scheduler tick drives this subscription.
      await vi.waitFor(() => {
        expect(snapshots.some((snapshot) => JSON.stringify(snapshot.discussion?.entries ?? []).includes("Explain through the live discussion"))).toBe(true);
        expect(snapshots.some((snapshot) => snapshot.discussion?.busy && snapshot.discussion.partial.length > 0 && snapshot.discussion.partial.length < explanation.length)).toBe(true);
        const latest = snapshots.at(-1)?.discussion;
        expect(latest?.busy).toBe(false);
        expect(JSON.stringify(latest?.entries)).toContain(explanation);
      }, waitOptions);
      for (const snapshot of snapshots) {
        expect(snapshot.run).toEqual(original.run);
        expect(snapshot.entries).toEqual(original.entries);
      }
    } finally { source.abort(); await watching; }
    const count = snapshots.length;
    faux.setResponses([fauxAssistantMessage("After disconnect")]);
    await followup(agent.id, worker.id, original.run.id, "Another question after disconnect");
    await answered(agent.id, worker.id, original.run.id, "After disconnect");
    expect(snapshots).toHaveLength(count);
  }, 15000);

  it("stops an ownerless discussion when the parent Agent is disabled", async () => {
    const agent = await createAgent(), worker = await createWorker(agent.id);
    faux.setResponses([fauxAssistantMessage("Original result")]);
    const original = await completedRun(agent.id, worker.id);
    const response = gate();
    let aborted = false;
    faux.setResponses([async (_request, options) => {
      const signal = options?.signal;
      const onAbort = () => { aborted = true; response.resolve(); };
      signal?.addEventListener("abort", onAbort, { once: true });
      try {
        if (signal?.aborted) onAbort();
        await response.promise;
        return fauxAssistantMessage("This explanation must not finish");
      } finally { signal?.removeEventListener("abort", onAbort); }
    }]);
    try {
      await followup(agent.id, worker.id, original.run.id, "Explain until the Agent is disabled");
      await vi.waitFor(() => expect(faux.state.callCount).toBe(2), waitOptions);
      expect((await runDetail(agent.id, worker.id, original.run.id)).discussion?.busy).toBe(true);
      await runtime.agents.setEnabled("owner", agent.id, false);
      expect(aborted).toBe(true);
      await vi.waitFor(async () => expect((await runDetail(agent.id, worker.id, original.run.id)).discussion?.busy).toBe(false), waitOptions);
      const after = await runDetail(agent.id, worker.id, original.run.id);
      expect(after.run).toEqual(original.run);
      expect(after.entries).toEqual(original.entries);
      expect(JSON.stringify(after.discussion?.entries)).not.toContain("This explanation must not finish");
      expect((await runtime.agents.owned("owner", agent.id)).enabled).toBe(false);
      expect(faux.state.callCount).toBe(2);
    } finally { response.resolve(); }
  });

  it("exposes model failures in the discussion without replacing the original result or transcript", async () => {
    const agent = await createAgent(), worker = await createWorker(agent.id);
    faux.setResponses([fauxAssistantMessage("Original result")]);
    const original = await completedRun(agent.id, worker.id);
    faux.setResponses([() => { throw new Error("Injected discussion model failure"); }]);
    await followup(agent.id, worker.id, original.run.id, "Explain despite a provider failure");
    await vi.waitFor(async () => {
      const detail = await runDetail(agent.id, worker.id, original.run.id);
      expect(detail.discussion?.busy).toBe(false);
      expect(detail.discussion?.error).toContain("Injected discussion model failure");
      expect(detail.run).toEqual(original.run);
      expect(detail.entries).toEqual(original.entries);
    }, waitOptions);
  });
});
