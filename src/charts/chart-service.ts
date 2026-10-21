import { randomUUID } from "node:crypto";
import type { SqliteExecutor } from "@earendil-works/pi-durable/storage/sqlite";
import { AgentError, requestId } from "../agents/agent-input.js";
import { intervalMillis } from "../hyperliquid/chart-contracts.js";
import { bodyHash, canonical } from "../canonical.js";
import type { SharedDatabase } from "../db/shared-database.js";
import { chartCapture, chartEdit, chartSettings, type ChartAttachment, type ChartContext, type ChartDocument, type ChartEdit, type ChartLayout } from "./contracts.js";

interface HistoryItem { document: ChartDocument; label: string }
interface History { past: HistoryItem[]; future: HistoryItem[]; label: string }
interface LayoutRow { id: string; document: string; revision: number; history: string; updated_at: number }
interface LayoutSummaryRow extends Omit<LayoutRow, "history"> { undo_count: number; redo_count: number; label: string }
const summaryColumns = "id,document,revision,updated_at,json_array_length(history,'$.past') AS undo_count,json_array_length(history,'$.future') AS redo_count,json_extract(history,'$.label') AS label";
interface SnapshotRow { id: string; context: string; image: string | null; request_hash: string }
function project(row: LayoutRow | LayoutSummaryRow): ChartLayout {
  const summary = "history" in row ? (() => { const h = JSON.parse(row.history) as History; return { canUndo: h.past.length > 0, canRedo: h.future.length > 0, lastChange: h.label }; })()
    : { canUndo: row.undo_count > 0, canRedo: row.redo_count > 0, lastChange: row.label };
  return { id: row.id, document: JSON.parse(row.document) as ChartDocument, revision: row.revision, updatedAt: row.updated_at, ...summary };
}
function attachment(id: string, context: ChartContext): ChartAttachment {
  return { snapshotId: id, layoutId: context.layoutId, name: context.document.name, coin: context.document.coin, interval: context.document.interval, capturedAt: context.capturedAt, image: context.screenshotIncluded, ...(context.selectedDrawingId ? { selectedDrawingId: context.selectedDrawingId } : {}) };
}
export class ChartService {
  private readonly listeners = new Map<string, Set<() => void>>();
  constructor(private readonly db: SharedDatabase) {}
  private notify(agentId: string) { for (const changed of this.listeners.get(agentId) ?? []) changed(); }
  /** Subscribe before the initial read so no committed edits can be missed. */
  async watch(agentId: string, signal: AbortSignal, publish: (layouts: ChartLayout[]) => void): Promise<void> {
    let dirty = true;
    let wake: (() => void) | undefined;
    const changed = () => { dirty = true; wake?.(); };
    const listeners = this.listeners.get(agentId) ?? new Set<() => void>();
    this.listeners.set(agentId, listeners); listeners.add(changed);
    signal.addEventListener("abort", changed);
    try {
      while (!signal.aborted) {
        if (dirty) {
          dirty = false;
          const snapshot = await this.list(agentId);
          if (!signal.aborted) publish(snapshot);
        }
        if (!dirty && !signal.aborted) await new Promise<void>((resolve) => { wake = resolve; });
        wake = undefined;
      }
    } finally {
      signal.removeEventListener("abort", changed);
      listeners.delete(changed);
      if (!listeners.size) this.listeners.delete(agentId);
    }
  }
  private async row(db: SqliteExecutor, agentId: string, id: string) {
    const row = await db.get<LayoutRow>("SELECT * FROM hv_chart_layouts WHERE agent_id=? AND id=?", agentId, requestId(id));
    if (!row) throw new AgentError("Chart layout not found", 404);
    return row;
  }
  async list(agentId: string): Promise<ChartLayout[]> {
    return (await this.db.all<LayoutSummaryRow>(`SELECT ${summaryColumns} FROM hv_chart_layouts WHERE agent_id=? ORDER BY updated_at DESC, id`, agentId)).map(project);
  }
  async get(agentId: string, id: string) {
    const row = await this.db.get<LayoutSummaryRow>(`SELECT ${summaryColumns} FROM hv_chart_layouts WHERE agent_id=? AND id=?`, agentId, requestId(id));
    if (!row) throw new AgentError("Chart layout not found", 404);
    return project(row);
  }
  async create(agentId: string, id: string, settings: unknown, sourceId?: string): Promise<ChartLayout> {
    id = requestId(id);
    const parsed = chartSettings(settings);
    const result = await this.db.transaction(async (db) => {
      const hash = bodyHash(canonical({ settings: parsed, sourceId: sourceId ?? null }));
      const saved = await db.get<{ hash: string }>("SELECT hash FROM hv_chart_operations WHERE agent_id=? AND key=?", agentId, `create:${id}`);
      if (saved) {
        if (saved.hash !== hash) throw new AgentError("Chart creation request already used", 409);
        return { layout: project(await this.row(db, agentId, id)), changed: false };
      }
      if ((await db.get<{ total: number }>("SELECT count(*) AS total FROM hv_chart_layouts WHERE agent_id=?", agentId))!.total >= 50) throw new AgentError("An Agent can have up to 50 chart layouts");
      const source = sourceId ? project(await this.row(db, agentId, sourceId)).document : undefined;
      const document: ChartDocument = { name: "Untitled chart", coin: "BTC", interval: "1h", drawings: [], notes: "", hideDrawings: false, magnet: true, ...source, ...parsed };
      // Copies get fresh object identities; old conversational references still identify the original.
      if (source) document.drawings = source.drawings.map((d) => ({ ...d, id: randomUUID() }));
      await db.run("INSERT INTO hv_chart_layouts(id,agent_id,document,revision,history,updated_at) VALUES (?,?,?,0,?,?)", id, agentId, JSON.stringify(document), JSON.stringify({ past: [], future: [], label: "Created layout" }), Date.now());
      await db.run("INSERT INTO hv_chart_operations(agent_id,key,hash,result) VALUES (?,?,?,?)", agentId, `create:${id}`, hash, id);
      return { layout: project(await this.row(db, agentId, id)), changed: true };
    });
    if (result.changed) this.notify(agentId);
    return result.layout;
  }
  async delete(agentId: string, id: string, expectedRevision: number) {
    await this.db.transaction(async (db) => {
      const row = await this.row(db, agentId, id);
      if (row.revision !== expectedRevision) throw new AgentError("Chart changed. Reload before deleting.", 409);
      await db.run("DELETE FROM hv_chart_layouts WHERE agent_id=? AND id=?", agentId, id);
    });
    this.notify(agentId);
  }
  async edit(agentId: string, id: string, expectedRevision: number, input: unknown, operationKey: string, actor: "user" | "agent" = "user") {
    const edit = chartEdit(input);
    if (actor === "agent" && edit.settings && Object.keys(edit.settings).some((key) => key !== "notes")) throw new AgentError("Agents may edit drawings and layout notes, not change your chart selection");
    return this.change(agentId, id, expectedRevision, operationKey, { edit, actor }, (document, history) => {
      const next = applyEdit(document, edit, actor);
      history.past.push({ document, label: history.label }); history.future = []; history.label = `${actor === "agent" ? "Agent" : "You"}: ${edit.label}`;
      return next;
    });
  }
  async undo(agentId: string, id: string, expectedRevision: number, operationKey: string, redo = false) {
    return this.change(agentId, id, expectedRevision, operationKey, { redo }, (document, history) => {
      const from = redo ? history.future : history.past, to = redo ? history.past : history.future;
      const target = from.pop();
      if (!target) throw new AgentError(redo ? "Nothing to redo" : "Nothing to undo", 409);
      to.push({ document, label: history.label }); history.label = target.label;
      return target.document;
    });
  }
  private async change(agentId: string, id: string, expectedRevision: number, key: string, input: object, apply: (document: ChartDocument, history: History) => ChartDocument) {
    const result = await this.db.transaction(async (db) => {
      const hash = bodyHash(canonical({ id, expectedRevision, input }));
      const saved = await db.get<{ hash: string }>("SELECT hash FROM hv_chart_operations WHERE agent_id=? AND key=?", agentId, key);
      if (saved && saved.hash !== hash) throw new AgentError("Chart operation ID already used with different content", 409);
      const row = await this.row(db, agentId, id);
      if (saved) return { layout: project(row), changed: false };
      if (row.revision !== expectedRevision) throw new AgentError("Chart changed elsewhere. Reload the layout and reconcile your edit.", 409);
      const document = JSON.parse(row.document) as ChartDocument, history = JSON.parse(row.history) as History;
      const next = apply(document, history);
      if (JSON.stringify(next).length > 250_000) throw new AgentError("Chart layout is too large");
      history.past = history.past.slice(-30); history.future = history.future.slice(-30);
      while (JSON.stringify(history).length > 2_000_000 && history.past.length > 1) history.past.shift();
      const updatedAt = Date.now();
      await db.run("UPDATE hv_chart_layouts SET document=?,revision=revision+1,history=?,updated_at=? WHERE agent_id=? AND id=?", JSON.stringify(next), JSON.stringify(history), updatedAt, agentId, id);
      await db.run("INSERT INTO hv_chart_operations(agent_id,key,hash,result) VALUES (?,?,?,?)", agentId, key, hash, id);
      return { layout: project(await this.row(db, agentId, id)), changed: true };
    });
    if (result.changed) this.notify(agentId);
    return result.layout;
  }
  /** Capture before durable submission. A retry cannot silently take a new picture or viewport. */
  async capture(agentId: string, id: string, message: string, input: unknown) {
    id = requestId(id);
    const capture = chartCapture(input), hash = bodyHash(canonical({ message, capture }));
    return this.db.transaction(async (db) => {
      const existing = await db.get<SnapshotRow>("SELECT * FROM hv_chart_snapshots WHERE agent_id=? AND id=?", agentId, id);
      if (existing) {
        if (existing.request_hash !== hash) throw new AgentError("Chart message request already used with different content", 409);
        return this.snapshotResult(existing);
      }
      const layout = project(await this.row(db, agentId, capture.layoutId));
      if (layout.revision !== capture.revision) throw new AgentError("Chart changed before sending. Review the updated chart and send again.", 409);
      if (capture.selectedDrawingId && !layout.document.drawings.some((d) => d.id === capture.selectedDrawingId && d.coin === layout.document.coin)) throw new AgentError("Selected chart object is unavailable");
      const { image, ...view } = capture;
      if (image) validateImage(image);
      const forming = capture.candles.find((c) => c.time * 1000 <= capture.capturedAt && c.time * 1000 + intervalMillis[layout.document.interval] > capture.capturedAt);
      const context: ChartContext = { ...view, document: layout.document, screenshotIncluded: Boolean(image), formingCandleTime: forming?.time ?? null };
      await db.run("INSERT INTO hv_chart_snapshots(id,agent_id,request_hash,context,image,created_at) VALUES (?,?,?,?,?,?)", id, agentId, hash, JSON.stringify(context), image ?? null, Date.now());
      return this.snapshotResult({ id, context: JSON.stringify(context), image: image ?? null, request_hash: hash });
    });
  }
  private snapshotResult(row: SnapshotRow) {
    const context = JSON.parse(row.context) as ChartContext;
    return { context, attachment: attachment(row.id, context), image: row.image };
  }
  async snapshot(agentId: string, id: string, offset = 0, limit = 100) {
    const row = await this.db.get<SnapshotRow>("SELECT * FROM hv_chart_snapshots WHERE agent_id=? AND id=?", agentId, requestId(id));
    if (!row) throw new AgentError("Chart snapshot not found", 404);
    const context = JSON.parse(row.context) as ChartContext;
    return { snapshotId: id, ...context, candles: context.candles.slice(offset, offset + limit), totalCandles: context.candles.length, nextOffset: offset + limit < context.candles.length ? offset + limit : null,
      warning: "Frozen browser display data, not verified execution prices. Candle times and range are UTC seconds; capturedAt is milliseconds. formingCandleTime identifies a forming candle using the browser capture clock, or null when none is identified. Use research_candles/TA tools for verified closed-candle research. Screenshot is attached to the original user input only." };
  }
}
function applyEdit(document: ChartDocument, edit: ChartEdit, actor: "user" | "agent"): ChartDocument {
  const drawings = new Map(document.drawings.map((d) => [d.id, d]));
  for (const id of edit.remove ?? []) {
    const old = drawings.get(id);
    if (!old) throw new AgentError("Drawing not found", 404);
    if (old.locked) throw new AgentError("Unlock the drawing before deleting it", 409);
    drawings.delete(id);
  }
  for (const supplied of edit.upsert ?? []) {
    const old = drawings.get(supplied.id);
    if (old && old.coin !== supplied.coin) throw new AgentError("Drawings cannot move between instruments");
    if (old?.locked && (actor === "agent" || canonical({ ...old, locked: false }) !== canonical(supplied))) throw new AgentError("Unlock the drawing before editing it", 409);
    drawings.set(supplied.id, { ...supplied, author: old?.author ?? actor });
  }
  if (drawings.size > 300) throw new AgentError("A layout can contain up to 300 drawings");
  return { ...document, ...edit.settings, drawings: [...drawings.values()] };
}
function validateImage(image: string) {
  const bytes = Buffer.from(image, "base64");
  if (bytes.toString("base64") !== image || bytes.length < 24 || !bytes.subarray(0, 8).equals(Buffer.from([137,80,78,71,13,10,26,10])) || bytes.toString("ascii", 12, 16) !== "IHDR") throw new AgentError("Chart screenshot must be a PNG image");
  const width = bytes.readUInt32BE(16), height = bytes.readUInt32BE(20);
  if (!width || !height || width > 4096 || height > 4096 || width * height > 8_000_000) throw new AgentError("Chart screenshot dimensions exceed limits");
}
