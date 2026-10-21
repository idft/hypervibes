import { index, integer, sqliteTable, text, blob, uniqueIndex } from "drizzle-orm/sqlite-core";
import { hvAgents } from "./agent-schema.js";

export const hvArtifacts = sqliteTable("hv_artifacts", {
  id: text("id").primaryKey(), agentId: text("agent_id").notNull().references(() => hvAgents.id, { onDelete: "cascade" }),
  title: text("title").notNull(), description: text("description").notNull(), mediaType: text("media_type").notNull(),
  payload: blob("payload", { mode: "buffer" }).notNull(), size: integer("size").notNull(),
  tags: text("tags").notNull(), instrument: text("instrument").notNull(), interval: text("interval").notNull(),
  conversationId: integer("conversation_id"), taskId: integer("task_id"), createdAt: integer("created_at").notNull(),
  expiresAt: integer("expires_at"),
}, (table) => [index("hv_artifacts_agent_idx").on(table.agentId, table.createdAt), uniqueIndex("hv_artifacts_identity_idx").on(table.agentId, table.id)]);
// Small replay receipts survive payload expiry/deletion. They deliberately hold
// no report content, preventing a delayed retry from resurrecting deleted data.
export const hvArtifactReceipts = sqliteTable("hv_artifact_receipts", {
  agentId: text("agent_id").notNull().references(() => hvAgents.id, { onDelete: "cascade" }), operationKey: text("operation_key").notNull(),
  requestHash: text("request_hash").notNull(), artifactId: text("artifact_id").notNull(),
}, (table) => [uniqueIndex("hv_artifact_receipts_operation_idx").on(table.agentId, table.operationKey)]);
export const hvArtifactLinks = sqliteTable("hv_artifact_links", {
  parent: text("parent").notNull().references(() => hvArtifacts.id, { onDelete: "cascade" }),
  child: text("child").notNull().references(() => hvArtifacts.id, { onDelete: "cascade" }),
}, (table) => [uniqueIndex("hv_artifact_links_pair_idx").on(table.parent, table.child), index("hv_artifact_links_child_idx").on(table.child)]);
