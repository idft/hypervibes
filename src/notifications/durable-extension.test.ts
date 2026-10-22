import { createModels } from "@earendil-works/pi-ai/models";
import { fauxAssistantMessage, fauxProvider, fauxToolCall } from "@earendil-works/pi-ai/providers/faux";
import { randomUUID } from "node:crypto";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createRuntime, type AppRuntime } from "../runtime.js";
import { testConfig } from "../test-config.js";
import { templateDraft } from "../agents/subagent-templates.js";

const message = { title: "Interim daily review", body: "Review m1: sparse evidence; no insight changes applied.", severity: "info" };
describe("notification tools in application harnesses", () => {
  let directory: string, runtime: AppRuntime, agentId: string, faux: ReturnType<typeof fauxProvider>;
  beforeEach(async () => {
    directory = mkdtempSync(join(tmpdir(), "hv-notification-tools-")); faux = fauxProvider();
    const models = createModels(); models.setProvider(faux.provider);
    runtime = await createRuntime(testConfig({ APP_SQLITE_PATH: join(directory, "app.sqlite") }), {
      models, model: { provider: "faux", modelId: "faux-1" }, accountFeedFactory: () => ({ close: async () => {} }), journalFeedFactory: () => ({ close: async () => {} }), journalHistoryFetcher: async () => [],
    });
    await runtime.database.run("INSERT INTO hv_users(id,name,email) VALUES ('owner','Owner','owner@example.test')");
    agentId = (await runtime.agents.create("owner", "Desk", randomUUID(), `0x${"a".repeat(40)}`)).id;
    runtime.start(); await runtime.agents.subagents.tick();
  });
  afterEach(async () => { await runtime.close(); rmSync(directory, { recursive: true, force: true }); });
  async function settled() { await vi.waitFor(async () => expect((await runtime.agents.chat("owner", agentId)).busy).toBe(false)); }
  it.each([false, true])("resolves root tool identity for direct/nested Code Mode calls (nested=%s)", async (nested) => {
    const root = (await runtime.agents.subagents.list("owner", agentId)).find((row) => row.kind === "orchestrator")!;
    await runtime.agents.subagents.updateTools("owner", agentId, root.id, ["send_notification", ...(nested ? ["codemode"] : [])]);
    faux.setResponses([
      fauxAssistantMessage(nested ? fauxToolCall("codemode", { code: `text(await tools.send_notification(${JSON.stringify(message)}));` }) : fauxToolCall("send_notification", message), { stopReason: "toolUse" }),
      fauxAssistantMessage("Summary queued"),
    ]);
    await runtime.agents.send("owner", agentId, "Send an update", randomUUID()); await settled();
    expect((await runtime.notifications.list(agentId)).items).toMatchObject([{ title: message.title, source: "Orchestrator", subagentId: root.id, runId: null, delivery: "web_only" }]);
    expect(await runtime.database.get("SELECT operation_key FROM hv_notification_operations WHERE agent_id=? AND operation_key LIKE 'tool:%'", agentId)).toMatchObject({ operation_key: expect.stringMatching(/^tool:\d+$/) });
  });
  it("lets a daily reviewer publish a full review then craft a separate summary, with run provenance", async () => {
    const draft = templateDraft("basic-review", { provider: "faux", modelId: "faux-1", thinkingLevel: "" });
    expect(draft.instructions).toContain("call send_notification once");
    const reviewer = await runtime.agents.subagents.create("owner", agentId, draft, randomUUID(), "basic-review");
    const end = Date.now(), start = end - 1000;
    faux.setResponses([
      fauxAssistantMessage(fauxToolCall("publish_review", { topic: "interim", summary: "Discovery summary", content: "Full report and limitations", payload: { version: 1, start, end, coverage: "incomplete", limitations: ["Sparse evidence"], findings: [], recommendations: [], memoryRefs: [] } }), { stopReason: "toolUse" }),
      fauxAssistantMessage(fauxToolCall("send_notification", message), { stopReason: "toolUse" }),
      fauxAssistantMessage("Published m1; sent concise summary; no insight changes"),
    ]);
    const run = await runtime.agents.subagents.run("owner", agentId, reviewer.id, "Perform an interim review", randomUUID());
    await vi.waitFor(async () => expect((await runtime.agents.subagents.detail("owner", agentId, reviewer.id)).runs[0]?.status).toBe("completed"));
    expect((await runtime.notifications.list(agentId)).items).toMatchObject([{ body: message.body, source: "Daily Reviewer", subagentId: reviewer.id, runId: run.id }]);
    expect((await runtime.agents.execution.memories.list(agentId, { kind: "review" })).items[0]?.summary).toBe("Discovery summary");
  });
  it("returns disabled to the model without fabricating a queued notification", async () => {
    await runtime.notifications.saveSettings(agentId, { enabled: false, submissions: true, fills: true });
    faux.setResponses([fauxAssistantMessage(fauxToolCall("send_notification", message), { stopReason: "toolUse" }), fauxAssistantMessage("Notifications disabled; continuing")]);
    await runtime.agents.send("owner", agentId, "Send update", randomUUID()); await settled();
    expect((await runtime.notifications.list(agentId)).total).toBe(0);
    expect(await runtime.database.get("SELECT result FROM hv_notification_operations WHERE agent_id=? AND operation_key LIKE 'tool:%'", agentId)).toEqual({ result: '{"status":"disabled"}' });
  });
});
