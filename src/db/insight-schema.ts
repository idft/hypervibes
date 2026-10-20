import { sql } from "drizzle-orm";
import { check, foreignKey, integer, primaryKey, sqliteTable, text, uniqueIndex } from "drizzle-orm/sqlite-core";
import { hvAgents } from "./agent-schema.js";
import { hvSubagents, hvSubagentRuns } from "./subagent-schema.js";
import { hvMemories } from "./memory-schema.js";

export const hvInsightSets = sqliteTable("hv_insight_sets", {
  id: text("id").primaryKey(), agentId: text("agent_id").notNull().references(() => hvAgents.id),
  scopeKind: text("scope_kind", { enum: ["agent", "worker"] }).notNull(), subagentId: text("subagent_id"), currentVersion: integer("current_version").notNull().default(0), updatedAt: integer("updated_at").notNull(),
}, (t) => [
  uniqueIndex("hv_insight_sets_identity_idx").on(t.agentId, t.id),
  uniqueIndex("hv_insight_sets_agent_idx").on(t.agentId).where(sql`${t.scopeKind} = 'agent'`),
  uniqueIndex("hv_insight_sets_worker_idx").on(t.agentId, t.subagentId).where(sql`${t.scopeKind} = 'worker'`),
  foreignKey({ columns: [t.agentId, t.subagentId], foreignColumns: [hvSubagents.agentId, hvSubagents.id] }),
  check("hv_insight_scope", sql`(${t.scopeKind} = 'agent' AND ${t.subagentId} IS NULL) OR (${t.scopeKind} = 'worker' AND ${t.subagentId} IS NOT NULL)`),
  check("hv_insight_head", sql`${t.currentVersion} BETWEEN 0 AND 9007199254740990`),
]);
export const hvInsightRevisions = sqliteTable("hv_insight_revisions", {
  agentId: text("agent_id").notNull(), setId: text("set_id").notNull(), version: integer("version").notNull(), content: text("content").notNull(), operation: text("operation", { enum: ["append", "replace"] }).notNull(), reason: text("reason").notNull(), createdAt: integer("created_at").notNull(),
  actorUserId: text("actor_user_id"), producerSubagentId: text("producer_subagent_id"), sourceRunId: text("source_run_id"), conversationId: integer("conversation_id"), toolTaskId: integer("tool_task_id"),
}, (t) => [
  primaryKey({ columns: [t.setId, t.version] }), uniqueIndex("hv_insight_revision_identity_idx").on(t.agentId, t.setId, t.version),
  foreignKey({ columns: [t.agentId, t.setId], foreignColumns: [hvInsightSets.agentId, hvInsightSets.id] }),
  foreignKey({ columns: [t.agentId, t.producerSubagentId], foreignColumns: [hvSubagents.agentId, hvSubagents.id] }),
  foreignKey({ columns: [t.agentId, t.sourceRunId, t.producerSubagentId], foreignColumns: [hvSubagentRuns.agentId, hvSubagentRuns.id, hvSubagentRuns.subagentId] }),
  check("hv_insight_revision_version", sql`${t.version} BETWEEN 1 AND 9007199254740990`),
  check("hv_insight_content_bound", sql`length(${t.content}) <= 8000`),
  check("hv_insight_operation", sql`${t.operation} IN ('append','replace')`),
  check("hv_insight_revision_actor", sql`(${t.actorUserId} IS NOT NULL AND ${t.sourceRunId} IS NULL) OR (${t.actorUserId} IS NULL AND ${t.producerSubagentId} IS NOT NULL)`),
]);
export const hvInsightRevisionMemories = sqliteTable("hv_insight_revision_memories", {
  agentId: text("agent_id").notNull(), setId: text("set_id").notNull(), version: integer("version").notNull(), memoryNumber: integer("memory_number").notNull(),
}, (t) => [
  primaryKey({ columns: [t.agentId, t.setId, t.version, t.memoryNumber] }),
  foreignKey({ columns: [t.agentId, t.setId, t.version], foreignColumns: [hvInsightRevisions.agentId, hvInsightRevisions.setId, hvInsightRevisions.version] }),
  foreignKey({ columns: [t.agentId, t.memoryNumber], foreignColumns: [hvMemories.agentId, hvMemories.number] }),
]);
