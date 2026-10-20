import { sql } from "drizzle-orm";
import { check, integer, sqliteTable, text } from "drizzle-orm/sqlite-core";
import { hvAgents } from "./agent-schema.js";

/**
 * Per-agent risk management settings. These are the only application-enforced
 * trading rules: every other trading behavior lives in worker instructions and
 * insights. A missing row means trading is disabled (fail closed) with no
 * instrument or notional limits configured yet.
 */
export const hvRiskSettings = sqliteTable("hv_risk_settings", {
  agentId: text("agent_id").primaryKey().references(() => hvAgents.id, { onDelete: "cascade" }),
  tradingEnabled: integer("trading_enabled", { mode: "boolean" }).notNull().default(false),
  /** null = all main-DEX perpetual instruments allowed; otherwise exact coin symbols. */
  allowedInstruments: text("allowed_instruments", { mode: "json" }).$type<string[] | null>(),
  /** Exact decimal USDC strings; null = uncapped. */
  maxOrderNotional: text("max_order_notional"),
  maxTotalNotional: text("max_total_notional"),
  updatedAt: integer("updated_at").notNull(),
}, (t) => [
  check("hv_risk_instruments_bound", sql`length(coalesce(${t.allowedInstruments}, '[]')) <= 4000 AND json_valid(coalesce(${t.allowedInstruments}, '[]'))`),
]);