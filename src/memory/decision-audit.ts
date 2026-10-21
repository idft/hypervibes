import type { SqliteExecutor } from "@earendil-works/pi-durable/storage/sqlite";
import type { SharedDatabase } from "../db/shared-database.js";
import { AgentError } from "../agents/agent-input.js";
import { integer } from "./validation.js";
import { timestamp, type FillAttribution } from "./memory-contracts.js";
interface FillRow { id: string; time: number; oid: string | null; cloid: string | null }
const fillColumns = "f.id,f.time,CAST(json_extract(f.data,'$.raw.oid') AS TEXT) AS oid,json_extract(f.data,'$.raw.cloid') AS cloid";
async function account(tx: SqliteExecutor, agentId: string) {
  const agent = await tx.get<{ account: string | null }>("SELECT trading_account_address AS account FROM hv_agents WHERE id=?", agentId);
  if (!agent?.account) throw new AgentError("Agent has no assigned trading account", 409);
  return agent.account.toLowerCase();
}
export async function attributeFills(tx: SqliteExecutor, agentId: string, owner: string, rows: FillRow[]): Promise<FillAttribution[]> {
  const results = [];
  for (const row of rows) {
    const cloid = row.cloid && /^0x[0-9a-fA-F]{32}$/.test(row.cloid) ? row.cloid : null;
    const oid = row.oid && /^[1-9][0-9]{0,15}$/.test(row.oid) && Number.isSafeInteger(Number(row.oid)) ? row.oid : null;
    const local = (cloid || oid) ? await tx.get<{ id: string; request_key: string; decision_ref: string | null }>("SELECT r.id,r.request_key, CASE WHEN m.number IS NOT NULL THEN 'm'||m.number ELSE NULL END AS decision_ref FROM hv_order_items i JOIN hv_order_actions a ON a.id=i.action_id JOIN hv_order_requests r ON r.id=a.request_id LEFT JOIN hv_memories m ON m.agent_id=r.agent_id AND 'm'||m.number=json_extract(r.input,'$.decision_ref') AND m.kind='decision' WHERE r.agent_id=? AND r.account=? AND r.kind='submit' AND ((? IS NOT NULL AND i.cloid=?) OR (? IS NOT NULL AND i.oid=?)) ORDER BY CASE WHEN i.cloid=? THEN 0 ELSE 1 END,r.created_at DESC,r.id DESC LIMIT 1", agentId, owner, cloid, cloid, oid, oid, cloid) : null;
    const origin = local ? await tx.get<{ producerSubagentId: string; sourceRunId: string | null; conversationId: number; toolTaskId: number }>("SELECT producer_subagent_id AS producerSubagentId,source_run_id AS sourceRunId,conversation_id AS conversationId,tool_task_id AS toolTaskId FROM hv_order_request_provenance WHERE agent_id=? AND request_id=?", agentId, local.id) : null;
    results.push({ fillId: row.id, time: row.time, exchangeOid: oid, requestId: local?.request_key ?? null, decisionRef: local?.decision_ref ?? null, origin: origin ?? null });
  }
  return results;
}
const coverage = "Local order matches only, Agent/account-scoped. Decision refs are optional model-authored audit links, not verified approval. External/unmatched fills remain unattributed, not evidence of absent rationale.";
export async function windowFillAudit(db: SharedDatabase, agentId: string, start: number, end: number, limit = 25, before?: { time: number; id: string }) {
  timestamp(start); timestamp(end); integer(limit, 1, 50, "fill audit page size");
  if (end <= start || end - start > 90 * 86400_000 || (before && (!Number.isSafeInteger(before.time) || before.id.length > 128))) throw new AgentError("Invalid bounded fill audit window/cursor");
  return db.transaction(async (tx) => {
    const owner = await account(tx, agentId);
    const rows = await tx.all<FillRow>(`SELECT ${fillColumns} FROM hv_journal_entries f WHERE f.account=? AND f.kind='fill' AND f.active=1 AND f.time>=? AND f.time<? AND (f.time<? OR (f.time=? AND f.id<?)) ORDER BY f.time DESC,f.id DESC LIMIT ?`, owner, start, end, before?.time ?? end, before?.time ?? end, before?.id ?? "", limit + 1);
    const tail = rows[limit - 1];
    return { items: await attributeFills(tx, agentId, owner, rows.slice(0, limit)), next: rows.length > limit && tail ? { time: tail.time, id: tail.id } : null, coverage };
  });
}
export async function journalDecisionAudit(db: SharedDatabase, agentId: string, targetId: string) {
  if (!targetId || targetId.length > 128) throw new AgentError("Invalid journal target");
  return db.transaction(async (tx) => {
    const owner = await account(tx, agentId);
    const target = await tx.get<{ kind: string }>("SELECT kind FROM hv_journal_entries WHERE account=? AND id=?", owner, targetId);
    if (!target) throw new AgentError("Journal target not found in this Agent", 404);
    const rows = target.kind === "fill" ? await tx.all<FillRow>(`SELECT ${fillColumns} FROM hv_journal_entries f WHERE f.account=? AND f.id=?`, owner, targetId) : target.kind === "trade" ? await tx.all<FillRow>(`SELECT DISTINCT ${fillColumns} FROM hv_journal_entries t JOIN json_each(t.data,'$.parts') p JOIN hv_journal_entries f ON f.account=t.account AND f.id=json_extract(p.value,'$.eventId') AND f.kind='fill' WHERE t.account=? AND t.id=? ORDER BY f.time,f.id LIMIT 51`, owner, targetId) : [];
    return { items: await attributeFills(tx, agentId, owner, rows.slice(0, 50)), truncated: rows.length > 50, coverage };
  });
}