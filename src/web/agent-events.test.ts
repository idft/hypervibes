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
import type { AgentDashboard } from "../hyperliquid/agent-dashboard.js";
import type { AccountFeedEvents } from "../hyperliquid/live-dashboard.js";
const expiry = () => new Date(Date.now() + 3_600_000);
async function readEvent<T = ChatEvent["data"]>(reader: ReadableStreamDefaultReader<Uint8Array>, buffer: { text: string }): Promise<{ event?: string; data: T }> {
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
  let feedEvents: AccountFeedEvents | undefined;
  const closeFeed = vi.fn(async () => {});
  const config = () => testConfig({ APP_SQLITE_PATH: join(directory, "app.sqlite") });
  const options = () => {
    const models = createModels();
    faux = fauxProvider();
    models.setProvider(faux.provider);
    return { models, model: { provider: "faux", modelId: "faux-1" }, journalHistoryFetcher: async () => [], accountFeedFactory: (_address: string, events: AccountFeedEvents) => {
      feedEvents = events;
      return { close: closeFeed };
    } };
  };
  const request = (id: string, signal?: AbortSignal) => new Request(`http://localhost:3003/api/agents/${id}/events`, {
    headers: { origin: "http://localhost:3003", cookie: "fixture" }, signal,
  });
  beforeEach(async () => {
    feedEvents = undefined;
    closeFeed.mockClear();
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

  it("restores linked-account monitoring on runtime startup without any dashboard connections", async () => {
    const agent = await runtime.agents.create("owner", "Trading", randomUUID(), `0x${"34".repeat(20)}`);
    await streams.close();
    await runtime.close();
    runtime = await createRuntime(config(), options());
    streams = new AgentEventStreams(runtime, "dashboard");
    expect(feedEvents).toBeUndefined();
    runtime.start();
    await vi.waitFor(() => expect(feedEvents).toBeDefined());
    feedEvents!.update("spot", { spotUsdc: "17" });
    expect(await runtime.hyperliquid.agentDashboard("owner", agent.id)).toMatchObject({ spotUsdc: "17" });
  });

  it("authorizes dashboard subscriptions and pushes cached/live state without tying the feed to browser lifetime", async () => {
    await streams.close();
    streams = new AgentEventStreams(runtime, "dashboard");
    const agent = await runtime.agents.create("owner", "Trading", randomUUID(), `0x${"12".repeat(20)}`);
    const crossSite = new Request(`http://localhost:3003/api/agents/${agent.id}/dashboard/events`, { headers: { origin: "https://evil.example" } });
    expect((await streams.open(crossSite, agent.id)).status).toBe(403);
    mocks.session.mockResolvedValue({ user: { id: "other" }, expiresAt: expiry() });
    expect((await streams.open(request(agent.id), agent.id)).status).toBe(404);
    expect(feedEvents).toBeUndefined();
    mocks.session.mockResolvedValue(null);
    expect((await streams.open(request(agent.id), agent.id)).status).toBe(401);
    mocks.session.mockResolvedValue({ user: { id: "owner" }, expiresAt: expiry() });
    const source = new AbortController();
    const response = await streams.open(request(agent.id, source.signal), agent.id);
    const reader = response.body!.getReader();
    const buffer = { text: "" };
    const initial = await readEvent<AgentDashboard>(reader, buffer);
    expect(initial.event).toBe("dashboard");
    expect(initial.data.perpEquity).toBeNull();
    feedEvents!.update("perps", { perpEquity: "9007199254740993.12", positions: [] });
    const updated = await readEvent<AgentDashboard>(reader, buffer);
    expect(updated.data.perpEquity).toBe("9007199254740993.12");
    source.abort();
    expect((await reader.read()).done).toBe(true);
    expect(closeFeed).not.toHaveBeenCalled();
    feedEvents!.update("spot", { spotUsdc: "23" });
    const reopened = await streams.open(request(agent.id), agent.id);
    const cached = await readEvent<AgentDashboard>(reopened.body!.getReader(), { text: "" });
    expect(cached.data.spotUsdc).toBe("23");
    expect(cached.data.perpEquity).toBe("9007199254740993.12");
    await streams.close();
    await runtime.close();
    expect(closeFeed).toHaveBeenCalledOnce();
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