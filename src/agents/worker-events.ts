import { randomUUID } from "node:crypto";
import type { SqliteExecutor } from "@earendil-works/pi-durable/storage/sqlite";
import type { WorkerRequestPayload } from "./worker-triggers.js";

/** Called inside the source transaction: no nested shared-queue access or Pi private-table access. */
export async function enqueueWorkerEvent(tx: SqliteExecutor, agentId: string, eventKey: string, payload: WorkerRequestPayload, occurredAt = Date.now()) {
  // Activation belongs to the subscription, not the worker's last unrelated edit.
  const targets = await tx.all<{ id: string }>(`SELECT s.id FROM hv_subagents s
    JOIN hv_worker_subscriptions subscription ON subscription.subagent_id=s.id
    WHERE s.agent_id=? AND s.kind='worker' AND s.enabled=1 AND s.archived=0
      AND EXISTS (SELECT 1 FROM hv_agents a WHERE a.id=s.agent_id AND a.enabled=1 AND a.state='ready')
      AND subscription.event_type=? AND subscription.source_id=? AND subscription.activated_at<=?`,
  agentId, payload.reason, payload.reason === "research" ? payload.callerSubagentId ?? "" : "", occurredAt);
  for (const target of targets) {
    await tx.run("INSERT INTO hv_worker_requests(id,agent_id,subagent_id,event_key,payload,created_at) VALUES (?,?,?,?,?,?) ON CONFLICT(agent_id,subagent_id,event_key) DO NOTHING", randomUUID(), agentId, target.id, eventKey, JSON.stringify(payload), Date.now());
  }
}

export async function enqueueAccountEvent(tx: SqliteExecutor, account: string, key: string, reason: "fill" | "order_status", task: string, occurredAt: number) {
  const agents = await tx.all<{ id: string }>("SELECT id FROM hv_agents WHERE trading_account_address=? AND state NOT IN ('stopping','stopped')", account.toLowerCase());
  for (const agent of agents) await enqueueWorkerEvent(tx, agent.id, key, { intent: "account_event", reason, task, callerSubagentId: null, callerRunId: null, researchRefs: [] }, occurredAt);
}
