import { createHash, randomUUID } from "node:crypto";
import type { SharedDatabase } from "../db/shared-database.js";
import type { SqliteExecutor } from "@earendil-works/pi-durable/storage/sqlite";
import { AgentError } from "../agents/agent-input.js";

export const artifactTypes = ["text/markdown", "text/plain", "text/csv", "application/json", "image/png", "image/jpeg", "image/webp"] as const;
export interface ArtifactInput {
  title: string; description?: string; mediaType: typeof artifactTypes[number]; content: string; encoding?: "utf8" | "base64";
  tags?: string[]; instrument?: string; interval?: string; attachments?: string[]; retained?: boolean;
}
export interface ArtifactMeta {
  id: string; agentId: string; title: string; description: string; mediaType: string; size: number;
  tags: string[]; instrument: string; interval: string; createdAt: number; expiresAt: number | null;
  conversationId: number | null; taskId: number | null;
}
interface Row extends Omit<ArtifactMeta, "tags"> { tags: string }
const columns = `id,agent_id AS agentId,title,description,media_type AS mediaType,size,tags,instrument,interval,created_at AS createdAt,expires_at AS expiresAt,conversation_id AS conversationId,task_id AS taskId`;
// Only a one-level report -> attachment graph is allowed. An unexpired report
// protects its children even when their own temporary retention has elapsed.
const visible = `(a.expires_at IS NULL OR a.expires_at>? OR EXISTS(SELECT 1 FROM hv_artifact_links l JOIN hv_artifacts p ON p.id=l.parent WHERE l.child=a.id AND (p.expires_at IS NULL OR p.expires_at>?)))`;
const meta = (row: Row): ArtifactMeta => ({ ...row, tags: JSON.parse(row.tags) as string[] });
const maxBytes = 5 * 1024 * 1024;
function text(value: string, max: number, label: string) { if (typeof value !== "string" || value.length > max) throw new AgentError(`Invalid ${label}`); return value.trim(); }

