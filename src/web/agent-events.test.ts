import { createModels } from "@earendil-works/pi-ai/models";
import { fauxAssistantMessage, fauxProvider } from "@earendil-works/pi-ai/providers/faux";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { randomUUID } from "node:crypto";import { hvUsers } from "../db/auth-schema.js";
import { createRuntime, type AppRuntime } from "../runtime.js";
import { testConfig } from "../test-config.js";
import { AgentEventStreams } from "./agent-events.js";

const mocks = vi.hoisted(() => ({ session: vi.fn() }));
vi.mock("../auth-user.js", () => ({ currentUserSession: mocks.session }));
import type { ChatEvent } from "./agent-events.js";
const expiry = () => new Date(Date.now() + 3_600_000);
async function readEvent(reader: ReadableStreamDefaultReader<Uint8Array>, buffer: { text: string }): Promise<ChatEvent> {
  for (;;) {
    const block = buffer.text.split("\n\n");
    buffer.text = block.pop() ?? "";
    const complete = block.find(Boolean);
    if (complete) {
      const event = /^event: (.+)$/m.exec(complete)?.[1];
      const data = /^data: (.+)$/m.exec(complete)?.[1];
      if (data === undefined) continue; // The initial retry hint is a comment-only frame.
      return { event, data: JSON.parse(data) };
    }
    const { value, done } = await reader.read();
    if (done) throw new Error("The chat stream closed before the next event");
    buffer.text += Buffer.from(value).toString("utf8");
  }
}

describe("Agent chat event stream", () => {
  let directory: string;
  let runtime: AppRuntime;
  let streams: AgentEventStreams;
  let faux: ReturnType<typeof fauxProvider>;
  const config = () => testConfig({ APP_SQLITE_PATH: join(directory, "app.sqlite") });
  const options = () => {
    const models = createModels();
    faux = fauxProvider();
    models.setProvider(faux.provider);
    return { models, model: { provider: "faux", modelId: "faux-1" } };
  };
  const request = (id: string, signal?: AbortSignal) => new Request(`http://localhost:3003/api/agents/${id}/events`, {
    headers: { origin: "http://localhost:3003", cookie: "fixture" }, signal,
  });
  beforeEach(async () => {
    directory = mkdtempSync(join(tmpdir(), "hv-agent-events-"));
    runtime = await createRuntime(config(), options());
    await runtime.db.insert(hvUsers).values([
      { id: "owner", name: "Owner", email: "owner@example.test" },
      { id: "other", name: "Other", email: "other@example.test" },
    ]).run();
    streams = new AgentEventStreams(runtime);
    mocks.session.mockResolvedValue({ user: { id: "owner", walletAddress: "0xfixture" }, expiresAt: expiry() });
    runtime.start();
  });
  afterEach(async () => {
    await streams.close();
    await runtime.close();
    rmSync(directory, { recursive: true, force: true });
  });

  it("pushes committed chat snapshots, rejects invalid requests and ends cleanly", { timeout: 30_000 }, async () => {
    expect((await streams.open(new Request("http://localhost:3003/api/agents/x/events", { method: "POST" }), "x")).status).toBe(405);
    expect((await streams.open(request(randomUUID()), randomUUID())).status).toBe(404);
    mocks.session.mockResolvedValue(null);
    const agent = await runtime.agents.create("owner", "Research", randomUUID());
    expect((await streams.open(request(agent.id), agent.id)).status).toBe(401);
    mocks.session.mockResolvedValue({ user: { id: "other", walletAddress: "0xother" }, expiresAt: expiry() });
    expect((await streams.open(request(agent.id), agent.id)).status).toBe(404);
    mocks.session.mockResolvedValue({ user: { id: "owner", walletAddress: "0xfixture" }, expiresAt: expiry() });

    const source = new AbortController();
    const response = await streams.open(request(agent.id, source.signal), agent.id);
    expect(response.status).toBe(200);
    expect(response.headers.get("content-type")).toBe("text/event-stream; charset=utf-8");
    expect(response.headers.get("cache-control")).toBe("no-store");
    const reader = response.body!.getReader();
    const buffer = { text: "" };
    const first = await readEvent(reader, buffer);
    expect(first.event).toBe("chat");
    expect(first.data.messages).toEqual([]);

    faux.setResponses([fauxAssistantMessage("A pushed answer")]);
    await runtime.agents.send("owner", agent.id, "Hello", randomUUID());
    let answer: ChatEvent | undefined;
    for (let attempts = 0; !answer && attempts < 40; attempts++) {
      const event = await readEvent(reader, buffer);
      if (event.data.messages.some((entry: { text: string }) => entry.text === "A pushed answer")) answer = event;
    }
    expect(answer?.data.busy).toBe(false);

    source.abort();
    const { done } = await reader.read();
    expect(done).toBe(true);
    // A closed stream releases its per-user slot and the server still serves a new one.
    const reopened = await streams.open(request(agent.id), agent.id);
    expect(reopened.status).toBe(200);
    await reopened.body!.cancel();
  });

  it("notifies subscribers of shutdown and stops admitting streams", { timeout: 30_000 }, async () => {
    const agent = await runtime.agents.create("owner", "Research", randomUUID());
    const response = await streams.open(request(agent.id), agent.id);
    const reader = response.body!.getReader();
    const buffer = { text: "" };
    await readEvent(reader, buffer); // initial snapshot
    await streams.close();
    const end = await readEvent(reader, buffer);
    expect(end.event).toBe("shutdown");
    expect((await streams.open(request(agent.id), agent.id)).status).toBe(503);
  });

  it("ends the stream when the session disappears between chat updates", { timeout: 30_000 }, async () => {
    vi.useFakeTimers();
    try {
      const agent = await runtime.agents.create("owner", "Research", randomUUID());
      const response = await streams.open(request(agent.id), agent.id);
      const reader = response.body!.getReader();
      const buffer = { text: "" };
      await readEvent(reader, buffer);
      mocks.session.mockResolvedValue(null);
      await vi.advanceTimersByTimeAsync(60_000);
      const end = await readEvent(reader, buffer);
      expect(end.event).toBe("unauthorized");
      await streams.close();
    } finally {
      vi.useRealTimers();
    }
  });
});