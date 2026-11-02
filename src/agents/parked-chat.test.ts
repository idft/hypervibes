import { BACKGROUND_CONTEXT as context } from "@earendil-works/chord/context";
import { createModels } from "@earendil-works/pi-ai/models";
import { fauxAssistantMessage, fauxProvider } from "@earendil-works/pi-ai/providers/faux";
import { Harness } from "@earendil-works/pi-durable";
import { randomUUID } from "node:crypto";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, expect, it, vi } from "vitest";
import { hvUsers } from "../db/auth-schema.js";
import { createRuntime, type AppRuntime } from "../runtime.js";
import { testConfig } from "../test-config.js";
import { MainRecoveryDoc } from "./agent-service.js";

let runtime: AppRuntime | undefined;
let directory: string | undefined;
afterEach(async () => {
  vi.restoreAllMocks();
  await runtime?.close();
  if (directory) rmSync(directory, { recursive: true, force: true });
});

it("finishes legacy chat cleanup and permits a fresh response", async () => {
  directory = mkdtempSync(join(tmpdir(), "hv-parked-chat-"));
  const faux = fauxProvider();
  const models = createModels(); models.setProvider(faux.provider);
  const opened = vi.spyOn(Harness, "open");
  runtime = await createRuntime(testConfig({ APP_SQLITE_PATH: join(directory, "app.sqlite") }), {
    models, model: { provider: faux.getModel().provider, modelId: faux.getModel().id },
    accountFeedFactory: () => ({ close: async () => {} }), journalFeedFactory: () => ({ close: async () => {} }), journalHistoryFetcher: async () => [],
  });
  await runtime.db.insert(hvUsers).values({ id: "owner", name: "Owner", email: "owner@example.test" }).run();
  runtime.start();
  const agent = await runtime.agents.create("owner", "Parked chat", randomUUID(), `0x${"a".repeat(40)}`);
  const harness = await opened.mock.results[0]!.value as Harness;
  expect(opened.mock.calls[0]![1].settings).not.toHaveProperty("restartLimit");
  const cancelOrders = vi.spyOn(runtime.orders, "cancelAll");
  faux.setResponses([async (_request, options) => {
    await new Promise<void>((resolve) => { options?.signal?.addEventListener("abort", () => resolve(), { once: true }); });
    return fauxAssistantMessage("Interrupted");
  }]);
  await runtime.agents.send("owner", agent.id, "Observe", randomUUID());
  await vi.waitFor(async () => expect((await runtime!.agents.chat("owner", agent.id)).busy).toBe(true));
  const graph = await harness.watchTaskGraph(context);
  const generation = Object.values(graph.value.tasks).find((task) => task.kind === "pi.generation");
  await graph.stop();
  expect(generation).toBeDefined();
  // Simulate a cleanup marker committed by the previous application version.
  const main = await harness.root(context);
  await main.commit(async (tx) => { (await tx.doc(MainRecoveryDoc, main.id)).tasks = [generation!.id]; }, context);
  await runtime.agents.subagents.tick();
  await vi.waitFor(async () => expect((await runtime!.agents.chat("owner", agent.id)).busy).toBe(false));
  expect(await main.commit(async (tx) => [...(await tx.doc(MainRecoveryDoc, main.id)).tasks], context)).toEqual([]);
  expect((await harness.getTask(generation!.id, context))!.state.status).toBe("terminal");
  expect(cancelOrders).not.toHaveBeenCalled();
  faux.setResponses([fauxAssistantMessage("Fresh response")]);
  await runtime.agents.send("owner", agent.id, "Try a new observation", randomUUID());
  await vi.waitFor(async () => expect((await runtime!.agents.chat("owner", agent.id)).messages.some((message) => message.text === "Fresh response")).toBe(true));
});
