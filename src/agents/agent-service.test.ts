import { createModels } from "@earendil-works/pi-ai/models";
import { fauxAssistantMessage, fauxProvider, fauxToolCall } from "@earendil-works/pi-ai/providers/faux";
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

describe("per-Agent durable execution", () => {
  let directory: string;
  let runtime: AppRuntime;
  let faux: ReturnType<typeof fauxProvider>;
  let models: ReturnType<typeof createModels>;
  const config = () => testConfig({ APP_SQLITE_PATH: join(directory, "app.sqlite") });
  const options = () => ({ models, model: { provider: faux.getModel().provider, modelId: faux.getModel().id }, accountFeedFactory: () => ({ close: async () => {} }), journalHistoryFetcher: async () => [] });
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
    const toolNames = ["get_account", "submit_orders", "cancel_orders", "cancel_all_orders", "list_orders", "get_order", "get_order_request", "list_subagents", "delegate_subagent", "get_subagent_run"];
    expect((await root.agent(context)).tools.map((tool) => tool.name)).toEqual(toolNames);
    await root.configure({ extensions: [], tools: [], instructions: "Trading and live exchange data are not configured." }, context);
    await runtime.close();
    runtime = await createRuntime(config(), options());
    const restored = await (await opened.mock.results[1]!.value as Harness).root(context);
    const resolved = await restored.agent(context);
    expect(restored.id).toBe(agent.conversationId);
    expect(resolved.extensions.map((extension) => extension.name)).toEqual(["hyperliquid", "subagents"]);
    expect(resolved.tools.map((tool) => tool.name)).toEqual(toolNames);
    expect(resolved.instructions).not.toContain("not configured");
    expect(resolved.model).toEqual(options().model);
    expect(faux.state.callCount).toBe(0);
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
    expect((await runtime.agents.chat("owner", agent.id)).queued).toBe(20);
    await runtime.agents.stop("owner", agent.id);
    const stopped = await runtime.agents.chat("owner", agent.id);
    expect(stopped.busy).toBe(false);
    expect(stopped.queued).toBe(0);
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
    await runtime.close();
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
      const runtime = await createRuntime(${JSON.stringify(config())}, { models, model: { provider: 'faux', modelId: 'faux-1' } });
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
  });

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
});
