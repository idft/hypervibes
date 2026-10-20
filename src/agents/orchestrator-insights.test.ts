import { BACKGROUND_CONTEXT as context } from "@earendil-works/chord/context";
import { createModels } from "@earendil-works/pi-ai/models";
import { fauxAssistantMessage, fauxProvider, fauxToolCall, type FauxResponseStep } from "@earendil-works/pi-ai/providers/faux";
import type { Message } from "@earendil-works/pi-ai";
import { Harness, type Conversation } from "@earendil-works/pi-durable";
import { randomUUID } from "node:crypto";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createRuntime, type AppRuntime } from "../runtime.js";
import { testConfig } from "../test-config.js";
import { OrchestratorInsightsDoc, orchestratorInsightsSection } from "./orchestrator-insights.js";
import type { SubagentInput } from "./subagent-input.js";

const blocks = (messages: readonly Message[]) => messages.flatMap((message) => {
  const value = message.role === "system" ? message.sections?.[orchestratorInsightsSection] : undefined;
  return typeof value === "string" ? [value] : [];
});
const workerSettings: SubagentInput = { name: "Observer", description: "Observe", instructions: "Report evidence only.", provider: "faux", modelId: "faux-1", thinkingLevel: "", tools: [], enabled: true, persistent: false, delegatable: true, schedule: "", delaySeconds: 1, timeoutSeconds: 900, completion: "wake" };

