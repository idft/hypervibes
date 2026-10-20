import { BACKGROUND_CONTEXT as context } from "@earendil-works/chord/context";
import { getSupportedThinkingLevels, type Models } from "@earendil-works/pi-ai/models";
import { InboxDoc, LiveDoc, type AgentChange, type Conversation, type ConversationId, type Cursor, type EntryRecord, type Extension, type Harness, type ModelRef } from "@earendil-works/pi-durable";
import { and, asc, count, desc, eq, inArray, isNull, lte, or, sql } from "drizzle-orm";
import { randomUUID } from "node:crypto";
import type { AgentRow } from "../db/agent-schema.js";import { hvSubagents, hvSubagentRuns, type SubagentRow, type SubagentRun } from "../db/subagent-schema.js";
import type { ApplicationDatabase, SharedDatabase } from "../db/shared-database.js";
import type { FastifyBaseLogger } from "fastify";
import type { ProviderService } from "../providers/provider-service.js";
import { runHistoryPageSize, runPagination, type RunHistory } from "./subagent-history.js";
import { AgentError, requestId } from "./agent-input.js";
import { createSubagentExtension, RunReceipt, type RunnerId, type SubagentExtension } from "./subagent-extension.js";
import { executionWarnings, type ExecutionWarning } from "./execution-warnings.js";
import { toolActivity, type ToolActivity } from "../research/tool-activity.js";
import type { ExecutionContextService } from "./execution-context.js";
import type { RunContext } from "./execution-context-contracts.js";
import { bodyHash, canonical } from "../canonical.js";
import { subagentTemplate } from "./subagent-templates.js";
import { assignment, defaultMainInstructions, mainToolNames, nextCandleRun, subagentInput, type AssignmentSettings } from "./subagent-input.js";

interface Binding extends SubagentExtension { harness: Harness; main: Conversation; hyperliquid: Extension; capabilities: readonly Extension[]; orchestratorInsights: Extension; userId: string }
export interface SubagentHost { owned(userId: string, id: string): Promise<AgentRow>; models(userId: string): Promise<Models>; changed(id: string): void }
/** Full run detail: snapshotted configuration plus the complete transcript. */
export interface RunDetail { definition: SubagentRow; run: SubagentRun; entries: EntryRecord[]; prior: EntryRecord[]; tools?: ToolActivity[]; context?: RunContext; warnings?: ExecutionWarning[]; publications?: { ref: string; kind: string; summary: string }[] }
const active = ["queued", "running"] as const;

