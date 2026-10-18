import { AgentError } from "../../agents/agent-input.js";
import type { SharedDatabase } from "../../db/shared-database.js";
import { instrument } from "../../memory/memory-contracts.js";
import { array, fields, object, positiveDecimal, unique } from "../../memory/validation.js";

/**
 * Per-agent risk management: the ONLY application-enforced trading rules.
 * All other trading behavior (strategy, sizing guidance, TP/SL levels, layering)
 * lives in worker instructions and learnings. Missing settings = trading disabled.
 */
export interface RiskSettings {
  tradingEnabled: boolean;
  /** null = every main-DEX perpetual instrument allowed. */
  allowedInstruments: string[] | null;
  /** Exact USDC decimal strings; null = uncapped. */
  maxOrderNotional: string | null;
  maxTotalNotional: string | null;
}

/** Operator-supplied settings are validated here; models never write them. */
export function riskSettingsInput(value: unknown): RiskSettings {
  const row = fields(object(value), ["tradingEnabled", "allowedInstruments", "maxOrderNotional", "maxTotalNotional"]);
  if (typeof row.tradingEnabled !== "boolean") throw new AgentError("Invalid trading enablement");
  const allowed = row.allowedInstruments == null ? null : unique(array(row.allowedInstruments, 50, instrument));
  if (allowed && !allowed.length) throw new AgentError("Select at least one allowed instrument, or clear the list for no limit");
  const notional = (candidate: unknown) => candidate == null || candidate === "" ? null : positiveDecimal(candidate);
  return { tradingEnabled: row.tradingEnabled, allowedInstruments: allowed, maxOrderNotional: notional(row.maxOrderNotional), maxTotalNotional: notional(row.maxTotalNotional) };
}

const columns = "trading_enabled,allowed_instruments,max_order_notional,max_total_notional";
function settings(row: { trading_enabled: number; allowed_instruments: string | null; max_order_notional: string | null; max_total_notional: string | null } | undefined): RiskSettings {
  return row ? { tradingEnabled: row.trading_enabled === 1, allowedInstruments: row.allowed_instruments == null ? null : JSON.parse(row.allowed_instruments) as string[], maxOrderNotional: row.max_order_notional, maxTotalNotional: row.max_total_notional }
    : { tradingEnabled: false, allowedInstruments: null, maxOrderNotional: null, maxTotalNotional: null };
}

export class RiskSettingsService {
  constructor(private readonly db: SharedDatabase) {}
  async get(agentId: string): Promise<RiskSettings> {
    return settings(await this.db.get(`SELECT ${columns} FROM hv_risk_settings WHERE agent_id=?`, agentId));
  }
  async save(agentId: string, value: unknown): Promise<RiskSettings> {
    const input = riskSettingsInput(value), now = Date.now();
    return settings(await this.db.transaction(async (tx) => {
      await tx.run("INSERT INTO hv_risk_settings(agent_id,trading_enabled,allowed_instruments,max_order_notional,max_total_notional,updated_at) VALUES (?,?,?,?,?,?) ON CONFLICT(agent_id) DO UPDATE SET trading_enabled=excluded.trading_enabled,allowed_instruments=excluded.allowed_instruments,max_order_notional=excluded.max_order_notional,max_total_notional=excluded.max_total_notional,updated_at=excluded.updated_at", agentId, input.tradingEnabled ? 1 : 0, input.allowedInstruments && JSON.stringify(input.allowedInstruments), input.maxOrderNotional, input.maxTotalNotional, now);
      return await tx.get(`SELECT ${columns} FROM hv_risk_settings WHERE agent_id=?`, agentId);
    }));
  }
}