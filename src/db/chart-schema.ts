import { index, integer, primaryKey, sqliteTable, text } from "drizzle-orm/sqlite-core";
import { hvAgents } from "./agent-schema.js";

export const hvChartLayouts = sqliteTable("hv_chart_layouts", {
  id: text("id").primaryKey(), agentId: text("agent_id").notNull().references(() => hvAgents.id, { onDelete: "cascade" }),
  document: text("document").notNull(), revision: integer("revision").notNull().default(0),
  history: text("history").notNull(), updatedAt: integer("updated_at").notNull(),
}, (table) => [index("hv_chart_layouts_agent_idx").on(table.agentId, table.updatedAt)]);

export const hvChartOperations = sqliteTable("hv_chart_operations", {
  agentId: text("agent_id").notNull().references(() => hvAgents.id, { onDelete: "cascade" }),
  key: text("key").notNull(), hash: text("hash").notNull(), result: text("result").notNull(),
}, (table) => [primaryKey({ columns: [table.agentId, table.key] })]);

export const hvChartSnapshots = sqliteTable("hv_chart_snapshots", {
  id: text("id").notNull(), agentId: text("agent_id").notNull().references(() => hvAgents.id, { onDelete: "cascade" }),
  requestHash: text("request_hash").notNull(), context: text("context").notNull(), image: text("image"), createdAt: integer("created_at").notNull(),
}, (table) => [primaryKey({ columns: [table.agentId, table.id] }), index("hv_chart_snapshots_agent_idx").on(table.agentId, table.createdAt)]);
