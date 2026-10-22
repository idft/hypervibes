import { blob, integer, primaryKey, sqliteTable, text, uniqueIndex } from "drizzle-orm/sqlite-core";
import { hvAgents } from "./agent-schema.js";

/** The gateway kinds the application understands; each is owned by its connector module. */
export const GATEWAY_TYPES = ["telegram"] as const;
export type GatewayType = (typeof GATEWAY_TYPES)[number];

/** One external gateway per Agent and type: bridges a bound chat with the Agent's persistent main conversation. */
export const hvGateways = sqliteTable("hv_gateways", {
  agentId: text("agent_id").notNull().references(() => hvAgents.id, { onDelete: "cascade" }),
  type: text("type", { enum: GATEWAY_TYPES }).notNull(),
  /** AES-256-GCM sealed bot token; never rendered, logged, or returned to clients. */
  botTokenCiphertext: blob("bot_token_ciphertext", { mode: "buffer" }).notNull(),
  /** SHA-256 of the plaintext token; a unique index stops two Agents sharing one bot. */
  botTokenHash: text("bot_token_hash").notNull(),
  /** Encryption key id that sealed the token, for future key rotation. */
  keyId: text("key_id").notNull(),
  /** The bot's own username on the gateway platform. */
  botUsername: text("bot_username").notNull(),
  /** The external chat that completed the link flow; null until linked. */
  chatId: integer("chat_id"),
  chatUsername: text("chat_username"),
  updatedAt: integer("updated_at").notNull(),
}, (table) => [
  primaryKey({ columns: [table.agentId, table.type] }),
  uniqueIndex("hv_gateways_token_hash_idx").on(table.botTokenHash),
]);

export type GatewayRow = typeof hvGateways.$inferSelect;