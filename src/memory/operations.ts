import { createHash } from "node:crypto";
import type { SqliteExecutor } from "@earendil-works/pi-durable/storage/sqlite";
import { AgentError } from "../agents/agent-input.js";
import { canonical } from "../canonical.js";
import { integer, text } from "./validation.js";

/** Server-only identity. Never deserialize this structure from model or HTTP input. */
export interface MemoryActor {
  agentId: string; userId: string | null; subagentId: string | null; runId: string | null;
  generationTaskId: number | null; inputScopeKey: string | null; conversationId: number | null; toolTaskId: number | null;
  origin: "automatic" | "user_directed"; expiresAt: number;
}
export type MemoryCapability = "publish_research" | "record_trade_decision" | "publish_review" | "append_learning" | "replace_learnings";
/** Must resolve persisted execution frames/policies; models cannot supply this authority. */
export type MemoryAuthorization = (tx: SqliteExecutor, actor: MemoryActor, capability: MemoryCapability, targetWorkerId?: string | null) => Promise<void>;
export function requestHash(actor: MemoryActor, operation: string, input: unknown): string {
  return createHash("sha256").update(canonical({ actor, operation, input })).digest("hex");
}
export async function operationReplay<T>(tx: SqliteExecutor, actor: MemoryActor, key: string, kind: string, hash: string): Promise<T | undefined> {
  text(key, 200, "operation key");
  const row = await tx.get<{ request_hash: string; operation_kind: string; result: string }>("SELECT request_hash,operation_kind,result FROM hv_memory_operations WHERE agent_id=? AND operation_key=?", actor.agentId, key);
  if (!row) return undefined;
  if (row.request_hash !== hash || row.operation_kind !== kind) throw new AgentError("Operation key reused with different arguments or actor", 409);
  return JSON.parse(row.result) as T;
}
export async function saveOperation<T>(tx: SqliteExecutor, actor: MemoryActor, key: string, kind: string, hash: string, result: T, now: number): Promise<T> {
  await tx.run("INSERT INTO hv_memory_operations(agent_id,operation_key,operation_kind,request_hash,result,created_at) VALUES (?,?,?,?,?,?)", actor.agentId, key, kind, hash, JSON.stringify(result), now);
  return result;
}
/** Call after receipt lookup: old successful effects remain replayable after expiry. */
export async function assertActiveActor(tx: SqliteExecutor, actor: MemoryActor, now: number) {
  integer(actor.expiresAt, 0, Number.MAX_SAFE_INTEGER, "actor deadline");
  if (now >= actor.expiresAt) throw new AgentError("Execution authorization expired; request a new assignment", 409);
  const owner = await tx.get<{ user_id: string }>("SELECT user_id FROM hv_agents WHERE id=?", actor.agentId);
  if (!owner) throw new AgentError("Agent not found", 404);
  if (actor.userId && actor.userId !== owner.user_id) throw new AgentError("Agent not found", 404);
  if (actor.runId) {
    if (!actor.subagentId) throw new AgentError("Missing run producer", 403);
    const run = await tx.get<{ status: string; conversation_id: number | null }>("SELECT status,conversation_id FROM hv_subagent_runs WHERE agent_id=? AND id=? AND subagent_id=?", actor.agentId, actor.runId, actor.subagentId);
    if (!run || !["queued", "running"].includes(run.status) || (run.conversation_id !== null && run.conversation_id !== actor.conversationId)) throw new AgentError("Run is no longer active or does not match execution", 409);
  } else if (actor.subagentId) {
    const root = await tx.get("SELECT id FROM hv_subagents WHERE agent_id=? AND id=? AND kind='orchestrator' AND archived=0", actor.agentId, actor.subagentId);
    if (!root) throw new AgentError("Invalid root producer", 403);
  } else if (!actor.userId) throw new AgentError("Unverified actor", 403);
}
