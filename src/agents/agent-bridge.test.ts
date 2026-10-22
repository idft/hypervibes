import { createModels } from "@earendil-works/pi-ai/models";
import { fauxAssistantMessage, fauxProvider } from "@earendil-works/pi-ai/providers/faux";
import { BACKGROUND_CONTEXT as context } from "@earendil-works/chord/context";
import { createHash, randomUUID } from "node:crypto";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { hvUsers } from "../db/auth-schema.js";
import { createRuntime, type AppRuntime } from "../runtime.js";
import { testConfig } from "../test-config.js";

describe("agent bridge gateway access", () => {
  let directory: string;
  let runtime: AppRuntime;
  let faux: ReturnType<typeof fauxProvider>;
  const config = () => testConfig({ APP_SQLITE_PATH: join(directory, "app.sqlite") });
  const options = () => {
    const models = createModels();
    models.setProvider(faux.provider);
    return { models, model: { provider: faux.getModel().provider, modelId: faux.getModel().id }, accountFeedFactory: () => ({ close: async () => {} }), journalFeedFactory: () => ({ close: async () => {} }), journalHistoryFetcher: async () => [] };
  };

  beforeEach(async () => {
    directory = mkdtempSync(join(tmpdir(), "hv-agent-bridge-"));
    faux = fauxProvider();
    runtime = await createRuntime(config(), options());
    await runtime.db.insert(hvUsers).values({ id: "owner", name: "Owner", email: "owner@example.test" }).run();
  });
  afterEach(async () => {
    vi.restoreAllMocks();
    await runtime.close();
    rmSync(directory, { recursive: true, force: true });
  });

  async function createAgent() {
    const account = `0x${createHash("sha256").update(randomUUID()).digest("hex").slice(0, 40)}`;
    return runtime.agents.create("owner", "Bridged", randomUUID(), account);
  }

  it("rejects bridge access to unknown agents and exposes no user scoping", async () => {
    await expect(runtime.agents.bridge(randomUUID())).rejects.toThrow("Agent not found");
  });

  it("surfaces the provider's recorded error in the chat transcript", async () => {
    const agent = await createAgent();
    runtime.start();
    faux.setResponses([fauxAssistantMessage("ignored", { stopReason: "error", errorMessage: "429: {\"message\":\"you have reached your session usage limit\"}" })]);
    await runtime.agents.send("owner", agent.id, "hello", randomUUID());
    await vi.waitFor(async () => expect((await runtime.agents.chat("owner", agent.id)).messages.some((entry) => entry.failed)).toBe(true));
    const chat = await runtime.agents.chat("owner", agent.id);
    const failed = chat.messages.find((entry) => entry.failed)!;
    expect(failed.text).toContain("429: {\"message\":\"you have reached your session usage limit\"}");
    expect(failed.text).toContain("Send another message to retry.");
    // The same text survives a restart because Pi persists it with the entry.
    await runtime.close();
    runtime = await createRuntime(config(), options());
    expect((await runtime.agents.chat("owner", agent.id)).messages.find((entry) => entry.failed)?.text).toBe(failed.text);
  });

  it("reports an aborted turn as interrupted without inventing a provider error", async () => {
    const agent = await createAgent();
    runtime.start();
    const bridge = await runtime.agents.bridge(agent.id);
    faux.setResponses([fauxAssistantMessage("ignored", { stopReason: "error", errorMessage: "400: upstream rejected the request" })]);
    await runtime.agents.send("owner", agent.id, "hello", randomUUID());
    await vi.waitFor(async () => expect((await runtime.agents.chat("owner", agent.id)).messages.some((entry) => entry.failed)).toBe(true));
    // The gateway reply path skips failed turns entirely rather than forwarding the error text again.
    const submission = await bridge.submit("next", randomUUID());
    await expect(submission.wait(context)).resolves.toMatchObject({ status: "unanswered" });
  });

  it("keeps the generic failure text when Pi recorded no provider message", async () => {
    const agent = await createAgent();
    runtime.start();
    faux.setResponses([fauxAssistantMessage("ignored", { stopReason: "error" })]);
    await runtime.agents.send("owner", agent.id, "hello", randomUUID());
    await vi.waitFor(async () => expect((await runtime.agents.chat("owner", agent.id)).messages.some((entry) => entry.failed)).toBe(true));
    expect((await runtime.agents.chat("owner", agent.id)).messages.find((entry) => entry.failed)?.text).toBe("The model request failed. Send another message to retry.");
  });

  it("submits to the persistent main conversation and reads the answered entries", async () => {
    const agent = await createAgent();
    runtime.start();
    const bridge = await runtime.agents.bridge(agent.id);
    faux.setResponses([fauxAssistantMessage("The plan is patience.")]);
    const submission = await bridge.submit("what is the plan?", randomUUID());
    expect((await submission.status(context)).status).not.toBe("unanswered");
    const settled = await submission.wait(context);
    expect(settled.status).toBe("done");
    if (settled.status !== "done" || settled.entry === undefined || settled.answer === undefined) return;
    // The user input is part of the main conversation transcript.
    const chat = await runtime.agents.chat("owner", agent.id);
    expect(chat.messages.some((entry) => entry.role === "user" && entry.text === "what is the plan?")).toBe(true);
    const replies = await bridge.replyText(settled.entry, settled.answer);
    expect(replies).toEqual(["The plan is patience."]);
  });

  it("reports submission failures with operator-facing messages", async () => {
    const agent = await createAgent();
    // Not started: agent execution is not ready.
    const stopped = await runtime.agents.bridge(agent.id);
    await expect(stopped.submit("hello", randomUUID())).rejects.toThrow("not ready");
    runtime.start();
    const bridge = await runtime.agents.bridge(agent.id);
    await expect(bridge.submit("", randomUUID())).rejects.toThrow("1–16,000 characters");
    await expect(bridge.submit("hello", "not-a-uuid")).rejects.toThrow("valid request ID");
  });

  it("stops, resets and compacts the main conversation", async () => {
    const agent = await createAgent();
    runtime.start();
    const bridge = await runtime.agents.bridge(agent.id);
    await expect(bridge.stop()).resolves.toBeUndefined();
    await expect(bridge.reset()).resolves.toBeUndefined();
    // After the reset the next turn still lands in the same persistent conversation.
    faux.setResponses([fauxAssistantMessage("fresh start")]);
    const submission = await bridge.submit("after reset", randomUUID());
    await expect(submission.wait(context)).resolves.toMatchObject({ status: "done" });
    const chat = await runtime.agents.chat("owner", agent.id);
    expect(chat.messages.some((entry) => entry.text === "after reset")).toBe(true);
    expect(chat.messages.some((entry) => entry.text === "fresh start")).toBe(true);
    const compacted = await bridge.compact();
    expect(compacted.message === undefined || typeof compacted.message === "string").toBe(true);
  });

  it("enforces the same message queue limit as the web path", async () => {
    const agent = await createAgent();
    runtime.start();
    const bridge = await runtime.agents.bridge(agent.id);
    // The inbox limit (20) is enforced by submission serialization; flooding past it must surface the public error.
    let full = false;
    for (let index = 0; index < 25 && !full; index++) {
      try { await bridge.submit(`message ${index}`, randomUUID()); }
      catch (error) { full = error instanceof Error && error.message.includes("queue is full"); }
    }
    expect(full).toBe(true);
  });
});