import { sql } from "drizzle-orm";
import { check, foreignKey, index, integer, primaryKey, sqliteTable, text } from "drizzle-orm/sqlite-core";
import { hvAgents } from "./agent-schema.js";
import { hvArtifacts } from "./artifact-schema.js";
import { hvSubagents, hvSubagentRuns } from "./subagent-schema.js";
import type { PublicationInput } from "../memory/memory-contracts.js";

export const hvMemoryCounters = sqliteTable("hv_memory_counters", {
  agentId: text("agent_id").primaryKey().references(() => hvAgents.id, { onDelete: "restrict" }),
  nextNumber: integer("next_number").notNull().default(1),
}, (t) => [check("hv_memory_counter_positive", sql`${t.nextNumber} BETWEEN 1 AND 9007199254740991`)]);
export const hvMemories = sqliteTable("hv_memories", {
  agentId: text("agent_id").notNull().references(() => hvAgents.id, { onDelete: "restrict" }), number: integer("number").notNull(),
  kind: text("kind", { enum: ["research", "decision", "review"] }).notNull(), topic: text("topic").notNull(), instrument: text("instrument"), interval: text("interval"),
  summary: text("summary").notNull(), content: text("content").notNull(), payload: text("payload", { mode: "json" }).$type<PublicationInput["payload"]>().notNull(),
  createdAt: integer("created_at").notNull(), evidenceAsOf: integer("evidence_as_of"), validUntil: integer("valid_until"),
  producerSubagentId: text("producer_subagent_id"), sourceRunId: text("source_run_id"), sourceGenerationTaskId: integer("source_generation_task_id"), sourceInputScopeKey: text("source_input_scope_key"),
  conversationId: integer("conversation_id"), toolTaskId: integer("tool_task_id"), executionOrigin: text("execution_origin", { enum: ["automatic", "user_directed"] }).notNull(),
  retractedAt: integer("retracted_at"), retractionReason: text("retraction_reason"),
}, (t) => [
  primaryKey({ columns: [t.agentId, t.number] }),
  foreignKey({ columns: [t.agentId, t.producerSubagentId], foreignColumns: [hvSubagents.agentId, hvSubagents.id] }),
  foreignKey({ columns: [t.agentId, t.sourceRunId, t.producerSubagentId], foreignColumns: [hvSubagentRuns.agentId, hvSubagentRuns.id, hvSubagentRuns.subagentId] }),
  index("hv_memories_history_idx").on(t.agentId, t.createdAt, t.number),
  index("hv_memories_stream_idx").on(t.agentId, t.kind, t.instrument, t.topic, t.interval, t.producerSubagentId, t.number),
  index("hv_memories_run_idx").on(t.agentId, t.sourceRunId),
  check("hv_memory_number_positive", sql`${t.number} BETWEEN 1 AND 9007199254740990`),
  check("hv_memory_kind", sql`${t.kind} IN ('research','decision','review')`),
  check("hv_memory_text_bounds", sql`length(${t.summary}) BETWEEN 1 AND 500 AND length(${t.content}) BETWEEN 1 AND 16000 AND length(CAST(${t.payload} AS BLOB)) <= 16384 AND json_valid(${t.payload})`),
  check("hv_memory_research_deadline", sql`${t.kind} != 'research' OR (${t.evidenceAsOf} IS NOT NULL AND ${t.validUntil} IS NOT NULL AND ${t.validUntil} > ${t.evidenceAsOf} AND ${t.instrument} IS NOT NULL AND ${t.interval} IS NOT NULL)`),
  check("hv_memory_run_producer", sql`${t.sourceRunId} IS NULL OR ${t.producerSubagentId} IS NOT NULL`),
  check("hv_memory_retraction", sql`(${t.retractedAt} IS NULL) = (${t.retractionReason} IS NULL)`),
]);
export const hvMemoryLinks = sqliteTable("hv_memory_links", {
  agentId: text("agent_id").notNull(), sourceNumber: integer("source_number").notNull(), targetNumber: integer("target_number").notNull(), relation: text("relation").notNull(),
}, (t) => [
  primaryKey({ columns: [t.agentId, t.sourceNumber, t.targetNumber, t.relation] }),
  foreignKey({ columns: [t.agentId, t.sourceNumber], foreignColumns: [hvMemories.agentId, hvMemories.number] }),
  foreignKey({ columns: [t.agentId, t.targetNumber], foreignColumns: [hvMemories.agentId, hvMemories.number] }),
  check("hv_memory_link_backward", sql`${t.targetNumber} < ${t.sourceNumber}`),
  check("hv_memory_link_relation", sql`${t.relation} IN ('based_on','corrects','supersedes','reviews')`),
]);
export const hvMemoryArtifacts = sqliteTable("hv_memory_artifacts", {
  agentId: text("agent_id").notNull(), memoryNumber: integer("memory_number").notNull(), artifactId: text("artifact_id").notNull(),
}, (t) => [
  primaryKey({ columns: [t.agentId, t.memoryNumber, t.artifactId] }),
  foreignKey({ columns: [t.agentId, t.memoryNumber], foreignColumns: [hvMemories.agentId, hvMemories.number] }),
  foreignKey({ columns: [t.agentId, t.artifactId], foreignColumns: [hvArtifacts.agentId, hvArtifacts.id] }),
  index("hv_memory_artifacts_evidence_idx").on(t.agentId, t.artifactId),
]);
export const hvMemoryOperations = sqliteTable("hv_memory_operations", {
  agentId: text("agent_id").notNull().references(() => hvAgents.id), operationKey: text("operation_key").notNull(),
  operationKind: text("operation_kind").notNull(), requestHash: text("request_hash").notNull(), result: text("result").notNull(), createdAt: integer("created_at").notNull(),
}, (t) => [primaryKey({ columns: [t.agentId, t.operationKey] })]);
