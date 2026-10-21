import { createModels } from "@earendil-works/pi-ai/models";
import { fauxProvider } from "@earendil-works/pi-ai/providers/faux";
import { randomUUID } from "node:crypto";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { hvUsers } from "../db/auth-schema.js";
import { createRuntime, type AppRuntime } from "../runtime.js";
import { testConfig } from "../test-config.js";

describe("bundled default sub-agent import", () => {
  let runtime: AppRuntime;
  let directory: string;
  const faux = fauxProvider();
  const models = createModels();
  models.setProvider(faux.provider);
  const model = { provider: faux.getModel().provider, modelId: faux.getModel().id };
  const config = () => testConfig({ APP_SQLITE_PATH: join(directory, "test.sqlite") });
  const options = { models, model, accountFeedFactory: () => ({ close: async () => {} }), journalFeedFactory: () => ({ close: async () => {} }), journalHistoryFetcher: async () => [] };
  const create = (include = false, request = randomUUID()) => runtime.agents.create("owner", "Desk", request, `0x${"a".repeat(40)}`, model, include);
  beforeEach(async () => {
    directory = mkdtempSync(join(tmpdir(), "hv-default-subagents-"));
    runtime = await createRuntime(config(), options);
    await runtime.db.insert(hvUsers).values([{ id: "owner", name: "Owner", email: "owner@example.test" }, { id: "other", name: "Other", email: "other@example.test" }]).run();
  });
  afterEach(async () => {
    await runtime.close();
    rmSync(directory, { recursive: true, force: true });
  });

  it("creates paused linked copies alongside a new Agent, without starting work or granting trading authority", async () => {
    const agent = await create(true);
    const definitions = await runtime.agents.subagents.list("owner", agent.id);
    const workers = definitions.filter((row) => row.kind === "worker");
    expect(workers).toHaveLength(3);
    const analyst = workers.find((row) => row.templateOrigin?.id === "basic-analysis")!;
    const trader = workers.find((row) => row.templateOrigin?.id === "basic-trading")!;
    const reviewer = workers.find((row) => row.templateOrigin?.id === "basic-review")!;
    expect(trader.triggers).toEqual({ inboxEnabled: true, researchSources: [analyst.id], workerDelegates: [], fills: true, orderChanges: true });
    expect(analyst.schedule).toBe("15m");
    expect(reviewer.schedule).toBe("1d");
    for (const worker of workers) {
      expect(worker).toMatchObject({ ...model, thinkingLevel: "", enabled: false, archived: false, persistent: false, conversationId: null, nextRunAt: null });
      expect(worker.tools).not.toContain("delegate_subagent");
      expect(worker.templateOrigin?.hash).toHaveLength(64);
    }
    expect(await runtime.agents.risk.get(agent.id)).toMatchObject({ tradingEnabled: false });
    expect(await runtime.database.all("SELECT * FROM hv_worker_subscriptions WHERE subagent_id=?", trader.id)).toEqual([]);
    expect(await runtime.database.all("SELECT id FROM hv_subagent_runs WHERE agent_id=?", agent.id)).toEqual([]);
    expect((await runtime.agents.chat("owner", agent.id)).messages).toEqual([]);
  });

  it("allows creation without defaults, and never backfills on a creation retry", async () => {
    const request = randomUUID();
    const agent = await create(false, request);
    expect(await runtime.agents.subagents.list("owner", agent.id)).toHaveLength(1);
    expect((await create(true, request)).id).toBe(agent.id);
    expect(await runtime.agents.subagents.list("owner", agent.id)).toHaveLength(1);
  });

  it("deduplicates concurrent Agent creation with defaults", async () => {
    const request = randomUUID();
    const [one, two] = await Promise.all([create(true, request), create(true, request)]);
    expect(one.id).toBe(two.id);
    expect(await runtime.agents.subagents.list("owner", one.id)).toHaveLength(4);
  });

  it("imports a separate connected set and preserves edits/archives across retries and restarts", async () => {
    const agent = await create(true);
    const original = await runtime.agents.subagents.list("owner", agent.id);
    const request = randomUUID();
    const [first, retry] = await Promise.all([runtime.agents.subagents.importDefaults("owner", agent.id, request), runtime.agents.subagents.importDefaults("owner", agent.id, request)]);
    expect(retry).toEqual(first);
    expect(first.every((row) => !original.some((existing) => existing.id === row.id))).toBe(true);
    expect(first[1]!.triggers.researchSources).toEqual([first[0]!.id]);
    await runtime.agents.subagents.updateIdentity("owner", agent.id, first[0]!.id, "Custom analyst", "Keep this");
    await runtime.agents.subagents.archive("owner", agent.id, first[2]!.id);
    await runtime.close();
    runtime = await createRuntime(config(), options);
    const restored = await runtime.agents.subagents.importDefaults("owner", agent.id, request);
    expect(restored[0]).toMatchObject({ name: "Custom analyst", description: "Keep this" });
    expect(restored[2]!.archived).toBe(true);
    expect(await runtime.agents.subagents.list("owner", agent.id)).toHaveLength(7);
    expect((await runtime.agents.subagents.list("owner", agent.id)).filter((row) => original.some((entry) => entry.id === row.id))).toEqual(original);
  });

  it("scopes import IDs to the Agent and requires ownership and valid request IDs", async () => {
    const one = await create();
    const two = await runtime.agents.create("owner", "Second", randomUUID(), `0x${"b".repeat(40)}`, model);
    const request = randomUUID();
    await expect(runtime.agents.subagents.importDefaults("other", one.id, request)).rejects.toMatchObject({ status: 404 });
    await expect(runtime.agents.subagents.importDefaults("owner", one.id, "bad")).rejects.toThrow("valid request ID");
    const first = await runtime.agents.subagents.importDefaults("owner", one.id, request);
    const second = await runtime.agents.subagents.importDefaults("owner", two.id, request);
    expect(second.every((row) => !first.some((entry) => entry.id === row.id))).toBe(true);
    expect(second[1]!.triggers.researchSources).toEqual([second[0]!.id]);
  });

  it("activates the saved subscriptions only when the Trader is explicitly enabled", async () => {
    const agent = await create(true);
    const workers = await runtime.agents.subagents.list("owner", agent.id);
    const trader = workers.find((row) => row.templateOrigin?.id === "basic-trading")!;
    await runtime.agents.subagents.updateSection("owner", agent.id, trader.id, "enabled", { enabled: true });
    const subscriptions = await runtime.database.all<{ event_type: string; source_id: string }>("SELECT event_type,source_id FROM hv_worker_subscriptions WHERE subagent_id=?", trader.id);
    expect(subscriptions).toEqual(expect.arrayContaining([{ event_type: "research", source_id: trader.triggers.researchSources[0] }, { event_type: "fill", source_id: "" }, { event_type: "order_status", source_id: "" }]));
  });

  it("rolls back every worker on import failure and the Agent on creation failure", async () => {
    const agent = await create();
    await runtime.database.exec("CREATE TRIGGER reject_reviewer BEFORE INSERT ON hv_subagents WHEN NEW.name='Daily Reviewer' BEGIN SELECT RAISE(ABORT, 'test import failure'); END");
    await expect(runtime.agents.subagents.importDefaults("owner", agent.id, randomUUID())).rejects.toThrow();
    expect(await runtime.agents.subagents.list("owner", agent.id)).toHaveLength(1);
    await expect(runtime.agents.create("owner", "Fail", randomUUID(), `0x${"c".repeat(40)}`, model, true)).rejects.toThrow();
    expect(await runtime.agents.list("owner")).toHaveLength(1);
  });
});
