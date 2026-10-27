import type { NotificationService } from "../notifications/notification-service.js";
import type { SqliteExecutor } from "@earendil-works/pi-durable/storage/sqlite";
import { BACKGROUND_CONTEXT as context, withAbortSignal } from "@earendil-works/chord/context";
import { getSupportedThinkingLevels, type Models } from "@earendil-works/pi-ai/models";
import { defineDoc, defineDocFamily, InboxDoc, LiveDoc, type AgentChange, type Conversation, type ConversationId, type Cursor, type EntryRecord, type Extension, type Harness, type ModelRef } from "@earendil-works/pi-durable";
import { and, asc, count, desc, eq, inArray, isNull, lte, or, sql } from "drizzle-orm";
import { randomUUID } from "node:crypto";
import { agentExecutable, assertAgentExecutable } from "./agent-enablement.js";
import { cancelTaskScope } from "./cancel-task-scope.js";
import type { AgentRow } from "../db/agent-schema.js";import { hvSubagents, hvSubagentRuns, hvWorkerRequests, type SubagentRow, type SubagentRun, type WorkerRequest } from "../db/subagent-schema.js";
import type { ApplicationDatabase, SharedDatabase } from "../db/shared-database.js";
import type { FastifyBaseLogger } from "fastify";
import type { ProviderService } from "../providers/provider-service.js";
import type { DashboardSubagent } from "./subagent-dashboard.js";
import { runHistoryPageSize, runPagination, type RunHistory } from "./subagent-history.js";
import { AgentError, requestId } from "./agent-input.js";
import { createSubagentExtension, RunReceipt, type RunnerId, type SubagentExtension } from "./subagent-extension.js";
import { executionWarnings, type ExecutionWarning } from "./execution-warnings.js";
import { toolActivity, type ToolActivity } from "../research/tool-activity.js";
import type { ExecutionContextService } from "./execution-context.js";
import type { RunContext } from "./execution-context-contracts.js";
import { chartToolNames } from "../charts/contracts.js";
import { bodyHash, canonical } from "../canonical.js";
import { subagentTemplate } from "./subagent-templates.js";
import { workerTriggers, type WorkerRequestPayload, type WorkerTriggers } from "./worker-triggers.js";
import { insertDefaultSubagents } from "./default-subagents.js";
import { requestBatch, inboxAssignment } from "./worker-inbox.js";
import { inspectRun, listRuns } from "./run-inspection.js";
import { assertActiveActor, type MemoryActor } from "../memory/operations.js";
import { assignment, defaultMainInstructions, mainToolNames, nextCandleRun, normalizeDelegationTools, subagentInput, type AssignmentSettings } from "./subagent-input.js";

interface Binding extends SubagentExtension { harness: Harness; main: Conversation; hyperliquid: Extension; capabilities: readonly Extension[]; orchestratorInsights: Extension; userId: string }
export interface SubagentHost { owned(userId: string, id: string): Promise<AgentRow>; models(userId: string): Promise<Models>; changed(id: string): void; signal(id: string): AbortSignal; recoverParkedMain?(agentId: string): Promise<void> }
/** Full run detail: snapshotted configuration plus the complete transcript. */
export interface RunDetail { definition: SubagentRow; run: SubagentRun; entries: EntryRecord[]; prior: EntryRecord[]; tools?: ToolActivity[]; context?: RunContext; warnings?: ExecutionWarning[]; publications?: { ref: string; kind: string; summary: string }[] }
const active = ["queued", "running"] as const;
const parkedRunReason = "Automatic recovery stopped after repeated interruptions. Existing exchange orders were not cancelled. Inspect account/order state before starting a new assignment.";
/** A public, durable cancellation intent. Never clear it before all work drains. */
export const ParkedRunRecovery = defineDocFamily<{ pending: boolean; done: boolean; conversationId: number | null }, null>({
  kind: "hv.parked-run-recovery", version: 1, family: true, scope: "conversation", history: "latest", fork: "initial",
  initial: () => ({ pending: false, done: false, conversationId: null }), checkpointWhen: () => false,
});

/** Drizzle and Durable can wrap native SQLite errors in a cause. */
function sqliteFailure(error: unknown, seen = new Set<unknown>()): boolean {
  if (!error || typeof error !== "object" || seen.has(error)) return false;
  seen.add(error);
  const code: unknown = Reflect.get(error, "code");
  return (typeof code === "string" && code.startsWith("ERR_SQLITE_"))
    || sqliteFailure(Reflect.get(error, "cause"), seen)
    || (error instanceof AggregateError && error.errors.some((item: unknown) => sqliteFailure(item, seen)));
}

// Public Durable document: an idempotent, schema-free upgrade marker. Once
// applied, an explicit later tool deselection survives subsequent restarts.
export const MarketSnapshotUpgradeDoc = defineDoc<{ applied: boolean }>({
  kind: "hv.market-snapshot-upgrade", version: 1, scope: "conversation", history: "latest", fork: "initial",
  initial: () => ({ applied: false }), checkpointWhen: () => false,
});

export const ChartToolsUpgradeDoc = defineDoc<{ applied: boolean }>({
  kind: "hv.chart-tools-upgrade", version: 1, scope: "conversation", history: "latest", fork: "initial", initial: () => ({ applied: false }), checkpointWhen: () => false,
});

