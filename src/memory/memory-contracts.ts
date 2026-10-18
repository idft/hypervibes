import { AgentError } from "../agents/agent-input.js";
import { candleIntervals } from "../agents/candle-intervals.js";
import { array, choice, fields, integer, text, unique } from "./validation.js";

export const memoryToolNames = ["publish_research", "record_trade_decision", "publish_review", "list_memories", "get_memory", "get_trading_context", "get_learnings", "append_learning", "replace_learnings", "research_decisions"] as const;
export const memoryKinds = ["research", "decision", "review"] as const;
export type MemoryKind = typeof memoryKinds[number];
export const relations = ["based_on", "corrects", "supersedes", "reviews"] as const;
export type MemoryRelation = typeof relations[number];
export interface FillAttribution { fillId: string; time: number; exchangeOid: string | null; requestId: string | null; decisionRef: string | null; origin: string }
export interface MemoryLink { ref: string; relation: MemoryRelation }
export interface ResearchPayload {
  version: 1; bias: "long" | "short" | "neutral"; setup: "ready" | "watch" | "none" | "withdrawn";
  observations: string[]; interpretation: string; invalidation: string[]; coverage: "complete" | "incomplete" | "unknown";
}
export interface DecisionPayload {
  version: 1; action: "enter" | "hold" | "skip" | "reduce" | "close" | "protect";
  direction: "long" | "short" | null; reason: string; researchRefs: string[]; plan: string;
}
export interface ReviewPayload {
  version: 1; start: number; end: number; coverage: "complete" | "incomplete" | "unknown";
  limitations: string[]; findings: string[]; recommendations: string[]; memoryRefs: string[];
}
export interface PublicationBase {
  topic: string; instrument: string | null; interval: string | null; summary: string; content: string;
  artifactIds: string[]; links: MemoryLink[];
}
export type PublicationInput = PublicationBase & (
  | { kind: "research"; evidenceAsOf: number; requestedValidUntil: number | null; payload: ResearchPayload }
  | { kind: "decision"; payload: DecisionPayload }
  | { kind: "review"; payload: ReviewPayload }
);
export function memoryRef(number: number): string {
  integer(number, 1, Number.MAX_SAFE_INTEGER, "memory number");
  return `m${number}`;
}
export function parseMemoryRef(value: string): number {
  if (typeof value !== "string" || !/^m[1-9][0-9]*$/.test(value) || value.trim() !== value) throw new AgentError("Use an exact memory reference such as m42");
  return integer(Number(value.slice(1)), 1, Number.MAX_SAFE_INTEGER, "memory reference");
}
export function reference(value: unknown): string {
  if (typeof value !== "string") throw new AgentError("Use an exact memory reference such as m42");
  parseMemoryRef(value); return value;
}
export function topic(value: unknown): string {
  if (typeof value !== "string" || !/^[a-z][a-z0-9_]{0,63}$/.test(value) || value.trim() !== value) throw new AgentError("Use a lowercase topic such as trend_setup");
  return value;
}
/** Syntax only; the host must also check the current main-DEX instrument catalog. */
export function instrument(value: unknown): string {
  if (typeof value !== "string" || !/^[A-Za-z0-9][A-Za-z0-9_-]{0,31}$/.test(value) || value.trim() !== value) throw new AgentError("Use an exact main-DEX perpetual symbol");
  return value;
}
export function interval(value: unknown): keyof typeof candleIntervals {
  if (typeof value !== "string" || !Object.hasOwn(candleIntervals, value)) throw new AgentError("Unsupported analytical interval");
  return value as keyof typeof candleIntervals;
}
export const timestamp = (value: unknown) => integer(value, 0, Number.MAX_SAFE_INTEGER, "UTC epoch milliseconds");
const strings = (value: unknown) => array(value, 20, (item) => text(item, 1000, "payload text"));
const refs = (value: unknown) => unique(array(value, 20, reference)).sort();
export function publicationInput(kind: MemoryKind, value: unknown): PublicationInput {
  const row = fields(value, ["topic", "instrument", "interval", "summary", "content", "artifactIds", "links", "payload", ...(kind === "research" ? ["evidenceAsOf", "requestedValidUntil"] : [])]);
  const base: PublicationBase = {
    topic: topic(row.topic), instrument: row.instrument == null ? null : instrument(row.instrument), interval: row.interval == null ? null : interval(row.interval),
    summary: text(row.summary, 500, "summary"), content: text(row.content, 16000, "content"),
    artifactIds: unique(array(row.artifactIds ?? [], 20, (id) => {
      if (typeof id !== "string" || !/^[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$/.test(id) || id.trim() !== id) throw new AgentError("Invalid artifact ID");
      return id;
    })).sort(),
    links: array(row.links ?? [], 20, (item) => { const link = fields(item, ["ref", "relation"]); return { ref: reference(link.ref), relation: choice(link.relation, relations) }; }),
  };
  unique(base.links.map((link) => `${link.relation}:${link.ref}`));
  base.links.sort((a, b) => parseMemoryRef(a.ref) - parseMemoryRef(b.ref) || a.relation.localeCompare(b.relation));
  const p = row.payload;
  let result: PublicationInput;
  if (kind === "research") {
    const payload = fields(p, ["version", "bias", "setup", "observations", "interpretation", "invalidation", "coverage"]);
    integer(payload.version, 1, 1, "payload version");
    if (!base.instrument || !base.interval) throw new AgentError("Research requires an instrument and interval");
    result = { ...base, kind, evidenceAsOf: timestamp(row.evidenceAsOf), requestedValidUntil: row.requestedValidUntil == null ? null : timestamp(row.requestedValidUntil), payload: {
      version: 1, bias: choice(payload.bias, ["long", "short", "neutral"]), setup: choice(payload.setup, ["ready", "watch", "none", "withdrawn"]),
      observations: strings(payload.observations), interpretation: text(payload.interpretation, 4000, "interpretation"), invalidation: strings(payload.invalidation), coverage: choice(payload.coverage, ["complete", "incomplete", "unknown"]),
    } };
  } else if (kind === "decision") {
    const payload = fields(p, ["version", "action", "direction", "reason", "researchRefs", "plan"]);
    integer(payload.version, 1, 1, "payload version");
    const action = choice(payload.action, ["enter", "hold", "skip", "reduce", "close", "protect"]);
    const direction = payload.direction == null ? null : choice(payload.direction, ["long", "short"]);
    if (action === "enter" && (!base.instrument || !direction || !payload.plan)) throw new AgentError("Entry decisions require instrument, direction, and protection plan");
    result = { ...base, kind, payload: { version: 1, action, direction, reason: text(payload.reason, 4000, "reason"), researchRefs: refs(payload.researchRefs), plan: text(payload.plan, 4000, "plan", action !== "enter") } };
  } else {
    const payload = fields(p, ["version", "start", "end", "coverage", "limitations", "findings", "recommendations", "memoryRefs"]);
    integer(payload.version, 1, 1, "payload version");
    const start = timestamp(payload.start), end = timestamp(payload.end);
    if (end <= start || end - start > 90 * 86400_000) throw new AgentError("Review window must be positive and at most 90 days");
    result = { ...base, kind, payload: { version: 1, start, end, coverage: choice(payload.coverage, ["complete", "incomplete", "unknown"]), limitations: strings(payload.limitations), findings: strings(payload.findings), recommendations: strings(payload.recommendations), memoryRefs: refs(payload.memoryRefs) } };
  }
  if (new TextEncoder().encode(JSON.stringify(result.payload)).length > 16 * 1024) throw new AgentError("Structured payload exceeds 16 KiB");
  return result;
}
export interface ResearchRequirement { subagentId: string; topic: string; interval: string; maxAgeSeconds: number }
export interface MemoryMetadata {
  ref: string; status: string; kind: MemoryKind; topic: string; instrument: string | null; interval: string | null; summary: string;
  createdAt: number; evidenceAsOf: number | null; validUntil: number | null; producerSubagentId: string | null; sourceRunId: string | null;
  retractedAt: number | null; retractionReason: string | null;
}
export interface MemoryDetail extends MemoryMetadata {
  content: string; payload: ResearchPayload | DecisionPayload | ReviewPayload; links: MemoryLink[]; artifactIds: string[];
  generationTaskId: number | null; inputScopeKey: string | null; conversationId: number | null; toolTaskId: number | null;
  executionOrigin: "automatic" | "user_directed";
}
export type ResearchStatus = "missing" | "fresh" | "stale" | "disabled" | "retracted" | "withdrawn" | "incomplete" | "invalid_evidence";
