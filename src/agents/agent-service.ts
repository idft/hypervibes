import { BACKGROUND_CONTEXT as context } from "@earendil-works/chord/context";
import { createModels, type Models } from "@earendil-works/pi-ai/models";
import { opencodeGoProvider } from "@earendil-works/pi-ai/providers/opencode-go";
import { createRegistry, Harness, InboxDoc, LiveDoc } from "@earendil-works/pi-durable";
import type { Conversation, ConversationView, EntryRecord, InboxState, LiveState, ModelRef } from "@earendil-works/pi-durable";
import { SqliteStorage } from "@earendil-works/pi-durable/storage/sqlite";
import { and, desc, eq } from "drizzle-orm";
import { randomUUID } from "node:crypto";
import { hvAgents, type AgentRow } from "../db/agent-schema.js";
import type { ApplicationDatabase, SharedDatabase } from "../db/shared-database.js";

import { AgentError, agentId, agentName, chatMessage, harnessPrefix, requestId } from "./agent-input.js";

const defaultModel: ModelRef = { provider: "opencode-go", modelId: "deepseek-v4.1-flash" };

export interface AgentServiceOptions {
  /** Injected model catalog for isolated tests; production installs only OpenCode Go. */
  models?: Models;
  model?: ModelRef;
}
interface RunningAgent { harness: Harness; main: Conversation }
export interface ChatEntry { id: string; role: "user" | "assistant"; text: string; failed: boolean }
export interface AgentChat {
  agent: AgentRow;
  model: ModelRef;
  configured: boolean;
  busy: boolean;
  queued: number;
  messages: ChatEntry[];
  partial: string;
}

