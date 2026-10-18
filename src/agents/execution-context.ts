import type { Context } from "@earendil-works/chord";
import { ToolTask, type ConversationId, type EntryId, type Harness, type ToolExecutionApi } from "@earendil-works/pi-durable";
import type { SqliteExecutor } from "@earendil-works/pi-durable/storage/sqlite";
import type { SharedDatabase } from "../db/shared-database.js";
import type { SubagentRow, SubagentRun } from "../db/subagent-schema.js";
import { LearningService } from "../memory/learning-service.js";
import { MemoryService } from "../memory/memory-service.js";
import { renderLearningSections } from "../memory/learning-contracts.js";
import type { MemoryActor, MemoryAuthorization } from "../memory/operations.js";
import { AgentError } from "./agent-input.js";
import type { RunContext } from "./execution-context-contracts.js";
import { workerInstructions } from "./subagent-input.js";

/**
 * Resolves tool calls to trusted application identity and freezes per-run
 * learning snapshots. There is no user/automatic origin classification: risk
 * settings are the only enforced trading policy, and tool selection is the
 * permission system.
 */
export class ExecutionContextService {
  private readonly bindings = new Map<string, { harness: Harness; root: ConversationId }>();
  readonly learnings: LearningService;
  readonly memories: MemoryService;
  constructor(private readonly db: SharedDatabase) {
    this.learnings = new LearningService(db, this.authorize);
    this.memories = new MemoryService(db, this.authorize);
  }
  bind(agentId: string, harness: Harness, root: ConversationId) { this.bindings.set(agentId, { harness, root }); }
  unbind(agentId: string) { this.bindings.delete(agentId); }

  /** Snapshots current learnings and renders the exact run instructions. */
  async snapshotRunTx(tx: SqliteExecutor, run: SubagentRun) {
    const agent = await this.learnings.snapshotTx(tx, run.agentId, { kind: "agent" });
    const worker = await this.learnings.snapshotTx(tx, run.agentId, { kind: "worker", subagentId: run.subagentId });
    const instructions = `${workerInstructions(run.config)}\n\n${renderLearningSections(agent, worker)}`;
    if (instructions.length > 100000) throw new AgentError("Assignment instructions exceed the context limit; consolidate configuration");
    const context: RunContext = { version: 2, instructions, learnings: { agent, worker }, learningStatus: "snapshot" };
    const serialized = JSON.stringify(context);
    if (serialized.length > 120000) throw new AgentError("Assignment context exceeds its storage bound; consolidate instructions/learnings");
    await tx.run("INSERT INTO hv_subagent_run_contexts(agent_id,run_id,context) VALUES (?,?,?)", run.agentId, run.id, serialized);
    return context;
  }
  private async runContextTx(tx: SqliteExecutor, agentId: string, runId: string): Promise<RunContext | null> {
    const saved = await tx.get<{ context: string }>("SELECT context FROM hv_subagent_run_contexts WHERE agent_id=? AND run_id=?", agentId, runId);
    return saved ? JSON.parse(saved.context) as RunContext : null;
  }
  async runContext(agentId: string, run: SubagentRun): Promise<RunContext> {
    return await this.db.transaction((tx) => this.runContextTx(tx, agentId, run.id))
      ?? { version: 2, instructions: run.status === "skipped" ? "No instructions were injected: this assignment was skipped before admission." : workerInstructions(run.config), learnings: null, learningStatus: run.status === "skipped" ? "not_admitted" : "legacy/none" };
  }

