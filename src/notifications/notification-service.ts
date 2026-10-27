import { randomUUID } from "node:crypto";
import type { SqliteExecutor } from "@earendil-works/pi-durable/storage/sqlite";
import type { FastifyBaseLogger } from "fastify";
import type { SharedDatabase } from "../db/shared-database.js";
import { AgentError } from "../agents/agent-input.js";
import type { ExecutionContextService } from "../agents/execution-context.js";
import { assertActiveActor, requestHash, type MemoryActor } from "../memory/operations.js";
import type { JournalEvent } from "../hyperliquid/journal-model.js";
import type { OrderView } from "../hyperliquid/orders/order-contracts.js";
import { chunkText } from "../gateway/handlers.js";
import type { NotificationCategory, NotificationInput, NotificationPage, NotificationRecord, NotificationResult, NotificationSettings } from "./contracts.js";
import { NotificationDeliveryError, type NotificationSender } from "./delivery.js";

const columns = `id,agent_id AS agentId,category,title,body,severity,source,subagent_id AS subagentId,run_id AS runId,
  conversation_id AS conversationId,order_request_id AS orderRequestId,created_at AS createdAt,delivery,attempts,sent_at AS sentAt,error`;
interface SettingsRow { enabled: number; submissions: number; fills: number; enabled_since: number; fills_since: number }
export interface NotificationProvenance { source: string; subagentId?: string | null; runId?: string | null; conversationId?: number | null; orderRequestId?: string | null }
interface DeliveryRow extends NotificationRecord { chatId: number; botTokenHash: string; messageIds: string }

function input(value: NotificationInput): Required<NotificationInput> {
  if (typeof value.title !== "string" || !value.title.trim() || value.title.length > 200) throw new AgentError("Notification title must contain 1–200 characters.");
  if (typeof value.body !== "string" || !value.body.trim() || value.body.length > 12000) throw new AgentError("Notification body must contain 1–12000 characters.");
  const severity = value.severity ?? "info";
  if (!["info", "warning", "error"].includes(severity)) throw new AgentError("Invalid notification severity.");
  return { title: value.title.trim(), body: value.body.trim(), severity };
}