/** Definitions and run intents belong to the app; Durable owns execution and reporting. */
export class SubagentService {
  private readonly bindings = new Map<string, Binding>();
  private readonly stoppedAgents = new Set<string>();
  private readonly pending = new Map<string, Promise<void>>();
  private timer?: ReturnType<typeof setInterval>;
  private ticking: Promise<void> = Promise.resolve();
  private tickActive = false;
  private ready = false;
  private starting?: Promise<void>;
  private stopping = false;
  private draining?: Promise<void>;
  private readonly dashboardListeners = new Map<string, Set<() => void>>();
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
  private dashboardChanged(agentId: string) { for (const listener of this.dashboardListeners.get(agentId) ?? []) listener(); }
  private historyChanged(agentId: string, id: string) {
    for (const listener of this.historyListeners.get(id) ?? []) listener();
    this.dashboardChanged(agentId);
  }
  constructor(private readonly db: ApplicationDatabase, private readonly database: SharedDatabase, private readonly providers: ProviderService, private readonly host: SubagentHost, private readonly execution: ExecutionContextService, private readonly notifications: NotificationService, private readonly injectedModels = false, private readonly log?: FastifyBaseLogger) {}
  extension(agentId: string, hyperliquid: Extension, capabilities: readonly Extension[]) { return createSubagentExtension(this, agentId, hyperliquid, capabilities, this.execution); }
  async bind(agent: AgentRow, harness: Harness, main: Conversation, hyperliquid: Extension, execution: SubagentExtension, capabilities: readonly Extension[], orchestratorInsights: Extension) {
    const resolved = await main.agent(context);
    const now = Date.now();
    await this.db.insert(hvSubagents).values({
      id: `main:${agent.id}`, agentId: agent.id, kind: "orchestrator", name: "Orchestrator", description: "Main conversation", instructions: defaultMainInstructions,
      provider: resolved.model!.provider, modelId: resolved.model!.modelId, thinkingLevel: resolved.thinkingLevel ?? "", tools: mainToolNames, conversationId: main.id, createdAt: now, updatedAt: now,
    }).onConflictDoUpdate({ target: hvSubagents.id, set: { conversationId: main.id } }).run();
    const binding = { harness, main, hyperliquid, capabilities, orchestratorInsights, ...execution, userId: agent.userId };
    // Normalize saved selections at startup; historical run snapshots stay immutable.
    for (const saved of await this.db.select().from(hvSubagents).where(eq(hvSubagents.agentId, agent.id)).all()) {
      const tools = normalizeDelegationTools(saved.tools);
      if (canonical(tools) !== canonical(saved.tools)) await this.db.update(hvSubagents).set({ tools, updatedAt: now }).where(eq(hvSubagents.id, saved.id)).run();
    }
    const row = (await this.db.select().from(hvSubagents).where(and(eq(hvSubagents.agentId, agent.id), eq(hvSubagents.kind, "orchestrator"))).get())!;
    const upgraded = await main.commit(async (tx) => (await tx.doc(MarketSnapshotUpgradeDoc, main.id)).applied, context);
    if (!upgraded) {
      if (!row.tools.includes("get_market_snapshot")) {
        row.tools = [...row.tools, "get_market_snapshot"];
        await this.db.update(hvSubagents).set({ tools: row.tools, updatedAt: now }).where(eq(hvSubagents.id, row.id)).run();
      }
      // Separate commits avoid nesting the shared database queue. A crash before
      // the marker simply retries the idempotent append on the next startup.
      await main.commit(async (tx) => { (await tx.doc(MarketSnapshotUpgradeDoc, main.id)).applied = true; }, context);
    }
    if (!await main.commit(async (tx) => (await tx.doc(ChartToolsUpgradeDoc, main.id)).applied, context)) {
      row.tools = [...new Set([...row.tools, ...chartToolNames])];
      await this.db.update(hvSubagents).set({ tools: row.tools, updatedAt: now }).where(eq(hvSubagents.id, row.id)).run();
      await main.commit(async (tx) => { (await tx.doc(ChartToolsUpgradeDoc, main.id)).applied = true; }, context);
    }
    await this.applyMain(binding, row);
    await this.recoverParkedRuns(agent.id, binding, true);
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
  async stopAgent(agentId: string) {
    this.stoppedAgents.add(agentId);
    await this.pending.get(agentId);
    await this.db.update(hvSubagents).set({ enabled: false, nextRunAt: null }).where(and(eq(hvSubagents.agentId, agentId), eq(hvSubagents.kind, "worker"))).run();
  }
  /** Clear application queues without changing each worker's configured enablement. */
  async discardPending(agentId: string) {
    const now = Date.now();
    const affected = await this.database.appTransaction(async (db, tx) => {
      const discarded = await db.update(hvWorkerRequests).set({ discardedReason: "Agent disabled; request will not replay" }).where(and(eq(hvWorkerRequests.agentId, agentId), isNull(hvWorkerRequests.runId), isNull(hvWorkerRequests.discardedReason))).returning().all();
      for (const request of discarded) await this.requestWarningTx(tx, request, request.discardedReason!);
      const definitions = await db.select().from(hvSubagents).where(and(eq(hvSubagents.agentId, agentId), eq(hvSubagents.kind, "worker"))).all();
      for (const row of definitions) await db.update(hvSubagents).set({ nextRunAt: row.enabled && !row.archived ? nextCandleRun(now, row.schedule, row.delaySeconds) : null }).where(eq(hvSubagents.id, row.id)).run();
      return [...new Set(discarded.map((row) => row.subagentId))];
    });
    for (const id of affected) this.historyChanged(agentId, id);
    if (affected.length) {
      this.notifications.changed(agentId);
      this.log?.warn({ agentId }, "Queued worker requests discarded because Agent was disabled");
    }
    this.dashboardChanged(agentId);
  }
  /** Only claim cancellation after the harness has confirmed its ownership tree is idle. */
  async cancelOutstanding(agentId: string) {
    const now = Date.now();
    const cancelled = await this.db.update(hvSubagentRuns).set({ status: "cancelled", result: "Agent disabled. Existing exchange orders and positions were not changed.", finishedAt: now, reportedAt: now }).where(and(eq(hvSubagentRuns.agentId, agentId), inArray(hvSubagentRuns.status, active))).returning({ id: hvSubagentRuns.subagentId }).all();
    for (const id of new Set(cancelled.map((row) => row.id))) this.historyChanged(agentId, id);
  }
  async execute<T>(id: string, action: () => Promise<T>, signal?: AbortSignal): Promise<T> {
    return this.serial(id, async () => { await assertAgentExecutable(this.database, id); return action(); }, signal);
  }
  private admitAgent(id: string) {
    if (this.stoppedAgents.has(id) || this.host.signal(id).aborted) throw new AgentError("Agent is being deleted", 409);
  }
  private admit(readOnly = false) { if (this.stopping || (!readOnly && this.draining)) throw new AgentError("Subagent execution is shutting down", 503); }
  private binding(id: string) { this.admit(true); this.admitAgent(id); const value = this.bindings.get(id); if (!value) throw new AgentError("Agent is unavailable. Retry initialization.", 503); return value; }
  serial<T>(id: string, action: () => Promise<T>, signal?: AbortSignal): Promise<T> {
    this.admit();
    this.admitAgent(id);
    const result = (this.pending.get(id) ?? Promise.resolve()).then(() => { this.admit(); this.admitAgent(id); signal?.throwIfAborted(); return action(); });
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
  async list(userId: string, agentId: string): Promise<SubagentRow[]> {
    await this.host.owned(userId, agentId);
    return this.db.select().from(hvSubagents).where(eq(hvSubagents.agentId, agentId)).orderBy(asc(hvSubagents.kind), asc(hvSubagents.createdAt), asc(hvSubagents.id)).all();
  }
  async dashboard(userId: string, agentId: string) {
    await this.host.owned(userId, agentId);
    return this.database.appTransaction(async (db) => {
      const definitions = await db.select({ id: hvSubagents.id, name: hvSubagents.name, enabled: hvSubagents.enabled, schedule: hvSubagents.schedule, nextRunAt: hvSubagents.nextRunAt }).from(hvSubagents).where(and(eq(hvSubagents.agentId, agentId), eq(hvSubagents.kind, "worker"), eq(hvSubagents.archived, false))).orderBy(asc(hvSubagents.createdAt), asc(hvSubagents.id)).all();
      const summaries = [];
      for (const definition of definitions) {
        const { id, status, createdAt, result } = hvSubagentRuns;
        const runs = await db.select({ id, status, createdAt, result: sql<string>`substr(${result}, 1, 240)` }).from(hvSubagentRuns).where(and(eq(hvSubagentRuns.agentId, agentId), eq(hvSubagentRuns.subagentId, definition.id))).orderBy(desc(createdAt), desc(id)).limit(1).all();
        if (runs[0]?.status === "running" || runs[0]?.status === "queued") {
          const previous = await db.select({ id, status, createdAt, result: sql<string>`substr(${result}, 1, 240)` }).from(hvSubagentRuns).where(and(eq(hvSubagentRuns.agentId, agentId), eq(hvSubagentRuns.subagentId, definition.id), sql`${status} not in ('running', 'queued')`)).orderBy(desc(createdAt), desc(id)).limit(1).all();
          runs.push(...previous);
        }
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
    return { definition, runs, pagination, schedulerError: this.schedulerError, workers: (await this.db.select({ id: hvSubagents.id, name: hvSubagents.name, archived: hvSubagents.archived, tools: hvSubagents.tools }).from(hvSubagents).where(and(eq(hvSubagents.agentId, agentId), eq(hvSubagents.kind, "worker"))).all()).filter((worker) => !worker.archived || [...definition.triggers.researchSources, ...definition.triggers.workerDelegates].includes(worker.id)) };
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
  async watchDashboard(userId: string, agentId: string, signal: AbortSignal, publish: (snapshot: DashboardSubagent[]) => void): Promise<void> {
    this.admit(true);
    const lifetime = AbortSignal.any([signal, this.historyShutdown.signal, this.host.signal(agentId)]);
    let finish!: () => void;
    const finished = new Promise<void>((resolve) => { finish = resolve; });
    this.historyFeeds.add(finished);
    let dirty = true;
    let wake: (() => void) | undefined;
    const changed = () => { dirty = true; wake?.(); };
    const listeners = this.dashboardListeners.get(agentId) ?? new Set<() => void>();
    this.dashboardListeners.set(agentId, listeners); listeners.add(changed);
    lifetime.addEventListener("abort", changed);
    try {
      while (!lifetime.aborted) {
        if (dirty) {
          dirty = false;
          const snapshot = await this.dashboard(userId, agentId);
          if (!lifetime.aborted) publish(snapshot);
        }
        if (!dirty && !lifetime.aborted) await new Promise<void>((resolve) => { wake = resolve; });
        wake = undefined;
      }
    } finally {
      lifetime.removeEventListener("abort", changed);
      listeners.delete(changed);
      if (!listeners.size) this.dashboardListeners.delete(agentId);
      this.historyFeeds.delete(finished); finish();
    }
  }
  async watchHistory(userId: string, agentId: string, id: string, page: number, signal: AbortSignal, publish: (history: RunHistory) => void): Promise<void> {
    this.admit(true);
    const lifetime = AbortSignal.any([signal, this.historyShutdown.signal, this.host.signal(agentId)]);
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
    this.admit(true);
    await this.ownedRun(userId, agentId, id, runId);
    const lifetime = AbortSignal.any([signal, this.historyShutdown.signal, this.host.signal(agentId)]);
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
  async importDefaults(userId: string, agentId: string, creationId: string) {
    await this.host.owned(userId, agentId);
    creationId = requestId(creationId);
    return this.serial(agentId, async () => {
      const main = (await this.list(userId, agentId)).find((row) => row.kind === "orchestrator");
      if (!main) throw new AgentError("Initialize the Agent before importing default sub-agents", 409);
      await this.validateModel(userId, main, main.thinkingLevel);
      const rows = await this.database.appTransaction((db) => insertDefaultSubagents(db, agentId, creationId, main));
      this.dashboardChanged(agentId);
      return rows;
    });
  }
  async create(userId: string, agentId: string, data: unknown, creationId: string, templateId = "") {
    await this.host.owned(userId, agentId); requestId(creationId);
    return this.serial(agentId, async () => {
      const input = subagentInput(data); await this.validateModel(userId, input, input.thinkingLevel);
      await this.validateTriggers(agentId, `worker:${creationId}`, workerTriggers(input.triggers), input);
      const template = templateId ? subagentTemplate(templateId) : null;
      const templateOrigin = template ? { id: template.id, version: template.version, hash: bodyHash(canonical(template)) } : null;
      if (template && input.enabled) throw new AgentError("Template creation is paused by default; enable it explicitly after reviewing the saved copy.");
      const id = `worker:${creationId}`;
      const existing = await this.db.select().from(hvSubagents).where(eq(hvSubagents.id, id)).get();
      if (existing) { if (existing.agentId !== agentId || canonical(existing.templateOrigin) !== canonical(templateOrigin) || Object.entries(input).some(([key, value]) => JSON.stringify(Reflect.get(existing, key)) !== JSON.stringify(value))) throw new AgentError("Creation request was already used with different settings.", 409); return existing; }
      const now = Date.now();
      const row = (await this.db.insert(hvSubagents).values({ ...input, templateOrigin, id, agentId, kind: "worker", nextRunAt: input.enabled ? nextCandleRun(now, input.schedule, input.delaySeconds) : null, createdAt: now, updatedAt: now }).returning().all())[0]!;
      this.dashboardChanged(agentId);
      return row;
    }).catch((error: unknown) => {
      if (!(error instanceof AgentError)) this.log?.error({ err: error, agentId, templateId, requestId: creationId }, "Sub-agent creation failed");
      throw error;
    });
  }
  private async validateTriggers(agentId: string, id: string, triggers: WorkerTriggers, input: { schedule: string; tools: string[] }, main = false, previous?: WorkerTriggers) {
    if (main && triggers.inboxEnabled) throw new AgentError("Only workers can receive event inbox assignments");
    if (triggers.inboxEnabled && input.tools.includes("delegate_subagent")) throw new AgentError("Inbox recipients cannot delegate recursively");
    const added = [...triggers.workerDelegates.filter((id) => !previous?.workerDelegates.includes(id)), ...triggers.researchSources.filter((id) => !previous?.researchSources.includes(id))];
    for (const sourceId of new Set(added)) {
      const source = await this.definition(agentId, sourceId);
      if (source.id === id || source.kind !== "worker" || source.archived) throw new AgentError("Select another non-archived worker from this Agent");
    }
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
      case "triggers": return this.updateSettings(userId, agentId, id, undefined, { triggers: get("triggers") });
      case "schedule": return this.updateSettings(userId, agentId, id, undefined, { schedule: get("schedule"), delaySeconds: get("delaySeconds"), completion: get("completion") });
      default: throw new AgentError("Unknown settings section.");
    }
  }
  async update(userId: string, agentId: string, id: string, data: unknown) {
    return this.updateSettings(userId, agentId, id, data);
  }
  private async updateSettings(userId: string, agentId: string, id: string, data: unknown, patch?: { triggers: unknown } | { tools: unknown } | { name: unknown; description: unknown } | { [K in keyof AssignmentSettings]: unknown } | { enabled: unknown } | { instructions: unknown } | { provider: unknown; modelId: unknown; thinkingLevel: unknown } | { schedule: unknown; delaySeconds: unknown; completion: unknown }) {
    await this.host.owned(userId, agentId);
    return this.serial(agentId, async () => {
      const current = await this.definition(agentId, id);
      if (current.archived) throw new AgentError("Archived workers cannot be edited.", 409);
      if (patch && ("enabled" in patch || "delegatable" in patch || "schedule" in patch) && current.kind === "orchestrator") throw new AgentError("The orchestrator cannot be paused or have assignment or schedule settings.", 409);
      const input = subagentInput(patch ? { ...current, ...patch } : data, current.kind === "orchestrator");
      const pausing = patch && "enabled" in patch && !input.enabled;
      if (!pausing) await this.validateModel(userId, input, input.thinkingLevel);
      await this.validateTriggers(agentId, id, workerTriggers(input.triggers), input, current.kind === "orchestrator", current.triggers);
      const binding = this.binding(agentId);
      if (current.kind === "orchestrator") {
        const live = await binding.harness.snapshot(LiveDoc, binding.main.id, context), inbox = await binding.harness.snapshot(InboxDoc, binding.main.id, context);
        if (live?.run || inbox?.items.length) throw new AgentError("Stop or finish queued work before editing the orchestrator.", 409);
        if (input.provider !== current.provider || input.modelId !== current.modelId) {
          const selected = (await this.host.models(userId)).getModel(input.provider, input.modelId);
          if (!selected?.input.includes("image") && (await binding.main.context(context)).messages.some((message) => message.role === "user" && Array.isArray(message.content) && message.content.some((part) => part.type === "image"))) {
            throw new AgentError("This conversation contains images. Reset context before switching to a text-only model; earlier messages remain available.", 409);
          }
        }
      } else if (!pausing && await this.db.select().from(hvSubagentRuns).where(and(eq(hvSubagentRuns.subagentId, id), inArray(hvSubagentRuns.status, active))).get()) throw new AgentError("Stop or finish the active assignment before editing this worker.", 409);
      const now = Date.now();
      const contextSettings = {
        delegatable: input.delegatable, persistent: input.persistent, timeoutSeconds: input.timeoutSeconds,
        conversationId: input.persistent === current.persistent ? current.conversationId : null,
      };
      const scheduleSettings = { schedule: input.schedule, delaySeconds: input.delaySeconds, completion: input.completion, nextRunAt: input.enabled ? nextCandleRun(now, input.schedule, input.delaySeconds) : null };
      const changes = !patch ? { ...input, ...contextSettings, ...scheduleSettings }
        : "triggers" in patch ? { triggers: workerTriggers(input.triggers) }
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
        if (this.ready && await agentExecutable(this.database, agentId)) binding.harness.resume();
      }
      this.host.changed(agentId); this.dashboardChanged(agentId); return row;
    });
  }
  async archive(userId: string, agentId: string, id: string) {
    await this.host.owned(userId, agentId);
    return this.serial(agentId, async () => {
      const row = await this.definition(agentId, id);
      if (row.kind === "orchestrator") throw new AgentError("The main orchestrator cannot be archived or deleted.", 409);
      if (await this.db.select().from(hvSubagentRuns).where(and(eq(hvSubagentRuns.subagentId, id), inArray(hvSubagentRuns.status, active))).get()) throw new AgentError("Stop or finish the active assignment before archiving.", 409);
      await this.db.update(hvSubagents).set({ archived: true, enabled: false, nextRunAt: null, updatedAt: Date.now() }).where(eq(hvSubagents.id, id)).run();
      this.dashboardChanged(agentId);
    });
  }
  async unarchive(userId: string, agentId: string, id: string) {
    await this.host.owned(userId, agentId);
    return this.serial(agentId, async () => {
      const current = await this.definition(agentId, id);
      if (current.kind === "orchestrator") throw new AgentError("The main orchestrator cannot be unarchived.", 409);
      if (!current.archived) return current;
      await this.validateTriggers(agentId, id, current.triggers, current, false, current.triggers);
      const row = (await this.db.update(hvSubagents).set({ archived: false, enabled: false, nextRunAt: null, updatedAt: Date.now() }).where(eq(hvSubagents.id, id)).returning().all())[0]!;
      this.host.changed(agentId); this.dashboardChanged(agentId);
      return row;
    });
  }
  async listWorkers(agentId: string, actor?: MemoryActor): Promise<(Pick<SubagentRow, "id" | "name" | "description" | "tools" | "enabled" | "delegatable"> & { inboxEnabled?: boolean })[]> {
    if (actor) {
      if (actor.agentId !== agentId) throw new AgentError("Invalid delegation caller", 403);
      await this.database.appTransaction((_db, tx) => assertActiveActor(tx, actor));
    }
    return (await this.db.select().from(hvSubagents).where(and(eq(hvSubagents.agentId, agentId), eq(hvSubagents.kind, "worker"), eq(hvSubagents.archived, false))).orderBy(asc(hvSubagents.createdAt), asc(hvSubagents.id)).all())
      .filter((target) => !actor?.runId || (target.triggers.inboxEnabled && target.enabled && target.delegatable && target.triggers.workerDelegates.includes(actor.subagentId!)))
      .map(({ id, name, description, tools, enabled, delegatable, triggers }) => ({ id, name, description, tools, enabled, delegatable, ...(triggers.inboxEnabled ? { inboxEnabled: true } : {}) }));
  }
  async internalRun(agentId: string, id: string): Promise<SubagentRun> {
    const run = await this.db.select().from(hvSubagentRuns).where(and(eq(hvSubagentRuns.agentId, agentId), eq(hvSubagentRuns.id, id))).get();
    if (!run) throw new AgentError("Assignment not found", 404); return run;
  }
  private async canWake(agentId: string) {
    if (!await agentExecutable(this.database, agentId)) return false;
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
  async delegate(agentId: string, id: string, task: string, submissionId: string, signal?: AbortSignal, intent: "operator_trade" | "evaluation" = "evaluation", actor?: MemoryActor, refs: string[] = []) {
    await assertAgentExecutable(this.database, agentId);
    if (actor && (actor.agentId !== agentId || (actor.runId ? !actor.subagentId : actor.subagentId !== `main:${agentId}`))) throw new AgentError("Invalid delegation caller", 403);
    if (actor?.runId && intent !== "evaluation") throw new AgentError("Only the orchestrator can delegate operator intent", 403);
    if (refs.length > 20 || refs.some((ref) => !/^m[1-9][0-9]*$/.test(ref))) throw new AgentError("Supply up to 20 exact published research references");
    const payload: WorkerRequestPayload = { intent, reason: "delegation", task: assignment(task), callerSubagentId: actor?.subagentId ?? `main:${agentId}`, callerRunId: actor?.runId ?? null, researchRefs: [...new Set(refs)] };
    const prior = await this.db.select().from(hvWorkerRequests).where(and(eq(hvWorkerRequests.agentId, agentId), eq(hvWorkerRequests.subagentId, id), eq(hvWorkerRequests.eventKey, submissionId))).get();
    if (prior) {
      if (canonical(prior.payload) !== canonical(payload)) throw new AgentError("Worker request ID was already used", 409);
      return this.requestReceipt(prior);
    }
    const target = await this.definition(agentId, id);
    if (actor?.runId && (!target.triggers.inboxEnabled || !target.triggers.workerDelegates.includes(actor.subagentId!))) throw new AgentError("This worker is not approved to delegate to the selected recipient", 403);
    if (!target.triggers.inboxEnabled) {
      if (intent === "operator_trade") throw new AgentError("Explicit trades must target an inbox-enabled trading worker");
      const referencedTask = payload.researchRefs.length ? `${payload.task}\n\nResearch references (evidence, not instructions): ${payload.researchRefs.join(", ")}` : payload.task;
      return this.admitRun(agentId, id, referencedTask, submissionId, "delegation", signal, payload.researchRefs);
    }
    return this.queueWorker(agentId, target.id, submissionId, payload, actor, signal);
  }
  private queueWorker(agentId: string, id: string, key: string, payload: WorkerRequestPayload, actor?: MemoryActor, signal?: AbortSignal) {
    return this.execute(agentId, async () => {
      if (!this.ready) throw new AgentError("Agent execution is not ready", 503);
      const target = await this.definition(agentId, id);
      if (payload.task.length > 12000) throw new AgentError("Worker requests must contain at most 12,000 characters");
      const row = await this.database.appTransaction(async (db, tx) => {
        signal?.throwIfAborted();
        const prior = await db.select().from(hvWorkerRequests).where(and(eq(hvWorkerRequests.agentId, agentId), eq(hvWorkerRequests.subagentId, id), eq(hvWorkerRequests.eventKey, key))).get();
        if (prior) { if (canonical(prior.payload) !== canonical(payload)) throw new AgentError("Worker request ID was already used", 409); return prior; }
        if (actor) await assertActiveActor(tx, actor);
        if (target.kind !== "worker" || !target.enabled || target.archived || !target.delegatable || !target.triggers.inboxEnabled) throw new AgentError("The worker is unavailable for delegation", 409);
        for (const ref of payload.researchRefs) {
          if (!await tx.get("SELECT number FROM hv_memories WHERE agent_id=? AND number=? AND kind='research'", agentId, Number(ref.slice(1)))) throw new AgentError("Research reference not found in this Agent", 404);
        }
        if (actor?.runId) {
          const source = await db.select().from(hvSubagents).where(and(eq(hvSubagents.agentId, agentId), eq(hvSubagents.id, actor.subagentId!))).get();
          if (!source || !source.enabled || source.archived || source.triggers.inboxEnabled || !normalizeDelegationTools(source.tools).includes("delegate_subagent") || !target.triggers.workerDelegates.includes(source.id)) throw new AgentError("Worker delegation permission is unavailable", 403);
          // Empty references must never vacuously deduplicate a self-contained task.
          if (payload.researchRefs.length) {
            const publications = await db.select().from(hvWorkerRequests).where(and(eq(hvWorkerRequests.agentId, agentId), eq(hvWorkerRequests.subagentId, id), inArray(hvWorkerRequests.eventKey, payload.researchRefs.map((ref) => `research:${ref.slice(1)}`)))).all();
            if (publications.length === payload.researchRefs.length) return publications[0]!;
          }
        }
        const pending = await db.select({ total: count() }).from(hvWorkerRequests).where(and(eq(hvWorkerRequests.subagentId, id), isNull(hvWorkerRequests.runId), isNull(hvWorkerRequests.discardedReason))).get();
        if (pending!.total >= 500) throw new AgentError("Worker inbox is full", 429);
        return (await db.insert(hvWorkerRequests).values({ id: randomUUID(), agentId, subagentId: id, eventKey: key, payload, createdAt: Date.now() }).returning().get())!;
      });
      this.historyChanged(agentId, id);
      return this.requestReceipt(row);
    }, signal);
  }
  private requestReceipt(row: WorkerRequest) { return { id: row.id, status: row.runId ? "dispatched" : row.discardedReason ? "discarded" : "pending", runId: row.runId }; }
  listRunEvidence(agentId: string, input: unknown) { return listRuns(this.database, agentId, input); }
  async readRunEvidence(agentId: string, id: string, input: unknown) {
    const request = await this.db.select().from(hvWorkerRequests).where(and(eq(hvWorkerRequests.agentId, agentId), eq(hvWorkerRequests.id, id))).get();
    if (request && !request.runId) return { request: { id: request.id, subagentId: request.subagentId, createdAt: request.createdAt, discardedReason: request.discardedReason }, run: null };
    const run = await this.internalRun(agentId, request?.runId ?? id);
    return { ...(request ? { requestId: request.id } : {}), ...await inspectRun(this.database, this.execution, this.binding(agentId).harness, agentId, run, input) };
  }
  async assignmentStatus(agentId: string, id: string) {
    const request = await this.db.select().from(hvWorkerRequests).where(and(eq(hvWorkerRequests.agentId, agentId), eq(hvWorkerRequests.id, id))).get();
    const view = (run: SubagentRun) => ({ id: run.id, subagentId: run.subagentId, status: run.status, task: run.task, result: run.result, createdAt: run.createdAt, finishedAt: run.finishedAt });
    if (!request) return view(await this.internalRun(agentId, id));
    return { request, run: request.runId ? view(await this.internalRun(agentId, request.runId)) : null };
  }
  private async requestWarningTx(tx: SqliteExecutor, request: WorkerRequest, reason: string, paused = false) {
    const worker = await tx.get<{ name: string }>("SELECT name FROM hv_subagents WHERE agent_id=? AND id=?", request.agentId, request.subagentId);
    const name = worker?.name ?? "Worker";
    return this.notifications.systemWarningTx(tx, request.agentId, `worker-request:${request.id}:${paused ? "paused" : "discarded"}`, {
      title: `${name} assignment ${paused ? "waiting while paused" : reason.startsWith("Request expired") ? "expired before starting" : "discarded before starting"}`,
      body: `${reason}\n\nRequest: ${request.id}\nAssignment: ${request.payload.task.slice(0, 10000)}\n\nExisting exchange orders were not cancelled. Inspect the current account before starting a new assignment.`,
      severity: "warning",
    }, { source: name, subagentId: request.subagentId });
  }
  private async discardRequest(request: WorkerRequest, reason: string) {
    const discarded = await this.database.appTransaction(async (db, tx) => {
      const row = await db.update(hvWorkerRequests).set({ discardedReason: reason }).where(and(eq(hvWorkerRequests.id, request.id), isNull(hvWorkerRequests.runId), isNull(hvWorkerRequests.discardedReason))).returning().get();
      if (!row) return false;
      await this.requestWarningTx(tx, row, reason);
      return true;
    });
    if (discarded) {
      this.notifications.changed(request.agentId);
      this.log?.warn({ agentId: request.agentId, subagentId: request.subagentId, requestId: request.id, reason }, "Worker request discarded before dispatch");
      this.historyChanged(request.agentId, request.subagentId);
    }
  }
  private async drainInbox(agentId: string, id: string, now: number) {
    await assertAgentExecutable(this.database, agentId);
    const binding = this.bindings.get(agentId); if (!binding) return;
    const target = await this.definition(agentId, id);
    if (target.archived || !target.triggers.inboxEnabled) {
      const requests = await this.db.select().from(hvWorkerRequests).where(and(eq(hvWorkerRequests.subagentId, id), isNull(hvWorkerRequests.runId), isNull(hvWorkerRequests.discardedReason))).all();
      for (const request of requests) await this.discardRequest(request, "Worker inbox disabled or archived");
      return;
    }
    if (!target.enabled) {
      const stalled = await this.db.select().from(hvWorkerRequests).where(and(eq(hvWorkerRequests.subagentId, id), isNull(hvWorkerRequests.runId), isNull(hvWorkerRequests.discardedReason), lte(hvWorkerRequests.createdAt, now - target.timeoutSeconds * 1000))).all();
      for (const request of stalled) {
        const warning = await this.database.transaction((tx) => this.requestWarningTx(tx, request, "The worker is paused. This request remains queued, but it will expire before dispatch when the worker resumes.", true));
        if (warning.created) {
          this.notifications.changed(agentId);
          this.log?.warn({ agentId, subagentId: id, requestId: request.id }, "Worker request waiting while paused");
        }
      }
      return;
    }
    const requests = await this.db.select().from(hvWorkerRequests).where(and(eq(hvWorkerRequests.subagentId, id), isNull(hvWorkerRequests.runId), isNull(hvWorkerRequests.discardedReason))).orderBy(sql`CASE json_extract(${hvWorkerRequests.payload}, '$.intent') WHEN 'account_event' THEN 0 WHEN 'operator_trade' THEN 1 ELSE 2 END`, asc(hvWorkerRequests.createdAt), asc(hvWorkerRequests.id)).limit(256).all();
    const pending = [];
    for (const request of requests) {
      const payload = request.payload;
      const revoked = payload.reason === "research" ? !target.triggers.researchSources.includes(payload.callerSubagentId ?? "")
        : payload.reason === "fill" ? !target.triggers.fills : payload.reason === "order_status" ? !target.triggers.orderChanges
          : !target.delegatable || (payload.callerRunId !== null && !target.triggers.workerDelegates.includes(payload.callerSubagentId ?? ""));
      if (revoked || now >= request.createdAt + target.timeoutSeconds * 1000) {
        await this.discardRequest(request, revoked ? "Trigger/delegation permission removed" : "Request expired before dispatch; inspect account before requesting again");
      } else pending.push(request);
    }
    if (!pending.length) return;
    const running = await this.db.select().from(hvSubagentRuns).where(and(eq(hvSubagentRuns.agentId, agentId), inArray(hvSubagentRuns.status, active))).all();
    if (running.length >= 4 || running.some((run) => run.subagentId === id) || await this.recoveryPending(agentId, id, binding)) return;
    await this.validateModel(binding.userId, target, target.thinkingLevel);
    const batch = requestBatch(pending);
    const run = await this.database.appTransaction(async (db, tx) => {
      const admitted = (await db.insert(hvSubagentRuns).values({ id: randomUUID(), agentId, subagentId: id, requestId: `inbox:${batch[0]!.id}`, source: batch[0]!.payload.reason === "delegation" ? "delegation" : "event", task: assignment(inboxAssignment(batch)), config: target, createdAt: Date.now() }).returning().get())!;
      await this.execution.snapshotRunTx(tx, admitted);
      await db.update(hvWorkerRequests).set({ runId: admitted.id }).where(inArray(hvWorkerRequests.id, batch.map((request) => request.id))).run();
      return admitted;
    });
    this.historyChanged(agentId, id);
    await this.dispatch(binding, run);
  }
  private admitRun(agentId: string, id: string, task: string, submissionId: string, source: SubagentRun["source"], signal?: AbortSignal, refs: string[] = []) {
    return this.execute(agentId, async () => {
      if (!this.ready) throw new AgentError("Agent execution is not ready", 503);
      task = assignment(task); const binding = this.binding(agentId);
      const previous = await this.db.select().from(hvSubagentRuns).where(and(eq(hvSubagentRuns.agentId, agentId), eq(hvSubagentRuns.requestId, submissionId))).get();
      if (previous) { if (previous.subagentId !== id || previous.task !== task) throw new AgentError("Assignment request was already used.", 409); return previous; }
      const definition = await this.definition(agentId, id);
      subagentInput(definition);
      if (definition.kind !== "worker" || (source !== "manual" && !definition.enabled) || definition.archived || (source === "delegation" && !definition.delegatable)) throw new AgentError("This worker is not available for this assignment.", 409);
      await this.validateModel(binding.userId, definition, definition.thinkingLevel);
      const runs = await this.db.select().from(hvSubagentRuns).where(and(eq(hvSubagentRuns.agentId, agentId), inArray(hvSubagentRuns.status, active))).all();
      if (runs.some((run) => run.subagentId === id) || await this.recoveryPending(agentId, id, binding)) throw new AgentError("This worker already has an active assignment.", 409);
      if (runs.length >= 4) throw new AgentError("This Agent already has four active workers.", 429);
      const run = await this.database.appTransaction(async (db, tx) => {
        signal?.throwIfAborted();
        for (const ref of refs) {
          if (!await tx.get("SELECT number FROM hv_memories WHERE agent_id=? AND number=? AND kind='research'", agentId, Number(ref.slice(1)))) throw new AgentError("Research reference not found in this Agent", 404);
        }
        const admitted = (await db.insert(hvSubagentRuns).values({ id: randomUUID(), agentId, subagentId: id, requestId: submissionId, source, task, config: definition, createdAt: Date.now() }).returning().all())[0]!;
        await this.execution.snapshotRunTx(tx, admitted);
        signal?.throwIfAborted();
        return admitted;
      });
      this.historyChanged(agentId, id);
      await this.dispatch(binding, run); return run;
    }, signal);
  }
  private async dispatch(binding: Binding, run: SubagentRun, reportOnly = false) {
    await assertAgentExecutable(this.database, run.agentId);
    try {
      const taskId = await binding.main.commit(async (tx) => {
        const receipt = await tx.doc(RunReceipt, binding.main.id, reportOnly ? `${run.id}:report` : run.id, null);
        if (receipt.taskId === null) receipt.taskId = await tx.createTask(binding.runner, { runId: run.id, reportOnly }, { ownership: { kind: "conversation" }, background: true });
        return receipt.taskId;
      }, context);
      await this.db.update(hvSubagentRuns).set({ taskId }).where(eq(hvSubagentRuns.id, run.id)).run();
      binding.harness.resume();
    } catch (error) {
      // Inbox/schedule admission creates the run inside the item action.
      this.log?.warn({ err: error, agentId: run.agentId, subagentId: run.subagentId, runId: run.id, reportOnly }, "Sub-agent run dispatch deferred");
      throw error;
    }
  }
  async started(run: SubagentRun, conversationId: number) {
    await this.database.appTransaction(async (db) => {
      await db.update(hvSubagentRuns).set({ conversationId, status: "running" }).where(and(eq(hvSubagentRuns.id, run.id), inArray(hvSubagentRuns.status, active))).run();
      if (run.config.persistent) await db.update(hvSubagents).set({ conversationId }).where(eq(hvSubagents.id, run.subagentId)).run();
    });
    this.historyChanged(run.agentId, run.subagentId);
  }
  async finish(id: string, status: "completed" | "failed" | "cancelled" | "skipped", result: string) {
    const run = await this.db.update(hvSubagentRuns).set({ status, result, finishedAt: Date.now() }).where(and(eq(hvSubagentRuns.id, id), inArray(hvSubagentRuns.status, active))).returning({ agentId: hvSubagentRuns.agentId, subagentId: hvSubagentRuns.subagentId }).get();
    if (run) this.historyChanged(run.agentId, run.subagentId);
  }
  async reported(id: string) { await this.db.update(hvSubagentRuns).set({ reportedAt: Date.now() }).where(eq(hvSubagentRuns.id, id)).run(); }
  async stop(userId: string, agentId: string, runId: string) {
    await this.host.owned(userId, agentId);
    return this.serial(agentId, async () => {
      const request = await this.db.select().from(hvWorkerRequests).where(and(eq(hvWorkerRequests.agentId, agentId), eq(hvWorkerRequests.id, runId))).get();
      if (request && !request.runId) {
        await this.db.update(hvWorkerRequests).set({ discardedReason: "Discarded by operator. Existing orders were not canceled." }).where(eq(hvWorkerRequests.id, request.id)).run();
        this.historyChanged(agentId, request.subagentId); return;
      }
      if (request?.runId) throw new AgentError("This request has already dispatched; stop its run explicitly", 409);
      const run = await this.internalRun(agentId, runId), binding = this.binding(agentId);
      if (!active.includes(run.status as typeof active[number])) return;
      // Recover a dispatch whose application task-ID update was interrupted.
      await this.dispatch(binding, run);
      const current = await this.internalRun(agentId, runId);
      await binding.harness.abortTask(current.taskId as RunnerId, context);
      await binding.harness.waitForTask(current.taskId as RunnerId, context);
    });
  }
  start(): Promise<void> {
    this.admit();
    return this.starting ??= this.startScheduler();
  }
  private async startScheduler() {
    if (this.ready) return;
    // No catch-up burst after downtime; pending admitted assignments are recovered separately.
    const definitions = await this.db.select().from(hvSubagents).where(eq(hvSubagents.kind, "worker")).all();
    for (const row of definitions) await this.db.update(hvSubagents).set({ nextRunAt: row.enabled && !row.archived ? nextCandleRun(Date.now(), row.schedule, row.delaySeconds) : null }).where(eq(hvSubagents.id, row.id)).run();
    if (this.stopping || this.draining) return;
    this.ready = true;
    this.log?.info("Starting sub-agent scheduler");
    this.timer = setInterval(() => { void this.tick(); }, 1000); this.timer.unref();
    await this.tick();
  }
  private async recoveryPending(agentId: string, subagentId: string, binding: Binding) {
    const runs = await this.db.select().from(hvSubagentRuns).where(and(eq(hvSubagentRuns.agentId, agentId), eq(hvSubagentRuns.subagentId, subagentId), or(inArray(hvSubagentRuns.status, active), isNull(hvSubagentRuns.reportedAt)))).all();
    for (const run of runs) if (await binding.main.commit(async (tx) => (await tx.doc(ParkedRunRecovery, binding.main.id, run.id, null)).pending, context)) return true;
    return false;
  }
  /** Outside the admission lock: abort APIs can join invocations waiting for that lock. */
  private async recoverParkedRuns(agentId: string, binding: Binding, markOnly = false) {
    const attached = await binding.harness.taskGraph(context);
    const graph = attached.value;
    attached.dispose();
    const liveRunners = new Map<string, number>();
    for (const node of Object.values(graph.tasks)) {
      if (node.kind !== "hv.subagent-runner") continue;
      const task = await binding.harness.getTask(node.id as RunnerId, context);
      const input = task?.input;
      if (input && typeof input === "object" && "runId" in input && typeof input.runId === "string") liveRunners.set(input.runId, node.id);
    }
    // Live runner IDs cover a report parked after reportedAt but before its
    // terminal commit. Pending cleanup otherwise keeps reportedAt null.
    const runs = await this.db.select().from(hvSubagentRuns).where(and(eq(hvSubagentRuns.agentId, agentId), or(inArray(hvSubagentRuns.status, active), isNull(hvSubagentRuns.reportedAt), inArray(hvSubagentRuns.id, [...liveRunners.keys()])))).all();
    for (const run of runs) {
      // Dispatch may have committed its receipt before the application task ID.
      const taskId = liveRunners.get(run.id) ?? run.taskId ?? await binding.main.commit(async (tx) => (await tx.doc(RunReceipt, binding.main.id, run.id, null)).taskId, context);
      if (taskId === null) continue;
      const recovery = await binding.main.commit(async (tx) => { const doc = await tx.doc(ParkedRunRecovery, binding.main.id, run.id, null); return { ...doc }; }, context);
      if (recovery.done) { await this.reported(run.id); continue; }
      const workerConversation = recovery.pending ? recovery.conversationId : active.includes(run.status as typeof active[number]) ? run.conversationId : null;
      const ids = new Set<number>([taskId]);
      const conversations = new Set<number>();
      if (workerConversation !== null) conversations.add(workerConversation);
      let changed = true;
      while (changed) {
        changed = false;
        for (const node of Object.values(graph.tasks)) {
          if (!ids.has(node.id) && !conversations.has(node.conversationId) && (node.owner === undefined || !ids.has(node.owner))) continue;
          if (!ids.has(node.id)) { ids.add(node.id); changed = true; }
          for (const id of node.conversations) if (!conversations.has(id)) { conversations.add(id); changed = true; }
        }
      }
      if (!recovery.pending && !Object.values(graph.tasks).some((node) => ids.has(node.id) && node.blocked === "restart_limit")) continue;
      await binding.main.commit(async (tx) => { const doc = await tx.doc(ParkedRunRecovery, binding.main.id, run.id, null); doc.pending = true; doc.conversationId = workerConversation; }, context);
      // Preserve the ID and completed results. Finish first so the runner's abort
      // handler cannot replace the failure with a generic cancellation result.
      await this.db.update(hvSubagentRuns).set({ taskId, reportedAt: null }).where(eq(hvSubagentRuns.id, run.id)).run();
      await this.finish(run.id, "failed", parkedRunReason);
      // abortTask marks while paused and bypasses restart parking for cancellation.
      // Mark the whole scope before resume; never retryTask or replay an input.
      const cancellation = withAbortSignal(AbortSignal.any([this.host.signal(agentId), this.historyShutdown.signal, AbortSignal.timeout(10_000)]), context);
      for (const id of ids) if (graph.tasks[String(id)]) await binding.harness.abortTask(id as RunnerId, cancellation);
      if (markOnly) continue;
      for (const id of conversations) await (await binding.harness.conversation(id as ConversationId, cancellation))?.abort(cancellation, { background: true });
      await cancelTaskScope(binding.harness, [taskId], workerConversation === null ? [] : [workerConversation], cancellation);
      // Suppress automatic reporting, including report-only tasks. The existing
      // result and any already delivered report remain unchanged.
      await binding.main.commit(async (tx) => { const doc = await tx.doc(ParkedRunRecovery, binding.main.id, run.id, null); doc.pending = false; doc.done = true; }, context);
      await this.reported(run.id);
      this.historyChanged(agentId, run.subagentId);
    }
  }
  /** Public clock seam for deterministic scheduling tests; never overlaps ticks. */
  tick(now = Date.now()): Promise<void> {
    if (this.stopping) return Promise.resolve();
    // start() initializes schedules asynchronously. A caller that awaits tick()
    // must also await that initialization and its first pass.
    if (!this.ready) return this.starting?.then(() => this.ready ? this.tick(now) : undefined) ?? Promise.resolve();
    if (this.tickActive) return this.ticking;
    this.tickActive = true;
    this.ticking = this.ticking.then(async () => {
      if (this.stopping || this.draining) return;
      const recoveryFailed = new Set<string>();
      for (const [agentId, binding] of this.bindings) {
        if (this.host.signal(agentId).aborted || !await agentExecutable(this.database, agentId)) continue;
        try {
          await this.host.recoverParkedMain?.(agentId);
          await this.recoverParkedRuns(agentId, binding);
        } catch (error) {
          recoveryFailed.add(agentId);
          this.log?.warn({ err: error, agentId }, "Parked-task cleanup deferred; new work remains blocked for this Agent");
          if (!this.database.available || sqliteFailure(error)) throw error;
        }
      }
      // Dispatch intents persisted before a crash; detect terminal task failures too.
      const pending = await this.db.select().from(hvSubagentRuns).where(or(inArray(hvSubagentRuns.status, active), isNull(hvSubagentRuns.reportedAt))).all();
      for (const run of pending) {
        if (this.stopping || this.draining) return;
        const binding = this.bindings.get(run.agentId); if (recoveryFailed.has(run.agentId) || !binding || this.host.signal(run.agentId).aborted || !await agentExecutable(this.database, run.agentId)) continue;
        await this.schedulerItem(run.agentId, run.subagentId, run.id, () => this.serial(run.agentId, async () => {
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
        }));
      }
      const inboxes = await this.db.selectDistinct({ agentId: hvWorkerRequests.agentId, id: hvWorkerRequests.subagentId }).from(hvWorkerRequests).where(and(isNull(hvWorkerRequests.runId), isNull(hvWorkerRequests.discardedReason))).all();
      for (const inbox of inboxes) {
        if (this.stopping || this.draining) return;
        if (recoveryFailed.has(inbox.agentId) || this.host.signal(inbox.agentId).aborted || !await agentExecutable(this.database, inbox.agentId)) continue;
        await this.schedulerItem(inbox.agentId, inbox.id, undefined, () => this.serial(inbox.agentId, () => this.drainInbox(inbox.agentId, inbox.id, now)));
      }
      const due = await this.db.select().from(hvSubagents).where(and(eq(hvSubagents.enabled, true), eq(hvSubagents.archived, false), lte(hvSubagents.nextRunAt, now))).all();
      for (const row of due) {
        if (this.stopping || this.draining) return;
        if (recoveryFailed.has(row.agentId) || this.host.signal(row.agentId).aborted || !await agentExecutable(this.database, row.agentId)) continue;
        await this.schedulerItem(row.agentId, row.id, undefined, () => this.schedule(row, now));
      }
      this.schedulerError = recoveryFailed.size ? "Parked-task cleanup is pending. Affected Agents will retry cancellation, not assignments." : null;
    }).catch((error: unknown) => {
      this.schedulerError = "Subagent scheduler encountered an error. It will retry; persisted assignments are retained.";
      this.log?.error(error, "Subagent scheduler encountered an error");
    }).finally(() => { this.tickActive = false; });
    return this.ticking;
  }
  /** Item failures can retry independently; storage and lifecycle failures stop the pass. */
  private async schedulerItem(agentId: string, subagentId: string, runId: string | undefined, action: () => Promise<void>) {
    try { await action(); }
    catch (error) {
      this.log?.warn({ err: error, agentId, subagentId, runId }, "Sub-agent scheduler item deferred");
      this.admit();
      this.admitAgent(agentId);
      if (!this.database.available || sqliteFailure(error) || (error instanceof Error && error.name === "AbortError")) throw error;
      // Recheck after asynchronous work: deletion/disablement can race admission.
      if (!await agentExecutable(this.database, agentId)) throw error;
    }
  }
  private schedule(candidate: SubagentRow, now: number) {
    return this.execute(candidate.agentId, async () => {
      const row = await this.definition(candidate.agentId, candidate.id);
      if (!row.enabled || row.archived || row.nextRunAt !== candidate.nextRunAt) return;
      const binding = this.bindings.get(row.agentId);
      let reason = now - row.nextRunAt! > 60000 ? "Missed candle interval; stale assignment skipped." : "";
      if (!binding) reason = "Agent execution is unavailable.";
      if (!reason && binding) {
        try { await this.validateModel(binding.userId, row, row.thinkingLevel); }
        catch (error) { if (!(error instanceof AgentError)) throw error; reason = error.message; }
      }
      if (!reason && binding && await this.recoveryPending(row.agentId, row.id, binding)) reason = "Worker cancellation is still pending.";
      if (!reason) subagentInput(row);
      const run = await this.database.appTransaction(async (db, tx) => {
        const running = await db.select().from(hvSubagentRuns).where(and(eq(hvSubagentRuns.agentId, row.agentId), inArray(hvSubagentRuns.status, active))).all();
        if (running.some((run) => run.subagentId === row.id)) reason = "Worker already has an active assignment.";
        else if (running.length >= 4) reason = "Agent already has four active workers.";
        // Advance the candle cursor and persist its intent atomically. Dispatch
        // is separately idempotent, so a crash cannot lose or duplicate the run.
        await db.update(hvSubagents).set({ nextRunAt: nextCandleRun(now, row.schedule, row.delaySeconds) }).where(eq(hvSubagents.id, row.id)).run();
        const admitted = (await db.insert(hvSubagentRuns).values({ id: randomUUID(), agentId: row.agentId, subagentId: row.id, requestId: `schedule:${row.id}:${row.nextRunAt}`, source: "schedule", task: `Scheduled ${row.schedule} assignment. UTC candle close: ${new Date(row.nextRunAt! - row.delaySeconds * 1000).toISOString()}. endMs: ${row.nextRunAt! - row.delaySeconds * 1000} (UTC milliseconds, exclusive closed-candle end).\n\nCarry out your responsibilities according to your instructions.`, config: row, status: reason ? "skipped" : "queued", result: reason, createdAt: now, finishedAt: reason ? now : null, reportedAt: reason ? now : null }).returning().all())[0]!;
        if (!reason) await this.execution.snapshotRunTx(tx, admitted);
        return admitted;
      });
      this.historyChanged(row.agentId, row.id);
      this.log?.info({ agentId: run.agentId, runId: run.id, subagentId: run.subagentId, status: run.status, reason: reason || undefined }, reason ? "Scheduled sub-agent run skipped" : "Scheduled sub-agent run");
      if (run.status === "queued" && binding) await this.dispatch(binding, run);
    });
  }
  drain() {
    if (this.draining) return this.draining;
    this.ready = false;
    if (this.timer) clearInterval(this.timer);
    return this.draining = (async () => { await this.starting; await this.ticking; await Promise.all(this.pending.values()); })();
  }
  async close() {
    const drained = this.drain();
    this.stopping = true;
    this.historyShutdown.abort();
    try { await drained; }
    finally { await Promise.all(this.historyFeeds); }
  }
}
