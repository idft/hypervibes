import { createHash } from "node:crypto";
import type { SqliteExecutor } from "@earendil-works/pi-durable/storage/sqlite";
import { AgentError } from "../agents/agent-input.js";
import { canonical } from "../canonical.js";
import { text } from "./validation.js";

/** Server-only identity. Never deserialize this structure from model or HTTP input. */
export interface MemoryActor {
  agentId: string; userId: string | null; subagentId: string | null; runId: string | null;
  conversationId: number | null; toolTaskId: number | null;
}
export type MemoryCapability = "publish_research" | "record_trade_decision" | "publish_review" | "append_insight" | "replace_insights" | "send_notification";
/** Resolves saved identity/permissions; models cannot supply this authority. */
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
/** Call after receipt lookup: successful effects remain replayable after their run finished. */
export async function assertActiveActor(tx: SqliteExecutor, actor: MemoryActor) {
  const owner = await tx.get<{ user_id: string; state: string }>("SELECT user_id,state FROM hv_agents WHERE id=?", actor.agentId);
  if (!owner) throw new AgentError("Agent not found", 404);
  if (owner.state === "stopping" || owner.state === "stopped") throw new AgentError("Agent is being deleted", 409);
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