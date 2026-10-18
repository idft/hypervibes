import { createHash } from "node:crypto";
import type { Context } from "@earendil-works/chord";
import { defineExtension, GenerationTask, hook, LiveDoc, type ConversationId, type Harness, type HookApi, type ToolExecutionApi } from "@earendil-works/pi-durable";
import type { SqliteExecutor } from "@earendil-works/pi-durable/storage/sqlite";
import type { SharedDatabase } from "../db/shared-database.js";
import type { SubagentRow, SubagentRun } from "../db/subagent-schema.js";
import { canonical } from "../canonical.js";
import { LearningService } from "../memory/learning-service.js";
import { MemoryService } from "../memory/memory-service.js";
import { renderLearningSections } from "../memory/learning-contracts.js";
import type { MemoryActor, MemoryAuthorization } from "../memory/operations.js";
import { AgentError } from "./agent-input.js";
import { automaticOrigin, type ExecutionOrigin, type RunContext } from "./execution-context-contracts.js";
import { workerInstructions } from "./subagent-input.js";
import { candleIntervals } from "./candle-intervals.js";

export const userAuthorizationMs = 15 * 60_000;
export function bodyHash(body: string) { return createHash("sha256").update(body).digest("hex"); }
interface FrameRow { generation_task_id: number; conversation_id: number; input_scope_key: string; inputs: string; run_id: string | null; origin: string; config: string; expires_at: number }
export interface ExecutionFrame { agentId: string; generationTaskId: number; conversationId: number; inputScopeKey: string; runId: string | null; origin: ExecutionOrigin; config: SubagentRow; expiresAt: number }
function frame(agentId: string, row: FrameRow): ExecutionFrame { return { agentId, generationTaskId: row.generation_task_id, conversationId: row.conversation_id, inputScopeKey: row.input_scope_key, runId: row.run_id, origin: JSON.parse(row.origin) as ExecutionOrigin, config: JSON.parse(row.config) as SubagentRow, expiresAt: row.expires_at }; }

