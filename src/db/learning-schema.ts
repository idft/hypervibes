import { sql } from "drizzle-orm";
import { check, foreignKey, integer, primaryKey, sqliteTable, text, uniqueIndex } from "drizzle-orm/sqlite-core";
import { hvAgents } from "./agent-schema.js";
import { hvSubagents, hvSubagentRuns } from "./subagent-schema.js";
import { hvMemories } from "./memory-schema.js";

export const hvLearningSets = sqliteTable("hv_learning_sets", {
  id: text("id").primaryKey(), agentId: text("agent_id").notNull().references(() => hvAgents.id),
  scopeKind: text("scope_kind", { enum: ["agent", "worker"] }).notNull(), subagentId: text("subagent_id"), currentVersion: integer("current_version").notNull().default(0), updatedAt: integer("updated_at").notNull(),
}, (t) => [
  uniqueIndex("hv_learning_sets_identity_idx").on(t.agentId, t.id),
  uniqueIndex("hv_learning_sets_agent_idx").on(t.agentId).where(sql`${t.scopeKind} = 'agent'`),
  uniqueIndex("hv_learning_sets_worker_idx").on(t.agentId, t.subagentId).where(sql`${t.scopeKind} = 'worker'`),
  foreignKey({ columns: [t.agentId, t.subagentId], foreignColumns: [hvSubagents.agentId, hvSubagents.id] }),
  check("hv_learning_scope", sql`(${t.scopeKind} = 'agent' AND ${t.subagentId} IS NULL) OR (${t.scopeKind} = 'worker' AND ${t.subagentId} IS NOT NULL)`),
  check("hv_learning_head", sql`${t.currentVersion} BETWEEN 0 AND 9007199254740990`),
]);
export const hvLearningRevisions = sqliteTable("hv_learning_revisions", {
  agentId: text("agent_id").notNull(), setId: text("set_id").notNull(), version: integer("version").notNull(), content: text("content").notNull(), operation: text("operation", { enum: ["append", "replace"] }).notNull(), reason: text("reason").notNull(), createdAt: integer("created_at").notNull(),
  actorUserId: text("actor_user_id"), producerSubagentId: text("producer_subagent_id"), sourceRunId: text("source_run_id"), conversationId: integer("conversation_id"), toolTaskId: integer("tool_task_id"),
}, (t) => [
  primaryKey({ columns: [t.setId, t.version] }), uniqueIndex("hv_learning_revision_identity_idx").on(t.agentId, t.setId, t.version),
  foreignKey({ columns: [t.agentId, t.setId], foreignColumns: [hvLearningSets.agentId, hvLearningSets.id] }),
  foreignKey({ columns: [t.agentId, t.producerSubagentId], foreignColumns: [hvSubagents.agentId, hvSubagents.id] }),
  foreignKey({ columns: [t.agentId, t.sourceRunId, t.producerSubagentId], foreignColumns: [hvSubagentRuns.agentId, hvSubagentRuns.id, hvSubagentRuns.subagentId] }),
  check("hv_learning_revision_version", sql`${t.version} BETWEEN 1 AND 9007199254740990`),
  check("hv_learning_content_bound", sql`length(${t.content}) <= 8000`),
  check("hv_learning_operation", sql`${t.operation} IN ('append','replace')`),
  check("hv_learning_revision_actor", sql`(${t.actorUserId} IS NOT NULL AND ${t.sourceRunId} IS NULL) OR (${t.actorUserId} IS NULL AND ${t.producerSubagentId} IS NOT NULL)`),
]);
export const hvLearningRevisionMemories = sqliteTable("hv_learning_revision_memories", {
  agentId: text("agent_id").notNull(), setId: text("set_id").notNull(), version: integer("version").notNull(), memoryNumber: integer("memory_number").notNull(),
}, (t) => [
  primaryKey({ columns: [t.agentId, t.setId, t.version, t.memoryNumber] }),
  foreignKey({ columns: [t.agentId, t.setId, t.version], foreignColumns: [hvLearningRevisions.agentId, hvLearningRevisions.setId, hvLearningRevisions.version] }),
  foreignKey({ columns: [t.agentId, t.memoryNumber], foreignColumns: [hvMemories.agentId, hvMemories.number] }),
]);