  /** Task-local identity survives replay; assignment entry boundaries prevent an
   * old call in a persistent conversation being attributed to a newer run. */
  async resolve(agentId: string, api: ToolExecutionApi, context: Context): Promise<{ actor: MemoryActor; config: SubagentRow }> {
    const memo = "hv.execution-identity.v2";
    const decode = (value: string) => {
      const resolved = JSON.parse(value) as { actor: MemoryActor; config: SubagentRow };
      if (resolved.actor.agentId !== agentId || resolved.actor.conversationId !== api.conversationId || resolved.actor.toolTaskId !== api.taskId) throw new AgentError("Tool identity mismatch", 403);
      return resolved;
    };
    const saved = await api.memo<string>(memo, context);
    if (saved !== undefined) return decode(saved);
    const binding = this.bindings.get(agentId);
    if (!binding) throw new AgentError("Execution binding unavailable", 503);
    let runId: string | null = null, config: SubagentRow;
    if (api.conversationId === binding.root) {
      const root = await this.db.get<{ id: string }>("SELECT id FROM hv_subagents WHERE agent_id=? AND kind='orchestrator'", agentId);
      if (!root) throw new AgentError("Root configuration unavailable", 503);
      config = await this.definition(agentId, root.id);
    } else {
      const assistant = await this.assistantEntry(api, context);
      const runs = await this.db.all<{ id: string; config: string }>("SELECT id,config FROM hv_subagent_runs WHERE agent_id=? AND conversation_id=? ORDER BY created_at DESC,id DESC LIMIT 100", agentId, api.conversationId);
      let matched: { id: string; config: string; entry: number } | undefined;
      const conversation = await binding.harness.conversation(api.conversationId, context);
      await conversation?.commit(async (tx) => {
        for (const run of runs) {
          const submission = await tx.submissionByRequest(api.conversationId, `assignment:${run.id}`);
          if (submission?.entry !== undefined && submission.entry <= assistant && (!matched || submission.entry > matched.entry)) matched = { ...run, entry: submission.entry };
        }
      }, context);
      if (!matched) throw new AgentError("Tool execution has no resolvable application identity", 403);
      config = JSON.parse(matched.config) as SubagentRow;
      runId = matched.id;
    }
    const actor: MemoryActor = { agentId, userId: null, subagentId: config.id, runId, conversationId: api.conversationId, toolTaskId: api.taskId };
    return decode(await api.memo(memo, JSON.stringify({ actor, config }), context));
  }
  /** Direct and Code Mode nested calls both lead to one model tool entry. */
  private async assistantEntry(api: ToolExecutionApi, context: Context): Promise<EntryId> {
    let id = api.taskId;
    const seen = new Set<number>();
    for (let depth = 0; depth < 32 && !seen.has(id); depth++) {
      seen.add(id);
      const task = await api.getTask(id, context);
      if (!task || task.conversationId !== api.conversationId) break;
      if (task.kind === ToolTask.definition.name) {
        const input = task.input as { kind?: string; assistant?: EntryId };
        if (input.kind === "model" && input.assistant !== undefined) return input.assistant;
      }
      if (task.owner === undefined) break;
      id = task.owner;
    }
    throw new AgentError("Cannot resolve tool assignment ancestry", 403);
  }
  private async definition(agentId: string, id: string): Promise<SubagentRow> {
    const row = await this.db.get<Record<string, unknown>>("SELECT * FROM hv_subagents WHERE agent_id=? AND id=?", agentId, id);
    if (!row) throw new AgentError("Subagent not found", 404);
    const result: Record<string, unknown> = {};
    for (const [key, value] of Object.entries(row)) result[key.replace(/_([a-z])/g, (_, letter: string) => letter.toUpperCase())] = value;
    for (const key of ["enabled", "archived", "delegatable", "persistent"]) result[key] = Boolean(result[key]);
    for (const key of ["tools", "templateOrigin"]) if (typeof result[key] === "string") result[key] = JSON.parse(result[key]);
    return result as unknown as SubagentRow;
  }
  /** Operator actor for authenticated UI memory/learning edits. */
  operator(agentId: string, userId: string): MemoryActor {
    return { agentId, userId, subagentId: null, runId: null, conversationId: null, toolTaskId: null };
  }

  readonly authorize: MemoryAuthorization = async (tx, actor, capability, targetWorkerId) => {
    if (actor.userId && actor.subagentId === null && actor.runId === null && actor.toolTaskId === null) return;
    const tools = await this.actorTools(tx, actor);
    if (!tools.includes(capability)) throw new AgentError("Tool permission not selected for this assignment", 403);
    if (targetWorkerId) {
      const target = await tx.get<{ kind: string }>("SELECT kind FROM hv_subagents WHERE agent_id=? AND id=?", actor.agentId, targetWorkerId);
      if (!target || target.kind !== "worker") throw new AgentError("Learning target is not a worker in this Agent", 403);
    }
  };
  /** Revalidates saved identity and returns the effective tool selection. */
  private async actorTools(tx: SqliteExecutor, actor: MemoryActor): Promise<string[]> {
    if (!actor.subagentId) throw new AgentError("Unverified actor", 403);
    if (actor.runId) {
      const run = await tx.get<{ status: string; conversation_id: number | null; subagent_id: string; config: string }>("SELECT status,conversation_id,subagent_id,config FROM hv_subagent_runs WHERE agent_id=? AND id=?", actor.agentId, actor.runId);
      if (!run || run.subagent_id !== actor.subagentId || !["queued", "running"].includes(run.status) || (run.conversation_id !== null && run.conversation_id !== actor.conversationId)) throw new AgentError("Run is no longer active or does not match execution", 409);
      return (JSON.parse(run.config) as SubagentRow).tools;
    }
    const orchestrator = await tx.get<{ tools: string }>("SELECT tools FROM hv_subagents WHERE agent_id=? AND id=? AND kind='orchestrator' AND archived=0", actor.agentId, actor.subagentId);
    if (!orchestrator) throw new AgentError("Invalid root producer", 403);
    return JSON.parse(orchestrator.tools) as string[];
  }
}