/** Payloads are immutable, root-Agent scoped and never stored in Pi documents. */
export class ArtifactService {
  constructor(private readonly db: SharedDatabase) {}
  private async cleanupTx(tx: SqliteExecutor, now: number) {
    // Delete expired parents first; child references no longer protect them.
    await tx.run(`DELETE FROM hv_artifacts WHERE id IN(SELECT a.id FROM hv_artifacts a WHERE NOT ${visible} LIMIT 100)`, now, now);
  }
  async cleanup(now = Date.now()) {
    await this.db.transaction((tx) => this.cleanupTx(tx, now));
    // No implicit rebuild of an existing NONE database. Fresh profiles can
    // reclaim space in small batches; WAL checkpointing remains separate.
    await this.db.all("PRAGMA incremental_vacuum(256)");
  }
  async create(agentId: string, operationKey: string, input: ArtifactInput, producer: { conversationId?: number; taskId?: number } = {}) {
    const title = text(input.title, 160, "title");
    if (!title || !artifactTypes.includes(input.mediaType)) throw new AgentError("Invalid artifact type/title");
    const description = text(input.description ?? "", 1000, "description");
    if (typeof input.content !== "string" || input.content.length > maxBytes * 1.4) throw new AgentError("Artifact exceeds 5 MiB");
    const image = input.mediaType.startsWith("image/");
    if (image !== (input.encoding === "base64")) throw new AgentError("Images require base64; text requires UTF-8");
    const payload = Buffer.from(input.content, image ? "base64" : "utf8");
    // Round-trip validation avoids permissive decoding and deep-regex stack
    // exhaustion on multi-megabyte images.
    if (image && payload.toString("base64") !== input.content) throw new AgentError("Invalid base64");
    if (!payload.length || payload.length > maxBytes) throw new AgentError("Artifacts must contain 1 byte to 5 MiB");
    if (image) {
      const valid = input.mediaType === "image/png" ? payload.subarray(0, 8).equals(Buffer.from([137,80,78,71,13,10,26,10])) : input.mediaType === "image/jpeg" ? payload[0] === 255 && payload[1] === 216 && payload[2] === 255 : payload.toString("ascii", 0, 4) === "RIFF" && payload.toString("ascii", 8, 12) === "WEBP";
      if (!valid) throw new AgentError("Image signature does not match its type");
    }
    if (input.mediaType === "application/json") { try { JSON.parse(input.content); } catch { throw new AgentError("Invalid JSON artifact"); } }
    if (input.tags && (!Array.isArray(input.tags) || input.tags.length > 12)) throw new AgentError("At most 12 tags");
    const tags = [...new Set((input.tags ?? []).map((tag) => text(tag, 40, "tag").toLowerCase()))].filter(Boolean).sort();
    const instrument = text(input.instrument ?? "", 64, "instrument"), interval = text(input.interval ?? "", 8, "interval");
    const embedded = input.mediaType === "text/markdown" ? [...input.content.matchAll(/artifact:([a-f0-9-]{36})/g)].map((match) => match[1]!) : [];
    const attachments = [...new Set([...(input.attachments ?? []), ...embedded])].sort();
    if (attachments.length > 20 || attachments.some((id) => !/^[a-f0-9-]{36}$/.test(id))) throw new AgentError("Invalid attachments (maximum 20)");
    if (attachments.length && input.mediaType !== "text/markdown") throw new AgentError("Only Markdown reports may contain attachments");
    const hash = createHash("sha256").update(JSON.stringify({ title, description, mediaType: input.mediaType, tags, instrument, interval, attachments, retained: Boolean(input.retained) })).update(payload).digest("hex");
    return this.db.transaction(async (tx) => {
      const existing = await tx.get<{ id: string; request_hash: string }>("SELECT artifact_id AS id,request_hash FROM hv_artifact_receipts WHERE agent_id=? AND operation_key=?", agentId, operationKey);
      if (existing) {
        if (existing.request_hash !== hash) throw new AgentError("Artifact operation key reused with different content", 409);
        return this.metadata(tx, agentId, existing.id);
      }
      await this.cleanupTx(tx, Date.now());
      for (const id of attachments) {
        const child = await this.metadata(tx, agentId, id);
        if (child.mediaType === "text/markdown") throw new AgentError("Nested reports are not supported");
      }
      const usage = await tx.get<{ bytes: number; total: number }>("SELECT coalesce(sum(size),0) AS bytes,count(*) AS total FROM hv_artifacts WHERE agent_id=?", agentId);
      if ((usage?.bytes ?? 0) + payload.length > 100 * 1024 * 1024 || (usage?.total ?? 0) >= 2000) throw new AgentError("Agent artifact quota reached. Delete old artifacts.", 409);
      const id = randomUUID(), now = Date.now();
      await tx.run(`INSERT INTO hv_artifacts(id,agent_id,title,description,media_type,payload,size,tags,instrument,interval,conversation_id,task_id,created_at,expires_at) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?)`, id, agentId, title, description, input.mediaType, payload, payload.length, JSON.stringify(tags), instrument, interval, producer.conversationId ?? null, producer.taskId ?? null, now, input.retained ? null : now + 7 * 86400_000);
      await tx.run("INSERT INTO hv_artifact_receipts(agent_id,operation_key,request_hash,artifact_id) VALUES (?,?,?,?)", agentId, operationKey, hash, id);
      for (const child of attachments) await tx.run("INSERT INTO hv_artifact_links(parent,child) VALUES (?,?)", id, child);
      return this.metadata(tx, agentId, id);
    });
  }
  /** Trusted tool task identity is immutable; reuse its original exported snapshot. */
  async operation(agentId: string, operationKey: string) {
    return this.db.transaction(async (tx) => {
      const receipt = await tx.get<{ id: string }>("SELECT artifact_id AS id FROM hv_artifact_receipts WHERE agent_id=? AND operation_key=?", agentId, operationKey);
      return receipt ? this.metadata(tx, agentId, receipt.id) : undefined;
    });
  }
  private async metadata(tx: SqliteExecutor, agentId: string, id: string) {
    const now = Date.now();
    const row = await tx.get<Row>(`SELECT ${columns} FROM hv_artifacts a WHERE a.agent_id=? AND a.id=? AND ${visible}`, agentId, id, now, now);
    if (!row) throw new AgentError("Artifact unavailable or expired", 404);
    return meta(row);
  }
  async list(agentId: string, filters: { offset?: number; instrument?: string; interval?: string; tag?: string; query?: string } = {}) {
    const offset = filters.offset ?? 0;
    if (!Number.isSafeInteger(offset) || offset < 0 || offset > 10000) throw new AgentError("Invalid artifact offset");
    const now = Date.now();
    const rows = await this.db.all<Row>(`SELECT ${columns} FROM hv_artifacts a WHERE agent_id=? AND ${visible} AND (?='' OR instrument=?) AND (?='' OR interval=?) AND (?='' OR EXISTS(SELECT 1 FROM json_each(a.tags) WHERE value=?)) AND (?='' OR instr(lower(title||' '||description),lower(?))>0) ORDER BY created_at DESC,id DESC LIMIT 51 OFFSET ?`, agentId, now, now, filters.instrument ?? "", filters.instrument ?? "", filters.interval ?? "", filters.interval ?? "", filters.tag ?? "", filters.tag ?? "", filters.query ?? "", filters.query ?? "", offset);
    return { items: rows.slice(0, 50).map(meta), nextOffset: rows.length > 50 ? offset + 50 : null };
  }
  async get(agentId: string, id: string) {
    return this.db.transaction(async (tx) => {
      const item = await this.metadata(tx, agentId, id);
      const row = (await tx.get<{ payload: Uint8Array }>("SELECT payload FROM hv_artifacts WHERE id=? AND agent_id=?", id, agentId))!;
      const attachments = await tx.all<{ id: string; title: string; mediaType: string }>("SELECT a.id,a.title,a.media_type AS mediaType FROM hv_artifact_links l JOIN hv_artifacts a ON a.id=l.child WHERE l.parent=? AND a.agent_id=?", id, agentId);
      return { ...item, payload: Buffer.from(row.payload), attachments };
    });
  }
  async read(agentId: string, id: string, offset = 0, limit = 16000) {
    if (!Number.isSafeInteger(offset) || offset < 0 || !Number.isSafeInteger(limit) || limit < 1 || limit > 16000) throw new AgentError("Invalid read bounds");
    const { payload, attachments, ...item } = await this.get(agentId, id);
    const content = item.mediaType.startsWith("image/") ? null : payload.toString("utf8");
    return { ...item, attachments, content: content?.slice(offset, offset + limit) ?? null, nextOffset: content && offset + limit < content.length ? offset + limit : null };
  }
  async retention(agentId: string, id: string, retained: boolean) {
    await this.db.transaction(async (tx) => {
      await this.metadata(tx, agentId, id);
      await tx.run("UPDATE hv_artifacts SET expires_at=? WHERE agent_id=? AND id=?", retained ? null : Date.now() + 7 * 86400_000, agentId, id);
    });
  }
  async remove(agentId: string, id: string) {
    await this.db.transaction(async (tx) => {
      await this.metadata(tx, agentId, id);
      const ref = await tx.get("SELECT l.parent FROM hv_artifact_links l JOIN hv_artifacts p ON p.id=l.parent WHERE l.child=? AND (p.expires_at IS NULL OR p.expires_at>?) LIMIT 1", id, Date.now());
      if (ref) throw new AgentError("Artifact is attached to a report. Delete the report first.", 409);
      await tx.run("DELETE FROM hv_artifacts WHERE agent_id=? AND id=?", agentId, id);
    });
  }
}
