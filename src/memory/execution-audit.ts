import { Type } from "@earendil-works/pi-ai";
import type { SharedDatabase } from "../db/shared-database.js";
import { AgentError } from "../agents/agent-input.js";
import { attributeFills } from "./decision-audit.js";
import { fields, integer } from "./validation.js";
import { timestamp } from "./memory-contracts.js";
import { DecisionPayloadSchema, Ref } from "./tool-schemas.js";
import { OrderViewSchema } from "../hyperliquid/orders/order-contracts.js";

const time = Type.Integer({ minimum: 0, maximum: Number.MAX_SAFE_INTEGER });
const object = { additionalProperties: false } as const;
export const ExecutionAuditInputSchema = Type.Object({
  start: time, end: time,
  section: Type.Union([Type.Literal("fills"), Type.Literal("requests"), Type.Literal("decisions")]),
  limit: Type.Optional(Type.Integer({ minimum: 1, maximum: 50 })),
  before: Type.Optional(Type.Object({ time, id: Type.String({ minLength: 1, maxLength: 128 }) }, object)),
}, object);
const nullableText = Type.Union([Type.String(), Type.Null()]);
const origin = Type.Union([Type.Object({ producerSubagentId: Type.String(), sourceRunId: nullableText, conversationId: time, toolTaskId: time }), Type.Null()]);
const record = Type.Record(Type.String(), Type.Unknown());
const auditFill = Type.Object({ fillId: Type.String(), time, exchangeOid: nullableText, requestId: nullableText, decisionRef: Type.Union([Ref, Type.Null()]), origin, symbol: nullableText, fill: Type.Union([record, Type.Null()]), fee: nullableText, pnl: nullableText });
const auditRequest = Type.Object({ requestId: Type.String(), kind: Type.String(), createdAt: time, input: record, origin,
  actions: Type.Array(Type.Object({ ordinal: time, state: Type.String(), updatedAt: time }), { maxItems: 100 }), orders: Type.Array(OrderViewSchema, { maxItems: 100 }), detailsTruncated: Type.Boolean() });
const auditDecision = Type.Object({ ref: Ref, instrument: nullableText, summary: Type.String(), producerSubagentId: nullableText, sourceRunId: nullableText, createdAt: time, payload: DecisionPayloadSchema, linkedRequests: Type.Array(Type.Object({ requestId: Type.String() }), { maxItems: 50 }), linksTruncated: Type.Boolean() });
export const ExecutionAuditResultSchema = Type.Object({ start: time, end: time, section: Type.String(), next: Type.Union([Type.Object({ time, id: Type.String() }), Type.Null()]), coverage: Type.String(),
  sync: Type.Array(Type.Object({ stream: Type.String(), cursor: time, updatedAt: Type.Union([time, Type.Null()]), error: nullableText })),
  projection: Type.Union([Type.Object({ revision: time, projectedRevision: time, historyLimited: Type.Integer({ minimum: 0, maximum: 1 }) }), Type.Null()]),
  items: Type.Array(Type.Union([auditFill, auditRequest, auditDecision]), { maxItems: 50 }) });
const coverage = "Read-only local evidence, not an assessment. Keep section/start/end unchanged while paging. Requests use creation time; fills use exchange time; decisions use publication time. Links may cross the window. Request outcomes are current local projections, not frozen as-of-window state. decision_ref is optional model-authored rationale, not verified approval. No linked request does not prove no action. External/unmatched fills and missing provenance remain unknown. Shared account positions are not independent worker portfolios. Failures before request persistence require run inspection.";
interface PageRow { id: string; time: number; data: string }