/** Definitions and run intents belong to the app; Durable owns execution and reporting. */
export class SubagentService {
  private readonly bindings = new Map<string, Binding>();
  private readonly pending = new Map<string, Promise<void>>();
  private timer?: ReturnType<typeof setInterval>;
  private ticking: Promise<void> = Promise.resolve();
  private tickActive = false;
  private ready = false;
  private stopping = false;
  private readonly historyListeners = new Map<string, Set<() => void>>();
  private readonly historyShutdown = new AbortController();
  private readonly historyFeeds = new Set<Promise<void>>();
  private schedulerFailure: string | null = null;
  get schedulerError() { return this.schedulerFailure; }
  set schedulerError(value: string | null) {
    if (this.schedulerFailure === value) return;
    this.schedulerFailure = value;
    for (const listeners of this.historyListeners.values()) for (const listener of listeners) listener();
  }
  private historyChanged(id: string) { for (const listener of this.historyListeners.get(id) ?? []) listener(); }
  constructor(private readonly db: ApplicationDatabase, private readonly database: SharedDatabase, private readonly providers: ProviderService, private readonly host: SubagentHost, private readonly execution: ExecutionContextService, private readonly injectedModels = false, private readonly log?: FastifyBaseLogger) {}
  extension(agentId: string, hyperliquid: Extension, capabilities: readonly Extension[]) { return createSubagentExtension(this, agentId, hyperliquid, capabilities, this.execution); }
  async bind(agent: AgentRow, harness: Harness, main: Conversation, hyperliquid: Extension, execution: SubagentExtension, capabilities: readonly Extension[], orchestratorInsights: Extension) {
    const resolved = await main.agent(context);
    const now = Date.now();
    await this.db.insert(hvSubagents).values({
      id: `main:${agent.id}`, agentId: agent.id, kind: "orchestrator", name: "Orchestrator", description: "Main conversation", instructions: defaultMainInstructions,
      provider: resolved.model!.provider, modelId: resolved.model!.modelId, thinkingLevel: resolved.thinkingLevel ?? "", tools: mainToolNames, conversationId: main.id, createdAt: now, updatedAt: now,
    }).onConflictDoNothing().run();
    const binding = { harness, main, hyperliquid, capabilities, orchestratorInsights, ...execution, userId: agent.userId };
    await this.applyMain(binding, (await this.db.select().from(hvSubagents).where(and(eq(hvSubagents.agentId, agent.id), eq(hvSubagents.kind, "orchestrator"))).get())!);
    // Mark expired children's pending generations/tools before *any* scheduler
    // resumes. abortTask only marks while paused; unlike conversation.abort it
    // does not enable scheduling. The runner then records/reports the timeout.
    const interrupted = await this.db.select().from(hvSubagentRuns).where(and(eq(hvSubagentRuns.agentId, agent.id), inArray(hvSubagentRuns.status, active))).all();
    const expiredChildren = new Set(interrupted.filter((run) => run.conversationId !== null && Date.now() >= run.createdAt + run.config.timeoutSeconds * 1000).map((run) => run.conversationId));
    if (expiredChildren.size) for (const task of (await harness.inspect(context)).tasks) {
      if (expiredChildren.has(task.record.conversationId)) await harness.abortTask(task.record.id, context);
    }
    this.bindings.set(agent.id, binding);
  }
  unbind(agentId: string) { this.bindings.delete(agentId); }
  private admit() { if (this.stopping) throw new AgentError("Subagent execution is shutting down", 503); }
  private binding(id: string) { this.admit(); const value = this.bindings.get(id); if (!value) throw new AgentError("Agent is unavailable. Retry initialization.", 503); return value; }
  serial<T>(id: string, action: () => Promise<T>, signal?: AbortSignal): Promise<T> {
    this.admit();
    const result = (this.pending.get(id) ?? Promise.resolve()).then(() => { this.admit(); signal?.throwIfAborted(); return action(); });
    const settled = result.then(() => {}, () => {}); this.pending.set(id, settled);
    void settled.then(() => { if (this.pending.get(id) === settled) this.pending.delete(id); });
    if (!signal) return result;
    // A task invocation must be able to leave an admission wait when aborted;
    // otherwise cancellation could join a task waiting on its caller's lock.
    let abort!: () => void;
    const interrupted = new Promise<never>((_resolve, reject) => { abort = () => reject(signal.reason); signal.addEventListener("abort", abort, { once: true }); if (signal.aborted) abort(); });
    return Promise.race([result, interrupted]).finally(() => signal.removeEventListener("abort", abort));
  }
  thinking(level: string): AgentChange["thinkingLevel"] { return (level || null) as AgentChange["thinkingLevel"]; }
  private async validateModel(userId: string, selection: ModelRef, thinkingLevel: string) {
    const model = this.injectedModels ? selection : await this.providers.validate(userId, selection);
    const catalog = (await this.host.models(userId)).getModel(model.provider, model.modelId);
    if (!catalog) throw new AgentError("Model is unavailable.", 409);
    if (thinkingLevel && !getSupportedThinkingLevels(catalog).some((level) => level === thinkingLevel)) throw new AgentError("Select a supported thinking level.");
    return model;
  }
  private async applyMain(binding: Binding, row: SubagentRow) {
    const available = [...(binding.hyperliquid.tools ?? []), ...(binding.extension.tools ?? []), ...binding.capabilities.flatMap((extension) => extension.tools ?? [])];
    await binding.main.configure({ model: { provider: row.provider, modelId: row.modelId }, thinkingLevel: this.thinking(row.thinkingLevel), extensions: [binding.hyperliquid, binding.extension, ...binding.capabilities.filter((extension) => extension.tools?.some((tool) => row.tools.includes(tool.name))), binding.orchestratorInsights], tools: available.filter((tool) => row.tools.includes(tool.name)), modelTools: null, instructions: row.instructions }, context);
  }
  async list(userId: string, agentId: string) {
    await this.host.owned(userId, agentId);
    return this.db.select().from(hvSubagents).where(eq(hvSubagents.agentId, agentId)).orderBy(asc(hvSubagents.kind), asc(hvSubagents.createdAt), asc(hvSubagents.id)).all();
  }
  async dashboard(userId: string, agentId: string) {
    await this.host.owned(userId, agentId);
    return this.database.appTransaction(async (db) => {
      const definitions = await db.select({ id: hvSubagents.id, name: hvSubagents.name, enabled: hvSubagents.enabled }).from(hvSubagents).where(and(eq(hvSubagents.agentId, agentId), eq(hvSubagents.kind, "worker"), eq(hvSubagents.archived, false))).orderBy(asc(hvSubagents.createdAt), asc(hvSubagents.id)).all();
      const summaries = [];
      for (const definition of definitions) {
        const { id, status, createdAt, result } = hvSubagentRuns;
        const runs = await db.select({ id, status, createdAt, result: sql<string>`substr(${result}, 1, 240)` }).from(hvSubagentRuns).where(and(eq(hvSubagentRuns.agentId, agentId), eq(hvSubagentRuns.subagentId, definition.id))).orderBy(desc(createdAt), desc(id)).limit(5).all();
        summaries.push({ ...definition, runs });
      }
      return summaries;
    });
  }
  async detail(userId: string, agentId: string, id: string) {
    await this.host.owned(userId, agentId);
    const definition = await this.definition(agentId, id);
    const filter = and(eq(hvSubagentRuns.agentId, agentId), eq(hvSubagentRuns.subagentId, id));
    const { runs, pagination } = await this.database.appTransaction(async (db) => {
      const total = (await db.select({ total: count() }).from(hvSubagentRuns).where(filter).get())!.total;
      const runs = await db.select().from(hvSubagentRuns).where(filter).orderBy(desc(hvSubagentRuns.createdAt), desc(hvSubagentRuns.id)).limit(runHistoryPageSize).all();
      return { runs, pagination: runPagination(total) };
    });
    return { definition, runs, pagination, schedulerError: this.schedulerError };
  }
  async history(userId: string, agentId: string, id: string, page = 1): Promise<RunHistory> {
    if (!Number.isSafeInteger(page) || page < 1) throw new AgentError("Invalid history page");
    await this.host.owned(userId, agentId);
    const definition = await this.definition(agentId, id);
    const filter = and(eq(hvSubagentRuns.agentId, agentId), eq(hvSubagentRuns.subagentId, id));
    return this.database.appTransaction(async (db) => {
      const total = (await db.select({ total: count() }).from(hvSubagentRuns).where(filter).get())!.total;
      const pagination = runPagination(total, page);
      // Only prompt previews are streamed; full prompts/results/configuration
      // belong on run details. Keep 50 valid, large assignments below the SSE cap.
      const { id: runId, subagentId, source, task, status, createdAt } = hvSubagentRuns;
      const runs = await db.select({ id: runId, subagentId, source, task: sql<string>`substr(${task}, 1, 512)`, status, createdAt }).from(hvSubagentRuns).where(filter).orderBy(desc(createdAt), desc(runId)).limit(runHistoryPageSize).offset((pagination.page - 1) * runHistoryPageSize).all();
      return { runs, pagination, schedulerError: this.schedulerError, schedule: { nextRunAt: definition.enabled && !definition.archived ? definition.nextRunAt : null, updatedAt: definition.updatedAt } };
    });
  }
  async watchHistory(userId: string, agentId: string, id: string, page: number, signal: AbortSignal, publish: (history: RunHistory) => void): Promise<void> {
    this.admit();
    const lifetime = AbortSignal.any([signal, this.historyShutdown.signal]);
    let finish!: () => void;
    const finished = new Promise<void>((resolve) => { finish = resolve; });
    this.historyFeeds.add(finished);
    let dirty = true;
    let wake: (() => void) | undefined;
    const changed = () => { dirty = true; wake?.(); };
    const listeners = this.historyListeners.get(id) ?? new Set<() => void>();
    this.historyListeners.set(id, listeners); listeners.add(changed);
    lifetime.addEventListener("abort", changed);
    try {
      while (!lifetime.aborted) {
        if (dirty) {
          dirty = false;
          const snapshot = await this.history(userId, agentId, id, page);
          if (!lifetime.aborted) publish(snapshot);
        }
        if (!dirty && !lifetime.aborted) await new Promise<void>((resolve) => { wake = resolve; });
        wake = undefined;
      }
    } finally {
      lifetime.removeEventListener("abort", changed);
      listeners.delete(changed);
      if (!listeners.size) this.historyListeners.delete(id);
      this.historyFeeds.delete(finished); finish();
    }
  }
  async runDetail(userId: string, agentId: string, id: string, runId: string): Promise<RunDetail> {
    await this.host.owned(userId, agentId);
    const definition = await this.definition(agentId, id);
    const run = await this.db.select().from(hvSubagentRuns).where(and(eq(hvSubagentRuns.id, runId), eq(hvSubagentRuns.agentId, agentId), eq(hvSubagentRuns.subagentId, id))).get();
    if (!run) throw new AgentError("Assignment not found", 404);
    const entries: EntryRecord[] = [], prior: EntryRecord[] = [];
    if (run.conversationId !== null) {
      const child = await this.binding(agentId).harness.conversation(run.conversationId as ConversationId, context);
      if (!child) throw new AgentError("Assignment conversation is unavailable", 503);
      // Read a consistent snapshot through Durable's public transaction API.
      // Submission boundaries keep later persistent assignments out of this run.
      await child.commit(async (tx) => {
        const submission = await tx.submissionByRequest(child.id, `assignment:${run.id}`);
        if (submission?.entry === undefined) return;
        let cursor: Cursor | undefined;
        let ended = false;
        do {
          const page = await tx.scanEntries({ conversationId: child.id, order: "ascending" }, 200, cursor);
          for (const entry of page.items) {
            // Workers receive assignments only through plain input submissions.
            // A subsequent user entry therefore starts the next assignment,
            // including when this submission ended without an answer.
            if (entry.id > submission.entry && entry.kind === "pi.user") { ended = true; break; }
            (entry.id < submission.entry ? prior : entries).push(entry);
          }
          cursor = page.next;
        } while (cursor && !ended);
      }, context);
    }
    const live = run.conversationId !== null && (run.status === "running" || run.status === "queued") ? await this.binding(agentId).harness.snapshot(LiveDoc, run.conversationId as ConversationId, context) : undefined;
    const publications = await this.database.all<{ ref: string; kind: string; summary: string }>("SELECT 'm'||number AS ref,kind,summary FROM hv_memories WHERE agent_id=? AND source_run_id=? ORDER BY number DESC LIMIT 50", agentId, run.id);
    return { definition, run, entries, prior, tools: toolActivity(entries, live), warnings: executionWarnings(entries), context: await this.execution.runContext(agentId, run), publications };
  }
  /** Verifies the viewer owns the Agent, worker and run without reading its transcript. */
  async ownedRun(userId: string, agentId: string, id: string, runId: string) {
    await this.host.owned(userId, agentId);
    await this.definition(agentId, id);
    const run = await this.internalRun(agentId, runId);
    if (run.subagentId !== id) throw new AgentError("Assignment not found", 404);
    return run;
  }
  /** One page-scoped subscription; reconnects receive a fresh run-detail snapshot. */
  async watchRun(userId: string, agentId: string, id: string, runId: string, signal: AbortSignal, publish: (detail: RunDetail) => void): Promise<void> {
    this.admit();
    await this.ownedRun(userId, agentId, id, runId);
    const lifetime = AbortSignal.any([signal, this.historyShutdown.signal]);
    let finish!: () => void;
    const finished = new Promise<void>((resolve) => { finish = resolve; });
    this.historyFeeds.add(finished);
    let dirty = true;
    let wake: (() => void) | undefined;
    const changed = () => { dirty = true; wake?.(); };
    const listeners = this.historyListeners.get(id) ?? new Set<() => void>();
    this.historyListeners.set(id, listeners); listeners.add(changed);
    lifetime.addEventListener("abort", changed);
    let watch: Awaited<ReturnType<Conversation["watch"]>> | undefined;
    let watched: number | null = null;
    try {
      while (!lifetime.aborted) {
        if (dirty) {
          dirty = false;
          // Status changes announce themselves through the history stream; the
          // child conversation watch covers transcript entries, which nothing
          // else observes while a worker executes.
          const run = await this.internalRun(agentId, runId);
          if (run.conversationId !== null && run.conversationId !== watched) {
            await watch?.stop();
            watch = undefined; watched = null;
            const child = await this.binding(agentId).harness.conversation(run.conversationId as ConversationId, context);
            if (child && !lifetime.aborted) {
              const attached = await child.watch(context);
              watch = attached; watched = child.id;
              attached.start(async () => { if (watch === attached) changed(); });
              void attached.closed.then(() => { if (watch === attached) { watch = undefined; watched = null; changed(); } });
            }
          }
          if (!lifetime.aborted) publish(await this.runDetail(userId, agentId, id, runId));
        }
        if (!dirty && !lifetime.aborted) await new Promise<void>((resolve) => { wake = resolve; });
        wake = undefined;
      }
    } finally {
      lifetime.removeEventListener("abort", changed);
      listeners.delete(changed);
      if (!listeners.size) this.historyListeners.delete(id);
      try { await watch?.stop(); }
      finally { this.historyFeeds.delete(finished); finish(); }
    }
  }
  private async definition(agentId: string, id: string) {
    const row = await this.db.select().from(hvSubagents).where(and(eq(hvSubagents.id, id), eq(hvSubagents.agentId, agentId))).get();
    if (!row) throw new AgentError("Subagent not found", 404); return row;
  }
  async create(userId: string, agentId: string, data: unknown, creationId: string, templateId = "") {
    await this.host.owned(userId, agentId); requestId(creationId);
    return this.serial(agentId, async () => {
      const input = subagentInput(data); await this.validateModel(userId, input, input.thinkingLevel);
      const template = templateId ? subagentTemplate(templateId) : null;
      const templateOrigin = template ? { id: template.id, version: template.version, hash: bodyHash(canonical(template)) } : null;
      if (template && input.enabled) throw new AgentError("Template creation is paused by default; enable it explicitly after reviewing the saved copy.");
      const id = `worker:${creationId}`;
      const existing = await this.db.select().from(hvSubagents).where(eq(hvSubagents.id, id)).get();
      if (existing) { if (existing.agentId !== agentId || canonical(existing.templateOrigin) !== canonical(templateOrigin) || Object.entries(input).some(([key, value]) => JSON.stringify(Reflect.get(existing, key)) !== JSON.stringify(value))) throw new AgentError("Creation request was already used with different settings.", 409); return existing; }
      const now = Date.now();
      return (await this.db.insert(hvSubagents).values({ ...input, templateOrigin, id, agentId, kind: "worker", nextRunAt: input.enabled ? nextCandleRun(now, input.schedule, input.delaySeconds) : null, createdAt: now, updatedAt: now }).returning().all())[0]!;
    }).catch((error: unknown) => {
      if (!(error instanceof AgentError)) this.log?.error({ err: error, agentId, templateId, requestId: creationId }, "Sub-agent creation failed");
      throw error;
    });
  }
  async updateTools(userId: string, agentId: string, id: string, tools: unknown) {
    return this.updateSettings(userId, agentId, id, undefined, { tools });
  }
  async updateIdentity(userId: string, agentId: string, id: string, name: unknown, description: unknown) {
    return this.updateSettings(userId, agentId, id, undefined, { name, description });
  }
  async updateAssignments(userId: string, agentId: string, id: string, data: unknown) {
    if (!data || typeof data !== "object") throw new AgentError("Invalid assignment settings.");
    return this.updateSettings(userId, agentId, id, undefined, {
      delegatable: Reflect.get(data, "delegatable") as unknown,
      persistent: Reflect.get(data, "persistent") as unknown, timeoutSeconds: Reflect.get(data, "timeoutSeconds") as unknown,
    });
  }
  async updateSection(userId: string, agentId: string, id: string, section: string, data: unknown) {
    if (!data || typeof data !== "object") throw new AgentError("Invalid settings.");
    const get = (key: string): unknown => Reflect.get(data, key);
    switch (section) {
      case "enabled": return this.updateSettings(userId, agentId, id, undefined, { enabled: get("enabled") });
      case "instructions": return this.updateSettings(userId, agentId, id, undefined, { instructions: get("instructions") });
      case "model": return this.updateSettings(userId, agentId, id, undefined, { provider: get("provider"), modelId: get("modelId"), thinkingLevel: get("thinkingLevel") });
      case "schedule": return this.updateSettings(userId, agentId, id, undefined, { schedule: get("schedule"), delaySeconds: get("delaySeconds"), completion: get("completion") });
      default: throw new AgentError("Unknown settings section.");
    }
  }
  async update(userId: string, agentId: string, id: string, data: unknown) {
    return this.updateSettings(userId, agentId, id, data);
  }
  private async updateSettings(userId: string, agentId: string, id: string, data: unknown, patch?: { tools: unknown } | { name: unknown; description: unknown } | { [K in keyof AssignmentSettings]: unknown } | { enabled: unknown } | { instructions: unknown } | { provider: unknown; modelId: unknown; thinkingLevel: unknown } | { schedule: unknown; delaySeconds: unknown; completion: unknown }) {
    await this.host.owned(userId, agentId);
    return this.serial(agentId, async () => {
      const current = await this.definition(agentId, id);
      if (current.archived) throw new AgentError("Archived workers cannot be edited.", 409);
      if (patch && ("enabled" in patch || "delegatable" in patch || "schedule" in patch) && current.kind === "orchestrator") throw new AgentError("The orchestrator cannot be paused or have assignment or schedule settings.", 409);
      const input = subagentInput(patch ? { ...current, ...patch } : data, current.kind === "orchestrator");
      await this.validateModel(userId, input, input.thinkingLevel);
      const binding = this.binding(agentId);
      if (current.kind === "orchestrator") {
        const live = await binding.harness.snapshot(LiveDoc, binding.main.id, context), inbox = await binding.harness.snapshot(InboxDoc, binding.main.id, context);
        if (live?.run || inbox?.items.length) throw new AgentError("Stop or finish queued work before editing the orchestrator.", 409);
      } else if (await this.db.select().from(hvSubagentRuns).where(and(eq(hvSubagentRuns.subagentId, id), inArray(hvSubagentRuns.status, active))).get()) throw new AgentError("Stop or finish the active assignment before editing this worker.", 409);
      const now = Date.now();
      const contextSettings = {
        delegatable: input.delegatable, persistent: input.persistent, timeoutSeconds: input.timeoutSeconds,
        conversationId: input.persistent === current.persistent ? current.conversationId : null,
      };
      const scheduleSettings = { schedule: input.schedule, delaySeconds: input.delaySeconds, completion: input.completion, nextRunAt: input.enabled ? nextCandleRun(now, input.schedule, input.delaySeconds) : null };
      const changes = !patch ? { ...input, ...contextSettings, ...scheduleSettings }
        : "tools" in patch ? { tools: input.tools }
        : "name" in patch ? { name: input.name, description: input.description }
        : "instructions" in patch ? { instructions: input.instructions }
        : "provider" in patch ? { provider: input.provider, modelId: input.modelId, thinkingLevel: input.thinkingLevel }
        : "schedule" in patch ? scheduleSettings
        : "enabled" in patch ? { enabled: input.enabled, nextRunAt: input.enabled === current.enabled ? current.nextRunAt : input.enabled ? nextCandleRun(now, input.schedule, input.delaySeconds) : null }
        : contextSettings;
      const row = (await this.db.update(hvSubagents).set({ ...changes, updatedAt: now }).where(eq(hvSubagents.id, id)).returning().all())[0]!;
      if (current.kind === "orchestrator") {
        await this.applyMain(binding, row);
        if (this.ready) binding.harness.resume();
      }
      this.host.changed(agentId); return row;
    });
  }
  async archive(userId: string, agentId: string, id: string) {
    await this.host.owned(userId, agentId);
    return this.serial(agentId, async () => {
      const row = await this.definition(agentId, id);
      if (row.kind === "orchestrator") throw new AgentError("The main orchestrator cannot be archived or deleted.", 409);
      if (await this.db.select().from(hvSubagentRuns).where(and(eq(hvSubagentRuns.subagentId, id), inArray(hvSubagentRuns.status, active))).get()) throw new AgentError("Stop or finish the active assignment before archiving.", 409);
      await this.db.update(hvSubagents).set({ archived: true, enabled: false, nextRunAt: null, updatedAt: Date.now() }).where(eq(hvSubagents.id, id)).run();
    });
  }
  async delegatable(agentId: string) {
    return (await this.db.select().from(hvSubagents).where(and(eq(hvSubagents.agentId, agentId), eq(hvSubagents.kind, "worker"), eq(hvSubagents.enabled, true), eq(hvSubagents.archived, false), eq(hvSubagents.delegatable, true))).all()).map(({ id, name, description, tools }) => ({ id, name, description, tools }));
  }
  async internalRun(agentId: string, id: string): Promise<SubagentRun> {
    const run = await this.db.select().from(hvSubagentRuns).where(and(eq(hvSubagentRuns.agentId, agentId), eq(hvSubagentRuns.id, id))).get();
    if (!run) throw new AgentError("Assignment not found", 404); return run;
  }
  private async canWake(agentId: string) {
    const binding = this.bindings.get(agentId); if (!binding || !this.ready || this.stopping) return false;
    const model = (await binding.main.agent(context)).model;
    return Boolean(model && (this.injectedModels || await this.providers.configured(binding.userId, model)));
  }
  async reportWhenReady(agentId: string, deliver: () => Promise<unknown>, signal: AbortSignal) {
    return this.serial(agentId, async () => {
      if (!await this.canWake(agentId)) return false;
      const binding = this.binding(agentId);
      const inbox = await binding.harness.snapshot(InboxDoc, binding.main.id, context);
      if ((inbox?.items.length ?? 0) >= 20) return false;
      signal.throwIfAborted(); await deliver(); return true;
    }, signal);
  }
  async run(userId: string, agentId: string, id: string, task: string, submissionId: string) {
    await this.host.owned(userId, agentId); requestId(submissionId);
    task = assignment(task);
    return this.admitRun(agentId, id, task, `manual:${submissionId}`, "manual");
  }
  delegate(agentId: string, id: string, task: string, submissionId: string, signal?: AbortSignal) { return this.admitRun(agentId, id, task, submissionId, "delegation", signal); }
  private admitRun(agentId: string, id: string, task: string, submissionId: string, source: SubagentRun["source"], signal?: AbortSignal) {
    return this.serial(agentId, async () => {
      if (!this.ready) throw new AgentError("Agent execution is not ready", 503);
      task = assignment(task); const binding = this.binding(agentId);
      const previous = await this.db.select().from(hvSubagentRuns).where(and(eq(hvSubagentRuns.agentId, agentId), eq(hvSubagentRuns.requestId, submissionId))).get();
      if (previous) { if (previous.subagentId !== id || previous.task !== task) throw new AgentError("Assignment request was already used.", 409); return previous; }
      const definition = await this.definition(agentId, id);
      subagentInput(definition);
      if (definition.kind !== "worker" || (source !== "manual" && !definition.enabled) || definition.archived || (source === "delegation" && !definition.delegatable)) throw new AgentError("This worker is not available for this assignment.", 409);
      await this.validateModel(binding.userId, definition, definition.thinkingLevel);
      const runs = await this.db.select().from(hvSubagentRuns).where(and(eq(hvSubagentRuns.agentId, agentId), inArray(hvSubagentRuns.status, active))).all();
      if (runs.some((run) => run.subagentId === id)) throw new AgentError("This worker already has an active assignment.", 409);
      if (runs.length >= 4) throw new AgentError("This Agent already has four active workers.", 429);
      const run = await this.database.appTransaction(async (db, tx) => {
        signal?.throwIfAborted();
        const admitted = (await db.insert(hvSubagentRuns).values({ id: randomUUID(), agentId, subagentId: id, requestId: submissionId, source, task, config: definition, createdAt: Date.now() }).returning().all())[0]!;
        await this.execution.snapshotRunTx(tx, admitted);
        signal?.throwIfAborted();
        return admitted;
      });
      this.historyChanged(id);
      await this.dispatch(binding, run); return run;
    }, signal);
  }
  private async dispatch(binding: Binding, run: SubagentRun, reportOnly = false) {
    const taskId = await binding.main.commit(async (tx) => {
      const receipt = await tx.doc(RunReceipt, binding.main.id, reportOnly ? `${run.id}:report` : run.id, null);
      if (receipt.taskId === null) receipt.taskId = await tx.createTask(binding.runner, { runId: run.id, reportOnly }, { ownership: { kind: "conversation" }, background: true });
      return receipt.taskId;
    }, context);
    await this.db.update(hvSubagentRuns).set({ taskId }).where(eq(hvSubagentRuns.id, run.id)).run();
    binding.harness.resume();
  }
  async started(run: SubagentRun, conversationId: number) {
    await this.database.appTransaction(async (db) => {
      await db.update(hvSubagentRuns).set({ conversationId, status: "running" }).where(and(eq(hvSubagentRuns.id, run.id), inArray(hvSubagentRuns.status, active))).run();
      if (run.config.persistent) await db.update(hvSubagents).set({ conversationId }).where(eq(hvSubagents.id, run.subagentId)).run();
    });
    this.historyChanged(run.subagentId);
  }
  async finish(id: string, status: "completed" | "failed" | "cancelled" | "skipped", result: string) {
    const run = await this.db.update(hvSubagentRuns).set({ status, result, finishedAt: Date.now() }).where(and(eq(hvSubagentRuns.id, id), inArray(hvSubagentRuns.status, active))).returning({ subagentId: hvSubagentRuns.subagentId }).get();
    if (run) this.historyChanged(run.subagentId);
  }
  async reported(id: string) { await this.db.update(hvSubagentRuns).set({ reportedAt: Date.now() }).where(eq(hvSubagentRuns.id, id)).run(); }
  async stop(userId: string, agentId: string, runId: string) {
    await this.host.owned(userId, agentId);
    return this.serial(agentId, async () => {
      const run = await this.internalRun(agentId, runId), binding = this.binding(agentId);
      if (!active.includes(run.status as typeof active[number])) return;
      // Recover a dispatch whose application task-ID update was interrupted.
      await this.dispatch(binding, run);
      const current = await this.internalRun(agentId, runId);
      await binding.harness.abortTask(current.taskId as RunnerId, context);
      await binding.harness.waitForTask(current.taskId as RunnerId, context);
    });
  }
  async start() {
    this.admit(); if (this.ready) return;
    // No catch-up burst after downtime; pending admitted assignments are recovered separately.
    const definitions = await this.db.select().from(hvSubagents).where(eq(hvSubagents.kind, "worker")).all();
    for (const row of definitions) await this.db.update(hvSubagents).set({ nextRunAt: row.enabled && !row.archived ? nextCandleRun(Date.now(), row.schedule, row.delaySeconds) : null }).where(eq(hvSubagents.id, row.id)).run();
    if (this.stopping) return;
    this.ready = true;
    this.log?.info("Starting sub-agent scheduler");
    this.timer = setInterval(() => { void this.tick(); }, 1000); this.timer.unref();
    await this.tick();
  }
  /** Public clock seam for deterministic scheduling tests; never overlaps ticks. */
  tick(now = Date.now()): Promise<void> {
    if (!this.ready || this.stopping) return Promise.resolve();
    if (this.tickActive) return this.ticking;
    this.tickActive = true;
    this.ticking = this.ticking.then(async () => {
      if (this.stopping) return;
      // Dispatch intents persisted before a crash; detect terminal task failures too.
      const pending = await this.db.select().from(hvSubagentRuns).where(or(inArray(hvSubagentRuns.status, active), isNull(hvSubagentRuns.reportedAt))).all();
      for (const run of pending) {
        const binding = this.bindings.get(run.agentId); if (!binding) continue;
        await this.serial(run.agentId, async () => {
          const current = await this.internalRun(run.agentId, run.id);
          if (current.taskId === null && active.includes(current.status as typeof active[number])) await this.dispatch(binding, current);
          else if (current.taskId !== null) {
            const task = await binding.harness.getTask(current.taskId as RunnerId, context);
            if (task?.state.status === "terminal" && active.includes(current.status as typeof active[number])) {
              const status = task.state.outcome.status === "aborted" ? "cancelled" : "failed";
              await this.finish(current.id, status, "Worker execution stopped unexpectedly. Inspect account/order state before retrying.");
              if (status === "failed") await this.dispatch(binding, current, true);
              else await this.reported(current.id);
            } else if (task?.state.status === "terminal" && current.reportedAt === null) {
              if (task.state.outcome.status !== "completed" && task.state.outcome.status !== "aborted") {
                await this.db.update(hvSubagentRuns).set({ result: `${current.result}\nCompletion reporting failed. Review this run manually.` }).where(eq(hvSubagentRuns.id, current.id)).run();
              }
              await this.reported(current.id);
            }
          }
        });
      }
      const due = await this.db.select().from(hvSubagents).where(and(eq(hvSubagents.enabled, true), eq(hvSubagents.archived, false), lte(hvSubagents.nextRunAt, now))).all();
      for (const row of due) {
        if (this.stopping) return;
        await this.schedule(row, now);
      }
      this.schedulerError = null;
    }).catch((error: unknown) => {
      this.schedulerError = "Subagent scheduler encountered an error. It will retry; persisted assignments are retained.";
      this.log?.error(error, "Subagent scheduler encountered an error");
    }).finally(() => { this.tickActive = false; });
    return this.ticking;
  }
  private schedule(candidate: SubagentRow, now: number) {
    return this.serial(candidate.agentId, async () => {
      const row = await this.definition(candidate.agentId, candidate.id);
      if (!row.enabled || row.archived || row.nextRunAt !== candidate.nextRunAt) return;
      const binding = this.bindings.get(row.agentId);
      let reason = now - row.nextRunAt! > 60000 ? "Missed candle interval; stale assignment skipped." : "";
      if (!binding) reason = "Agent execution is unavailable.";
      if (!reason && binding) {
        try { await this.validateModel(binding.userId, row, row.thinkingLevel); }
        catch (error) { if (!(error instanceof AgentError)) throw error; reason = error.message; }
      }
      if (!reason) subagentInput(row);
      const run = await this.database.appTransaction(async (db, tx) => {
        const running = await db.select().from(hvSubagentRuns).where(and(eq(hvSubagentRuns.agentId, row.agentId), inArray(hvSubagentRuns.status, active))).all();
        if (running.some((run) => run.subagentId === row.id)) reason = "Worker already has an active assignment.";
        else if (running.length >= 4) reason = "Agent already has four active workers.";
        // Advance the candle cursor and persist its intent atomically. Dispatch
        // is separately idempotent, so a crash cannot lose or duplicate the run.
        await db.update(hvSubagents).set({ nextRunAt: nextCandleRun(now, row.schedule, row.delaySeconds) }).where(eq(hvSubagents.id, row.id)).run();
        const admitted = (await db.insert(hvSubagentRuns).values({ id: randomUUID(), agentId: row.agentId, subagentId: row.id, requestId: `schedule:${row.id}:${row.nextRunAt}`, source: "schedule", task: `Scheduled ${row.schedule} assignment. UTC candle close: ${new Date(row.nextRunAt! - row.delaySeconds * 1000).toISOString()}.\n\nCarry out your responsibilities according to your instructions.`, config: row, status: reason ? "skipped" : "queued", result: reason, createdAt: now, finishedAt: reason ? now : null, reportedAt: reason ? now : null }).returning().all())[0]!;
        if (!reason) await this.execution.snapshotRunTx(tx, admitted);
        return admitted;
      });
      this.historyChanged(row.id);
      this.log?.info({ agentId: run.agentId, runId: run.id, subagentId: run.subagentId, status: run.status, reason: reason || undefined }, reason ? "Scheduled sub-agent run skipped" : "Scheduled sub-agent run");
      if (run.status === "queued" && binding) await this.dispatch(binding, run);
    });
  }
  async close() { this.stopping = true; this.ready = false; this.historyShutdown.abort(); if (this.timer) clearInterval(this.timer); await this.ticking; await Promise.all(this.pending.values()); await Promise.all(this.historyFeeds); }
}
