import { sql } from "drizzle-orm";
import { check, index, integer, sqliteTable, text, uniqueIndex } from "drizzle-orm/sqlite-core";
import { hvUsers } from "./auth-schema.js";

export const hvAgents = sqliteTable("hv_agents", {
  id: text("id").primaryKey(),
  userId: text("user_id").notNull().references(() => hvUsers.id, { onDelete: "restrict" }),
  name: text("name").notNull(),
  creationRequestId: text("creation_request_id").notNull(),
  state: text("state", { enum: ["provisioning", "ready", "failed"] }).notNull().default("provisioning"),
  conversationId: integer("conversation_id"),
  error: text("error"),
  createdAt: integer("created_at").notNull(),
  updatedAt: integer("updated_at").notNull(),
}, (table) => [
  index("hv_agents_owner_idx").on(table.userId),
  uniqueIndex("hv_agents_creation_request_idx").on(table.userId, table.creationRequestId),
  check("hv_agents_name_check", sql`length(trim(${table.name})) BETWEEN 1 AND 80`),
  check("hv_agents_state_check", sql`${table.state} IN ('provisioning', 'ready', 'failed')`),
]);

export type AgentRow = typeof hvAgents.$inferSelect;
