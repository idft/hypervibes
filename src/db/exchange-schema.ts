import { check, integer, sqliteTable, text, blob } from "drizzle-orm/sqlite-core";
import { sql } from "drizzle-orm";
import { hvUsers } from "./auth-schema.js";

/**
 * Per-user Hyperliquid settings: the single trading signer (API wallet) and
 * builder-fee approval state. The private key is encrypted at rest and is
 * loaded only by exchange-signing code.
 */
export const hvHyperliquidSettings = sqliteTable("hv_hyperliquid_settings", {
  userId: text("user_id").primaryKey().references(() => hvUsers.id, { onDelete: "restrict" }),
  apiWalletAddress: text("api_wallet_address"),
  privateKeyCiphertext: blob("private_key_ciphertext", { mode: "buffer" }),
  privateKeyKeyId: text("private_key_key_id"),
  apiWalletApprovedAt: integer("api_wallet_approved_at"),
  apiWalletExpiresAt: integer("api_wallet_expires_at"),
  apiWalletExpiryCheckedAt: integer("api_wallet_expiry_checked_at"),
  builderFeeTenthsOfBp: integer("builder_fee_tenths_of_bp").notNull().default(0),
  builderFeeApprovedAt: integer("builder_fee_approved_at"),
  createdAt: integer("created_at").notNull(),
  updatedAt: integer("updated_at").notNull(),
}, (table) => [
  check("hv_hyperliquid_settings_builder_fee_check", sql`${table.builderFeeTenthsOfBp} BETWEEN 0 AND 100`),
]);

export type HyperliquidSettingsRow = typeof hvHyperliquidSettings.$inferSelect;