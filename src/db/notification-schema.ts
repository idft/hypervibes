import { index, integer, sqliteTable, text, uniqueIndex } from "drizzle-orm/sqlite-core";
import { hvAgents } from "./agent-schema.js";

export const hvNotificationSettings = sqliteTable("hv_notification_settings", {
  agentId: text("agent_id").primaryKey().references(() => hvAgents.id, { onDelete: "cascade" }),
  enabled: integer("enabled", { mode: "boolean" }).notNull().default(true),
  submissions: integer("submissions", { mode: "boolean" }).notNull().default(true),
  fills: integer("fills", { mode: "boolean" }).notNull().default(true),
  enabledSince: integer("enabled_since").notNull(),
  fillsSince: integer("fills_since").notNull(),
});

export const hvNotifications = sqliteTable("hv_notifications", {
  id: text("id").primaryKey(),
  agentId: text("agent_id").notNull().references(() => hvAgents.id, { onDelete: "cascade" }),
  eventKey: text("event_key").notNull(),
  category: text("category", { enum: ["message", "submission", "fill"] }).notNull(),
  title: text("title").notNull(),
  body: text("body").notNull(),
  severity: text("severity", { enum: ["info", "warning", "error"] }).notNull(),
  source: text("source").notNull(),
  subagentId: text("subagent_id"),
  runId: text("run_id"),
  conversationId: integer("conversation_id"),
  orderRequestId: text("order_request_id"),
  createdAt: integer("created_at").notNull(),
  delivery: text("delivery", { enum: ["web_only", "pending", "sent", "failed", "suppressed"] }).notNull(),
  chatId: integer("chat_id"),
  botTokenHash: text("bot_token_hash"),
  messageIds: text("message_ids", { mode: "json" }).$type<number[]>().notNull().default([]),
  attempts: integer("attempts").notNull().default(0),
  nextAttemptAt: integer("next_attempt_at").notNull(),
  sentAt: integer("sent_at"),
  error: text("error"),
}, (table) => [
  uniqueIndex("hv_notifications_event_idx").on(table.agentId, table.eventKey),
  index("hv_notifications_history_idx").on(table.agentId, table.createdAt, table.id),
  index("hv_notifications_delivery_idx").on(table.delivery, table.nextAttemptAt),
]);

/** Receipts include disabled tool calls so replay cannot send them after re-enabling. */
export const hvNotificationOperations = sqliteTable("hv_notification_operations", {
  agentId: text("agent_id").notNull().references(() => hvAgents.id, { onDelete: "cascade" }),
  operationKey: text("operation_key").notNull(),
  requestHash: text("request_hash").notNull(),
  result: text("result").notNull(),
}, (table) => [uniqueIndex("hv_notification_operations_key_idx").on(table.agentId, table.operationKey)]);
