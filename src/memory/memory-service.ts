import type { SqliteExecutor } from "@earendil-works/pi-durable/storage/sqlite";
import { AgentError } from "../agents/agent-input.js";
import { candleIntervals } from "../agents/candle-intervals.js";
import type { SharedDatabase } from "../db/shared-database.js";
import { artifactVisibleSql } from "../research/artifacts.js";
import { assertActiveActor, operationReplay, requestHash, saveOperation, type MemoryActor, type MemoryAuthorization } from "./operations.js";
import { instrument, interval, memoryKinds, memoryRef, parseMemoryRef, publicationInput, researchRequirements, timestamp, topic, type MemoryDetail, type MemoryKind, type MemoryMetadata, type ResearchPayload, type ResearchStatus } from "./memory-contracts.js";
import { choice, fields, integer, text } from "./validation.js";

interface Row {
  number: number; status: string; kind: MemoryKind; topic: string; instrument: string | null; interval: string | null; summary: string; content: string; payload: string;
  created_at: number; evidence_as_of: number | null; valid_until: number | null; producer_subagent_id: string | null; source_run_id: string | null;
  conversation_id: number | null; tool_task_id: number | null;
  retracted_at: number | null; retraction_reason: string | null;
}
export interface PublicationResult { ref: string; kind: MemoryKind; createdAt: number; validUntil: number | null }
/** Host validity cap: research cannot claim validity longer than this past its evidence. */
export const maxResearchAgeSeconds = 7 * 86400;
const columns = "number,kind,topic,instrument,interval,summary,created_at,evidence_as_of,valid_until,producer_subagent_id,source_run_id,retracted_at,retraction_reason";
function statusSql(now: number) {
  timestamp(now);
  return `CASE WHEN kind!='research' THEN 'durable' WHEN producer_subagent_id IS NOT NULL AND NOT EXISTS(SELECT 1 FROM hv_subagents p WHERE p.agent_id=hv_memories.agent_id AND p.id=hv_memories.producer_subagent_id AND p.enabled=1 AND p.archived=0) THEN 'disabled' WHEN retracted_at IS NOT NULL THEN 'retracted' WHEN evidence_as_of>${now} THEN 'invalid_evidence' WHEN json_extract(payload,'$.setup')='withdrawn' THEN 'withdrawn' WHEN json_extract(payload,'$.coverage')!='complete' THEN 'incomplete' WHEN valid_until<=${now} THEN 'stale' ELSE 'fresh' END`;
}
function metadata(row: Row): MemoryMetadata {
  return { status: row.status, ref: memoryRef(row.number), kind: row.kind, topic: row.topic, instrument: row.instrument, interval: row.interval, summary: row.summary, createdAt: row.created_at, evidenceAsOf: row.evidence_as_of, validUntil: row.valid_until, producerSubagentId: row.producer_subagent_id, sourceRunId: row.source_run_id, retractedAt: row.retracted_at, retractionReason: row.retraction_reason };
}
export class MemoryService {
  constructor(private readonly db: SharedDatabase, private readonly authorize: MemoryAuthorization, private readonly clock: () => number = Date.now) {}
  publishResearch(actor: MemoryActor, key: string, input: unknown) { return this.publish(actor, key, "research", input); }
  recordDecision(actor: MemoryActor, key: string, input: unknown) { return this.publish(actor, key, "decision", input); }
  publishReview(actor: MemoryActor, key: string, input: unknown) { return this.publish(actor, key, "review", input); }
  private async publish(actor: MemoryActor, key: string, kind: MemoryKind, value: unknown): Promise<PublicationResult> {
    const input = publicationInput(kind, value);
    const operation = kind === "research" ? "publish_research" : kind === "decision" ? "record_trade_decision" : "publish_review";
    const hash = requestHash(actor, operation, input);
    return this.db.transaction(async (tx) => {
      const replay = await operationReplay<PublicationResult>(tx, actor, key, operation, hash);
      if (replay) return replay;
      const now = timestamp(this.clock());
      await assertActiveActor(tx, actor);
      await this.authorize(tx, actor, operation);
      let deadline: number | null = null;
      if (input.kind === "research") {
        const width = candleIntervals[interval(input.interval)] * 1000;
        if (input.evidenceAsOf > now || input.evidenceAsOf % width !== 0) throw new AgentError("Evidence must be an aligned closed-candle end boundary in UTC milliseconds, not in the future");
        // Host policy, not model prose, bounds how long research stays actionable.
        deadline = timestamp(input.evidenceAsOf + maxResearchAgeSeconds * 1000);
        if (input.requestedValidUntil !== null) deadline = Math.min(deadline, input.requestedValidUntil);
        if (deadline <= input.evidenceAsOf) throw new AgentError("Research validity must follow its evidence boundary");
      }
      const cited = input.kind === "decision" ? input.payload.researchRefs : input.kind === "review" ? input.payload.memoryRefs : [];
      const links = [...input.links];
      for (const ref of cited) if (!links.some((link) => link.ref === ref && link.relation === (kind === "review" ? "reviews" : "based_on"))) links.push({ ref, relation: kind === "review" ? "reviews" : "based_on" });
      if (links.length > 20) throw new AgentError("At most 20 combined memory relationships");
      for (const link of links) {
        const target = await this.row(tx, actor.agentId, parseMemoryRef(link.ref));
        if (input.kind === "decision" && input.payload.researchRefs.includes(link.ref) && target.kind !== "research") throw new AgentError("Decision research refs must identify research publications");
        if (link.relation === "corrects" || link.relation === "supersedes") {
          if (input.kind !== "research" || target.kind !== "research" || target.producer_subagent_id !== actor.subagentId || target.instrument !== input.instrument || target.topic !== input.topic || target.interval !== input.interval) throw new AgentError("Corrections require the same research producer and stream");
          deadline = Math.min(deadline!, target.valid_until!);
          if (deadline <= input.evidenceAsOf) throw new AgentError("A correction cannot renew validity; publish new analysis instead");
        }
      }
      for (const id of input.artifactIds) {
        const evidence = await tx.get(`SELECT a.id FROM hv_artifacts a WHERE a.agent_id=? AND a.id=? AND ${artifactVisibleSql}`, actor.agentId, id, now, now);
        if (!evidence) throw new AgentError("Artifact unavailable, foreign, or expired", 404);
      }
      // Linking retains existing bytes; never creates an alternative quota pool.
      if (input.artifactIds.length) {
        const usage = await tx.get<{ bytes: number; total: number }>("SELECT coalesce(sum(size),0) AS bytes,count(*) AS total FROM hv_artifacts WHERE agent_id=?", actor.agentId);
        if ((usage?.bytes ?? 0) > 100 * 1024 * 1024 || (usage?.total ?? 0) > 2000) throw new AgentError("Artifact retention quota exceeded", 409);
      }
      await tx.run("INSERT INTO hv_memory_counters(agent_id,next_number) VALUES (?,1) ON CONFLICT DO NOTHING", actor.agentId);
      const counter = (await tx.get<{ next_number: number }>("SELECT next_number FROM hv_memory_counters WHERE agent_id=?", actor.agentId))!;
      const number = integer(counter.next_number, 1, Number.MAX_SAFE_INTEGER - 1, "next memory number");
      await tx.run("UPDATE hv_memory_counters SET next_number=? WHERE agent_id=?", number + 1, actor.agentId);
      await tx.run("INSERT INTO hv_memories(agent_id,number,kind,topic,instrument,interval,summary,content,payload,created_at,evidence_as_of,valid_until,producer_subagent_id,source_run_id,conversation_id,tool_task_id) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)", actor.agentId, number, kind, input.topic, input.instrument, input.interval, input.summary, input.content, JSON.stringify(input.payload), now, input.kind === "research" ? input.evidenceAsOf : null, deadline, actor.subagentId, actor.runId, actor.conversationId, actor.toolTaskId);
      for (const link of links) await tx.run("INSERT INTO hv_memory_links(agent_id,source_number,target_number,relation) VALUES (?,?,?,?)", actor.agentId, number, parseMemoryRef(link.ref), link.relation);
      for (const id of input.artifactIds) await tx.run("INSERT INTO hv_memory_artifacts(agent_id,memory_number,artifact_id) VALUES (?,?,?)", actor.agentId, number, id);
      return saveOperation(tx, actor, key, operation, hash, { ref: memoryRef(number), kind, createdAt: now, validUntil: deadline }, now);
    });
  }
  private async row(tx: SqliteExecutor, agentId: string, number: number): Promise<Row> {
    const row = await tx.get<Row>(`SELECT *,${statusSql(this.clock())} AS status FROM hv_memories WHERE agent_id=? AND number=?`, agentId, number);
    if (!row) throw new AgentError("Memory not found in this Agent", 404);
    return row;
  }
  async get(agentId: string, ref: string): Promise<MemoryDetail> {
    const number = parseMemoryRef(ref);
    return this.db.transaction(async (tx) => {
      const row = await this.row(tx, agentId, number);
      const links = await tx.all<{ target_number: number; relation: MemoryDetail["links"][number]["relation"] }>("SELECT target_number,relation FROM hv_memory_links WHERE agent_id=? AND source_number=? ORDER BY target_number,relation", agentId, number);
      const artifacts = await tx.all<{ artifact_id: string }>("SELECT artifact_id FROM hv_memory_artifacts WHERE agent_id=? AND memory_number=? ORDER BY artifact_id", agentId, number);
      return { ...metadata(row), content: row.content, payload: JSON.parse(row.payload) as MemoryDetail["payload"], links: links.map((link) => ({ ref: memoryRef(link.target_number), relation: link.relation })), artifactIds: artifacts.map((item) => item.artifact_id), conversationId: row.conversation_id, toolTaskId: row.tool_task_id };
    });
  }
  async list(agentId: string, value: unknown = {}) {
    const filter = fields(value, ["kind", "instrument", "topic", "interval", "producerSubagentId", "sourceRunId", "query", "status", "start", "end", "limit", "cursor"]);
    const kind = filter.kind === undefined ? "" : choice(filter.kind, memoryKinds);
    const coin = filter.instrument === undefined ? "" : instrument(filter.instrument);
    const stream = filter.topic === undefined ? "" : topic(filter.topic);
    const width = filter.interval === undefined ? "" : interval(filter.interval);
    const producer = filter.producerSubagentId === undefined ? "" : text(filter.producerSubagentId, 100, "producer ID");
    const run = filter.sourceRunId === undefined ? "" : text(filter.sourceRunId, 100, "run ID");
    const query = filter.query === undefined ? "" : text(filter.query, 200, "search", true);
    const status = filter.status === undefined ? "" : choice(filter.status, ["durable", "missing", "fresh", "stale", "disabled", "retracted", "withdrawn", "incomplete", "invalid_evidence"]);
    const start = filter.start === undefined ? 0 : timestamp(filter.start), end = filter.end === undefined ? Number.MAX_SAFE_INTEGER : timestamp(filter.end);
    if (end <= start) throw new AgentError("Invalid memory time window");
    const limit = filter.limit === undefined ? 25 : integer(filter.limit, 1, 50, "page size");
    const cursor = filter.cursor == null ? null : fields(filter.cursor, ["createdAt", "ref"]);
    const time = cursor ? timestamp(cursor.createdAt) : Number.MAX_SAFE_INTEGER;
    const number = cursor ? parseMemoryRef(text(cursor.ref, 32, "cursor reference")) : Number.MAX_SAFE_INTEGER;
    const rows = await this.db.all<Row>(`SELECT ${columns},${statusSql(this.clock())} AS status FROM hv_memories WHERE agent_id=? AND created_at>=? AND created_at<? AND (?='' OR status=?) AND (?='' OR kind=?) AND (?='' OR instrument=?) AND (?='' OR topic=?) AND (?='' OR interval=?) AND (?='' OR producer_subagent_id=?) AND (?='' OR source_run_id=?) AND (?='' OR instr(lower(summary),lower(?))>0) AND (created_at<? OR (created_at=? AND number<?)) ORDER BY created_at DESC,number DESC LIMIT ?`, agentId, start, end, status, status, kind, kind, coin, coin, stream, stream, width, width, producer, producer, run, run, query, query, time, time, number, limit + 1);
    const items = rows.slice(0, limit).map(metadata), last = items.at(-1);
    return { items, nextCursor: rows.length > limit && last ? { createdAt: last.createdAt, ref: last.ref } : null };
  }
  /** Advisory current-research retrieval for caller-supplied stream requirements. */
  tradingContext(agentId: string, coin: string, value: unknown) {
    return this.db.transaction((tx) => this.tradingContextTx(tx, agentId, coin, value));
  }
  async tradingContextTx(tx: SqliteExecutor, agentId: string, coin: string, value: unknown) {
    instrument(coin);
    const requirements = researchRequirements(value);
    const now = timestamp(this.clock());
    const items = [];
    for (const requirement of requirements) {
      const producer = await tx.get<{ enabled: number; archived: number }>("SELECT enabled,archived FROM hv_subagents WHERE agent_id=? AND id=? AND kind='worker'", agentId, requirement.subagentId);
      // Select the latest BEFORE evaluating status: no resurrection of old setups.
      const row = await tx.get<Row>("SELECT * FROM hv_memories WHERE agent_id=? AND kind='research' AND instrument=? AND topic=? AND interval=? AND producer_subagent_id=? ORDER BY number DESC LIMIT 1", agentId, coin, requirement.topic, requirement.interval, requirement.subagentId);
      const payload = row ? JSON.parse(row.payload) as ResearchPayload : null;
      const deadline = row ? Math.min(row.valid_until!, row.evidence_as_of! + requirement.maxAgeSeconds * 1000) : null;
      let status: ResearchStatus = "fresh";
      if (!producer || !producer.enabled || producer.archived) status = "disabled";
      else if (!row) status = "missing";
      else if (row.retracted_at !== null) status = "retracted";
      else if (row.evidence_as_of! > now || row.evidence_as_of! % (candleIntervals[interval(requirement.interval)] * 1000) !== 0) status = "invalid_evidence";
      else if (payload!.setup === "withdrawn") status = "withdrawn";
      else if (payload!.coverage !== "complete") status = "incomplete";
      else if (now >= deadline!) status = "stale";
      const eligible = status === "fresh" && payload?.setup === "ready";
      items.push({ producerSubagentId: requirement.subagentId, topic: requirement.topic, interval: requirement.interval, ref: row ? memoryRef(row.number) : null, status, evidenceAsOf: row?.evidence_as_of ?? null, validUntil: deadline, summary: status === "fresh" ? row!.summary : null, payload: status === "fresh" ? payload : null, eligible });
    }
    // Character counts are not token budgets. Never silently omit requested
    // evidence and then report a successful freshness picture.
    const overBudget = new TextEncoder().encode(JSON.stringify(items)).length > 64 * 1024;
    const bounded = overBudget ? items.map((item) => ({ ...item, summary: null, payload: null, eligible: false })) : items;
    const allReady = !overBudget && bounded.length > 0 && bounded.every((item) => item.eligible);
    return { observedAt: now, instrument: coin, overBudget, allReady, requirements: bounded };
  }
}