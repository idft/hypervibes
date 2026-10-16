import { blob, integer, primaryKey, sqliteTable, text } from "drizzle-orm/sqlite-core";
import { hvUsers } from "./auth-schema.js";

export const hvProviderConnections = sqliteTable("hv_provider_connections", {
  userId: text("user_id").notNull().references(() => hvUsers.id, { onDelete: "cascade" }),
  providerId: text("provider_id").notNull(),
  authType: text("auth_type", { enum: ["api_key", "oauth"] }).notNull(),
  ciphertext: blob("ciphertext", { mode: "buffer" }).notNull(),
  keyId: text("key_id").notNull(),
  catalog: text("catalog"),
  updatedAt: integer("updated_at").notNull(),
}, (table) => [primaryKey({ columns: [table.userId, table.providerId] })]);

/** Stable installation identity required by the OpenAI OAuth registration flow. */
export const hvInstallation = sqliteTable("hv_installation", {
  id: integer("id").primaryKey(),
  deviceId: text("device_id").notNull(),
});
