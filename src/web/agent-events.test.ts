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
import type { DashboardSubagent } from "../agents/subagent-dashboard.js";
import type { RunHistory } from "../agents/subagent-history.js";
import type { RunDetail } from "../agents/subagent-service.js";
import { AgentEventStreams } from "./agent-events.js";
import type { ChartLayout } from "../charts/contracts.js";

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

describe("Agent multiplexed event stream", () => {
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
  const streamRequest = (id: string, topics: string[], extra = "", signal?: AbortSignal) => new Request(`http://localhost:3003/api/agents/${id}/stream?topics=${topics.join(",")}${extra}`, {
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
    streams = new AgentEventStreams(runtime, "agent");
    mocks.session.mockResolvedValue({ user: { id: "owner", walletAddress: "0xfixture" }, expiresAt: expiry() });
    runtime.start();
  });
  afterEach(async () => {
    await streams.close();
    await runtime.close();
    rmSync(directory, { recursive: true, force: true });
  });

  it("rejects invalid multiplex requests before opening any watcher", async () => {
    const agent = await runtime.agents.create("owner", "Research", randomUUID(), `0x${"12".repeat(20)}`);
    expect((await streams.open(new Request(`http://localhost:3003/api/agents/${agent.id}/stream?topics=chat`, { method: "POST" }), agent.id)).status).toBe(405);
    expect((await streams.open(streamRequest(agent.id, ["nope"]), agent.id)).status).toBe(400);
    expect((await streams.open(streamRequest(agent.id, []), agent.id)).status).toBe(400);
    expect((await streams.open(streamRequest(agent.id, ["chart"]), agent.id)).status).toBe(400);
    expect((await streams.open(streamRequest(agent.id, ["chat"]), randomUUID())).status).toBe(404);
    mocks.session.mockResolvedValue(null);
    expect((await streams.open(streamRequest(agent.id, ["chat"]), agent.id)).status).toBe(401);
    mocks.session.mockResolvedValue({ user: { id: "other", walletAddress: "0xother" }, expiresAt: expiry() });
    expect((await streams.open(streamRequest(agent.id, ["chat"]), agent.id)).status).toBe(404);
    mocks.session.mockResolvedValue({ user: { id: "owner", walletAddress: "0xfixture" }, expiresAt: expiry() });
    const evil = new Request(`http://localhost:3003/api/agents/${agent.id}/stream?topics=chat`, { headers: { origin: "https://evil.example" } });
    expect((await streams.open(evil, agent.id)).status).toBe(403);
    // Authentication runs before any market request.
    const selection = vi.spyOn(runtime.hyperliquid.charts, "selection");
    try {
      mocks.session.mockResolvedValue({ user: { id: "other" }, expiresAt: expiry() });
      expect((await streams.open(streamRequest(randomUUID(), ["chart"], "&coin=BTC&interval=1h"), randomUUID())).status).toBe(404);
      expect(selection).not.toHaveBeenCalled();
    } finally {
      mocks.session.mockResolvedValue({ user: { id: "owner", walletAddress: "0xfixture" }, expiresAt: expiry() });
      selection.mockRestore();
    }
    // Duplicates collapse to a single subscription.
    const response = await streams.open(streamRequest(agent.id, ["chat", "chat"]), agent.id);
    expect(response.status).toBe(200);
    expect(response.headers.get("content-type")).toBe("text/event-stream; charset=utf-8");
    expect(response.headers.get("cache-control")).toBe("no-store");
    const dupReader = response.body!.getReader();
    const event = await readEvent(dupReader, { text: "" });
    expect(event.event).toBe("chat");
    await dupReader.cancel();
  });

  it("delivers an initial frame for every requested topic over one response", async () => {
    const agent = await runtime.agents.create("owner", "Research", randomUUID(), `0x${"12".repeat(20)}`);
    const topics = ["chat", "dashboard", "indicators", "chart-layouts", "subagents", "notifications"];
    const response = await streams.open(streamRequest(agent.id, topics), agent.id);
    expect(response.status).toBe(200);
    const reader = response.body!.getReader();
    const buffer = { text: "" };
    const seen = new Set<string>();
    for (let i = 0; i < topics.length; i++) seen.add((await readEvent(reader, buffer)).event!);
    expect([...seen].sort()).toEqual([...topics].sort());
    await reader.cancel();
  });

  it("pushes notification revisions without exposing another Agent's content", async () => {
    const agent = await runtime.agents.create("owner", "Research", randomUUID(), `0x${"12".repeat(20)}`);
    const response = await streams.open(streamRequest(agent.id, ["notifications"]), agent.id);
    const reader = response.body!.getReader(), buffer = { text: "" };
    expect(await readEvent(reader, buffer)).toEqual({ event: "notifications", data: { revision: 0 } });
    await runtime.notifications.send(agent.id, "test", { title: "Update", body: "Useful content" });
    expect(await readEvent(reader, buffer)).toEqual({ event: "notifications", data: { revision: 1 } });
    await reader.cancel();
  });

  it("pushes committed chat snapshots over the multiplexed stream and ends cleanly", { timeout: 30_000 }, async () => {
    const agent = await runtime.agents.create("owner", "Research", randomUUID(), `0x${"12".repeat(20)}`);
    const source = new AbortController();
    const response = await streams.open(streamRequest(agent.id, ["chat"], "", source.signal), agent.id);
    expect(response.status).toBe(200);
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
    const reopened = await streams.open(streamRequest(agent.id, ["chat"]), agent.id);
    expect(reopened.status).toBe(200);
    await reopened.body!.cancel();
  });

  it("pushes committed chart layouts without polling and resynchronizes on reconnect", async () => {
    const agent = await runtime.agents.create("owner", "Charts", randomUUID(), `0x${"12".repeat(20)}`);
    const response = await streams.open(streamRequest(agent.id, ["chart-layouts"]), agent.id);
    const reader = response.body!.getReader(), buffer = { text: "" };
    expect((await readEvent<ChartLayout[]>(reader, buffer)).data).toEqual([]);
    const list = vi.spyOn(runtime.agents.charts, "list");
    try {
      vi.useFakeTimers();
      await vi.advanceTimersByTimeAsync(60_000);
      expect(list).not.toHaveBeenCalled();
    } finally { vi.useRealTimers(); }
    const layout = await runtime.agents.charts.create(agent.id, randomUUID(), { name: "BTC levels" });
    expect((await readEvent<ChartLayout[]>(reader, buffer)).data).toEqual([layout]);
    list.mockClear();
    await runtime.agents.charts.edit(agent.id, layout.id, 0, { label: "Renamed" }, "rename");
    expect((await readEvent<ChartLayout[]>(reader, buffer)).data[0]).toMatchObject({ revision: 1 });
    await reader.cancel();
    vi.restoreAllMocks();
  });

  it("pushes cached/live dashboard state without tying the feed to browser lifetime", async () => {
    const agent = await runtime.agents.create("owner", "Trading", randomUUID(), `0x${"12".repeat(20)}`);
    const createdFeed = feedEvents;
    const response = await streams.open(streamRequest(agent.id, ["dashboard"]), agent.id);
    const reader = response.body!.getReader();
    const buffer = { text: "" };
    const initial = await readEvent<AgentDashboard>(reader, buffer);
    expect(initial.event).toBe("dashboard");
    expect(initial.data.perpEquity).toBeNull();
    feedEvents!.update("perps", { perpEquity: "9007199254740993.12", positions: [] });
    const updated = await readEvent<AgentDashboard>(reader, buffer);
    expect(updated.data.perpEquity).toBe("9007199254740993.12");
    await reader.cancel();
    expect(closeFeed).not.toHaveBeenCalled();
    expect(feedEvents).toBe(createdFeed);
    feedEvents!.update("spot", { spotUsdc: "23" });
    const reopened = await streams.open(streamRequest(agent.id, ["dashboard"]), agent.id);
    const reopenedReader = reopened.body!.getReader();
    const cached = await readEvent<AgentDashboard>(reopenedReader, { text: "" });
    expect(cached.data.spotUsdc).toBe("23");
    expect(cached.data.perpEquity).toBe("9007199254740993.12");
    await reopenedReader.cancel();
  });


  it("pushes dashboard workers without polling", { timeout: 30_000 }, async () => {
    const agent = await runtime.agents.create("owner", "Research", randomUUID(), `0x${"12".repeat(20)}`);
    await runtime.agents.subagents.tick();
    const response = await streams.open(streamRequest(agent.id, ["subagents"]), agent.id);
    const reader = response.body!.getReader(), buffer = { text: "" };
    expect((await readEvent<DashboardSubagent[]>(reader, buffer)).data).toEqual([]);
    const dashboard = vi.spyOn(runtime.agents.subagents, "dashboard");
    try {
      await runtime.agents.subagents.tick();
      expect(dashboard).not.toHaveBeenCalled();
      const worker = await runtime.agents.subagents.create("owner", agent.id, { name: "Research", description: "", instructions: "Find evidence", provider: "faux", modelId: "faux-1", thinkingLevel: "", tools: [], enabled: false, delegatable: true, persistent: false, schedule: "", delaySeconds: 30, completion: "save", timeoutSeconds: 900 }, randomUUID());
      expect((await readEvent<DashboardSubagent[]>(reader, buffer)).data[0]).toMatchObject({ id: worker.id, enabled: false });
    } finally { dashboard.mockRestore(); }
    await reader.cancel();
  });

  it("keeps per-topic newest-wins: a busy topic never drops another topic's update", async () => {
    const agent = await runtime.agents.create("owner", "Research", randomUUID(), `0x${"12".repeat(20)}`);
    const flooding = vi.spyOn(runtime.agents, "watchChat").mockImplementation(async (_userId, _id, signal, publish) => {
      for (let revision = 0; revision < 20; revision++) publish({ marker: revision } as unknown as Parameters<typeof publish>[0]);
      await new Promise<void>((resolve) => signal.addEventListener("abort", () => resolve(), { once: true }));
    });
    try {
      const response = await streams.open(streamRequest(agent.id, ["chat", "indicators"]), agent.id);
      const reader = response.body!.getReader(), buffer = { text: "" };
      const events: { event?: string; data: { marker?: number; revision?: number } }[] = [];
      for (let i = 0; i < 4 && !events.some((entry) => entry.event === "indicators"); i++) {
        events.push(await readEvent(reader, buffer));
      }
      expect(events.some((entry) => entry.event === "indicators")).toBe(true);
      const chats = events.filter((entry) => entry.event === "chat");
      expect(chats.length).toBeGreaterThan(0);
      expect(chats.at(-1)!.data.marker).toBe(19);
      await reader.cancel();
    } finally { flooding.mockRestore(); }
  });

  it("isolates oversize frames to their topic and keeps the connection alive", async () => {
    const agent = await runtime.agents.create("owner", "Research", randomUUID(), `0x${"12".repeat(20)}`);
    const stopped = vi.fn();
    const oversized = vi.spyOn(runtime.agents.indicators, "watch").mockImplementation(async (_id, signal, publish) => {
      const done = new Promise<void>((resolve) => signal.addEventListener("abort", () => { stopped(); resolve(); }, { once: true }));
      publish({ revision: 1, filler: "x".repeat(10_000) } as unknown as Parameters<typeof publish>[0]);
      await done;
    });
    try {
      const response = await streams.open(streamRequest(agent.id, ["chat", "indicators"]), agent.id);
      const reader = response.body!.getReader(), buffer = { text: "" };
      const first = await readEvent<{ topic?: string; message?: string }>(reader, buffer);
      const second = await readEvent<{ topic?: string; message?: string }>(reader, buffer);
      const frames = [first, second];
      const failure = frames.find((entry) => entry.event === "stream-error")!;
      expect(failure.data.topic).toBe("indicators");
      expect(failure.data.message).toBeTruthy();
      expect(frames.some((entry) => entry.event === "chat")).toBe(true);
      expect(stopped).toHaveBeenCalledOnce();
      // The surviving topic still streams: a committed chat update arrives on the same connection.
      faux.setResponses([fauxAssistantMessage("Still live")]);
      await runtime.agents.send("owner", agent.id, "Hello again", randomUUID());
      let live: ChatEvent | undefined;
      for (let attempts = 0; !live && attempts < 40; attempts++) {
        const event = await readEvent(reader, buffer);
        if (event.event === "chat" && event.data.messages.some((entry: { text: string }) => entry.text === "Still live")) live = event;
      }
      expect(live).toBeDefined();
      await reader.cancel();
    } finally { oversized.mockRestore(); }
  });

  it("counts a multiplexed stream as one connection toward the per-user cap", async () => {
    const agent = await runtime.agents.create("owner", "Research", randomUUID(), `0x${"12".repeat(20)}`);
    const controllers = Array.from({ length: 10 }, () => new AbortController());
    const opened = await Promise.all(controllers.map((controller) => streams.open(streamRequest(agent.id, ["indicators"], "", controller.signal), agent.id)));
    expect(opened.map((response) => response.status)).toEqual(Array(10).fill(200));
    expect((await streams.open(streamRequest(agent.id, ["indicators"]), agent.id)).status).toBe(429);
    controllers[0]!.abort();
    await Promise.resolve();
    const reopened = await streams.open(streamRequest(agent.id, ["indicators"]), agent.id);
    expect(reopened.status).toBe(200);
    await reopened.body!.cancel();
    for (const controller of controllers.slice(1)) controller.abort();
    await Promise.all(opened.slice(1).map((response) => response.body!.cancel()));
  });

  it("stops every topic watcher when the browser disconnects", async () => {
    const agent = await runtime.agents.create("owner", "Research", randomUUID(), `0x${"12".repeat(20)}`);
    const chatStopped = vi.fn(), indicatorsStopped = vi.fn();
    const chatWatch = vi.spyOn(runtime.agents, "watchChat").mockImplementation(async (_userId, _id, signal) => {
      await new Promise<void>((resolve) => signal.addEventListener("abort", () => { chatStopped(); resolve(); }, { once: true }));
    });
    const indicatorsWatch = vi.spyOn(runtime.agents.indicators, "watch").mockImplementation(async (_id, signal) => {
      await new Promise<void>((resolve) => signal.addEventListener("abort", () => { indicatorsStopped(); resolve(); }, { once: true }));
    });
    try {
      const source = new AbortController();
      const response = await streams.open(streamRequest(agent.id, ["chat", "indicators"], "", source.signal), agent.id);
      expect(response.status).toBe(200);
      source.abort();
      await vi.waitFor(() => expect(chatStopped).toHaveBeenCalledOnce());
      expect(indicatorsStopped).toHaveBeenCalledOnce();
      await response.body!.cancel();
    } finally {
      chatWatch.mockRestore();
      indicatorsWatch.mockRestore();
    }
  });

  it("immediately closes multiplexed streams when deletion starts and refuses reconnects", async () => {
    const agent = await runtime.agents.create("owner", "Research", randomUUID(), `0x${"12".repeat(20)}`);
    const response = await streams.open(streamRequest(agent.id, ["chat"]), agent.id);
    const reader = response.body!.getReader();
    const buffer = { text: "" };
    expect((await readEvent(reader, buffer)).event).toBe("chat");
    await runtime.agents.delete("owner", agent.id);
    expect((await readEvent(reader, buffer)).event).toBe("unavailable");
    expect((await reader.read()).done).toBe(true);
    expect((await streams.open(streamRequest(agent.id, ["chat"]), agent.id)).status).toBe(404);
    expect(closeFeed).toHaveBeenCalled();
  });

  it("authenticates chart topics before market requests and closes their backend watcher", async () => {
    const agent = await runtime.agents.create("owner", "Chart", randomUUID(), `0x${"12".repeat(20)}`);
    const selection = vi.spyOn(runtime.hyperliquid.charts, "selection").mockResolvedValue({ coin: "BTC", interval: "1h" });
    const stopped = vi.fn();
    const watch = vi.spyOn(runtime.hyperliquid.charts, "watch").mockImplementation((_selection, signal, publish) => new Promise<void>((resolve) => {
      publish({ candles: [], status: "connecting" });
      signal.addEventListener("abort", () => { stopped(); resolve(); }, { once: true });
    }));
    try {
      mocks.session.mockResolvedValue({ user: { id: "other" }, expiresAt: expiry() });
      expect((await streams.open(streamRequest(agent.id, ["chart"], "&coin=BTC&interval=1h"), agent.id)).status).toBe(404);
      expect(selection).not.toHaveBeenCalled();
      mocks.session.mockResolvedValue({ user: { id: "owner" }, expiresAt: expiry() });
      const response = await streams.open(streamRequest(agent.id, ["chart"], "&coin=BTC&interval=1h"), agent.id);
      const chartReader = response.body!.getReader();
      const event = await readEvent(chartReader, { text: "" });
      expect(event).toEqual({ event: "chart", data: { candles: [], status: "connecting" } });
      expect(selection).toHaveBeenCalledWith("BTC", "1h");
      expect(watch).toHaveBeenCalledOnce();
      await chartReader.cancel();
      await vi.waitFor(() => expect(stopped).toHaveBeenCalledOnce());
    } finally {
      vi.restoreAllMocks();
      mocks.session.mockResolvedValue({ user: { id: "owner", walletAddress: "0xfixture" }, expiresAt: expiry() });
    }
  });

  it("notifies subscribers of shutdown and stops admitting streams", { timeout: 30_000 }, async () => {
    const agent = await runtime.agents.create("owner", "Research", randomUUID(), `0x${"12".repeat(20)}`);
    const response = await streams.open(streamRequest(agent.id, ["chat", "indicators"]), agent.id);
    const reader = response.body!.getReader();
    const buffer = { text: "" };
    await readEvent(reader, buffer); // initial snapshot
    await streams.close();
    const end = await readEvent(reader, buffer);
    expect(end.event).toBe("shutdown");
    expect((await streams.open(streamRequest(agent.id, ["chat"]), agent.id)).status).toBe(503);
  });

  it("ends the stream when the session disappears between chat updates", { timeout: 30_000 }, async () => {
    vi.useFakeTimers();
    try {
      const agent = await runtime.agents.create("owner", "Research", randomUUID(), `0x${"12".repeat(20)}`);
      const response = await streams.open(streamRequest(agent.id, ["chat"]), agent.id);
      const reader = response.body!.getReader();
      const buffer = { text: "" };
      await readEvent(reader, buffer);
      mocks.session.mockResolvedValue(null);
      await vi.advanceTimersByTimeAsync(60_000);
      const end = await readEvent(reader, buffer);
      expect(end.event).toBe("unauthorized");
      expect((await reader.read()).done).toBe(true);
      await streams.close();
    } finally {
      vi.useRealTimers();
    }
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
});
