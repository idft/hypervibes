import type { OrderService } from "../hyperliquid/orders/order-service.js";
import { createHyperliquidExtension } from "../hyperliquid/durable-extension.js";
import type { HyperliquidService } from "../hyperliquid/service.js";
import { BACKGROUND_CONTEXT as context } from "@earendil-works/chord/context";
import { getSupportedThinkingLevels, type Models } from "@earendil-works/pi-ai/models";
import { createRegistry, Harness, InboxDoc, LiveDoc } from "@earendil-works/pi-durable";
import type { Conversation, ConversationView, EntryRecord, InboxState, LiveState, ModelRef } from "@earendil-works/pi-durable";
import { SqliteStorage } from "@earendil-works/pi-durable/storage/sqlite";
import { and, desc, eq } from "drizzle-orm";
import { randomUUID } from "node:crypto";
import { hvAgents, type AgentRow } from "../db/agent-schema.js";
import type { ApplicationDatabase, SharedDatabase } from "../db/shared-database.js";

import { AgentError, agentId, agentName, chatMessage, harnessPrefix, requestId } from "./agent-input.js";
import type { ProviderService } from "../providers/provider-service.js";

// Existing scaffold roots retain their original model, never silently switch.
const legacyModel: ModelRef = { provider: "opencode-go", modelId: "deepseek-v4.1-flash" };

export interface AgentServiceOptions {
  /** Injected model catalog for isolated tests; never used by the production host. */
  models?: Models;
  model?: ModelRef;
}
interface RunningAgent { harness: Harness; main: Conversation }
export interface ChatEntry { id: string; role: "user" | "assistant"; text: string; failed: boolean }
export interface AgentChat {
  agent: AgentRow;
  model: ModelRef;
  configured: boolean;
  thinkingLevel?: string;
  busy: boolean;
  queued: number;
  messages: ChatEntry[];
  partial: string;
}

/** Owns every Agent harness; no request owns a harness or waits for model work. */
export class AgentService {
  private readonly running = new Map<string, RunningAgent>();
  private readonly opening = new Map<string, Promise<RunningAgent>>();
  private readonly submitting = new Map<string, Promise<void>>();
  private readonly lifecycle = new Map<string, Set<() => void>>();
  private readonly feedShutdown = new AbortController();
  private readonly feeds = new Set<Promise<void>>();
  private stopping = false;
  private scheduling = false;
  private closing?: Promise<void>;
  private resuming: Promise<void> = Promise.resolve();
  constructor(private readonly db: ApplicationDatabase, private readonly database: SharedDatabase, private readonly providers: ProviderService, private readonly hyperliquid: HyperliquidService, private readonly orders: OrderService, private readonly options: AgentServiceOptions = {}) {}
  private notify(id: string) { for (const listener of this.lifecycle.get(id) ?? []) listener(); }
  private models(userId: string) { return this.options.models ? Promise.resolve(this.options.models) : this.providers.models(userId); }
  private async configured(agent: AgentRow, current: RunningAgent) {
    const model = (await current.main.agent(context)).model;
    return Boolean(model && (this.options.models || await this.providers.configured(agent.userId, model)));
  }
  async providersChanged(userId: string) {
    if (this.stopping) return;
    for (const agent of await this.list(userId)) {
      const current = this.running.get(agent.id);
      if (current && this.scheduling && await this.configured(agent, current) && !this.stopping) current.harness.resume();
      this.notify(agent.id);
    }
  }
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
  private static uniqueAccountViolation(error: unknown): boolean {
    let current: unknown = error;
    for (let depth = 0; current !== null && typeof current === "object" && depth < 5; depth++) {
      const message = (current as { message?: unknown }).message;
      if (typeof message === "string" && message.includes("UNIQUE constraint failed") && message.includes("trading_account_address")) return true;
      current = (current as { cause?: unknown }).cause;
    }
    return false;
  }

