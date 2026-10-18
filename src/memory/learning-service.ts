import { randomUUID } from "node:crypto";
import type { SqliteExecutor } from "@earendil-works/pi-durable/storage/sqlite";
import { AgentError } from "../agents/agent-input.js";
import type { SharedDatabase } from "../db/shared-database.js";
import { learningMutation, learningScope, type LearningScope, type LearningSnapshot } from "./learning-contracts.js";
import { memoryRef, parseMemoryRef, timestamp } from "./memory-contracts.js";
import { assertActiveActor, operationReplay, requestHash, saveOperation, type MemoryActor, type MemoryAuthorization } from "./operations.js";
import { fields, integer } from "./validation.js";

interface SetRow { id: string; current_version: number }
export interface LearningRevision extends LearningSnapshot {
  operation: "append" | "replace"; reason: string; createdAt: number; evidenceRefs: string[];
  userId: string | null; producerSubagentId: string | null; sourceRunId: string | null;
}
interface RevisionRow {
  version: number; content: string; operation: "append" | "replace"; reason: string; created_at: number;
  actor_user_id: string | null; producer_subagent_id: string | null; source_run_id: string | null;
}
export class LearningConflict extends AgentError {
  constructor(readonly currentVersion: number) { super(`Learnings changed to v${currentVersion}; reread and reconcile your draft`, 409); }
}
export class LearningService {
  constructor(private readonly db: SharedDatabase, private readonly authorize: MemoryAuthorization, private readonly clock: () => number = Date.now) {}
  private async checkScope(tx: SqliteExecutor, agentId: string, scope: LearningScope) {
    const owner = await tx.get("SELECT id FROM hv_agents WHERE id=?", agentId);
    if (!owner) throw new AgentError("Agent not found", 404);
    if (scope.kind === "worker" && !await tx.get("SELECT id FROM hv_subagents WHERE agent_id=? AND id=? AND kind='worker'", agentId, scope.subagentId)) throw new AgentError("Worker not found in this Agent", 404);
  }
  private set(tx: SqliteExecutor, agentId: string, scope: LearningScope) {
    return tx.get<SetRow>("SELECT id,current_version FROM hv_learning_sets WHERE agent_id=? AND scope_kind=? AND subagent_id IS ?", agentId, scope.kind, scope.kind === "worker" ? scope.subagentId : null);
  }
  /** Can be used inside the owning admission transaction, without re-entering the queue. */
  async snapshotTx(tx: SqliteExecutor, agentId: string, scope: LearningScope): Promise<LearningSnapshot> {
    await this.checkScope(tx, agentId, scope);
    const set = await this.set(tx, agentId, scope);
    if (!set || set.current_version === 0) return { scope, version: 0, content: "" };
    const row = await tx.get<{ content: string }>("SELECT content FROM hv_learning_revisions WHERE agent_id=? AND set_id=? AND version=?", agentId, set.id, set.current_version);
    if (!row) throw new Error("Learning head has no revision");
    return { scope, version: set.current_version, content: row.content };
  }
  current(agentId: string, value: unknown) {
    const scope = learningScope(value);
    return this.db.transaction((tx) => this.snapshotTx(tx, agentId, scope));
  }
  append(actor: MemoryActor, key: string, input: unknown) { return this.mutate(actor, key, "append", input); }
  replace(actor: MemoryActor, key: string, input: unknown) { return this.mutate(actor, key, "replace", input); }
  private async mutate(actor: MemoryActor, key: string, operation: "append" | "replace", value: unknown) {
    const input = learningMutation(value, operation), kind = operation === "append" ? "append_learning" : "replace_learnings";
    const hash = requestHash(actor, kind, input);
    return this.db.transaction(async (tx) => {
      const replay = await operationReplay<{ scope: LearningScope; version: number }>(tx, actor, key, kind, hash);
      if (replay) return replay;
      const now = timestamp(this.clock());
      await assertActiveActor(tx, actor);
      await this.authorize(tx, actor, kind, input.scope.kind === "worker" ? input.scope.subagentId : null);
      await this.checkScope(tx, actor.agentId, input.scope);
      let set = await this.set(tx, actor.agentId, input.scope);
      if (!set) {
        set = { id: randomUUID(), current_version: 0 };
        await tx.run("INSERT INTO hv_learning_sets(id,agent_id,scope_kind,subagent_id,current_version,updated_at) VALUES (?,?,?,?,0,?)", set.id, actor.agentId, input.scope.kind, input.scope.kind === "worker" ? input.scope.subagentId : null, now);
      }
      if (operation === "replace" && set.current_version !== input.expectedVersion) throw new LearningConflict(set.current_version);
      let content = input.content;
      const refs = new Set(input.evidenceRefs.map(parseMemoryRef));
      if (operation === "append" && set.current_version > 0) {
        const previous = await tx.get<{ content: string }>("SELECT content FROM hv_learning_revisions WHERE agent_id=? AND set_id=? AND version=?", actor.agentId, set.id, set.current_version);
        if (!previous) throw new Error("Learning head has no revision");
        content = previous.content ? `${previous.content}\n\n---\n\n${content}` : content;
        const inherited = await tx.all<{ memory_number: number }>("SELECT memory_number FROM hv_learning_revision_memories WHERE agent_id=? AND set_id=? AND version=?", actor.agentId, set.id, set.current_version);
        for (const ref of inherited) refs.add(ref.memory_number);
      }
      if (content.length > 8000) throw new AgentError("Learning document exceeds 8,000 characters; reread and consolidate using replace", 409);
      for (const ref of refs) if (!await tx.get("SELECT number FROM hv_memories WHERE agent_id=? AND number=?", actor.agentId, ref)) throw new AgentError("Evidence memory not found in this Agent", 404);
      const version = integer(set.current_version + 1, 1, Number.MAX_SAFE_INTEGER - 1, "learning version");
      await tx.run("INSERT INTO hv_learning_revisions(agent_id,set_id,version,content,operation,reason,created_at,actor_user_id,producer_subagent_id,source_run_id,conversation_id,tool_task_id) VALUES (?,?,?,?,?,?,?,?,?,?,?,?)", actor.agentId, set.id, version, content, operation, input.reason, now, actor.userId, actor.subagentId, actor.runId, actor.conversationId, actor.toolTaskId);
      for (const ref of refs) await tx.run("INSERT INTO hv_learning_revision_memories(agent_id,set_id,version,memory_number) VALUES (?,?,?,?)", actor.agentId, set.id, version, ref);
      await tx.run("UPDATE hv_learning_sets SET current_version=?,updated_at=? WHERE agent_id=? AND id=?", version, now, actor.agentId, set.id);
      return saveOperation(tx, actor, key, kind, hash, { scope: input.scope, version }, now);
    });
  }
  async revision(agentId: string, value: unknown, version: number): Promise<LearningRevision> {
    const scope = learningScope(value);
    integer(version, 1, Number.MAX_SAFE_INTEGER - 1, "revision version");
    return this.db.transaction(async (tx) => {
      await this.checkScope(tx, agentId, scope);
      const set = await this.set(tx, agentId, scope);
      const row = set && await tx.get<RevisionRow>("SELECT * FROM hv_learning_revisions WHERE agent_id=? AND set_id=? AND version=?", agentId, set.id, version);
      if (!set || !row) throw new AgentError("Learning revision not found", 404);
      const refs = await tx.all<{ memory_number: number }>("SELECT memory_number FROM hv_learning_revision_memories WHERE agent_id=? AND set_id=? AND version=? ORDER BY memory_number", agentId, set.id, version);
      return { scope, version, content: row.content, operation: row.operation, reason: row.reason, createdAt: row.created_at, userId: row.actor_user_id, producerSubagentId: row.producer_subagent_id, sourceRunId: row.source_run_id, evidenceRefs: refs.map((ref) => memoryRef(ref.memory_number)) };
    });
  }
  async history(agentId: string, value: unknown, options: unknown = {}) {
    const scope = learningScope(value), input = fields(options, ["beforeVersion", "limit"]);
    const before = input.beforeVersion === undefined ? Number.MAX_SAFE_INTEGER : integer(input.beforeVersion, 1, Number.MAX_SAFE_INTEGER, "history cursor");
    const limit = input.limit === undefined ? 25 : integer(input.limit, 1, 25, "history page size");
    return this.db.transaction(async (tx) => {
      await this.checkScope(tx, agentId, scope);
      const set = await this.set(tx, agentId, scope);
      const rows = set ? await tx.all<Omit<RevisionRow, "content">>("SELECT version,operation,reason,created_at,actor_user_id,producer_subagent_id,source_run_id FROM hv_learning_revisions WHERE agent_id=? AND set_id=? AND version<? ORDER BY version DESC LIMIT ?", agentId, set.id, before, limit + 1) : [];
      const items = rows.slice(0, limit).map((row) => ({ scope, version: row.version, operation: row.operation, reason: row.reason, createdAt: row.created_at, userId: row.actor_user_id, producerSubagentId: row.producer_subagent_id, sourceRunId: row.source_run_id }));
      return { items, nextBeforeVersion: rows.length > limit ? items.at(-1)!.version : null };
    });
  }
}