/** Owns every Agent harness; no request owns a harness or waits for model work. */
export class AgentService {
  private readonly models: Models;
  private readonly model: ModelRef;
  private readonly running = new Map<string, RunningAgent>();
  private readonly opening = new Map<string, Promise<RunningAgent>>();
  private readonly submitting = new Map<string, Promise<void>>();
  private readonly lifecycle = new Map<string, Set<() => void>>();
  private readonly feedShutdown = new AbortController();
  private readonly feeds = new Set<Promise<void>>();
  private stopping = false;
  private scheduling = false;
  private closing?: Promise<void>;
  constructor(private readonly db: ApplicationDatabase, private readonly database: SharedDatabase, private readonly options: AgentServiceOptions = {}) {
    const models = createModels();
    models.setProvider(opencodeGoProvider());
    this.models = options.models ?? models;
    this.model = options.model ?? defaultModel;
  }
  private notify(id: string) { for (const listener of this.lifecycle.get(id) ?? []) listener(); }
  private configured() { return Boolean(this.options.models || process.env.OPENCODE_API_KEY?.trim()); }
  private admit() { if (this.stopping) throw new AgentError("Agent execution is shutting down", 503); }
  async list(userId: string) {
    this.admit();
    return this.db.select().from(hvAgents).where(eq(hvAgents.userId, userId)).orderBy(desc(hvAgents.createdAt)).all();
  }
  async owned(userId: string, id: string) {
    this.admit();
    const row = await this.db.select().from(hvAgents).where(and(eq(hvAgents.id, agentId(id)), eq(hvAgents.userId, userId))).get();
    if (!row) throw new AgentError("Agent not found", 404);
    return row;
  }
  async create(userId: string, name: string, creationRequestId: string) {
    this.admit();
    name = agentName(name);
    creationRequestId = requestId(creationRequestId);
    const now = Date.now();
    const [row] = await this.db.insert(hvAgents).values({
      id: randomUUID(), userId, name, creationRequestId, createdAt: now, updatedAt: now,
    }).onConflictDoNothing({ target: [hvAgents.userId, hvAgents.creationRequestId] }).returning().all();
    const agent = row ?? await this.db.select().from(hvAgents).where(and(eq(hvAgents.userId, userId), eq(hvAgents.creationRequestId, creationRequestId))).get();
    if (!agent) throw new AgentError("Unable to create Agent", 503);
    if (agent.name !== name) throw new AgentError("This creation request was already used with a different name", 409);
    try { await this.open(agent); } catch { /* Persisted failure is returned for visible retry. */ }
    return this.owned(userId, agent.id);
  }
  private open(agent: AgentRow): Promise<RunningAgent> {
    this.admit();
    const running = this.running.get(agent.id);
    if (running) return Promise.resolve(running);
    const existing = this.opening.get(agent.id);
    if (existing) return existing;
    const opening = this.initialize(agent).finally(() => { this.opening.delete(agent.id); });
    this.opening.set(agent.id, opening);
    return opening;
  }
  private async initialize(agent: AgentRow): Promise<RunningAgent> {
    let harness: Harness | undefined;
    try {
      await this.db.update(hvAgents).set({ state: "provisioning", error: null, updatedAt: Date.now() }).where(eq(hvAgents.id, agent.id)).run();
      this.notify(agent.id);
      const storage = await SqliteStorage.open(this.database.borrow(), { prefix: harnessPrefix(agent.id) });
      try {
        harness = await Harness.open(storage, {
          models: this.models, registry: createRegistry(),
          settings: { retry: { maxRetries: 2 }, stream: { timeoutMs: 120_000 } },
        }, context);
      } catch (error) { await storage.close(context); throw error; }
      // root() is idempotent inside this Agent's store, including a crash before
      // the application records its ID. No cross-store transaction is required.
      const main = await harness.root(context, { agent: {
        model: this.model, extensions: [], tools: [],
        instructions: "You are the user's HyperVibes assistant. Help discuss trading strategies and answer questions. Trading and live exchange data are not configured. Do not claim to place orders or access live account information.",
      } });
      if (agent.conversationId !== null && agent.conversationId !== main.id) throw new Error("Agent root identity changed");
      this.admit();
      await this.db.update(hvAgents).set({ state: "ready", conversationId: main.id, error: null, updatedAt: Date.now() }).where(eq(hvAgents.id, agent.id)).run();
      const current = { harness, main };
      this.running.set(agent.id, current);
      this.notify(agent.id);
      void harness.closed.then(async (end) => {
        if (this.running.get(agent.id) !== current) return;
        this.running.delete(agent.id);
        if (end.reason === "failed" && !this.stopping) {
          await this.db.update(hvAgents).set({ state: "failed", error: "Agent execution stopped. Retry initialization.", updatedAt: Date.now() }).where(eq(hvAgents.id, agent.id)).run();
          this.notify(agent.id);
        }
      }).catch(() => { /* A shared-storage failure is also reported by runtime health. */ });
      if (this.scheduling && this.configured()) harness.resume();
      return current;
    } catch (error) {
      await harness?.close(context);
      if (!this.stopping) {
        await this.db.update(hvAgents).set({ state: "failed", error: "Unable to initialize Agent. Retry initialization.", updatedAt: Date.now() }).where(eq(hvAgents.id, agent.id)).run();
        this.notify(agent.id);
      }
      throw error;
    }
  }
  start() {
    this.admit();
    this.scheduling = true;
    if (this.configured()) for (const { harness } of this.running.values()) harness.resume();
  }
  async recover() {
    const agents = await this.db.select().from(hvAgents).all();
    for (const agent of agents) {
      try { await this.open(agent); } catch { /* One failed Agent must not hide other Agents. */ }
    }
  }
  async retry(userId: string, id: string) {
    const agent = await this.owned(userId, id);
    try { await this.open(agent); } catch { /* Return the persisted, sanitized failure. */ }
    return this.owned(userId, id);
  }
  async chat(userId: string, id: string): Promise<AgentChat> {
    const agent = await this.owned(userId, id);
    const current = this.running.get(agent.id);
    if (!current) return this.projectChat(agent, []);
    const { harness, main } = current;
    const [entries, live, inbox] = await Promise.all([
      main.entries({ order: "descending" }, 100, undefined, context),
      harness.snapshot(LiveDoc, main.id, context), harness.snapshot(InboxDoc, main.id, context),
    ]);
    return this.projectChat(agent, [...entries.items].reverse(), live, inbox);
  }
  private projectChat(agent: AgentRow, entries: readonly EntryRecord[], live?: Readonly<LiveState>, inbox?: Readonly<InboxState>): AgentChat {
    const result: AgentChat = { agent, model: this.model, configured: this.configured(), busy: false, queued: 0, messages: [], partial: "" };
    result.busy = Boolean(live?.run);
    result.queued = inbox?.items.length ?? 0;
    for (const entry of entries.slice(-100)) {
      for (const [index, message] of (entry.model ?? []).entries()) {
        if (message.role !== "user" && message.role !== "assistant") continue;
        const failed = message.role === "assistant" && (message.stopReason === "error" || message.stopReason === "aborted");
        const text = failed ? "Response interrupted or failed. You can send another message to retry." : typeof message.content === "string" ? message.content : message.content.filter((part) => part.type === "text").map((part) => part.text).join("");
        if (text) result.messages.push({ id: `${entry.id}:${index}`, role: message.role, text, failed });
      }
    }
    result.partial = live?.generation?.message?.content.filter((part) => part.type === "text").map((part) => part.text).join("") ?? "";
    return result;
  }
  /** Pushes committed snapshots; no SQL reads are needed for conversation updates. */
  async watchChat(userId: string, id: string, signal: AbortSignal, publish: (chat: AgentChat) => void): Promise<void> {
    this.admit();
    id = agentId(id);
    const lifetime = AbortSignal.any([signal, this.feedShutdown.signal]);
    let finish!: () => void;
    const finished = new Promise<void>((resolve) => { finish = resolve; });
    this.feeds.add(finished);
    let wake: (() => void) | undefined;
    let changed = true;
    let lifecycleChanged = true;
    let binding: RunningAgent | undefined;
    let watch: Awaited<ReturnType<Conversation["watch"]>> | undefined;
    let view: ConversationView | undefined;
    const notify = () => { changed = true; wake?.(); };
    const onLifecycle = () => { lifecycleChanged = true; notify(); };
    const listeners = this.lifecycle.get(id) ?? new Set<() => void>();
    this.lifecycle.set(id, listeners);
    listeners.add(onLifecycle);
    lifetime.addEventListener("abort", notify);
    try {
      let agent = await this.owned(userId, id);
      while (!lifetime.aborted) {
        if (lifecycleChanged) {
          lifecycleChanged = false;
          agent = await this.owned(userId, id);
          const current = this.running.get(agent.id);
          if (current !== binding) {
            await watch?.stop();
            watch = undefined;
            view = undefined;
            binding = current;
            if (current && !lifetime.aborted) {
              const attached = await current.main.watch(context);
              watch = attached;
              view = attached.value;
              attached.start(async (value) => {
                if (watch === attached) { view = value; notify(); }
              });
              void attached.closed.then(() => { if (watch === attached) onLifecycle(); });
            }
          }
        }
        if (lifetime.aborted) break;
        if (changed) {
          changed = false;
          publish(this.projectChat(agent, view?.entries ?? [],
            view?.docs["pi.live"] as unknown as LiveState | undefined,
            view?.docs["pi.inbox"] as unknown as InboxState | undefined));
        }
        if (!changed && !lifecycleChanged) await new Promise<void>((resolve) => { wake = resolve; });
        wake = undefined;
      }
    } finally {
      listeners.delete(onLifecycle);
      if (listeners.size === 0) this.lifecycle.delete(id);
      lifetime.removeEventListener("abort", notify);
      try { await watch?.stop(); }
      finally { this.feeds.delete(finished); finish(); }
    }
  }
  async send(userId: string, id: string, message: string, submissionRequestId: string) {
    const agent = await this.owned(userId, id);
    if (!this.scheduling) throw new AgentError("Agent execution is not ready", 503);
    message = chatMessage(message);
    submissionRequestId = requestId(submissionRequestId);
    if (!this.configured()) throw new AgentError("Chat requires OPENCODE_API_KEY on the server", 503);
    // Serialize admission (not model execution) so concurrent browser requests
    // cannot race past the persisted inbox limit.
    const operation = (this.submitting.get(agent.id) ?? Promise.resolve()).then(async () => {
      this.admit();
      const current = this.running.get(agent.id);
      if (!current) throw new AgentError("Agent is unavailable. Retry initialization.", 503);
      const inbox = await current.harness.snapshot(InboxDoc, current.main.id, context);
      if ((inbox?.items.length ?? 0) >= 20) throw new AgentError("Agent message queue is full. Wait for a response.", 429);
      const submission = await current.main.submit({ type: "input", content: message, requestId: submissionRequestId }, context);
      return { submissionId: submission.id };
    });
    const settled = operation.then(() => {}, () => {});
    this.submitting.set(agent.id, settled);
    void settled.then(() => { if (this.submitting.get(agent.id) === settled) this.submitting.delete(agent.id); });
    return operation;
  }
  async stop(userId: string, id: string) {
    const agent = await this.owned(userId, id);
    await this.running.get(agent.id)?.main.abort(context, { background: true });
  }
  close() {
    this.stopping = true;
    this.feedShutdown.abort();
    return this.closing ??= (async () => {
      await Promise.all(this.feeds);
      await Promise.all(this.submitting.values());
      await Promise.allSettled(this.opening.values());
      const results = await Promise.allSettled([...this.running.values()].map(({ harness }) => harness.close(context)));
      this.running.clear();
      const errors = results.filter((result) => result.status === "rejected").map((result) => result.reason as unknown);
      if (errors.length) throw new AggregateError(errors, "Unable to close Agent execution");
    })();
  }
}
