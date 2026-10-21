import type { Context } from "@earendil-works/chord";
import type { ToolExecutionApi } from "@earendil-works/pi-durable";
import type { InfoClient } from "@nktkas/hyperliquid";
import type { SharedDatabase } from "../../db/shared-database.js";
import type { ExecutionContextService } from "../../agents/execution-context.js";
import type { MemoryActor } from "../../memory/operations.js";
import { assertActiveActor } from "../../memory/operations.js";
import { HyperliquidError } from "../errors.js";
import { RiskSettingsService } from "./risk-settings.js";
import { compare, decimal, notional, notionalAtLeast, positionSize, sumDecimals } from "./order-math.js";
import type { OrderView } from "./order-contracts.js";
import type { PreparedAction } from "./order-store.js";

/** Opaque, process-local authority; no model/API field can deserialize a handle. */
export type OrderExecutionHandle = object;

/** Identity/tool permissions plus risk limits, never strategy or research policy. */
export class TradeAdmissionService {
  private readonly handles = new WeakMap<OrderExecutionHandle, { actor: MemoryActor; tools: string[] }>();
  private readonly risk: RiskSettingsService;
  constructor(private readonly db: SharedDatabase, private readonly execution: ExecutionContextService) {
    this.risk = new RiskSettingsService(db);
  }
  async handle(agentId: string, api: ToolExecutionApi, context: Context): Promise<OrderExecutionHandle> {
    const { actor, config } = await this.execution.resolve(agentId, api, context);
    const handle = Object.freeze({});
    this.handles.set(handle, { actor, tools: config.tools });
    return handle;
  }
  private authority(agentId: string, handle?: OrderExecutionHandle) {
    const value = handle && this.handles.get(handle);
    if (!value || value.actor.agentId !== agentId) throw new HyperliquidError("A verified model execution handle is required for model order mutations", 403);
    return value;
  }
  /** Only the trusted handle can supply request provenance, never model arguments. */
  origin(agentId: string, handle: OrderExecutionHandle | undefined, capability: string) {
    const { actor, tools } = this.authority(agentId, handle);
    if (!tools.includes(capability)) throw new HyperliquidError("Order mutation permission not selected", 403);
    return actor;
  }
  /** Read-only planners need identity, not submit_orders permission. */
  planning(agentId: string, handle?: OrderExecutionHandle) { this.authority(agentId, handle); }

  /** Called under the existing per-Agent mutation lock, including before a retry sends. */
  async validate(agentId: string, info: InfoClient, account: string, signal: AbortSignal, actions: PreparedAction[], handle?: OrderExecutionHandle): Promise<void> {
    const { actor, tools } = this.authority(agentId, handle);
    if (!tools.includes("submit_orders")) throw new HyperliquidError("Order mutation permission not selected", 403);
    await this.db.transaction((tx) => assertActiveActor(tx, actor));
    const entries = actions.flatMap((action) => action.items).filter((view) => !view.reduceOnly);
    // Reduce-only orders cannot increase exposure. The exchange remains authoritative
    // for position quantity/side; disabling entries never blocks protective management.
    if (!entries.length) return;
    const settings = await this.risk.get(agentId);
    if (!settings.tradingEnabled) throw new HyperliquidError("Trading is disabled in this Agent's risk settings. Enable trading in the Agent's risk settings before opening exposure; enabling a worker does not bypass this shared gate.", 409);
    const mids = settings.maxOrderNotional || settings.maxTotalNotional ? await info.allMids({ dex: "" }, signal) : {};
    const quote = (coin: string) => {
      const value = mids[coin];
      if (typeof value !== "string") throw new HyperliquidError(`Current market quote for ${coin} unavailable`, 503);
      decimal(value);
      return value;
    };
    for (const entry of entries) {
      if (settings.allowedInstruments && !settings.allowedInstruments.includes(entry.symbol)) throw new HyperliquidError(`Instrument ${entry.symbol} is not in this Agent's allowed instruments`, 409);
      if (!entry.size || !entry.price) throw new HyperliquidError("Prepared entry size/price unavailable");
      if (!notionalAtLeast(entry.size, entry.price, "10")) throw new HyperliquidError("Opening notional is below the $10 venue minimum; never silently increase size");
      if (settings.maxOrderNotional && compare(notional(entry.size, higher(quote(entry.symbol), entry.price)), settings.maxOrderNotional) > 0) throw new HyperliquidError("Opening notional exceeds the configured maximum order notional", 409);
    }
    if (!settings.maxTotalNotional) return;

    const [state, live, local] = await Promise.all([
      info.clearinghouseState({ user: account as `0x${string}`, dex: "" }, signal),
      info.frontendOpenOrders({ user: account as `0x${string}`, dex: "" }, signal),
      this.db.all<{ view: string }>(`SELECT i.view FROM hv_order_items i
        JOIN hv_order_actions a ON a.id=i.action_id JOIN hv_order_requests r ON r.id=a.request_id
        WHERE r.agent_id=? AND r.account=? AND i.id NOT IN (SELECT value FROM json_each(?)) AND r.kind='submit'
        AND json_extract(i.view,'$.reduceOnly')=0 AND
        (a.state IN ('prepared','submitting','unknown') OR i.active=1 OR
          (json_extract(i.view,'$.orderStatus')='filled' AND i.updated_at>?)) LIMIT 1001`, agentId, account, JSON.stringify(actions.flatMap((action) => action.items.map((item) => item.id))), Date.now() - 30000),
    ]);
    if (!Array.isArray(state.assetPositions) || !Array.isArray(live)) throw new HyperliquidError("Current exposure data unavailable", 503);
    if (local.length > 1000) throw new HyperliquidError("Pending exposure exceeds the risk inspection budget", 409);
    const amounts = state.assetPositions.flatMap(({ position }) => {
      const exposure = positionSize(position.szi);
      return exposure.side === "flat" ? [] : [notional(exposure.size, quote(position.coin))];
    });
    const pending = [...entries];
    // Do not double-count an open order reported by both local storage and the venue.
    const cloids = new Set<string>(), oids = new Set<string>();
    for (const order of live) {
      if (order.reduceOnly) continue;
      if (order.cloid) cloids.add(order.cloid);
      oids.add(String(order.oid));
      amounts.push(notional(order.sz, higher(quote(order.coin), order.limitPx)));
    }
    for (const { view } of local) {
      const order = JSON.parse(view) as OrderView;
      if (order.cloid && cloids.has(order.cloid) || order.exchangeOid && oids.has(order.exchangeOid)) continue;
      // Unknown/pending submissions and very recent fills reserve capacity while
      // account snapshots catch up. Recent fills may briefly be counted twice.
      pending.push(order);
    }
    for (const order of pending) {
      if (!order.size || !order.price) throw new HyperliquidError("Pending exposure size/price unavailable", 503);
      amounts.push(notional(order.size, higher(quote(order.symbol), order.price)));
    }
    if (compare(sumDecimals(amounts), settings.maxTotalNotional) > 0) throw new HyperliquidError("Projected total open notional (positions plus pending entries) exceeds the configured maximum total notional", 409);
  }
}
function higher(a: string, b: string): string { return compare(a, b) >= 0 ? a : b; }