/** The only package-version-dependent adapter for Durable 1.3 task ancestry/inputs. */
export class ExecutionContextService {
  private readonly harnesses = new Map<string, { harness: Harness; root: ConversationId }>();
  readonly learnings: LearningService;
  readonly memories: MemoryService;
  constructor(private readonly db: SharedDatabase, private readonly clock: () => number = Date.now) {
    this.learnings = new LearningService(db, this.authorize, clock);
    this.memories = new MemoryService(db, this.authorize, async (tx, actor, input) => {
      const resolved = await this.actorFrame(tx, actor);
      const policy = resolved.config.workflow;
      if (policy?.purpose !== "analysis" || !policy.instruments.includes(input.instrument!) || policy.topic !== input.topic || policy.interval !== input.interval) throw new AgentError("Research publication must match the saved analysis workflow", 403);
      const context = actor.runId ? await this.runContextTx(tx, actor.agentId, actor.runId) : null;
      const width = candleIntervals[policy.interval as keyof typeof candleIntervals] * 1000;
      return { maxAgeSeconds: policy.maxResearchAgeSeconds, ...(context?.scheduledBoundary != null ? { evidenceBoundary: Math.floor(context.scheduledBoundary / width) * width } : {}) };
    }, clock);
  }
  bind(agentId: string, harness: Harness, root: ConversationId) { this.harnesses.set(agentId, { harness, root }); }
  unbind(agentId: string) { this.harnesses.delete(agentId); }
  extension(agentId: string) {
    return defineExtension({ name: "execution-context", hooks: [hook(GenerationTask, { beforeRequest: async (_request, api, context) => { await this.capture(agentId, api, context); return undefined; } })] });
  }
  async admitInput(agentId: string, userId: string, requestId: string, body: string) {
    const hash = bodyHash(body);
    return this.db.transaction(async (tx) => {
      const existing = await tx.get<{ body_hash: string; user_id: string; expires_at: number }>("SELECT body_hash,user_id,expires_at FROM hv_agent_input_origins WHERE agent_id=? AND request_id=?", agentId, requestId);
      if (existing) {
        if (existing.body_hash !== hash || existing.user_id !== userId) throw new AgentError("Input request ID reused with a different body or user", 409);
        return existing.expires_at;
      }
      const owner = await tx.get<{ user_id: string }>("SELECT user_id FROM hv_agents WHERE id=?", agentId);
      if (owner?.user_id !== userId) throw new AgentError("Agent not found", 404);
      const now = this.clock(), expires = now + userAuthorizationMs;
      await tx.run("INSERT INTO hv_agent_input_origins(agent_id,request_id,body_hash,user_id,admitted_at,expires_at) VALUES (?,?,?,?,?,?)", agentId, requestId, hash, userId, now, expires);
      return expires;
    });
  }
  async snapshotRunTx(tx: SqliteExecutor, run: SubagentRun, origin: ExecutionOrigin, scheduledBoundary: number | null = null) {
    const agent = await this.learnings.snapshotTx(tx, run.agentId, { kind: "agent" });
    const worker = await this.learnings.snapshotTx(tx, run.agentId, { kind: "worker", subagentId: run.subagentId });
    const instructions = `${workerInstructions(run.config)}\n\nHost execution origin: ${origin.mode}. ${origin.mode === "user_directed" ? "Follow only the actual user's assignment. This finite authorization is not an approval of invented trades." : "Automatic evaluation: new exposure requires enabled policy and current usable research."}\nWorkflow policy (host snapshot; learnings cannot change it): ${JSON.stringify(run.config.workflow ?? null)}\n\n${renderLearningSections(agent, worker)}`;
    if (instructions.length > 100000) throw new AgentError("Assignment instructions exceed the context limit; consolidate configuration");
    const context: RunContext = { version: 1, origin, instructions, scheduledBoundary, learnings: { agent, worker }, learningStatus: "snapshot" };
    const serialized = JSON.stringify(context);
    if (serialized.length > 120000) throw new AgentError("Assignment context exceeds its storage bound; consolidate instructions/workflow/learnings");
    await tx.run("INSERT INTO hv_subagent_run_contexts(agent_id,run_id,context) VALUES (?,?,?)", run.agentId, run.id, serialized);
    return context;
  }
  private async runContextTx(tx: SqliteExecutor, agentId: string, runId: string): Promise<RunContext | null> {
    const saved = await tx.get<{ context: string }>("SELECT context FROM hv_subagent_run_contexts WHERE agent_id=? AND run_id=?", agentId, runId);
    return saved ? JSON.parse(saved.context) as RunContext : null;
  }
  async runContext(agentId: string, run: SubagentRun): Promise<RunContext> {
    return await this.db.transaction((tx) => this.runContextTx(tx, agentId, run.id)) ?? { version: 1, origin: run.source === "schedule" ? { mode: "automatic", reason: "schedule" } : automaticOrigin, instructions: run.status === "skipped" ? "No instructions were injected: this assignment was skipped before admission." : workerInstructions(run.config), scheduledBoundary: null, learnings: null, learningStatus: run.status === "skipped" ? "not_admitted" : "legacy/none" };
  }
  private async capture(agentId: string, api: HookApi, context: Context) {
    const binding = this.harnesses.get(agentId);
    if (!binding) throw new AgentError("Execution binding unavailable", 503);
    const live = await api.snapshot(LiveDoc, api.conversationId, context);
    if (!live?.run || live.run.taskId !== api.taskId || !live.run.inputs.length || live.run.inputs.length > 20) throw new AgentError("Unverified generation input batch", 403);
    const inputs = [...live.run.inputs].sort((a, b) => a - b);
    const identity = canonical(inputs);
    const existing = await this.db.get<FrameRow>("SELECT * FROM hv_agent_execution_frames WHERE agent_id=? AND generation_task_id=?", agentId, api.taskId);
    if (existing) {
      const saved = JSON.parse(existing.inputs) as number[] | { version: 1; submissionIds: number[] };
      const savedIdentity = canonical(Array.isArray(saved) ? saved : saved.submissionIds);
      if (savedIdentity !== identity || existing.conversation_id !== api.conversationId) throw new AgentError("Generation input identity changed", 409);
      return;
    }
    const submissions = await Promise.all(inputs.map(async (id) => {
      const submission = await binding.harness.submission(id, context);
      const record = await submission?.status(context);
      if (record?.type !== "input" || record.status !== "placed" || record.conversationId !== api.conversationId) return null;
      const conversation = await binding.harness.conversation(api.conversationId, context);
      const entry = await conversation?.commit((tx) => tx.entry(record.entry), context);
      const user = entry?.model?.find((message) => message.role === "user");
      const content = user?.content;
      const body = typeof content === "string" ? content : Array.isArray(content) && content.every((part) => part.type === "text") ? content.map((part) => part.type === "text" ? part.text : "").join("") : null;
      return { requestId: record.requestId, body };
    }));
    const inputAudit = JSON.stringify({ version: 1, submissionIds: inputs, requests: submissions.map((submission, index) => ({ submissionId: inputs[index], requestId: submission?.requestId ?? null, bodyHash: submission?.body == null ? null : bodyHash(submission.body) })) });
    // All Durable reads finish before entering the application queue.
    await this.db.transaction(async (tx) => {
      const now = this.clock();
      let origin: ExecutionOrigin;
      let run: Pick<SubagentRun, "id" | "createdAt" | "config"> | null = null;
      let config: SubagentRow;
      if (api.conversationId !== binding.root) {
        const assignment = submissions.length === 1 ? submissions[0] : null;
        if (!assignment?.requestId?.startsWith("assignment:")) throw new AgentError("Worker input has no unambiguous assignment", 403);
        const row = await tx.get<{ config: string; id: string; task: string; conversation_id: number; created_at: number }>("SELECT id,config,task,conversation_id,created_at FROM hv_subagent_runs WHERE agent_id=? AND id=?", agentId, assignment.requestId.slice(11));
        if (!row || row.conversation_id !== api.conversationId || assignment.body !== row.task) throw new AgentError("Worker assignment identity mismatch", 403);
        config = JSON.parse(row.config) as SubagentRow;
        run = { id: row.id, createdAt: row.created_at, config };
        origin = (await this.runContextTx(tx, agentId, run.id))?.origin ?? automaticOrigin;
      } else {
        const root = await tx.get<{ id: string }>("SELECT id FROM hv_subagents WHERE agent_id=? AND kind='orchestrator'", agentId);
        if (!root) throw new AgentError("Root configuration unavailable", 503);
        const definition = await tx.get<Record<string, unknown>>("SELECT * FROM hv_subagents WHERE id=?", root.id);
        // Drizzle decoding is unnecessary here: copy through an explicit mapper.
        config = definitionFromSql(definition!);
        const receipts = [];
        for (const submission of submissions) {
          const receipt = submission?.requestId && submission.body !== null ? await tx.get<{ user_id: string; body_hash: string; expires_at: number }>("SELECT user_id,body_hash,expires_at FROM hv_agent_input_origins WHERE agent_id=? AND request_id=?", agentId, submission.requestId) : null;
          receipts.push(receipt && receipt.body_hash === bodyHash(submission!.body!) && receipt.expires_at > now ? receipt : null);
        }
        if (receipts.length && receipts.every((receipt) => receipt !== null) && new Set(receipts.map((receipt) => receipt!.user_id)).size === 1) origin = { mode: "user_directed", userId: receipts[0]!.user_id, userInputRequestIds: submissions.map((submission) => submission!.requestId!), expiresAt: Math.min(...receipts.map((receipt) => receipt!.expires_at)) };
        else origin = { mode: "automatic", reason: submissions.some((submission) => submission?.requestId?.startsWith("report:")) ? "worker_report" : receipts.some(Boolean) ? "mixed_inputs" : "unverified" };
      }
      const deadline = Math.min(run ? run.createdAt + config.timeoutSeconds * 1000 : now + userAuthorizationMs, origin.mode === "user_directed" ? origin.expiresAt : Number.MAX_SAFE_INTEGER);
      await tx.run("INSERT INTO hv_agent_execution_frames(agent_id,generation_task_id,conversation_id,input_scope_key,inputs,run_id,origin,config,created_at,expires_at) VALUES (?,?,?,?,?,?,?,?,?,?) ON CONFLICT DO NOTHING", agentId, api.taskId, api.conversationId, bodyHash(`${api.conversationId}:${identity}`), inputAudit, run?.id ?? null, JSON.stringify(origin), JSON.stringify(config), now, deadline);
    });
  }
  async resolve(agentId: string, api: ToolExecutionApi, context: Context): Promise<{ frame: ExecutionFrame; actor: MemoryActor }> {
    let id = api.taskId;
    const seen = new Set<number>();
    for (let n = 0; n < 32 && !seen.has(id); n++) {
      seen.add(id);
      const task = await api.getTask(id, context);
      if (!task || task.conversationId !== api.conversationId) break;
      const saved = await this.db.get<FrameRow>("SELECT * FROM hv_agent_execution_frames WHERE agent_id=? AND generation_task_id=?", agentId, id);
      if (saved && task.kind === GenerationTask.definition.name && saved.conversation_id === api.conversationId) {
        const resolved = frame(agentId, saved);
        return { frame: resolved, actor: { agentId, userId: null, subagentId: resolved.config.id, runId: resolved.runId, generationTaskId: resolved.generationTaskId, inputScopeKey: resolved.runId ? null : resolved.inputScopeKey, conversationId: resolved.conversationId, toolTaskId: api.taskId, origin: resolved.origin.mode, expiresAt: resolved.expiresAt } };
      }
      if (task.owner === undefined) break;
      id = task.owner;
    }
    throw new AgentError("Tool execution has no verified generation frame; no manual exemption is available", 403);
  }
  async frame(agentId: string, generationTaskId: number) {
    const row = await this.db.get<FrameRow>("SELECT * FROM hv_agent_execution_frames WHERE agent_id=? AND generation_task_id=?", agentId, generationTaskId);
    if (!row) throw new AgentError("Execution frame not found", 403);
    return frame(agentId, row);
  }
  private async actorFrame(tx: SqliteExecutor, actor: MemoryActor) {
    const row = actor.generationTaskId == null ? null : await tx.get<FrameRow>("SELECT * FROM hv_agent_execution_frames WHERE agent_id=? AND generation_task_id=?", actor.agentId, actor.generationTaskId);
    if (!row) throw new AgentError("Verified execution frame required", 403);
    const resolved = frame(actor.agentId, row);
    if (resolved.config.id !== actor.subagentId || resolved.runId !== actor.runId || resolved.conversationId !== actor.conversationId || resolved.expiresAt !== actor.expiresAt || resolved.origin.mode !== actor.origin || (actor.runId === null && resolved.inputScopeKey !== actor.inputScopeKey)) throw new AgentError("Actor does not match immutable execution frame", 403);
    return resolved;
  }
  readonly authorize: MemoryAuthorization = async (tx, actor, capability, target) => {
    if (actor.userId && actor.subagentId === null && actor.runId === null && actor.generationTaskId === null && actor.toolTaskId === null) return;
    const resolved = await this.actorFrame(tx, actor);
    if (!resolved.config.tools.includes(capability)) throw new AgentError("Tool permission not selected for this assignment", 403);
    if (capability === "append_learning" || capability === "replace_learnings") {
      const workflow = resolved.config.workflow;
      if (!resolved.runId || workflow?.purpose !== "review" || (target != null && !workflow.learningTargetSubagentIds.includes(target))) throw new AgentError("Reviewer learning target is not authorized", 403);
    }
  };
  operator(agentId: string, userId: string): MemoryActor { return { agentId, userId, subagentId: null, runId: null, generationTaskId: null, inputScopeKey: null, conversationId: null, toolTaskId: null, origin: "user_directed", expiresAt: Number.MAX_SAFE_INTEGER }; }
}
function definitionFromSql(row: Record<string, unknown>): SubagentRow {
  const result: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(row)) result[key.replace(/_([a-z])/g, (_, letter: string) => letter.toUpperCase())] = value;
  for (const key of ["enabled", "archived", "delegatable", "persistent"]) result[key] = Boolean(result[key]);
  for (const key of ["tools", "workflow", "templateOrigin"]) if (typeof result[key] === "string") result[key] = JSON.parse(result[key]);
  return result as unknown as SubagentRow;
}