/** Application-owned inbox and durable Telegram outbox. Never performs network I/O in a SQL transaction. */
export class NotificationService {
  private sender?: NotificationSender;
  private stopping = false;
  private started = false;
  private timer?: ReturnType<typeof setTimeout>;
  private running?: Promise<void>;
  private closing?: Promise<void>;
  private readonly shutdown = new AbortController();
  private readonly listeners = new Map<string, Set<() => void>>();
  private readonly chatNext = new Map<string, number>();
  constructor(private readonly db: SharedDatabase, private readonly origin: string, private readonly logger?: FastifyBaseLogger, private readonly clock = Date.now) {}
  installSender(sender: NotificationSender) { this.sender = sender; }
  private async settingsTx(tx: SqliteExecutor, agentId: string): Promise<SettingsRow> {
    const agent = await tx.get<{ created_at: number; state: string }>("SELECT created_at,state FROM hv_agents WHERE id=?", agentId);
    if (!agent) throw new AgentError("Agent not found", 404);
    await tx.run("INSERT INTO hv_notification_settings(agent_id,enabled_since,fills_since) VALUES (?,?,?) ON CONFLICT DO NOTHING", agentId, agent.created_at, agent.created_at);
    return (await tx.get<SettingsRow>("SELECT * FROM hv_notification_settings WHERE agent_id=?", agentId))!;
  }
  async getSettings(agentId: string): Promise<NotificationSettings> {
    const row = await this.db.transaction((tx) => this.settingsTx(tx, agentId));
    return { enabled: Boolean(row.enabled), submissions: Boolean(row.submissions), fills: Boolean(row.fills) };
  }
  async saveSettings(agentId: string, value: unknown): Promise<NotificationSettings> {
    if (!value || typeof value !== "object") throw new AgentError("Invalid notification settings.");
    const settings = value as NotificationSettings;
    if ([settings.enabled, settings.submissions, settings.fills].some((v) => typeof v !== "boolean")) throw new AgentError("Invalid notification settings.");
    await this.db.transaction(async (tx) => {
      const previous = await this.settingsTx(tx, agentId), now = this.clock();
      await tx.run(`UPDATE hv_notification_settings SET enabled=?,submissions=?,fills=?,enabled_since=?,fills_since=? WHERE agent_id=?`,
        Number(settings.enabled), Number(settings.submissions), Number(settings.fills),
        settings.enabled && !previous.enabled ? now : previous.enabled_since, settings.fills && !previous.fills ? now : previous.fills_since, agentId);
      await tx.run(`UPDATE hv_notifications SET delivery='suppressed',error='Notifications disabled before delivery' WHERE agent_id=? AND delivery='pending'
        AND (?=0 OR (category='submission' AND ?=0) OR (category='fill' AND ?=0))`, agentId, Number(settings.enabled), Number(settings.submissions), Number(settings.fills));
    });
    this.changed(agentId);
    return { enabled: settings.enabled, submissions: settings.submissions, fills: settings.fills };
  }
  private async queueTx(tx: SqliteExecutor, agentId: string, eventKey: string, category: NotificationCategory, value: NotificationInput, provenance: NotificationProvenance): Promise<NotificationResult> {
    const existing = await tx.get<{ id: string }>("SELECT id FROM hv_notifications WHERE agent_id=? AND event_key=?", agentId, eventKey);
    if (existing) return { status: "queued", id: existing.id };
    const skipped = await tx.get("SELECT 1 FROM hv_notification_operations WHERE agent_id=? AND operation_key=?", agentId, `skipped:${eventKey}`);
    if (skipped) return { status: "disabled" };
    const settings = await this.settingsTx(tx, agentId);
    if (!settings.enabled || (category === "submission" && !settings.submissions) || (category === "fill" && !settings.fills)) {
      await tx.run("INSERT INTO hv_notification_operations(agent_id,operation_key,request_hash,result) VALUES (?,?,'',?) ON CONFLICT DO NOTHING", agentId, `skipped:${eventKey}`, JSON.stringify({ status: "disabled" }));
      return { status: "disabled" };
    }
    const agent = await tx.get<{ state: string }>("SELECT state FROM hv_agents WHERE id=?", agentId);
    if (!agent || ["stopping", "stopped"].includes(agent.state)) return { status: "disabled" };
    const id = await this.persistTx(tx, agentId, eventKey, category, value, provenance, true);
    return { status: "queued", id };
  }
  private async persistTx(tx: SqliteExecutor, agentId: string, eventKey: string, category: NotificationCategory, value: NotificationInput, provenance: NotificationProvenance, deliver: boolean): Promise<string> {
    const message = input(value), now = this.clock(), id = randomUUID();
    const gateway = deliver ? await tx.get<{ chat_id: number | null; bot_token_hash: string }>("SELECT chat_id,bot_token_hash FROM hv_gateways WHERE agent_id=? AND type='telegram'", agentId) : undefined;
    const connected = gateway?.chat_id !== null && gateway?.chat_id !== undefined;
    await tx.run(`INSERT INTO hv_notifications(id,agent_id,event_key,category,title,body,severity,source,subagent_id,run_id,conversation_id,order_request_id,created_at,delivery,chat_id,bot_token_hash,next_attempt_at)
      VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)`, id, agentId, eventKey, category, message.title, message.body, message.severity, provenance.source,
      provenance.subagentId ?? null, provenance.runId ?? null, provenance.conversationId ?? null, provenance.orderRequestId ?? null, now,
      connected ? "pending" : "web_only", gateway?.chat_id ?? null, connected ? gateway!.bot_token_hash : null, now);
    return id;
  }
  /** Always records a system event. The caller must call changed(agentId) after the transaction commits. */
  async systemWarningTx(tx: SqliteExecutor, agentId: string, eventKey: string, value: NotificationInput, provenance: NotificationProvenance): Promise<{ id: string; created: boolean }> {
    input(value);
    if (!eventKey.trim() || eventKey.length > 200) throw new AgentError("Invalid notification event key.");
    const key = `system:${eventKey}`;
    const existing = await tx.get<{ id: string }>("SELECT id FROM hv_notifications WHERE agent_id=? AND event_key=?", agentId, key);
    if (existing) return { id: existing.id, created: false };
    const settings = await this.settingsTx(tx, agentId);
    const agent = await tx.get<{ state: string }>("SELECT state FROM hv_agents WHERE id=?", agentId);
    const deliver = Boolean(settings.enabled) && !!agent && !["stopping", "stopped"].includes(agent.state);
    const id = await this.persistTx(tx, agentId, key, "message", value, provenance, deliver);
    return { id, created: true };
  }
  /** Trusted backend entry point. The key identifies one logical application event. */
  async send(agentId: string, eventKey: string, value: NotificationInput): Promise<NotificationResult> {
    input(value);
    if (!eventKey.trim() || eventKey.length > 200) throw new AgentError("Invalid notification event key.");
    const result = await this.db.transaction((tx) => this.queueTx(tx, agentId, `backend:${eventKey}`, "message", value, { source: "Backend" }));
    this.changed(agentId);
    return result;
  }
  async sendFromTool(actor: MemoryActor, value: NotificationInput, execution: Pick<ExecutionContextService, "authorize">): Promise<NotificationResult> {
    const message = input(value), key = `tool:${actor.toolTaskId}`, hash = requestHash(actor, "send_notification", message);
    const result = await this.db.transaction(async (tx) => {
      const receipt = await tx.get<{ request_hash: string; result: string }>("SELECT request_hash,result FROM hv_notification_operations WHERE agent_id=? AND operation_key=?", actor.agentId, key);
      if (receipt) {
        if (receipt.request_hash !== hash) throw new AgentError("Notification operation reused with different arguments or actor", 409);
        return JSON.parse(receipt.result) as NotificationResult;
      }
      if (actor.toolTaskId === null || actor.conversationId === null || !actor.subagentId) throw new AgentError("Unverified notification producer", 403);
      await assertActiveActor(tx, actor);
      await execution.authorize(tx, actor, "send_notification");
      const worker = await tx.get<{ name: string; kind: string }>("SELECT name,kind FROM hv_subagents WHERE agent_id=? AND id=?", actor.agentId, actor.subagentId);
      const result = await this.queueTx(tx, actor.agentId, key, "message", message, { source: worker?.name ?? "Agent", subagentId: actor.subagentId, runId: actor.runId, conversationId: actor.conversationId });
      await tx.run("INSERT INTO hv_notification_operations(agent_id,operation_key,request_hash,result) VALUES (?,?,?,?)", actor.agentId, key, hash, JSON.stringify(result));
      return result;
    });
    this.changed(actor.agentId);
    return result;
  }
  /** Runs inside the order outcome transaction; one aggregate receipt per request, plus uncertainty resolution. */
  async orderOutcomeTx(tx: SqliteExecutor, actionId: string) {
    const request = await tx.get<{ id: string; agent_id: string; request_key: string; created_at: number }>(`SELECT r.* FROM hv_order_requests r JOIN hv_order_actions a ON a.request_id=r.id WHERE a.id=? AND r.kind IN ('submit','close_position')`, actionId);
    if (!request) return;
    const actions = await tx.all<{ state: string }>("SELECT state FROM hv_order_actions WHERE request_id=?", request.id);
    const existing = await tx.get<{ title: string }>("SELECT title FROM hv_notifications WHERE agent_id=? AND event_key=?", request.agent_id, `order:${request.id}:submission`);
    if (actions.some((a) => a.state === "submitting")) return;
    // A crash can leave later legs prepared forever. Still warn immediately about
    // an uncertain attempted leg, without waiting for a model to replay the request.
    if (actions.some((a) => a.state === "prepared") && !actions.some((a) => a.state === "unknown") && !existing) return;
    const settings = await this.settingsTx(tx, request.agent_id);
    if (request.created_at < settings.enabled_since) return;
    const items = await tx.all<{ view: string }>(`SELECT i.view FROM hv_order_items i JOIN hv_order_actions a ON a.id=i.action_id WHERE a.request_id=? ORDER BY a.ordinal,i.ordinal`, request.id);
    const views = items.map((row) => JSON.parse(row.view) as OrderView);
    const unknown = views.some((v) => v.commandStatus === "unknown");
    if (existing && (existing.title !== "Order submission outcome unknown" || unknown)) return;
    if (existing && await tx.get("SELECT 1 FROM hv_notifications WHERE agent_id=? AND event_key=?", request.agent_id, `order:${request.id}:resolved`)) return;
    const rejected = views.some((v) => v.commandStatus === "rejected" || v.commandStatus === "not_sent");
    const accepted = views.some((v) => ["accepted", "reconciled"].includes(v.commandStatus));
    const title = existing ? "Order submission outcome resolved" : unknown ? "Order submission outcome unknown" : rejected ? accepted ? "Order submission partially accepted" : "Order submission rejected / not sent" : "Orders submitted";
    const body = [`Request: ${request.request_key}`, ...views.map((v) => `${v.symbol} ${v.side} ${v.kind}: ${v.size ?? "unknown"} @ ${v.triggerPrice ?? v.price ?? "market"} — ${v.commandStatus}${v.exchangeOid ? ` (order ${v.exchangeOid})` : ""}${v.error ? `; ${v.error}` : ""}`), unknown ? "Unknown is not failure. Reconcile this request; do not resubmit with a new ID." : "Submission acceptance is not proof of a fill. Fill notifications report actual executions."].join("\n");
    const provenance = await tx.get<{ subagent_id: string; source_run_id: string | null; conversation_id: number }>("SELECT producer_subagent_id AS subagent_id,source_run_id,conversation_id FROM hv_order_request_provenance WHERE request_id=?", request.id);
    const result = await this.queueTx(tx, request.agent_id, `order:${request.id}:${existing ? "resolved" : "submission"}`, "submission", { title, body: body.slice(0, 12000), severity: unknown || (rejected && accepted) ? "warning" : rejected ? "error" : "info" }, { source: "Order execution", subagentId: provenance?.subagent_id, runId: provenance?.source_run_id, conversationId: provenance?.conversation_id, orderRequestId: request.id });
    return result.status === "queued" ? request.agent_id : undefined;
  }
  /** Called only for newly inserted exchange fills, atomically with journal evidence. */
  async fillsTx(tx: SqliteExecutor, account: string, events: JournalEvent[]) {
    const changed = new Set<string>();
    if (!events.length) return [];
    const agents = await tx.all<{ id: string }>("SELECT id FROM hv_agents WHERE trading_account_address=? AND state NOT IN ('stopping','stopped')", account);
    for (const agent of agents) {
      const settings = await this.settingsTx(tx, agent.id);
      if (!settings.enabled || !settings.fills) continue;
      // Never announce old history imports. The persisted floor survives restarts and re-enabling.
      const fresh = events.filter((event) => event.time >= Math.max(settings.enabled_since, settings.fills_since));
      const groups = new Map<string, JournalEvent[]>();
      for (const event of fresh) {
        const raw = event.raw as { oid?: unknown } | null;
        const key = typeof raw?.oid === "number" && Number.isSafeInteger(raw.oid) ? `${event.symbol}:${raw.oid}` : event.id;
        const group = groups.get(key) ?? []; group.push(event); groups.set(key, group);
      }
      for (const group of groups.values()) {
        group.sort((a, b) => a.time - b.time || a.id.localeCompare(b.id));
        const event = group[0]!;
        const raw = event.raw as { oid?: unknown; cloid?: unknown; sz?: unknown; px?: unknown; side?: unknown; liquidation?: unknown } | null;
        const oid = typeof raw?.oid === "number" && Number.isSafeInteger(raw.oid) ? String(raw.oid) : null;
        const cloid = typeof raw?.cloid === "string" ? raw.cloid : null;
        const local = oid || cloid ? await tx.get<{ request_id: string; producer_subagent_id: string | null; source_run_id: string | null }>(`SELECT r.id AS request_id,p.producer_subagent_id,p.source_run_id FROM hv_order_items i JOIN hv_order_actions a ON a.id=i.action_id JOIN hv_order_requests r ON r.id=a.request_id LEFT JOIN hv_order_request_provenance p ON p.request_id=r.id WHERE r.agent_id=? AND r.account=? AND (i.oid=? OR i.cloid=?) LIMIT 1`, agent.id, account, oid, cloid) : undefined;
        const title = `${event.symbol ?? "Account"} order fill${raw?.liquidation ? " (liquidation)" : ""}`;
        const executions = group.slice(0, 30).map((entry) => {
          const data = entry.raw as { side?: unknown; sz?: unknown; px?: unknown } | null;
          const side = entry.fill?.side ?? data?.side, size = entry.fill?.size ?? data?.sz, price = entry.fill?.price ?? data?.px;
          return `${side === "B" || side === "buy" ? "Buy" : side === "A" || side === "sell" ? "Sell" : "Unknown side"} ${typeof size === "string" ? size : "unknown quantity"} ${entry.symbol ?? ""} @ ${typeof price === "string" ? price : "unknown price"}`;
        });
        const body = `${executions.join("\n")}${group.length > 30 ? `\n${group.length - 30} more executions; see account transaction history.` : ""}\n${group.length > 1 ? `${group.length} executions observed together\n` : ""}${oid ? `Order: ${oid}\n` : ""}Observed: ${new Date(event.time).toISOString()}\nOrigin: ${local ? "HyperVibes order" : "External or unknown (not linked to a HyperVibes order)"}\nThese are executed quantities, not proof that the entire order filled.`;
        const result = await this.queueTx(tx, agent.id, `fill:${event.id}`, "fill", { title: title.slice(0, 200), body: body.slice(0, 12000), severity: raw?.liquidation ? "warning" : "info" }, { source: local ? "Account fill · HyperVibes" : "Account fill · external/unknown", subagentId: local?.producer_subagent_id, runId: local?.source_run_id, orderRequestId: local?.request_id });
        if (result.status === "queued") changed.add(agent.id);
      }
    }
    return [...changed];
  }
  async list(agentId: string, page = 1): Promise<NotificationPage> {
    if (!Number.isSafeInteger(page) || page < 1 || page > 100000) throw new AgentError("Invalid notification page.");
    return this.db.transaction(async (tx) => {
      const total = (await tx.get<{ total: number }>("SELECT count(*) AS total FROM hv_notifications WHERE agent_id=?", agentId))!.total;
      const pages = Math.max(1, Math.ceil(total / 50)); page = Math.min(page, pages);
      const items = await tx.all<NotificationRecord>(`SELECT ${columns} FROM hv_notifications WHERE agent_id=? ORDER BY created_at DESC,id DESC LIMIT 50 OFFSET ?`, agentId, (page - 1) * 50);
      return { items, page, pages, total };
    });
  }
  async get(agentId: string, id: string): Promise<NotificationRecord> {
    const row = await this.db.get<NotificationRecord>(`SELECT ${columns} FROM hv_notifications WHERE agent_id=? AND id=?`, agentId, id);
    if (!row) throw new AgentError("Notification not found", 404);
    return row;
  }
  /** Called after the owning transaction commits; delivery never waits on a model. */
  changed(agentId: string) {
    for (const listener of this.listeners.get(agentId) ?? []) listener();
    if (this.started && !this.stopping) this.schedule(0);
  }
  async watch(agentId: string, signal: AbortSignal, publish: (snapshot: { revision: number }) => void) {
    let revision = 0, dirty = true, wake: (() => void) | undefined;
    const changed = () => { dirty = true; wake?.(); };
    const listeners = this.listeners.get(agentId) ?? new Set<() => void>(); listeners.add(changed); this.listeners.set(agentId, listeners);
    signal.addEventListener("abort", changed);
    try {
      while (!signal.aborted) {
        if (dirty) { dirty = false; publish({ revision: revision++ }); }
        await new Promise<void>((resolve) => { wake = resolve; if (dirty || signal.aborted) resolve(); });
        wake = undefined;
      }
    } finally { listeners.delete(changed); if (!listeners.size) this.listeners.delete(agentId); signal.removeEventListener("abort", changed); }
  }
  start() { if (this.started || this.stopping) return; this.started = true; this.schedule(0); }
  private schedule(ms: number) {
    clearTimeout(this.timer);
    this.timer = setTimeout(() => { void this.flush().catch((error: unknown) => this.logger?.error(error, "Notification delivery pass failed")).finally(() => { if (!this.stopping) this.schedule(1000); }); }, ms);
    this.timer.unref();
  }
  /** Single-flight delivery pass, also used by isolated tests. */
  flush(): Promise<void> {
    if (this.stopping) return Promise.resolve();
    return this.running ??= this.deliver().finally(() => { this.running = undefined; });
  }
  private async deliver() {
    if (!this.sender) return;
    const rows = await this.db.all<DeliveryRow>(`SELECT ${columns},chat_id AS chatId,bot_token_hash AS botTokenHash,message_ids AS messageIds FROM hv_notifications WHERE id IN (
      SELECT id FROM (SELECT id,row_number() OVER (PARTITION BY bot_token_hash,chat_id ORDER BY created_at,id) AS position
      FROM hv_notifications WHERE delivery='pending' AND next_attempt_at<=?) WHERE position=1
    ) ORDER BY created_at,id LIMIT 50`, this.clock());
    for (const row of rows) {
      if (this.stopping) return;
      const chatKey = `${row.botTokenHash}:${row.chatId}`;
      if ((this.chatNext.get(chatKey) ?? 0) > this.clock()) continue;
      const allowed = await this.db.transaction(async (tx) => {
        const settings = await this.settingsTx(tx, row.agentId);
        const current = await tx.get<{ delivery: string }>("SELECT delivery FROM hv_notifications WHERE id=?", row.id);
        if (!current || current.delivery !== "pending") return false;
        const gateway = await tx.get("SELECT 1 FROM hv_gateways WHERE agent_id=? AND type='telegram' AND chat_id=? AND bot_token_hash=?", row.agentId, row.chatId, row.botTokenHash);
        const agent = await tx.get<{ state: string }>("SELECT state FROM hv_agents WHERE id=?", row.agentId);
        if (!settings.enabled || (row.category === "submission" && !settings.submissions) || (row.category === "fill" && !settings.fills) || !gateway || !agent || ["stopping", "stopped"].includes(agent.state)) {
          await tx.run("UPDATE hv_notifications SET delivery='suppressed',error='Notifications disabled or Telegram destination changed' WHERE id=?", row.id);
          return false;
        }
        return true;
      });
      if (!allowed) { this.changed(row.agentId); continue; }
      const emoji = row.severity === "error" ? "❌" : row.severity === "warning" ? "⚠️" : "ℹ️";
      const chunks = chunkText(`${emoji} ${row.title}\n\n${row.body}\n\n${new URL(`/agents/${encodeURIComponent(row.agentId)}/notifications/${row.id}`, this.origin).href}`, 4000);
      const ids = JSON.parse(row.messageIds) as number[];
      try {
        const messageId = await this.sender({ agentId: row.agentId, chatId: row.chatId, botTokenHash: row.botTokenHash }, chunks[ids.length]!, this.shutdown.signal);
        ids.push(messageId);
        await this.db.run(`UPDATE hv_notifications SET message_ids=?,delivery=CASE WHEN ?=1 THEN 'sent' ELSE delivery END,sent_at=?,error=NULL,next_attempt_at=? WHERE id=?`, JSON.stringify(ids), ids.length === chunks.length ? 1 : 0, ids.length === chunks.length ? this.clock() : null, this.clock() + 1100, row.id);
        this.chatNext.set(chatKey, this.clock() + 1100);
      } catch (error) {
        if (this.stopping) return; // Interrupted sends remain recoverable; Telegram has no idempotency key.
        const detail = error instanceof NotificationDeliveryError ? error : new NotificationDeliveryError("Telegram delivery failed; retrying");
        const attempts = row.attempts + 1;
        const delay = Math.max(detail.retryAfterMs ?? 0, Math.min(900000, 1000 * 2 ** attempts));
        await this.db.run("UPDATE hv_notifications SET attempts=?,delivery=?,error=?,next_attempt_at=? WHERE id=? AND delivery='pending'", attempts, detail.suppressed ? "suppressed" : detail.permanent || attempts >= 8 ? "failed" : "pending", detail.message, this.clock() + delay, row.id);
        this.chatNext.set(chatKey, this.clock() + delay);
      }
      this.changed(row.agentId);
    }
  }
  close(): Promise<void> { return this.closing ??= (async () => { this.stopping = true; clearTimeout(this.timer); this.shutdown.abort(); await this.running; this.chatNext.clear(); })(); }
}