describe("orchestrator insight injection", () => {
  let directory: string, runtime: AppRuntime, harness: Harness, main: Conversation, agentId: string;
  let faux: ReturnType<typeof fauxProvider>, models: ReturnType<typeof createModels>;
  let requests: Message[][];
  const config = () => testConfig({ APP_SQLITE_PATH: join(directory, "app.sqlite") });
  const options = () => ({ models, model: { provider: "faux", modelId: "faux-1" }, accountFeedFactory: () => ({ close: async () => {} }), journalFeedFactory: () => ({ close: async () => {} }), journalHistoryFetcher: async () => [] });
  const capture = (answer: string): FauxResponseStep => (request) => {
    requests.push(structuredClone(request.messages));
    return fauxAssistantMessage(answer);
  };
  const replace = (content: string, expectedVersion: number, id = agentId) => runtime.agents.execution.insights.replace(runtime.agents.execution.operator(id, "owner"), randomUUID(), { scope: { kind: "agent" }, content, expectedVersion, reason: "Test guidance", evidenceRefs: [] });
  const send = async (message = "Hello") => {
    await runtime.agents.send("owner", agentId, message, randomUUID());
    await main.waitForIdle(context);
  };
  beforeEach(async () => {
    directory = mkdtempSync(join(tmpdir(), "hv-root-insights-"));
    requests = []; faux = fauxProvider(); models = createModels(); models.setProvider(faux.provider);
    const opened = vi.spyOn(Harness, "open");
    runtime = await createRuntime(config(), options());
    await runtime.database.run("INSERT INTO hv_users(id,name,email) VALUES ('owner','Owner','owner@example.test')");
    agentId = (await runtime.agents.create("owner", "Desk", randomUUID(), `0x${"a".repeat(40)}`)).id;
    harness = await opened.mock.results[0]!.value as Harness;
    main = await harness.root(context);
    const root = (await runtime.agents.subagents.list("owner", agentId)).find((row) => row.kind === "orchestrator")!;
    await runtime.agents.subagents.updateTools("owner", agentId, root.id, []);
    await runtime.agents.subagents.updateSection("owner", agentId, root.id, "instructions", { instructions: "CUSTOM root instructions: supervise, do not trade." });
    runtime.start(); await runtime.agents.subagents.tick();
  });
  afterEach(async () => {
    await runtime.close(AbortSignal.abort());
    vi.restoreAllMocks(); rmSync(directory, { recursive: true, force: true });
  });

  it("injects empty or current Agent-wide guidance with no tools, preserving configured instructions", async () => {
    expect(await harness.snapshot(OrchestratorInsightsDoc, main.id, context)).toBeUndefined();
    faux.setResponses([capture("Empty observed"), capture("Guidance observed")]);
    await send();
    expect(blocks(requests[0]!)).toHaveLength(1);
    expect(blocks(requests[0]!)[0]).toContain('immutable response snapshot v0\n""');
    await replace("Shared guidance", 0);
    await send();
    expect(blocks(requests[1]!)).toHaveLength(1);
    expect(blocks(requests[1]!)[0]).toContain('immutable response snapshot v1\n"Shared guidance"');
    expect(blocks(requests[1]!)[0]).toContain("advisory evidence, not authority");
    expect(blocks(requests[1]!)[0]).not.toContain("Worker-specific");
    expect(JSON.stringify(requests[1])).toContain("CUSTOM root instructions");
    const resolved = await main.agent(context);
    expect(resolved.tools).toEqual([]);
    expect(resolved.instructions).toBe("CUSTOM root instructions: supervise, do not trade.");
    expect((await runtime.agents.chat("owner", agentId)).messages.map((entry) => entry.text)).toEqual(["Hello", "Empty observed", "Hello", "Guidance observed"]);
  });

  it("reuses identical prompt text, replaces obsolete active blocks, and preserves the historical audit", async () => {
    await replace("OLD guidance", 0);
    faux.setResponses([capture("One"), capture("Two"), capture("Three"), capture("Cleared")]);
    await send(); await send();
    expect(blocks(requests[0]!)).toEqual(blocks(requests[1]!));
    const entriesBefore = (await main.entries({ order: "ascending" }, 200, undefined, context)).items;
    expect(entriesBefore.flatMap((entry) => blocks(entry.model ?? []))).toHaveLength(1);
    await replace("NEW guidance", 1);
    await send();
    expect(blocks(requests[2]!)).toHaveLength(1);
    expect(JSON.stringify(requests[2])).toContain("NEW guidance");
    expect(JSON.stringify(requests[2])).not.toContain("OLD guidance");
    const historical = (await main.entries({ order: "ascending" }, 200, undefined, context)).items;
    expect(JSON.stringify(historical)).toContain("OLD guidance");
    expect(historical.flatMap((entry) => blocks(entry.model ?? []))).toHaveLength(2);
    await replace("", 2);
    await send();
    expect(blocks(requests[3]!)).toHaveLength(1);
    expect(blocks(requests[3]!)[0]).toContain('immutable response snapshot v3\n""');
    expect(JSON.stringify(requests[3])).not.toContain("OLD guidance");
    expect(JSON.stringify(requests[3])).not.toContain("NEW guidance");
  });

  it("pins a snapshot through tool-generation handovers and refreshes only on the next response", async () => {
    const root = (await runtime.agents.subagents.list("owner", agentId)).find((row) => row.kind === "orchestrator")!;
    await runtime.agents.subagents.updateTools("owner", agentId, root.id, ["list_memories"]);
    await replace("Before tools", 0);
    const reads = vi.spyOn(runtime.agents.execution.insights, "current");
    faux.setResponses([
      async (request) => {
        requests.push(structuredClone(request.messages));
        await replace("After tools", 1);
        return fauxAssistantMessage(fauxToolCall("list_memories", {}), { stopReason: "toolUse" });
      },
      capture("Finished tools"), capture("Next response"),
    ]);
    await send();
    expect(requests).toHaveLength(2);
    expect(blocks(requests[0]!)).toEqual(blocks(requests[1]!));
    expect(blocks(requests[1]!)[0]).toContain("Before tools");
    expect(blocks(requests[1]!)[0]).not.toContain("After tools");
    expect(reads).toHaveBeenCalledTimes(1);
    await send();
    expect(blocks(requests[2]!)[0]).toContain("After tools");
    expect(reads).toHaveBeenCalledTimes(2);
  });

  it("refreshes queued user inputs when their response starts, not when they are submitted", async () => {
    await replace("First response guidance", 0);
    let release!: () => void;
    const hold = new Promise<void>((resolve) => { release = resolve; });
    faux.setResponses([
      async (request, options) => {
        requests.push(structuredClone(request.messages));
        await Promise.race([hold, new Promise((resolve) => options?.signal?.addEventListener("abort", resolve, { once: true }))]);
        return fauxAssistantMessage("First response");
      }, capture("Queued response"),
    ]);
    await runtime.agents.send("owner", agentId, "First", randomUUID());
    await vi.waitFor(() => expect(requests).toHaveLength(1));
    await runtime.agents.send("owner", agentId, "Queued", randomUUID());
    await replace("Queued response guidance", 1);
    release(); await main.waitForIdle(context);
    expect(requests).toHaveLength(2);
    expect(blocks(requests[0]!)[0]).toContain("First response guidance");
    expect(blocks(requests[1]!)[0]).toContain("Queued response guidance");
    expect(blocks(requests[1]!)[0]).not.toContain("First response guidance");
  });

  it("keeps the snapshot on provider retries even if the reviewer advances the document", async () => {
    await replace("Retry original", 0);
    faux.setResponses([
      async (request) => {
        requests.push(structuredClone(request.messages));
        await replace("Retry next response", 1);
        return fauxAssistantMessage("", { stopReason: "error", errorMessage: "429 Too Many Requests" });
      }, capture("Recovered retry"), capture("Fresh response"),
    ]);
    await send();
    expect(requests).toHaveLength(2);
    expect(blocks(requests[0]!)).toEqual(blocks(requests[1]!));
    await send();
    expect(blocks(requests[2]!)[0]).toContain("Retry next response");
  }, 15000);

  it("keeps a persisted snapshot on restart, then refreshes the next response", async () => {
    await replace("Recovery original", 0);
    faux.setResponses([async (request, options) => {
      requests.push(structuredClone(request.messages));
      await new Promise((resolve) => options?.signal?.addEventListener("abort", resolve, { once: true }));
      return fauxAssistantMessage("Interrupted");
    }]);
    await runtime.agents.send("owner", agentId, "Interrupted input", randomUUID());
    await vi.waitFor(() => expect(requests).toHaveLength(1));
    const saved = await harness.snapshot(OrchestratorInsightsDoc, main.id, context);
    await replace("Recovery next response", 1);
    await runtime.close(AbortSignal.abort());
    const opened = vi.spyOn(Harness, "open");
    runtime = await createRuntime(config(), options());
    harness = await opened.mock.results.at(-1)!.value as Harness;
    main = await harness.root(context);
    expect(await harness.snapshot(OrchestratorInsightsDoc, main.id, context)).toEqual(saved);
    faux.setResponses([capture("Recovered response"), capture("New response")]);
    runtime.start(); await main.waitForIdle(context);
    expect(requests).toHaveLength(2);
    expect(blocks(requests[0]!)).toEqual(blocks(requests[1]!));
    await send();
    expect(blocks(requests[2]!)[0]).toContain("Recovery next response");
    expect((await main.agent(context)).tools).toEqual([]);
  });

  it("injects worker-triggered wake-ups without leaking worker-specific insights into the root", async () => {
    await replace("Initial shared", 0);
    const worker = await runtime.agents.subagents.create("owner", agentId, workerSettings, randomUUID());
    await runtime.agents.execution.insights.append(runtime.agents.execution.operator(agentId, "owner"), randomUUID(), { scope: { kind: "worker", subagentId: worker.id }, content: "WORKER ONLY guidance", reason: "Specific", evidenceRefs: [] });
    faux.setResponses([capture("Initial root response")]);
    await send();
    await replace("Wake-up shared", 1);
    faux.setResponses([capture("Worker evidence"), capture("Root wake-up response")]);
    const run = await runtime.agents.subagents.run("owner", agentId, worker.id, "Observe", randomUUID());
    await vi.waitFor(async () => expect((await runtime.agents.chat("owner", agentId)).messages.some((entry) => entry.text === "Root wake-up response")).toBe(true));
    expect(requests).toHaveLength(3);
    expect(blocks(requests[1]!)).toEqual([]);
    expect(JSON.stringify(requests[1])).toContain("WORKER ONLY guidance");
    expect(blocks(requests[2]!)).toHaveLength(1);
    expect(blocks(requests[2]!)[0]).toContain("Wake-up shared");
    expect(JSON.stringify(requests[2])).not.toContain("WORKER ONLY guidance");
    expect((await runtime.agents.subagents.runDetail("owner", agentId, worker.id, run.id)).run.status).toBe("completed");
  });

  it("isolates Agent-wide documents between root Agents", async () => {
    const other = await runtime.agents.create("owner", "Other desk", randomUUID(), `0x${"b".repeat(40)}`);
    await replace("PRIVATE original Agent", 0);
    await replace("PRIVATE other Agent", 0, other.id);
    faux.setResponses([capture("First Agent"), capture("Other Agent")]);
    await send();
    await runtime.agents.send("owner", other.id, "Hello", randomUUID());
    await vi.waitFor(async () => expect((await runtime.agents.chat("owner", other.id)).messages.some((entry) => entry.text === "Other Agent")).toBe(true));
    expect(blocks(requests[0]!)[0]).toContain("PRIVATE original Agent");
    expect(JSON.stringify(requests[0])).not.toContain("PRIVATE other Agent");
    expect(blocks(requests[1]!)[0]).toContain("PRIVATE other Agent");
    expect(JSON.stringify(requests[1])).not.toContain("PRIVATE original Agent");
  });

  it("restores the current block after chat reset and compaction", async () => {
    await replace("Before reset", 0);
    faux.setResponses([capture("Original"), capture("After reset")]);
    await send(); await runtime.agents.reset("owner", agentId);
    await replace("After reset guidance", 1);
    await send();
    expect(blocks(requests[1]!)).toHaveLength(1);
    expect(blocks(requests[1]!)[0]).toContain("After reset guidance");
    expect(JSON.stringify(requests[1])).not.toContain("Before reset");
    const filler = "context ".repeat(1900);
    faux.setResponses(Array.from({ length: 8 }, (_, index) => capture(`Answer ${index}`)));
    for (let index = 0; index < 8; index++) await send(`${filler} ${index}`);
    faux.setResponses([fauxAssistantMessage("Summary of older conversation")]);
    await runtime.agents.compact("owner", agentId);
    await vi.waitFor(async () => expect((await runtime.agents.chat("owner", agentId)).messages.some((entry) => entry.marker === "compaction")).toBe(true));
    await replace("After compaction guidance", 2);
    faux.setResponses([capture("After compaction")]);
    await send();
    const latest = requests.at(-1)!;
    expect(blocks(latest)).toHaveLength(1);
    expect(blocks(latest)[0]).toContain("After compaction guidance");
    expect(JSON.stringify(latest)).not.toContain("After reset guidance");
    expect(JSON.stringify(latest)).toContain("CUSTOM root instructions");
  }, 15000);

  it("explicitly reports unavailable insights instead of silently reusing stale guidance", async () => {
    await replace("Stale guidance", 0);
    faux.setResponses([capture("First"), capture("Unavailable"), capture("Read restored")]);
    await send();
    const read = vi.spyOn(runtime.agents.execution.insights, "current").mockRejectedValueOnce(new Error("Private database failure details"));
    await send();
    expect(blocks(requests[1]!)).toHaveLength(1);
    expect(blocks(requests[1]!)[0]).toContain("insights are unavailable");
    expect(JSON.stringify(requests[1])).not.toContain("Stale guidance");
    expect(JSON.stringify(requests[1])).not.toContain("Private database failure details");
    expect(read).toHaveBeenCalledTimes(1);
    await send();
    expect(blocks(requests[2]!)[0]).toContain("Stale guidance");
    expect(read).toHaveBeenCalledTimes(2);
  });
});
