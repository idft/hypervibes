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
  /** Creation intent only; current selection is owned by Durable's pi.agent. */
  initialProvider: text("initial_provider"),
  initialModelId: text("initial_model_id"),
  error: text("error"),
  /** Linked Hyperliquid main account or sub-account; null for chat-only agents. */
  tradingAccountAddress: text("trading_account_address"),
  createdAt: integer("created_at").notNull(),
  updatedAt: integer("updated_at").notNull(),
}, (table) => [
  index("hv_agents_owner_idx").on(table.userId),
  uniqueIndex("hv_agents_creation_request_idx").on(table.userId, table.creationRequestId),
  uniqueIndex("hv_agents_trading_account_idx").on(table.tradingAccountAddress),
  check("hv_agents_name_check", sql`length(trim(${table.name})) BETWEEN 1 AND 80`),
  check("hv_agents_state_check", sql`${table.state} IN ('provisioning', 'ready', 'failed')`),
]);

export type AgentRow = typeof hvAgents.$inferSelect;
