import { BACKGROUND_CONTEXT as context } from "@earendil-works/chord/context";
import { createModels } from "@earendil-works/pi-ai/models";
import { fauxAssistantMessage, fauxProvider } from "@earendil-works/pi-ai/providers/faux";
import { Harness } from "@earendil-works/pi-durable";
import { eq } from "drizzle-orm";
import { randomUUID } from "node:crypto";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { hvAgents } from "../db/agent-schema.js";
import { hvUsers } from "../db/auth-schema.js";
import { createRuntime, type AppRuntime } from "../runtime.js";
import { testConfig } from "../test-config.js";
import { harnessPrefix } from "./agent-input.js";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

const account = `0x${"a".repeat(40)}`;
const otherAccount = `0x${"b".repeat(40)}`;
describe("agent deletion", () => {
  let directory: string, runtime: AppRuntime;
  let faux: ReturnType<typeof fauxProvider>, models: ReturnType<typeof createModels>;
  const accountClosed = vi.fn(), journalClosed = vi.fn(), accountOpened = vi.fn(), journalOpened = vi.fn();
  const config = () => testConfig({ APP_SQLITE_PATH: join(directory, "app.sqlite") });
  const options = () => ({
    models, model: { provider: faux.getModel().provider, modelId: faux.getModel().id },
    accountFeedFactory: (address: string) => { accountOpened(address); return { close: async () => { accountClosed(address); } }; },
    journalFeedFactory: (address: string) => { journalOpened(address); return { close: async () => { journalClosed(address); } }; },
    journalHistoryFetcher: async () => [],
  });
  const create = (address = account) => runtime.agents.create("owner", "Trader", randomUUID(), address);
  beforeEach(async () => {
    vi.clearAllMocks();
    directory = mkdtempSync(join(tmpdir(), "hv-agent-deletion-"));
    faux = fauxProvider(); models = createModels(); models.setProvider(faux.provider);
    runtime = await createRuntime(config(), options());
    await runtime.db.insert(hvUsers).values([{ id: "owner", name: "Owner", email: "owner@example.test" }, { id: "other", name: "Other", email: "other@example.test" }]).run();
    runtime.start();
  });
  afterEach(async () => { vi.restoreAllMocks(); await runtime.close(); rmSync(directory, { recursive: true, force: true }); });

  it("stops live execution and feeds, preserves Pi storage and other agents, and permits immediate account reuse", async () => {
    const agent = await create(), other = await create(otherAccount);
    faux.setResponses([async (_context, options) => {
      await new Promise<void>((resolve) => { options?.signal?.addEventListener("abort", () => resolve(), { once: true }); });
      return fauxAssistantMessage("aborted");
    }]);
    await runtime.agents.send("owner", agent.id, "Start", randomUUID());
    await vi.waitFor(() => expect(faux.state.callCount).toBe(1));
    await vi.waitFor(() => expect(journalOpened).toHaveBeenCalledWith(account));
    const lifetime = runtime.agents.signal(agent.id);
    await Promise.all([runtime.agents.delete("owner", agent.id), runtime.agents.delete("owner", agent.id)]);
    expect(lifetime.aborted).toBe(true);
    expect(accountClosed).toHaveBeenCalledWith(account);
    expect(journalClosed).toHaveBeenCalledWith(account);
    expect(accountClosed).not.toHaveBeenCalledWith(otherAccount);
    expect((await runtime.agents.list("owner")).map((row) => row.id)).toEqual([other.id]);
    const tables = await runtime.database.all<{ name: string }>("SELECT name FROM sqlite_schema WHERE type='table'");
    expect(tables.some((table) => table.name.startsWith(harnessPrefix(agent.id)))).toBe(true);
    await expect(runtime.agents.send("owner", agent.id, "No", randomUUID())).rejects.toMatchObject({ status: 404 });
    const replacement = await create();
    expect(replacement.id).not.toBe(agent.id);
    expect((await runtime.agents.chat("owner", replacement.id)).messages).toEqual([]);
    expect(accountOpened.mock.calls.filter(([address]) => address === account)).toHaveLength(2);
    await vi.waitFor(() => expect(journalOpened.mock.calls.filter(([address]) => address === account)).toHaveLength(2));
    await runtime.close();
    const opened = vi.spyOn(Harness, "open");
    runtime = await createRuntime(config(), options());
    expect(opened).toHaveBeenCalledTimes(2);
    expect(await runtime.database.all("PRAGMA foreign_key_check")).toEqual([]);
  });

  it("checks ownership before stopping anything", async () => {
    const agent = await create();
    await expect(runtime.agents.delete("other", agent.id)).rejects.toMatchObject({ status: 404 });
    expect(runtime.agents.signal(agent.id).aborted).toBe(false);
    expect((await runtime.agents.owned("owner", agent.id)).state).toBe("ready");
  });

  it("retains the account and exposes retry when harness shutdown fails", async () => {
    const opened = vi.spyOn(Harness, "open");
    const agent = await create();
    const harness = await opened.mock.results[0]!.value as Harness;
    vi.spyOn(harness, "close").mockRejectedValueOnce(new Error("shutdown failure"));
    await expect(runtime.agents.delete("owner", agent.id)).rejects.toThrow("shutdown failed");
    expect(await runtime.agents.owned("owner", agent.id, true)).toMatchObject({ state: "stopping", tradingAccountAddress: account, error: expect.stringContaining("Retry deletion") });
    expect((await runtime.agents.chat("owner", agent.id)).agent.state).toBe("stopping");
    await expect(create()).rejects.toThrow("already assigned");
    await expect(runtime.agents.retry("owner", agent.id)).rejects.toMatchObject({ status: 409 });
    await expect(runtime.hyperliquid.agentTradingAccount("owner", agent.id)).rejects.toMatchObject({ status: 409 });
    await expect(runtime.agents.risk.save(agent.id, { tradingEnabled: true, allowedInstruments: null, maxOrderNotional: null, maxTotalNotional: null })).rejects.toThrow("being deleted");
    await runtime.agents.delete("owner", agent.id);
    expect((await create()).state).toBe("ready");
  });

  it("keeps the account reserved until order operations finish, with no new admissions", async () => {
    const agent = await create();
    const original = runtime.orders.stopAgent.bind(runtime.orders);
    const held = deferred<void>();
    vi.spyOn(runtime.orders, "stopAgent").mockImplementation(async (id) => { await original(id); await held.promise; });
    const deleting = runtime.agents.delete("owner", agent.id);
    try {
      await vi.waitFor(async () => expect((await runtime.agents.owned("owner", agent.id, true)).state).toBe("stopping"));
      await expect(create()).rejects.toThrow("already assigned");
      await expect(runtime.agents.send("owner", agent.id, "No", randomUUID())).rejects.toMatchObject({ status: 409 });
      expect(() => runtime.agents.subagents.serial(agent.id, async () => {})).toThrow("being deleted");
    } finally { held.resolve(); await deleting; }
    expect((await create()).state).toBe("ready");
  });

  it.each(["stopping", "stopped"] as const)("finishes persisted %s deletion at startup without opening its harness or feeds", async (state) => {
    const agent = await create();
    await runtime.db.update(hvAgents).set({ state }).where(eq(hvAgents.id, agent.id)).run();
    await runtime.close();
    accountOpened.mockClear(); journalOpened.mockClear();
    const opened = vi.spyOn(Harness, "open");
    runtime = await createRuntime(config(), options());
    runtime.start();
    expect(opened).not.toHaveBeenCalled();
    expect(await runtime.agents.list("owner")).toEqual([]);
    expect(accountOpened).not.toHaveBeenCalled();
    expect(journalOpened).not.toHaveBeenCalled();
    expect((await create()).state).toBe("ready");
  });

  it("does not release an account when feed teardown fails", async () => {
    const agent = await create();
    const reconcile = vi.spyOn(runtime.hyperliquid, "reconcileLinkedAccounts").mockRejectedValueOnce(new Error("feed failure"));
    await expect(runtime.agents.delete("owner", agent.id)).rejects.toThrow("feed failure");
    expect((await runtime.agents.owned("owner", agent.id, true)).state).toBe("stopped");
    await expect(create()).rejects.toThrow("already assigned");
    reconcile.mockRestore();
    await runtime.agents.delete("owner", agent.id);
  });

  it("retains partially initialized harnesses when their cleanup fails", async () => {
    const original = Harness.open.bind(Harness);
    const entered = deferred<Harness>(), release = deferred<void>();
    vi.spyOn(Harness, "open").mockImplementationOnce(async (...args) => {
      const harness = await original(...args);
      entered.resolve(harness);
      await release.promise;
      return harness;
    });
    const creating = create().catch((error: unknown) => error);
    const harness = await entered.promise;
    const close = vi.spyOn(harness, "close").mockRejectedValue(new Error("cleanup failure"));
    const agent = (await runtime.agents.list("owner"))[0]!;
    const deleting = runtime.agents.delete("owner", agent.id);
    const failed = expect(deleting).rejects.toThrow("shutdown failed");
    await vi.waitFor(() => expect(runtime.agents.signal(agent.id).aborted).toBe(true));
    release.resolve();
    await failed; await creating;
    expect((await runtime.agents.owned("owner", agent.id, true)).state).toBe("stopping");
    await expect(create()).rejects.toThrow("already assigned");
    close.mockRestore();
    await runtime.agents.delete("owner", agent.id);
    await expect(harness.inspect(context)).rejects.toThrow();
  });

  it("waits for concurrent initialization and never resumes its harness", async () => {
    const original = Harness.open.bind(Harness);
    const entered = deferred<Harness>(), release = deferred<void>();
    vi.spyOn(Harness, "open").mockImplementationOnce(async (...args) => {
      const harness = await original(...args);
      entered.resolve(harness);
      await release.promise;
      return harness;
    });
    const creating = create().catch((error: unknown) => error);
    const harness = await entered.promise;
    const resume = vi.spyOn(harness, "resume");
    const agent = (await runtime.agents.list("owner"))[0]!;
    const deleting = runtime.agents.delete("owner", agent.id);
    await vi.waitFor(() => expect(runtime.agents.signal(agent.id).aborted).toBe(true));
    release.resolve();
    await deleting; await creating;
    expect(resume).not.toHaveBeenCalled();
    await expect(harness.inspect(context)).rejects.toThrow();
    expect(await runtime.agents.list("owner")).toEqual([]);
  });
});