  async create(userId: string, name: string, creationRequestId: string, tradingAccountAddress?: string, selection?: ModelRef) {
    this.admit();
    name = agentName(name);
    creationRequestId = requestId(creationRequestId);
    const account = typeof tradingAccountAddress === "string" && tradingAccountAddress ? tradingAccountAddress : null;
    const model = this.options.models ? selection ?? this.options.model ?? legacyModel : await this.providers.validate(userId, selection);
    const now = Date.now();
    let agent: AgentRow | undefined;
    try {
      const [row] = await this.db.insert(hvAgents).values({
        id: randomUUID(), userId, name, creationRequestId, tradingAccountAddress: account,
        initialProvider: model.provider, initialModelId: model.modelId, createdAt: now, updatedAt: now,
      }).onConflictDoNothing({ target: [hvAgents.userId, hvAgents.creationRequestId] }).returning().all();
      agent = row ?? await this.db.select().from(hvAgents).where(and(eq(hvAgents.userId, userId), eq(hvAgents.creationRequestId, creationRequestId))).get();
    } catch (error) {
      // The unique trading-account index is the one domain conflict the caller
      // can resolve; every other failure must surface as a retryable error.
      if (AgentService.uniqueAccountViolation(error)) {
        throw new AgentError("That trading account is already assigned to another agent.", 409);
      }
      throw error;
    }
    if (!agent) throw new AgentError("Unable to create Agent", 503);
    if (agent.name !== name || agent.tradingAccountAddress !== account || agent.initialProvider !== model.provider || agent.initialModelId !== model.modelId) {
      throw new AgentError("This creation request was already used with different settings", 409);
    }
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
      const registry = createRegistry();
      const hyperliquid = createHyperliquidExtension(this.hyperliquid, agent.userId, agent.id, this.orders);
      registry.install(hyperliquid);
      const storage = await SqliteStorage.open(this.database.borrow(), { prefix: harnessPrefix(agent.id) });
      try {
        harness = await Harness.open(storage, {
          models: await this.models(agent.userId), registry,
          settings: { retry: { maxRetries: 2 }, stream: { timeoutMs: 120_000 } },
        }, context);
      } catch (error) { await storage.close(context); throw error; }
      // root() is idempotent inside this Agent's store, including a crash before
      // the application records its ID. No cross-store transaction is required.
      const main = await harness.root(context, { agent: {
        model: agent.initialProvider && agent.initialModelId ? { provider: agent.initialProvider, modelId: agent.initialModelId } : this.options.model ?? legacyModel,
      } });
      if (agent.conversationId !== null && agent.conversationId !== main.id) throw new Error("Agent root identity changed");
      // root options apply only on creation. Configure before scheduling so old
      // scaffold roots also gain the extension without changing model/history.
      await main.configure({
        extensions: [hyperliquid], tools: null,
        instructions: "You are the user's HyperVibes assistant. Help discuss trading strategies and answer questions. Use the available tools for account information and order execution, following the Hyperliquid extension's instructions. Describe confirmed outcomes accurately; never claim an unknown action failed or succeeded.",
      }, context);
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
      if (this.scheduling && await this.configured(agent, current)) harness.resume();
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
    if (this.options.models) for (const { harness } of this.running.values()) harness.resume();
    else this.resuming = this.resumeConfigured().catch(() => { /* Storage health reports failures; admission also checks configuration. */ });
  }
  private async resumeConfigured() {
    const agents = await this.db.select().from(hvAgents).all();
    for (const agent of agents) {
      if (this.stopping) return;
      const current = this.running.get(agent.id);
      if (current && await this.configured(agent, current) && !this.stopping) current.harness.resume();
    }
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
  private async projectChat(agent: AgentRow, entries: readonly EntryRecord[], live?: Readonly<LiveState>, inbox?: Readonly<InboxState>): Promise<AgentChat> {
    const current = this.running.get(agent.id);
    const resolved = await current?.main.agent(context);
    const model = resolved?.model ?? (agent.initialProvider && agent.initialModelId ? { provider: agent.initialProvider, modelId: agent.initialModelId } : this.options.model ?? legacyModel);
    const configured = Boolean(this.options.models || await this.providers.configured(agent.userId, model));
    const result: AgentChat = { agent, model, configured, thinkingLevel: resolved?.thinkingLevel, busy: false, queued: 0, messages: [], partial: "" };
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
          publish(await this.projectChat(agent, view?.entries ?? [],
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
    // Serialize admission (not model execution) so concurrent browser requests
    // cannot race past the persisted inbox limit.
    return this.serialize(agent.id, async () => {
      this.admit();
      const current = this.running.get(agent.id);
      if (!current) throw new AgentError("Agent is unavailable. Retry initialization.", 503);
      if (!await this.configured(agent, current)) throw new AgentError("Reconnect this Agent's provider or select an available model on the Providers page.", 409);
      const inbox = await current.harness.snapshot(InboxDoc, current.main.id, context);
      if ((inbox?.items.length ?? 0) >= 20) throw new AgentError("Agent message queue is full. Wait for a response.", 429);
      const submission = await current.main.submit({ type: "input", content: message, requestId: submissionRequestId }, context);
      return { submissionId: submission.id };
    });
  }
  private serialize<T>(id: string, action: () => Promise<T>) {
    const operation = (this.submitting.get(id) ?? Promise.resolve()).then(action);
    const settled = operation.then(() => {}, () => {});
    this.submitting.set(id, settled);
    void settled.then(() => { if (this.submitting.get(id) === settled) this.submitting.delete(id); });
    return operation;
  }
  async selectModel(userId: string, id: string, selection: ModelRef, thinkingLevel?: string) {
    const agent = await this.owned(userId, id);
    return this.serialize(agent.id, async () => {
      this.admit();
      const model = this.options.models ? selection : await this.providers.validate(userId, selection);
      const current = this.running.get(agent.id);
      if (!current) throw new AgentError("Agent is unavailable. Retry initialization.", 503);
      // Change only at idle: a prepared request may retain its previous model,
      // and changing providers must not reroute already queued spending.
      const live = await current.harness.snapshot(LiveDoc, current.main.id, context);
      const inbox = await current.harness.snapshot(InboxDoc, current.main.id, context);
      if (live?.run || inbox?.items.length) throw new AgentError("Stop or finish queued work before changing models.", 409);
      const catalogModel = (await this.models(userId)).getModel(model.provider, model.modelId);
      if (!catalogModel) throw new AgentError("Model is unavailable.", 409);
      const level = thinkingLevel ? getSupportedThinkingLevels(catalogModel).find((value) => value === thinkingLevel) : undefined;
      if (thinkingLevel && !level) throw new AgentError("Select a supported thinking level.");
      await current.main.configure({ model, thinkingLevel: level ?? null }, context);
      this.notify(agent.id);
      if (this.scheduling && !this.stopping) current.harness.resume();
    });
  }
  async stop(userId: string, id: string) {
    const agent = await this.owned(userId, id);
    await this.running.get(agent.id)?.main.abort(context, { background: true });
  }
  close() {
    this.stopping = true;
    this.feedShutdown.abort();
    return this.closing ??= (async () => {
      await this.resuming;
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
