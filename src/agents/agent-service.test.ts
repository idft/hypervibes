import { createModels } from "@earendil-works/pi-ai/models";
import { fauxAssistantMessage, fauxProvider, fauxToolCall, type FauxResponseStep } from "@earendil-works/pi-ai/providers/faux";
import { BACKGROUND_CONTEXT as context } from "@earendil-works/chord/context";
import { Harness } from "@earendil-works/pi-durable";
import { SqliteStorage } from "@earendil-works/pi-durable/storage/sqlite";
import { eq } from "drizzle-orm";
import { HttpTransport } from "@nktkas/hyperliquid";
import { loadAccountStatus } from "../hyperliquid/account-status.js";
import { createHash, randomUUID } from "node:crypto";
import { mkdtempSync, rmSync, utimesSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { hvAgents } from "../db/agent-schema.js";
import { hvUsers } from "../db/auth-schema.js";
import { createRuntime, type AppRuntime } from "../runtime.js";
import { testConfig } from "../test-config.js";
import { agentId, agentName, chatMessage, harnessPrefix, requestId } from "./agent-input.js";

/** A faux model response that stays pending until `hold` settles or the request is aborted. */
const heldResponse = (text: string, hold: Promise<void>): FauxResponseStep =>
  async (_context, options) => {
    await Promise.race([hold, new Promise((resolve) => options?.signal?.addEventListener("abort", resolve, { once: true }))]);
    return fauxAssistantMessage(text);
  };

describe("per-Agent durable execution", () => {
  let directory: string;
  let runtime: AppRuntime;
  let faux: ReturnType<typeof fauxProvider>;
  let models: ReturnType<typeof createModels>;
  const config = () => testConfig({ APP_SQLITE_PATH: join(directory, "app.sqlite") });
  const options = () => ({ models, model: { provider: faux.getModel().provider, modelId: faux.getModel().id }, accountFeedFactory: () => ({ close: async () => {} }), journalFeedFactory: () => ({ close: async () => {} }), journalHistoryFetcher: async () => [] });
  const create = (name = "BTC Momentum", id = randomUUID(), user = "owner") => runtime.agents.create(user, name, id, `0x${createHash("sha256").update(`${user}/${id}`).digest("hex").slice(0, 40)}`);
  beforeEach(async () => {
    directory = mkdtempSync(join(tmpdir(), "hv-agents-"));
    faux = fauxProvider();
    models = createModels();
    models.setProvider(faux.provider);
    runtime = await createRuntime(config(), options());
    await runtime.db.insert(hvUsers).values([
      { id: "owner", name: "Owner", email: "owner@example.test" },
      { id: "other", name: "Other", email: "other@example.test" },
    ]).run();
    runtime.start();
  });
  afterEach(async () => {
    vi.restoreAllMocks();
    await runtime.close();
    rmSync(directory, { recursive: true, force: true });
  });

  it("requires a valid linked account before persisting or initializing an agent", async () => {
    for (const account of ["", "none", "invalid", undefined]) {
      await expect(runtime.agents.create("owner", "Invalid", randomUUID(), account as string)).rejects.toThrow("linked Hyperliquid trading account is required");
    }
    expect(await runtime.agents.list("owner")).toEqual([]);
    expect(faux.state.callCount).toBe(0);
  });

  it("persists creation thinking for the orchestrator and default workers across retries and restart", async () => {
    const thinking = fauxProvider({ models: [{ id: "thinking", reasoning: true }] });
    models.setProvider(thinking.provider);
    const selection = { provider: thinking.getModel().provider, modelId: thinking.getModel().id };
    const id = randomUUID(), account = `0x${"b".repeat(40)}`;
    const agent = await runtime.agents.create("owner", "Thinking", id, account, selection, true, "high");
    expect((await runtime.agents.chat("owner", agent.id)).thinkingLevel).toBe("high");
    const definitions = await runtime.agents.subagents.list("owner", agent.id);
    expect(definitions).toHaveLength(4);
    expect(definitions.every((row) => row.thinkingLevel === "high")).toBe(true);
    expect(definitions.find((row) => row.kind === "orchestrator")!.conversationId).toBe(agent.conversationId);
    await runtime.agents.create("owner", "Thinking", id, account, selection, true, "high");
    await runtime.close();
    runtime = await createRuntime(config(), options());
    expect((await runtime.agents.chat("owner", agent.id)).thinkingLevel).toBe("high");
  });

  it("rejects unsupported creation thinking before persisting an agent", async () => {
    await expect(runtime.agents.create("owner", "Invalid", randomUUID(), `0x${"b".repeat(40)}`, options().model, false, "invalid")).rejects.toThrow("supported thinking level");
    expect(await runtime.agents.list("owner")).toEqual([]);
  });

  it("creates one idle root without calling a model; isolates harnesses in the same SQLite file", async () => {
    const [one, two] = await Promise.all([create(), create("ETH Research")]);
    expect(one.state).toBe("ready");
    expect(one.conversationId).not.toBeNull();
    expect((await runtime.agents.chat("owner", one.id)).messages).toEqual([]);
    expect(faux.state.callCount).toBe(0);
    const tables = await runtime.database.all<{ name: string }>("SELECT name FROM sqlite_schema WHERE type = 'table'");
    expect(tables.some(({ name }) => name.startsWith(harnessPrefix(one.id)))).toBe(true);
    expect(tables.some(({ name }) => name.startsWith(harnessPrefix(two.id)))).toBe(true);
    faux.setResponses([fauxAssistantMessage("Only Agent one sees this")]);
    await runtime.agents.send("owner", one.id, "Hello", randomUUID());
    await vi.waitFor(async () => expect((await runtime.agents.chat("owner", one.id)).messages.some((entry) => entry.text === "Only Agent one sees this")).toBe(true));
    expect((await runtime.agents.chat("owner", two.id)).messages).toEqual([]);
  });

  it("enables the native account tool for old roots on reopen without changing model or history", async () => {
    const opened = vi.spyOn(Harness, "open");
    const agent = await create();
    const harness = await opened.mock.results[0]!.value as Harness;
    const root = await harness.root(context);
    const toolNames = ["get_account", "get_market_snapshot", "prepare_protected_entry", "prepare_position_protection", "get_protection_status", "submit_orders", "cancel_orders", "cancel_all_orders", "list_orders", "get_order", "get_order_request", "list_subagents", "delegate_subagent", "list_subagent_runs", "get_subagent_run", "record_trade_decision", "list_memories", "get_memory", "get_trading_context", "list_instruments"];
    expect((await root.agent(context)).tools.map((tool) => tool.name)).toEqual(toolNames);
    await root.configure({ extensions: [], tools: [], instructions: "Trading and live exchange data are not configured." }, context);
    await runtime.close();
    runtime = await createRuntime(config(), options());
    const restored = await (await opened.mock.results[1]!.value as Harness).root(context);
    const resolved = await restored.agent(context);
    expect(restored.id).toBe(agent.conversationId);
    expect(resolved.extensions.map((extension) => extension.name)).toEqual(["hyperliquid", "subagents", "memory", "research", "orchestrator-insights"]);
    expect(resolved.tools.map((tool) => tool.name)).toEqual(toolNames);
    expect(resolved.instructions).not.toContain("not configured");
    expect(resolved.model).toEqual(options().model);
    expect(faux.state.callCount).toBe(0);
  });

  it("binds instrument discovery to each Agent and reads updated allowlists on subsequent calls", async () => {
    const one = await create(), two = await create("Second", randomUUID(), "other");
    const catalog = vi.spyOn(runtime.hyperliquid.charts, "instruments").mockResolvedValue(["BTC", "ETH"]);
    for (const [index, [agent, owner, allowed]] of ([[one, "owner", "BTC"], [two, "other", "ETH"], [one, "owner", "ETH"]] as const).entries()) {
      await runtime.agents.risk.save(agent.id, { tradingEnabled: false, allowedInstruments: [allowed], maxOrderNotional: null, maxTotalNotional: null });
      const answer = `Catalog checked ${index}`;
      faux.setResponses([
        fauxAssistantMessage(fauxToolCall("list_instruments", {}), { stopReason: "toolUse" }),
        (transcript) => {
          const result = transcript.messages.filter((message) => message.role === "toolResult").at(-1);
          expect(result).toMatchObject({ toolName: "list_instruments", isError: false, content: [{ type: "text", text: JSON.stringify({ instruments: [
            { symbol: "BTC", allowedForTrading: allowed === "BTC" }, { symbol: "ETH", allowedForTrading: allowed === "ETH" },
          ] }) }] });
          return fauxAssistantMessage(answer);
        },
      ]);
      await runtime.agents.send(owner, agent.id, "List instruments", randomUUID());
      await vi.waitFor(async () => expect((await runtime.agents.chat(owner, agent.id)).messages.some((entry) => entry.text === answer)).toBe(true));
    }
    expect(catalog).toHaveBeenCalledTimes(3);
  });

  it("executes get_account through Durable with a trusted per-Agent binding", async () => {
    const one = await create();
    const two = await create("Second", randomUUID(), "other");
    const empty = await loadAccountStatus(new HttpTransport(), null);
    const status = vi.spyOn(runtime.hyperliquid, "agentAccountStatus").mockImplementation(async (owner, id) => ({ ...empty, address: (await runtime.agents.owned(owner, id)).tradingAccountAddress, errors: [] }));
    for (const [agent, owner] of [[one, "owner"], [two, "other"]] as const) {
      faux.setResponses([
        fauxAssistantMessage(fauxToolCall("get_account", {}), { stopReason: "toolUse" }),
        (transcript) => {
          const result = transcript.messages.find((message) => message.role === "toolResult");
          expect(result).toMatchObject({ role: "toolResult", toolName: "get_account", isError: false });
          expect(JSON.stringify(result)).toContain(agent.tradingAccountAddress);
          return fauxAssistantMessage("Account checked");
        },
      ]);
      await runtime.agents.send(owner, agent.id, "Check my account", randomUUID());
      await vi.waitFor(async () => expect((await runtime.agents.chat(owner, agent.id)).messages.some((entry) => entry.text === "Account checked")).toBe(true));
      const chat = await runtime.agents.chat(owner, agent.id);
      expect(chat.messages.map((entry) => entry.role)).toEqual(["user", "tool", "assistant"]);
      expect(chat.messages[1]).toMatchObject({ tool: { name: "get_account", status: "done", arguments: "{}", output: expect.any(String) } });
      const history = await runtime.agents.chatHistory(owner, agent.id);
      expect(history.messages).toEqual(chat.messages);
      expect(status).toHaveBeenLastCalledWith(owner, agent.id, expect.any(AbortSignal));
    }
    expect(status).toHaveBeenCalledTimes(2);
    faux.setResponses([
      fauxAssistantMessage(fauxToolCall("get_account", { address: "another-account" }), { stopReason: "toolUse" }),
      fauxAssistantMessage("Invalid arguments rejected"),
    ]);
    await runtime.agents.send("owner", one.id, "Try another account", randomUUID());
    await vi.waitFor(async () => expect((await runtime.agents.chat("owner", one.id)).messages.some((entry) => entry.text === "Invalid arguments rejected")).toBe(true));
    expect(status).toHaveBeenCalledTimes(2);
  });

  it("deduplicates concurrent creation and rejects reuse with a different name", async () => {
    const id = randomUUID();
    const [first, second] = await Promise.all([create("Same", id), create("Same", id)]);
    expect(first.id).toBe(second.id);
    expect(await runtime.agents.list("owner")).toHaveLength(1);
    await expect(create("Different", id)).rejects.toMatchObject({ status: 409 });
    expect((await create("Same", id, "other")).id).not.toBe(first.id);
  });

  it("scopes list, chat, retry, message and cancellation to the owner", async () => {
    const agent = await create();
    expect(await runtime.agents.list("other")).toEqual([]);
    for (const operation of [
      () => runtime.agents.chat("other", agent.id),
      () => runtime.agents.retry("other", agent.id),
      () => runtime.agents.send("other", agent.id, "hello", randomUUID()),
      () => runtime.agents.stop("other", agent.id),
    ]) await expect(operation()).rejects.toMatchObject({ status: 404 });
    expect(faux.state.callCount).toBe(0);
  });

  it("bounds concurrent inbox admission and stops active and queued work", async () => {
    faux = fauxProvider({ tokensPerSecond: 1 });
    models.setProvider(faux.provider);
    faux.setResponses([fauxAssistantMessage("A long streaming response that remains active while inputs are queued.")]);
    const agent = await create();
    await runtime.agents.send("owner", agent.id, "Start", randomUUID());
    await vi.waitFor(() => expect(faux.state.callCount).toBe(1));
    const results = await Promise.allSettled(Array.from({ length: 25 }, (_, index) => runtime.agents.send("owner", agent.id, `Queued ${index}`, randomUUID())));
    expect(results.filter((result) => result.status === "fulfilled")).toHaveLength(20);
    expect(results.filter((result) => result.status === "rejected")).toHaveLength(5);
    const queued = (await runtime.agents.chat("owner", agent.id)).queuedMessages;
    expect(queued).toHaveLength(20);
    expect(queued.every((item) => item.text.startsWith("Queued "))).toBe(true);
    await runtime.agents.stop("owner", agent.id);
    const stopped = await runtime.agents.chat("owner", agent.id);
    expect(stopped.busy).toBe(false);
    expect(stopped.queuedMessages).toEqual([]);
    expect(faux.state.callCount).toBe(1);
  }, 15_000);

  it("persists chat and deduplicates message retries across a backend restart", async () => {
    const agent = await create();
    const id = randomUUID();
    faux.setResponses([fauxAssistantMessage("Hello back")]);
    const [first, second] = await Promise.all([
      runtime.agents.send("owner", agent.id, "Hello", id), runtime.agents.send("owner", agent.id, "Hello", id),
    ]);
    expect(first).toEqual(second);
    await vi.waitFor(async () => expect((await runtime.agents.chat("owner", agent.id)).messages.map((entry) => entry.text)).toEqual(["Hello", "Hello back"]));
    await runtime.close();
    runtime = await createRuntime(config(), options());
    runtime.start();
    expect((await runtime.agents.owned("owner", agent.id)).conversationId).toBe(agent.conversationId);
    expect(await runtime.agents.send("owner", agent.id, "Hello", id)).toEqual(first);
    expect((await runtime.agents.chat("owner", agent.id)).messages.map((entry) => entry.text)).toEqual(["Hello", "Hello back"]);
    expect(faux.state.callCount).toBe(1);
  });

  it("recovers the same root after an interrupted application provisioning update", async () => {
    const agent = await create();
    await runtime.db.update(hvAgents).set({ state: "provisioning", conversationId: null }).where(eq(hvAgents.id, agent.id)).run();
    await runtime.close();
    runtime = await createRuntime(config(), options());
    const recovered = await runtime.agents.owned("owner", agent.id);
    expect(recovered.state).toBe("ready");
    expect(recovered.conversationId).toBe(agent.conversationId);
    expect(faux.state.callCount).toBe(0);
  });

  it("exposes conversation stats, then starts a fresh context on reset", async () => {
    const agent = await create();
    faux.setResponses([fauxAssistantMessage("Hello back")]);
    await runtime.agents.send("owner", agent.id, "Hello", randomUUID());
    await vi.waitFor(async () => expect((await runtime.agents.chat("owner", agent.id)).messages.map((entry) => entry.text)).toEqual(["Hello", "Hello back"]));
    const before = await runtime.agents.chat("owner", agent.id);
    expect(before.usage.cacheWrite).toBeGreaterThan(0);
    expect(before.usage.output).toBeGreaterThan(0);
    expect(before.context).toEqual({ tokens: expect.any(Number), window: 128000 });
    expect(before.context!.tokens).toBeGreaterThan(0);
    // The whole conversation is visible; there is nothing earlier to load.
    expect(before.history).toBe(false);
    await runtime.agents.reset("owner", agent.id);
    await vi.waitFor(async () => {
      // The live tail is the fresh context: only the reset divider.
      const chat = await runtime.agents.chat("owner", agent.id);
      expect(chat.messages.map((entry) => entry.text)).toEqual(["Context reset"]);
      // The usage ledger and the model window remain visible.
      expect(chat.usage.cacheWrite).toBe(before.usage.cacheWrite);
      expect(chat.context!.window).toBe(128000);
    });
    const history = await runtime.agents.chatHistory("owner", agent.id);
    expect(history.more).toBe(false);
    // The full history stays browsable before the divider.
    expect(history.messages.map((entry) => entry.text)).toEqual(["Hello", "Hello back", "Context reset"]);
    const chat = await runtime.agents.chat("owner", agent.id);
    expect(chat.messages.map((entry) => entry.text)).toEqual(["Context reset"]);
    expect(chat.agent.conversationId).toBe(before.agent.conversationId);
    // The prior session sits below the fresh context and can be scrolled up to.
    expect(chat.history).toBe(true);
  });

  it("pages transcript history below a requested entry", async () => {
    const agent = await create();
    faux.setResponses([fauxAssistantMessage("First reply"), fauxAssistantMessage("Second reply")]);
    await runtime.agents.send("owner", agent.id, "One", randomUUID());
    await vi.waitFor(async () => expect((await runtime.agents.chat("owner", agent.id)).messages.some((entry) => entry.text === "First reply")).toBe(true));
    await runtime.agents.send("owner", agent.id, "Two", randomUUID());
    await vi.waitFor(async () => expect((await runtime.agents.chat("owner", agent.id)).messages.some((entry) => entry.text === "Second reply")).toBe(true));
    const chat = await runtime.agents.chat("owner", agent.id);
    expect(chat.messages.map((entry) => entry.text)).toEqual(["One", "First reply", "Two", "Second reply"]);
    const newest = await runtime.agents.chatHistory("owner", agent.id);
    expect(newest.messages.map((entry) => entry.text)).toEqual(["One", "First reply", "Two", "Second reply"]);
    expect(newest.more).toBe(false);
    const oldestEntryId = Number(chat.messages[0]!.id.split(":")[0]);
    const older = await runtime.agents.chatHistory("owner", agent.id, oldestEntryId);
    expect(older.messages).toEqual([]);
    expect(older.more).toBe(false);
  });

  it("compacts a long conversation on request", async () => {
    const agent = await create();
    const filler = "context ".repeat(1900);
    const answers = Array.from({ length: 8 }, (_, index) => fauxAssistantMessage(`Answer ${index}`));
    faux.setResponses(answers);
    for (let index = 0; index < 8; index++) {
      await runtime.agents.send("owner", agent.id, `${filler} ${index}`, randomUUID());
      await vi.waitFor(() => expect(faux.state.callCount).toBe(index + 1));
    }
    const grown = await runtime.agents.chat("owner", agent.id);
    expect(grown.messages).toHaveLength(16);
    expect(grown.compacting).toBe(false);
    faux.setResponses([fauxAssistantMessage("Summary of the conversation so far")]);
    const outcome = await runtime.agents.compact("owner", agent.id);
    // A real summarization reports through the event stream, not the request.
    expect(outcome.message).toBeUndefined();
    await vi.waitFor(async () => {
      const chat = await runtime.agents.chat("owner", agent.id);
      expect(chat.messages.some((entry) => entry.marker === "compaction")).toBe(true);
      expect(chat.compacting).toBe(false);
    }, { timeout: 10_000 });
    const compacted = await runtime.agents.chat("owner", agent.id);
    // The compacted context starts at the divider; the newest turns stay verbatim.
    expect(compacted.messages[0]!.marker).toBe("compaction");
    expect(compacted.messages.some((entry) => entry.text === "Answer 7")).toBe(true);
    expect(compacted.compactionError).toBeUndefined();
    // The summarized turns are no longer part of the context but remain browsable.
    expect(compacted.messages.some((entry) => entry.text === "Answer 0")).toBe(false);
    const history = await runtime.agents.chatHistory("owner", agent.id);
    expect(history.messages.some((entry) => entry.text === "Answer 0")).toBe(true);
  }, 15_000);

  it("explains a declined compaction on a short conversation", async () => {
    const agent = await create();
    faux.setResponses([fauxAssistantMessage("Hello back")]);
    await runtime.agents.send("owner", agent.id, "Hello", randomUUID());
    await vi.waitFor(async () => expect((await runtime.agents.chat("owner", agent.id)).messages.map((entry) => entry.text)).toEqual(["Hello", "Hello back"]));
    const declined = await runtime.agents.compact("owner", agent.id);
    expect(declined.message).toContain("Nothing to compact yet");
    // The decline left the transcript untouched.
    const chat = await runtime.agents.chat("owner", agent.id);
    expect(chat.compacting).toBe(false);
    expect(chat.compactionError).toBeUndefined();
    expect(chat.messages.some((entry) => entry.marker === "compaction")).toBe(false);
  });

  it("keeps failed provisioning visible, sanitized and retryable", async () => {
    const fail = vi.spyOn(SqliteStorage, "open").mockRejectedValueOnce(new Error("secret credential fixture"));
    const agent = await create();
    expect(agent.state).toBe("failed");
    expect(agent.error).not.toContain("secret");
    fail.mockRestore();
    expect((await runtime.agents.retry("owner", agent.id)).state).toBe("ready");
    expect(await runtime.agents.list("owner")).toHaveLength(1);
  });

  it("marks a failed harness unavailable without closing another Agent or shared SQL", async () => {
    const agent = await create();
    const other = await create("Other Agent");
    vi.spyOn(SqliteStorage.prototype, "commit").mockRejectedValueOnce(new Error("storage failure fixture"));
    await expect(runtime.agents.send("owner", agent.id, "Hello", randomUUID())).rejects.toThrow();
    await vi.waitFor(async () => expect((await runtime.agents.owned("owner", agent.id)).state).toBe("failed"));
    expect((await runtime.agents.owned("owner", agent.id)).error).not.toContain("storage failure fixture");
    expect((await runtime.agents.chat("owner", other.id)).agent.state).toBe("ready");
    expect((await runtime.agents.retry("owner", agent.id)).state).toBe("ready");
  });

  it("resumes unfinished model work only after the host enables execution", async () => {
    faux = fauxProvider({ tokensPerSecond: 20 });
    models.setProvider(faux.provider);
    const agent = await create();
    faux.setResponses([fauxAssistantMessage("A long response that will still be streaming when the application closes."), fauxAssistantMessage("Recovered response")]);
    await runtime.agents.send("owner", agent.id, "Please answer", randomUUID());
    await vi.waitFor(() => expect(faux.state.callCount).toBe(1));
    await runtime.close(AbortSignal.abort());
    runtime = await createRuntime(config(), options());
    expect(faux.state.callCount).toBe(1);
    runtime.start();
    await vi.waitFor(async () => expect((await runtime.agents.chat("owner", agent.id)).messages.some((entry) => entry.text === "Recovered response")).toBe(true), { timeout: 10_000 });
    expect(faux.state.callCount).toBe(2);
  });

  it("recovers admitted work after a process kill and stale ownership lock", async () => {
    const agent = await create();
    await runtime.close();
    const child = spawnSync(process.execPath, ["--import", "tsx", "--input-type=module", "-e", `
      import { createRuntime } from './src/runtime.ts';
      import { createModels } from '@earendil-works/pi-ai/models';
      import { fauxProvider, fauxAssistantMessage } from '@earendil-works/pi-ai/providers/faux';
      const faux = fauxProvider({ tokensPerSecond: 1 });
      faux.setResponses([fauxAssistantMessage('This response must not finish before the process is killed.')]);
      const models = createModels();
      models.setProvider(faux.provider);
      const runtime = await createRuntime(${JSON.stringify(config())}, { models, model: { provider: 'faux', modelId: 'faux-1' }, accountFeedFactory: () => ({ close: async () => {} }), journalFeedFactory: () => ({ close: async () => {} }), journalHistoryFetcher: async () => [] });
      runtime.start();
      await runtime.agents.send('owner', '${agent.id}', 'Survive a crash', '${randomUUID()}');
      while (faux.state.callCount === 0) await new Promise(resolve => setTimeout(resolve, 5));
      process.kill(process.pid, 'SIGKILL');
    `], { cwd: process.cwd(), encoding: "utf8", timeout: 15_000 });
    expect(child.error).toBeUndefined();
    expect(child.signal, child.stderr).toBe("SIGKILL");
    // The process is confirmed dead. Advance only its orphaned heartbeat instead
    // of making this isolated test wait the production 30-second stale interval.
    const past = new Date(Date.now() - 60_000);
    utimesSync(`${config().databasePath}.lock`, past, past);
    faux.setResponses([fauxAssistantMessage("Recovered after process kill")]);
    runtime = await createRuntime(config(), options());
    expect(faux.state.callCount).toBe(0);
    runtime.start();
    await vi.waitFor(async () => expect((await runtime.agents.chat("owner", agent.id)).messages.some((entry) => entry.text === "Recovered after process kill")).toBe(true));
    expect((await runtime.agents.owned("owner", agent.id)).conversationId).toBe(agent.conversationId);
  }, 20000); // Child startup/recovery must outlive its bounded 15s subprocess deadline.

  it("preserves existing agents without credentials but refuses creation and new model calls", async () => {
    const agent = await create();
    await runtime.close();
    runtime = await createRuntime(config());
    runtime.start();
    expect((await runtime.agents.chat("owner", agent.id)).configured).toBe(false);
    await expect(create()).rejects.toMatchObject({ status: 409 });
    await expect(runtime.agents.send("owner", agent.id, "Hello", randomUUID())).rejects.toMatchObject({ status: 409 });
  });

  it("validates IDs, names, message sizes and seals admission on shutdown", async () => {
    expect(() => agentName("   ")).toThrow();
    expect(() => agentName("x".repeat(81))).toThrow();
    expect(() => agentName("bad\nname")).toThrow();
    expect(agentName("  Research  ")).toBe("Research");
    expect(() => chatMessage("x".repeat(16001))).toThrow();
    expect(() => agentId("../../other")).toThrow();
    expect(() => requestId("invalid")).toThrow();
    const closing = runtime.close();
    await expect(create()).rejects.toMatchObject({ status: 503 });
    await closing;
  });

  it("waits for in-flight agent jobs to complete instead of interrupting them", async () => {
    const agent = await create();
    let release!: () => void;
    const hold = new Promise<void>((resolve) => { release = resolve; });
    faux.setResponses([heldResponse("Delayed done", hold)]);
    await runtime.agents.send("owner", agent.id, "Hold this", randomUUID());
    await vi.waitFor(() => expect(faux.state.callCount).toBe(1));
    let closed = false;
    const closing = runtime.close().then(() => { closed = true; });
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(closed).toBe(false); // close is waiting for the in-flight generation
    release();
    await closing;
    expect(closed).toBe(true);
    runtime = await createRuntime(config(), options());
    const chat = await runtime.agents.chat("owner", agent.id);
    expect(chat.messages.some((entry) => entry.text === "Delayed done" && !entry.failed)).toBe(true);
  });

  it("keeps reads and live subscriptions available while draining workers but rejects new execution", async () => {
    const agent = await create();
    const worker = await runtime.agents.subagents.create("owner", agent.id, {
      name: "Research", description: "", instructions: "Research the assignment.",
      provider: "faux", modelId: "faux-1", thinkingLevel: "", tools: [],
      enabled: true, delegatable: true, persistent: false, schedule: "", delaySeconds: 30,
      completion: "save", timeoutSeconds: 900,
    }, randomUUID());
    let release!: () => void;
    const hold = new Promise<void>((resolve) => { release = resolve; });
    const viewer = new AbortController();
    const statuses: string[] = [];
    faux.setResponses([heldResponse("Worker finished during shutdown", hold)]);
    const run = await runtime.agents.subagents.run("owner", agent.id, worker.id, "Hold this", randomUUID());
    await vi.waitFor(() => expect(faux.state.callCount).toBe(1));
    const watching = runtime.agents.subagents.watchRun("owner", agent.id, worker.id, run.id, viewer.signal, (detail) => { statuses.push(detail.run.status); });
    await vi.waitFor(() => expect(statuses).toContain("running"));
    let drained = false;
    const draining = runtime.agents.drain().then(() => { drained = true; });
    try {
      expect(runtime.status().database).toBe("connected");
      expect((await runtime.agents.list("owner"))[0]?.id).toBe(agent.id);
      expect((await runtime.agents.subagents.runDetail("owner", agent.id, worker.id, run.id)).run.status).toBe("running");
      await expect(create()).rejects.toMatchObject({ status: 503 });
      await expect(runtime.agents.send("owner", agent.id, "New work", randomUUID())).rejects.toMatchObject({ status: 503 });
      await expect(runtime.agents.compact("owner", agent.id)).rejects.toMatchObject({ status: 503 });
      await expect(runtime.agents.subagents.run("owner", agent.id, worker.id, "New work", randomUUID())).rejects.toMatchObject({ status: 503 });
      const chats: boolean[] = [];
      const chatWatch = runtime.agents.watchChat("owner", agent.id, viewer.signal, () => { chats.push(true); });
      await vi.waitFor(() => expect(chats.length).toBeGreaterThan(0));
      expect(drained).toBe(false);
      release();
      await draining;
      await vi.waitFor(() => expect(statuses).toContain("completed"));
      expect((await runtime.agents.subagents.runDetail("owner", agent.id, worker.id, run.id)).run.result).toContain("Worker finished during shutdown");
      viewer.abort();
      await chatWatch;
    } finally {
      release();
      viewer.abort();
      await watching;
      await draining;
    }
  });

  it("aborts remaining in-flight agent jobs when shutdown is forced by a signal", async () => {
    const agent = await create();
    faux.setResponses([heldResponse("Never finished", new Promise<void>(() => {}))]);
    await runtime.agents.send("owner", agent.id, "Hold this", randomUUID());
    await vi.waitFor(() => expect(faux.state.callCount).toBe(1));
    const force = new AbortController();
    const closing = runtime.close(force.signal);
    force.abort();
    await closing; // resolves once the forced wait aborts the generation
    runtime = await createRuntime(config(), options());
    const chat = await runtime.agents.chat("owner", agent.id);
    expect(chat.messages.some((entry) => entry.text === "Never finished")).toBe(false);
    expect(chat.messages.some((entry) => entry.text === "Hold this")).toBe(true);
  });

  it("aborts in-flight agent jobs when the shutdown grace period elapses", async () => {
    const agent = await create();
    await runtime.close();
    runtime = await createRuntime(config(), { ...options(), shutdownGraceMs: 25 });
    runtime.start();
    faux.setResponses([heldResponse("Grace elapsed", new Promise<void>(() => {}))]);
    await runtime.agents.send("owner", agent.id, "Hold this", randomUUID());
    await vi.waitFor(() => expect(faux.state.callCount).toBe(1));
    await runtime.close(); // resolves once the 25ms grace abandons the wait
    runtime = await createRuntime(config(), options());
    const chat = await runtime.agents.chat("owner", agent.id);
    expect(chat.messages.some((entry) => entry.text === "Grace elapsed")).toBe(false);
    expect(chat.messages.some((entry) => entry.text === "Hold this")).toBe(true);
  });
});