/** Deterministic joins only: the reviewing model interprets the evidence. */
export async function executionAudit(db: SharedDatabase, agentId: string, value: unknown) {
  const input = fields(value, ["start", "end", "section", "limit", "before"]);
  const start = timestamp(input.start), end = timestamp(input.end);
  if (end <= start || end - start > 90 * 86400_000) throw new AgentError("Use a positive audit window of at most 90 days");
  const section = input.section;
  if (section !== "fills" && section !== "requests" && section !== "decisions") throw new AgentError("Select fills, requests, or decisions");
  const limit = input.limit === undefined ? 10 : integer(input.limit, 1, 50, "audit page size");
  const before = input.before === undefined ? null : fields(input.before, ["time", "id"]);
  const beforeTime = before ? timestamp(before.time) : end;
  if (before && (typeof before.id !== "string" || !before.id || before.id.length > 128)) throw new AgentError("Invalid audit cursor");
  const beforeId = before ? String(before.id) : "";
  return db.transaction(async (tx) => {
    const agent = await tx.get<{ account: string | null }>("SELECT trading_account_address AS account FROM hv_agents WHERE id=?", agentId);
    if (!agent) throw new AgentError("Agent not found", 404);
    const account = agent.account?.toLowerCase();
    if (!account) throw new AgentError("Agent has no trading account", 409);
    const sources = {
      fills: { table: "hv_journal_entries", key: "id", time: "time", data: "data", where: "account=? AND kind='fill' AND active=1", owner: account },
      requests: { table: "hv_order_requests", key: "id", time: "created_at", data: "input", where: "agent_id=? AND account=?", owner: agentId },
      decisions: { table: "hv_memories", key: "CAST(number AS TEXT)", time: "created_at", data: "payload", where: "agent_id=? AND kind='decision'", owner: agentId },
    };
    const source = sources[section];
    // Identifiers are host constants, never interpolated model input.
    const rows = await tx.all<PageRow>(`SELECT ${source.key} AS id,${source.time} AS time,${source.data} AS data FROM ${source.table} WHERE ${source.where} AND ${source.time}>=? AND ${source.time}<? AND (${source.time}<? OR (${source.time}=? AND ${source.key}<?)) ORDER BY ${source.time} DESC,${source.key} DESC LIMIT ?`, source.owner, ...(section === "requests" ? [account] : []), start, end, beforeTime, beforeTime, beforeId, limit + 1);
    const page = rows.slice(0, limit), tail = page.at(-1);
    const items: object[] = [];
    for (const row of page) {
      if (section === "fills") {
        const event = JSON.parse(row.data) as { symbol?: string; fee?: string | null; pnl?: string | null; fill?: unknown; raw?: { oid?: number; cloid?: string } };
        const [attribution] = await attributeFills(tx, agentId, account, [{ id: row.id, time: row.time, oid: event.raw?.oid === undefined ? null : String(event.raw.oid), cloid: event.raw?.cloid ?? null }]);
        items.push({ ...attribution, symbol: event.symbol ?? null, fill: event.fill ?? null, fee: event.fee ?? null, pnl: event.pnl ?? null });
      } else if (section === "requests") {
        const request = await tx.get<{ requestId: string; kind: string }>("SELECT request_key AS requestId,kind FROM hv_order_requests WHERE agent_id=? AND id=?", agentId, row.id);
        const origin = await tx.get("SELECT producer_subagent_id AS producerSubagentId,source_run_id AS sourceRunId,conversation_id AS conversationId,tool_task_id AS toolTaskId FROM hv_order_request_provenance WHERE agent_id=? AND request_id=?", agentId, row.id);
        const actions = await tx.all("SELECT ordinal,state,updated_at AS updatedAt FROM hv_order_actions WHERE request_id=? ORDER BY ordinal LIMIT 101", row.id);
        const orders = await tx.all<{ view: string }>("SELECT i.view FROM hv_order_items i JOIN hv_order_actions a ON a.id=i.action_id WHERE a.request_id=? ORDER BY a.ordinal,i.ordinal LIMIT 101", row.id);
        items.push({ ...request, createdAt: row.time, input: JSON.parse(row.data) as object, origin: origin ?? null, actions: actions.slice(0, 100), orders: orders.slice(0, 100).map((item) => JSON.parse(item.view) as object), detailsTruncated: actions.length > 100 || orders.length > 100 });
      } else {
        const decision = await tx.get("SELECT instrument,summary,producer_subagent_id AS producerSubagentId,source_run_id AS sourceRunId FROM hv_memories WHERE agent_id=? AND number=?", agentId, Number(row.id));
        const linked = await tx.all<{ requestId: string }>("SELECT request_key AS requestId FROM hv_order_requests WHERE agent_id=? AND account=? AND json_extract(input,'$.decision_ref')=? ORDER BY created_at DESC,id DESC LIMIT 51", agentId, account, `m${row.id}`);
        items.push({ ...decision, ref: `m${row.id}`, createdAt: row.time, payload: JSON.parse(row.data) as object, linkedRequests: linked.slice(0, 50), linksTruncated: linked.length > 50 });
      }
    }
    const sync = await tx.all("SELECT stream,cursor,updated_at AS updatedAt,error FROM hv_journal_sync WHERE account=? ORDER BY stream", account);
    const projection = await tx.get("SELECT revision,projected_revision AS projectedRevision,history_limited AS historyLimited FROM hv_journal_accounts WHERE account=?", account);
    return { start, end, section, next: rows.length > limit && tail ? { time: tail.time, id: tail.id } : null, coverage, sync, projection: projection ?? null, items };
  });
}
