import { sql } from "drizzle-orm";
import { check, index, integer, sqliteTable, text, uniqueIndex } from "drizzle-orm/sqlite-core";
import { hvAgents } from "./agent-schema.js";

// Public application storage, never Pi's private tables. Request identity and
// prepared wire payloads are immutable. Amounts remain strings inside JSON TEXT.
export const hvOrderRequests = sqliteTable("hv_order_requests", {
  id: text("id").primaryKey(), agentId: text("agent_id").notNull().references(() => hvAgents.id),
  account: text("account").notNull(), requestKey: text("request_key").notNull(), kind: text("kind").notNull(),
  input: text("input").notNull(), createdAt: integer("created_at").notNull(),
}, (t) => [uniqueIndex("hv_order_request_key").on(t.agentId, t.requestKey), index("hv_order_requests_agent").on(t.agentId, t.createdAt),
  check("hv_order_request_kind", sql`${t.kind} IN ('submit', 'cancel', 'cancel_all')`)]);
export const hvOrderActions = sqliteTable("hv_order_actions", {
  id: text("id").primaryKey(), requestId: text("request_id").notNull().references(() => hvOrderRequests.id),
  ordinal: integer("ordinal").notNull(), payload: text("payload").notNull(),
  state: text("state").notNull(), nonce: integer("nonce"), expiresAt: integer("expires_at").notNull(),
  updatedAt: integer("updated_at").notNull(),
}, (t) => [uniqueIndex("hv_order_action_ordinal").on(t.requestId, t.ordinal), index("hv_order_actions_state").on(t.state, t.updatedAt),
  check("hv_order_action_state", sql`${t.state} IN ('prepared', 'submitting', 'unknown', 'done', 'not_sent')`)]);
export const hvOrderItems = sqliteTable("hv_order_items", {
  id: text("id").primaryKey(), actionId: text("action_id").notNull().references(() => hvOrderActions.id),
  ordinal: integer("ordinal").notNull(), symbol: text("symbol").notNull(), cloid: text("cloid"), oid: text("oid"),
  active: integer("active").notNull(), view: text("view").notNull(), updatedAt: integer("updated_at").notNull(),
}, (t) => [uniqueIndex("hv_order_item_ordinal").on(t.actionId, t.ordinal), uniqueIndex("hv_order_item_cloid").on(t.cloid),
  index("hv_order_items_active").on(t.active, t.updatedAt), index("hv_order_items_oid").on(t.oid),
  check("hv_order_item_active", sql`${t.active} IN (0, 1)`)]);
export const hvOrderEvents = sqliteTable("hv_order_events", {
  id: integer("id").primaryKey({ autoIncrement: true }), itemId: text("item_id").notNull().references(() => hvOrderItems.id),
  source: text("source").notNull(), time: integer("time").notNull(), data: text("data").notNull(),
}, (t) => [index("hv_order_events_item").on(t.itemId, t.id)]);
export const hvExchangeNonces = sqliteTable("hv_exchange_nonces", {
  signer: text("signer").primaryKey(), nonce: integer("nonce").notNull(),
});
