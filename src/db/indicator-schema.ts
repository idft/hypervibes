import { sql } from "drizzle-orm";
import { check, index, integer, primaryKey, sqliteTable, text } from "drizzle-orm/sqlite-core";
import { hvAgents } from "./agent-schema.js";

export const hvIndicators = sqliteTable("hv_indicators", {
  id: text("id").primaryKey(), agentId: text("agent_id").notNull().references(() => hvAgents.id, { onDelete: "cascade" }),
  name: text("name").notNull(), description: text("description").notNull(), interpretation: text("interpretation").notNull(),
  currentVersion: integer("current_version").notNull(), createdAt: integer("created_at").notNull(), updatedAt: integer("updated_at").notNull(),
}, (t) => [index("hv_indicators_agent_idx").on(t.agentId, t.updatedAt), check("hv_indicators_version_positive", sql`${t.currentVersion} > 0`)]);
export const hvIndicatorVersions = sqliteTable("hv_indicator_versions", {
  indicatorId: text("indicator_id").notNull().references(() => hvIndicators.id, { onDelete: "cascade" }), version: integer("version").notNull(),
  source: text("source").notNull(), sourceHash: text("source_hash").notNull(), metadata: text("metadata").notNull(),
  engineVersion: text("engine_version").notNull(), wrapperVersion: integer("wrapper_version").notNull(),
  createdAt: integer("created_at").notNull(), actorKind: text("actor_kind", { enum: ["user", "agent"] }).notNull(),
  userId: text("user_id"), subagentId: text("subagent_id"), runId: text("run_id"), conversationId: text("conversation_id"), taskId: text("task_id"),
}, (t) => [primaryKey({ columns: [t.indicatorId, t.version] }), check("hv_indicator_versions_positive", sql`${t.version} > 0`)]);
export const hvIndicatorOperations = sqliteTable("hv_indicator_operations", {
  agentId: text("agent_id").notNull().references(() => hvAgents.id, { onDelete: "cascade" }),
  key: text("key").notNull(), hash: text("hash").notNull(), result: text("result").notNull(),
}, (t) => [primaryKey({ columns: [t.agentId, t.key] })]);
