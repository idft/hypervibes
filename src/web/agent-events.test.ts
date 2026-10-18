import { createModels } from "@earendil-works/pi-ai/models";
import { fauxAssistantMessage, fauxProvider } from "@earendil-works/pi-ai/providers/faux";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { randomUUID } from "node:crypto";import { hvUsers } from "../db/auth-schema.js";
import { createRuntime, type AppRuntime } from "../runtime.js";
import { testConfig } from "../test-config.js";
import { hvSubagentRuns } from "../db/subagent-schema.js";
import type { RunHistory } from "../agents/subagent-history.js";
import type { RunDetail } from "../agents/subagent-service.js";
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
    return { models, model: { provider: "faux", modelId: "faux-1" }, journalHistoryFetcher: async () => [], journalFeedFactory: () => ({ close: async () => {} }), accountFeedFactory: (_address: string, events: AccountFeedEvents) => {
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
    const agent = await runtime.agents.create("owner", "Research", randomUUID(), `0x${"12".repeat(20)}`);
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
    feedEvents = undefined;
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
    const createdFeed = feedEvents;
    const crossSite = new Request(`http://localhost:3003/api/agents/${agent.id}/dashboard/events`, { headers: { origin: "https://evil.example" } });
    expect((await streams.open(crossSite, agent.id)).status).toBe(403);
    mocks.session.mockResolvedValue({ user: { id: "other" }, expiresAt: expiry() });
    expect((await streams.open(request(agent.id), agent.id)).status).toBe(404);
    // Rejected subscriptions must not open any additional account feed.
    expect(feedEvents).toBe(createdFeed);
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

  it("pushes paginated run history on creation/completion, authorizes its scope and closes its watcher", async () => {
    await streams.close(); streams = new AgentEventStreams(runtime, "runs");
    const agent = await runtime.agents.create("owner", "Research", randomUUID(), `0x${"12".repeat(20)}`);
    await runtime.agents.subagents.tick();
    const worker = await runtime.agents.subagents.create("owner", agent.id, { name: "Research", description: "", instructions: "Find evidence", provider: "faux", modelId: "faux-1", thinkingLevel: "", tools: [], enabled: true, delegatable: true, persistent: false, schedule: "", delaySeconds: 30, completion: "save", timeoutSeconds: 900 }, randomUUID());
    const runRequest = (page = "1", signal?: AbortSignal) => new Request(`http://localhost:3003/api/agents/${agent.id}/sub-agents/${worker.id}/runs/events?page=${page}`, { headers: { origin: runtime.origin }, signal });
    expect((await streams.open(runRequest("0"), agent.id, worker.id)).status).toBe(400);
    expect((await streams.open(runRequest(), agent.id, "missing")).status).toBe(404);
    mocks.session.mockResolvedValue({ user: { id: "other" }, expiresAt: expiry() });
    expect((await streams.open(runRequest(), agent.id, worker.id)).status).toBe(404);
    mocks.session.mockResolvedValue({ user: { id: "owner" }, expiresAt: expiry() });
    await runtime.db.insert(hvSubagentRuns).values(Array.from({ length: 51 }, (_, index) => ({ id: `old-${index}`, agentId: agent.id, subagentId: worker.id, requestId: `old:${index}`, source: "schedule" as const, task: "Old task", config: worker, status: "skipped" as const, createdAt: index, reportedAt: 1 }))).run();
    const source = new AbortController();
    const response = await streams.open(runRequest("2", source.signal), agent.id, worker.id);
    const reader = response.body!.getReader(), buffer = { text: "" };
    const initial = await readEvent<RunHistory>(reader, buffer);
    expect(initial.event).toBe("runs");
    expect(initial.data.pagination).toMatchObject({ page: 2, pages: 2, total: 51 });
    expect(initial.data.runs).toHaveLength(1);
    const history = vi.spyOn(runtime.agents.subagents, "history");
    await runtime.agents.subagents.tick();
    expect(history).not.toHaveBeenCalled(); // Idle scheduler ticks do not poll history.
    faux.setResponses([fauxAssistantMessage("Pushed worker answer")]);
    const run = await runtime.agents.subagents.run("owner", agent.id, worker.id, "New task", randomUUID());
    const updated = await readEvent<RunHistory>(reader, buffer);
    expect(updated.data.pagination).toMatchObject({ page: 2, total: 52 });
    expect(updated.data.runs).toHaveLength(2);
    source.abort();
    await streams.close();
    // Reconnecting to page one obtains current state, not missed-event replay.
    streams = new AgentEventStreams(runtime, "runs");
    const live = await streams.open(runRequest(), agent.id, worker.id);
    const liveReader = live.body!.getReader(), liveBuffer = { text: "" };
    let snapshot = (await readEvent<RunHistory>(liveReader, liveBuffer)).data;
    for (let i = 0; i < 10 && snapshot.runs.find((item) => item.id === run.id)?.status !== "completed"; i++) snapshot = (await readEvent<RunHistory>(liveReader, liveBuffer)).data;
    expect(snapshot.runs.find((item) => item.id === run.id)?.status).toBe("completed");
    expect(snapshot.runs[0]).not.toHaveProperty("result");
    await streams.close();
    expect((await readEvent(liveReader, liveBuffer)).event).toBe("shutdown");
    vi.restoreAllMocks();
  });

  it("pushes live run-detail snapshots, authorizes its scope and closes its watcher", async () => {
    await streams.close(); streams = new AgentEventStreams(runtime, "run");
    const agent = await runtime.agents.create("owner", "Research", randomUUID(), `0x${"12".repeat(20)}`);
    await runtime.agents.subagents.tick();
    const worker = await runtime.agents.subagents.create("owner", agent.id, { name: "Research", description: "", instructions: "Find evidence", provider: "faux", modelId: "faux-1", thinkingLevel: "", tools: [], enabled: true, delegatable: true, persistent: false, schedule: "", delaySeconds: 30, completion: "save", timeoutSeconds: 900 }, randomUUID());
    faux.setResponses([fauxAssistantMessage("Pushed worker answer")]);
    const run = await runtime.agents.subagents.run("owner", agent.id, worker.id, "New task", randomUUID());
    const runRequest = (runId: string, signal?: AbortSignal) => new Request(`http://localhost:3003/api/agents/${agent.id}/sub-agents/${worker.id}/runs/${runId}/events`, { headers: { origin: runtime.origin }, signal });
    expect((await streams.open(runRequest("missing"), agent.id, worker.id, "missing")).status).toBe(404);
    const otherWorker = await runtime.agents.subagents.create("owner", agent.id, { name: "Other", description: "", instructions: "Find evidence", provider: "faux", modelId: "faux-1", thinkingLevel: "", tools: [], enabled: true, delegatable: true, persistent: false, schedule: "", delaySeconds: 30, completion: "save", timeoutSeconds: 900 }, randomUUID());
    expect((await streams.open(runRequest(run.id), agent.id, otherWorker.id, run.id)).status).toBe(404);
    mocks.session.mockResolvedValue({ user: { id: "other" }, expiresAt: expiry() });
    expect((await streams.open(runRequest(run.id), agent.id, worker.id, run.id)).status).toBe(404);
    mocks.session.mockResolvedValue({ user: { id: "owner" }, expiresAt: expiry() });
    const source = new AbortController();
    const response = await streams.open(runRequest(run.id, source.signal), agent.id, worker.id, run.id);
    const reader = response.body!.getReader(), buffer = { text: "" };
    const first = await readEvent<RunDetail>(reader, buffer);
    expect(first.event).toBe("run");
    expect(first.data.run.id).toBe(run.id);
    let snapshot = first.data;
    for (let i = 0; i < 20 && snapshot.run.status !== "completed"; i++) snapshot = (await readEvent<RunDetail>(reader, buffer)).data;
    expect(snapshot.run.status).toBe("completed");
    expect(snapshot.run.result).toBe("Pushed worker answer");
    expect(JSON.stringify(snapshot.entries)).toContain("New task");
    expect(JSON.stringify(snapshot.entries)).toContain("Pushed worker answer");
    source.abort();
    await streams.close();
    vi.restoreAllMocks();
  });

  it("authenticates chart subscriptions before market requests and closes their backend watcher", async () => {
    await streams.close();
    streams = new AgentEventStreams(runtime, "chart");
    const agent = await runtime.agents.create("owner", "Chart", randomUUID(), `0x${"12".repeat(20)}`);
    const selection = vi.spyOn(runtime.hyperliquid.charts, "selection").mockResolvedValue({ coin: "BTC", interval: "1h" });
    const stopped = vi.fn();
    const watch = vi.spyOn(runtime.hyperliquid.charts, "watch").mockImplementation((_selection, signal, publish) => new Promise<void>((resolve) => {
      publish({ candles: [], status: "connecting" });
      signal.addEventListener("abort", () => { stopped(); resolve(); }, { once: true });
    }));
    const chartRequest = () => new Request(`http://localhost:3003/api/agents/${agent.id}/chart/events?coin=BTC&interval=1h`, { headers: { origin: runtime.origin } });
    mocks.session.mockResolvedValue({ user: { id: "other" }, expiresAt: expiry() });
    expect((await streams.open(chartRequest(), agent.id)).status).toBe(404);
    expect(selection).not.toHaveBeenCalled();
    mocks.session.mockResolvedValue({ user: { id: "owner" }, expiresAt: expiry() });
    const response = await streams.open(chartRequest(), agent.id);
    const event = await readEvent(response.body!.getReader(), { text: "" });
    expect(event).toEqual({ event: "chart", data: { candles: [], status: "connecting" } });
    expect(selection).toHaveBeenCalledWith("BTC", "1h");
    expect(watch).toHaveBeenCalledOnce();
    await streams.close();
    expect(stopped).toHaveBeenCalledOnce();
  });

  it("notifies subscribers of shutdown and stops admitting streams", { timeout: 30_000 }, async () => {
    const agent = await runtime.agents.create("owner", "Research", randomUUID(), `0x${"12".repeat(20)}`);
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
      const agent = await runtime.agents.create("owner", "Research", randomUUID(), `0x${"12".repeat(20)}`);
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