import type { Context } from "@earendil-works/chord";
import type { ToolExecutionApi } from "@earendil-works/pi-durable";
import type { SqliteExecutor } from "@earendil-works/pi-durable/storage/sqlite";
import { loadAccountStatus } from "../account-status.js";
import type { HttpTransport, InfoClient } from "@nktkas/hyperliquid";
import type { SharedDatabase } from "../../db/shared-database.js";
import type { ExecutionContextService, ExecutionFrame } from "../../agents/execution-context.js";
import { bodyHash } from "../../agents/execution-context.js";
import { canonical } from "../../canonical.js";
import { assertActiveActor, type MemoryActor } from "../../memory/operations.js";
import { parseMemoryRef, type DecisionPayload } from "../../memory/memory-contracts.js";
import { HyperliquidError } from "../errors.js";
import type { SubmitInput } from "./order-contracts.js";
import { verifyProtection } from "./protection.js";
import { compare, decimal, guardPrice, notionalAtLeast, notionalWithin, positionSize } from "./order-math.js";
import { protectionPrices } from "./trade-planning.js";
import { instrument, instruments } from "./order-prepare.js";
import type { PreparedAction, RequestRow } from "./order-store.js";

/** Opaque, process-local authority; no model/API field can deserialize a handle. */
export type OrderExecutionHandle = object;
interface Authority { frame: ExecutionFrame; actor: MemoryActor }
export interface OrderAdmission { authority: Authority; decisionNumber: number | null; slotKey: string | null; researchRefs: string[] }
export class TradeAdmissionService {
  private readonly handles = new WeakMap<OrderExecutionHandle, Authority>();
  constructor(private readonly db: SharedDatabase, private readonly execution: ExecutionContextService, private readonly transport: HttpTransport) {}
  async handle(agentId: string, api: ToolExecutionApi, context: Context): Promise<OrderExecutionHandle> {
    const resolved = await this.execution.resolve(agentId, api, context);
    const handle = Object.freeze({}); this.handles.set(handle, resolved); return handle;
  }
  private authority(agentId: string, handle?: OrderExecutionHandle): Authority {
    const value = handle && this.handles.get(handle);
    if (!value || value.frame.agentId !== agentId) throw new HyperliquidError("A verified model execution handle is required for new model order mutations", 403);
    return value;
  }
  planning(agentId: string, handle?: OrderExecutionHandle) {
    const authority = this.authority(agentId, handle);
    if (Date.now() >= authority.frame.expiresAt) throw new HyperliquidError("Execution authorization expired", 409);
    return authority.frame;
  }
  async validate(agentId: string, input: SubmitInput, info: InfoClient, account: string, signal: AbortSignal, actions: PreparedAction[], handle?: OrderExecutionHandle, originalRequestId?: string): Promise<OrderAdmission | null> {
    const openings = input.orders.filter((order) => !order.reduce_only && ["limit", "market"].includes(order.order_type));
    if (openings.length && (openings.length !== 1 || input.orders.length !== 1)) throw new HyperliquidError("Use one protected opening order and one decision per request");
    let authority: Authority;
    if (originalRequestId) {
      const stored = await this.db.get<{ generation_task_id: number }>("SELECT generation_task_id FROM hv_order_execution_contexts WHERE agent_id=? AND request_id=?", agentId, originalRequestId);
      if (!stored) throw new HyperliquidError("Prepared legacy order has no admission context; it cannot be sent", 409);
      const frame = await this.execution.frame(agentId, stored.generation_task_id);
      authority = { frame, actor: { agentId, userId: null, subagentId: frame.config.id, runId: frame.runId, generationTaskId: frame.generationTaskId, inputScopeKey: frame.runId ? null : frame.inputScopeKey, conversationId: frame.conversationId, toolTaskId: null, origin: frame.origin.mode, expiresAt: frame.expiresAt } };
    } else authority = this.authority(agentId, handle);
    const { frame } = authority;
    if (!frame.config.tools.includes("submit_orders")) throw new HyperliquidError("Order mutation permission not selected", 403);
    if (!openings.length) {
      // No fresh research/decision is required for protection/reduction. Stable
      // execution authority still is: an old task cannot manage a new run's
      // unrelated position merely because the request is reduce-only.
      const admission: OrderAdmission = { authority, decisionNumber: input.decision_ref ? parseMemoryRef(input.decision_ref) : null, slotKey: null, researchRefs: [] };
      await this.db.transaction((tx) => this.checkTx(tx, agentId, input, admission));
      const state = await info.clearinghouseState({ user: account as `0x${string}`, dex: "" }, signal);
      const mids = input.orders.some((order) => ["take_profit", "stop_loss"].includes(order.order_type)) ? await info.allMids({ dex: "" }, signal) : {};
      for (const order of input.orders) {
        const position = state.assetPositions.find(({ position }) => position.coin === order.symbol)?.position;
        const exposure = positionSize(position?.szi ?? "0");
        if (exposure.side === "flat" || order.side !== (exposure.side === "long" ? "sell" : "buy") || compare(order.size, exposure.size) > 0) throw new HyperliquidError("Reduce-only management requires current exposure, closing side and size no greater than that exposure");
        if (["take_profit", "stop_loss"].includes(order.order_type)) {
          const quote = mids[order.symbol];
          if (typeof quote !== "string") throw new HyperliquidError("Current protection quote unavailable", 503);
          const direction = compare(order.trigger_price ?? "", quote);
          const above = (order.order_type === "take_profit") === (exposure.side === "long");
          if (above ? direction <= 0 : direction >= 0) throw new HyperliquidError("Standalone protection trigger is on the wrong side of the current market");
        }
      }
      return admission;
    }
    if (!input.decision_ref) throw new HyperliquidError("New exposure requires decision_ref from record_trade_decision");
    const number = parseMemoryRef(input.decision_ref), order = openings[0]!;
    const policy = frame.config.workflow?.purpose === "trading" ? frame.config.workflow : null;
    if (policy && !policy.instruments.includes(order.symbol)) throw new HyperliquidError("Instrument is outside the saved trading policy");
    const automatic = frame.origin.mode !== "user_directed";
    if (automatic && (!policy?.automaticEntriesEnabled || !policy.requiredResearch.length || !frame.runId)) throw new HyperliquidError("Automatic new exposure requires an explicitly enabled applicable trading workflow and required research", 409);
    const context = automatic ? await this.execution.memories.tradingContext(agentId, order.symbol, policy!.requiredResearch) : null;
    if (context && !context.eligibleForAutomaticEntry) throw new HyperliquidError(`Automatic entry blocked: ${context.overBudget ? "required evidence over budget" : context.requirements.map((item) => `${item.ref ?? "missing"}:${item.status}`).join(", ")}`, 409);
    if (automatic && order.side !== "buy") throw new HyperliquidError("Starter automatic policy permits long entries only");
    if (order.take_profits?.length !== 1 || order.stop_losses?.length !== 1 || compare(order.take_profits[0]!.size ?? order.size, order.size) !== 0 || compare(order.stop_losses[0]!.size ?? order.size, order.size) !== 0) throw new HyperliquidError("Every new entry requires one full-size attached TP and market SL");
    const prepared = actions[0]?.items[0];
    if (!prepared?.price) throw new HyperliquidError("Prepared entry price unavailable");
    const meta = instrument(await instruments(info, signal), order.symbol);
    if (!notionalAtLeast(order.size, prepared.price, "10")) throw new HyperliquidError("Opening notional is below the $10 venue minimum; never silently increase size");
    if (policy) {
      if (compare(order.size, policy.baseSizeByInstrument[order.symbol] ?? "") > 0 || !notionalWithin(order.size, prepared.price, policy.maxEntryNotional)) throw new HyperliquidError("Opening size/notional exceeds configured limits");
      const maximumLoss = protectionPrices(prepared.price, order.side === "buy", meta.szDecimals, policy.takeProfitBps, policy.stopLossBps).sl;
      if (order.side === "buy" ? compare(order.stop_losses[0]!.trigger_price, maximumLoss) < 0 : compare(order.stop_losses[0]!.trigger_price, maximumLoss) > 0) throw new HyperliquidError("Stop-loss exceeds configured maximum loss distance");
      if (automatic) {
        if (compare(order.size, policy.baseSizeByInstrument[order.symbol]!) !== 0) throw new HyperliquidError("Automatic entries use the configured base quantity");
        const prices = protectionPrices(prepared.price, order.side === "buy", meta.szDecimals, policy.takeProfitBps, policy.stopLossBps);
        if (compare(order.take_profits[0]!.trigger_price, prices.tp) !== 0 || compare(order.stop_losses[0]!.trigger_price, prices.sl) !== 0) throw new HyperliquidError("Automatic TP/SL must match exact configured protection distances");
      }
    }
    // Fresh HTTP account/quote observations happen INSIDE the existing per-Agent
    // mutation lock, never inside a SQLite transaction. Unknown is not empty.
    const health = await loadAccountStatus(this.transport, account, undefined, signal);
    if (!Object.values(health.sections).every((section) => section.fresh) || health.accountAbstraction === null || !["default", "disabled", "unifiedAccount", "portfolioMargin"].includes(health.accountAbstraction)) throw new HyperliquidError("Current perps/orders/collateral/account-mode health is incomplete", 503);
    const [state, live, mids] = await Promise.all([info.clearinghouseState({ user: account as `0x${string}`, dex: "" }, signal), info.frontendOpenOrders({ user: account as `0x${string}`, dex: "" }, signal), info.allMids({ dex: "" }, signal)]);
    if (!Array.isArray(state.assetPositions) || !Array.isArray(live) || typeof mids[order.symbol] !== "string" || !mids[order.symbol]) throw new HyperliquidError("Current account/orders/quote data unavailable", 503);
    decimal(mids[order.symbol]!);
    if (state.assetPositions.some(({ position }) => position.coin === order.symbol && positionSize(position.szi).side !== "flat")) throw new HyperliquidError("Existing exposure for this instrument must be managed, not pyramided", 409);
    if (order.order_type === "market" || order.time_in_force === "ioc") {
      const guard = guardPrice(mids[order.symbol]!, order.side === "buy", meta.szDecimals);
      if (order.side === "buy" ? compare(prepared.price, guard) > 0 : compare(prepared.price, guard) < 0) throw new HyperliquidError("Entry draft exceeds the current 0.5% IOC price guard; refresh the plan without changing explicit prices", 409);
    }
    if (live.length > 100 || state.assetPositions.length > 100) throw new HyperliquidError("Protection inspection exceeds the 100-order budget; new exposure blocked", 409);
    const protection = await verifyProtection(info, account, state.assetPositions.map(({ position }) => ({ coin: position.coin, size: position.szi })), live, mids, signal);
    if (protection.some((position) => !position.covered)) throw new HyperliquidError("Existing position protection cannot be verified. Repair/reconcile before new exposure", 409);
    if (live.some((item) => item.coin === order.symbol)) throw new HyperliquidError("Resting orders already exist for this instrument; no pyramiding or blind cancellation", 409);
    const local = await this.db.get("SELECT i.id FROM hv_order_items i JOIN hv_order_actions a ON a.id=i.action_id JOIN hv_order_requests r ON r.id=a.request_id WHERE r.agent_id=? AND r.id!=? AND i.symbol=? AND json_extract(i.view,'$.reduceOnly')=0 AND (a.state IN ('prepared','submitting','unknown') OR i.active=1 OR (json_extract(i.view,'$.orderStatus')='filled' AND i.updated_at>?)) LIMIT 1", agentId, originalRequestId ?? "", order.symbol, Date.now() - 30000);
    if (local) throw new HyperliquidError("Another local entry is active, unresolved or too recent to exclude delayed account propagation", 409);
    const refs = context?.requirements.map((item) => item.ref!).sort() ?? [];
    const slotKey = automatic ? `setup:${bodyHash(canonical({ trader: frame.config.id, instrument: order.symbol, direction: order.side, refs }))}` : `decision:${number}`;
    const admission = { authority, decisionNumber: number, slotKey, researchRefs: refs };
    await this.db.transaction((tx) => this.checkTx(tx, agentId, input, admission));
    return admission;
  }
  private async checkTx(tx: SqliteExecutor, agentId: string, input: SubmitInput, admission: OrderAdmission) {
    const { frame, actor } = admission.authority;
    await assertActiveActor(tx, actor, Date.now());
    const opening = input.orders.some((order) => !order.reduce_only && ["limit", "market"].includes(order.order_type));
    if (!opening && admission.decisionNumber === null) return;
    const decision = await tx.get<{ kind: string; instrument: string; payload: string; source_run_id: string | null; source_input_scope_key: string | null; producer_subagent_id: string | null; retracted_at: number | null }>("SELECT kind,instrument,payload,source_run_id,source_input_scope_key,producer_subagent_id,retracted_at FROM hv_memories WHERE agent_id=? AND number=?", agentId, admission.decisionNumber);
    const payload = decision ? JSON.parse(decision.payload) as DecisionPayload : null;
    const order = input.orders[0]!;
    if (!payload || !decision || decision.kind !== "decision" || decision.retracted_at !== null) throw new HyperliquidError("Decision is unavailable or is not a decision publication", 403);
    const sameScope = frame.runId ? decision.source_run_id === frame.runId : decision.source_run_id === null && decision.source_input_scope_key === frame.inputScopeKey;
    if (!sameScope || decision.producer_subagent_id !== frame.config.id || !input.orders.every((item) => item.symbol === decision.instrument)) throw new HyperliquidError("Decision does not match this instrument and authorized run/root input scope", 403);
    if (!opening) {
      if (!["reduce", "close", "protect"].includes(payload.action)) throw new HyperliquidError("Management decision must be reduce, close or protect", 403);
      return;
    }
    if (payload.action !== "enter" || payload.direction !== (order.side === "buy" ? "long" : "short")) throw new HyperliquidError("Decision action/direction does not match the opening order", 403);
    if (frame.origin.mode === "automatic") {
      const policy = frame.config.workflow;
      if (policy?.purpose !== "trading" || !policy.automaticEntriesEnabled) throw new HyperliquidError("Missing automatic entry policy", 409);
      const current = await this.execution.memories.tradingContextTx(tx, agentId, order.symbol, policy.requiredResearch);
      if (!current.eligibleForAutomaticEntry || canonical(current.requirements.map((item) => item.ref!).sort()) !== canonical(admission.researchRefs)) throw new HyperliquidError("Research changed or expired before entry admission", 409);
      if (!admission.researchRefs.every((ref) => payload.researchRefs.includes(ref))) throw new HyperliquidError("Decision must cite every mandatory current research publication");
      const producer = await tx.get<{ enabled: number; archived: number }>("SELECT enabled,archived FROM hv_subagents WHERE agent_id=? AND id=?", agentId, frame.config.id);
      if (!producer?.enabled || producer.archived) throw new HyperliquidError("Trader is disabled or archived", 409);
    }
  }
  async persist(tx: SqliteExecutor, request: RequestRow, input: SubmitInput, admission: OrderAdmission | null) {
    if (!admission) return;
    await this.checkTx(tx, request.agentId, input, admission);
    if (admission.slotKey !== null) {
      const previous = await tx.get<{ request_id: string }>("SELECT request_id FROM hv_automatic_entry_claims WHERE agent_id=? AND slot_key=?", request.agentId, admission.slotKey);
      if (previous && previous.request_id !== request.id) throw new HyperliquidError(`This decision/setup already belongs to order request ${previous.request_id}; reconcile the original, never generate a new ID`, 409);
      await tx.run("INSERT INTO hv_automatic_entry_claims(agent_id,slot_key,request_id) VALUES (?,?,?) ON CONFLICT DO NOTHING", request.agentId, admission.slotKey, request.id);
    }
    const { frame } = admission.authority;
    await tx.run("INSERT INTO hv_order_execution_contexts(agent_id,request_id,decision_number,generation_task_id,run_id,origin,policy,slot_key) VALUES (?,?,?,?,?,?,?,?)", request.agentId, request.id, admission.decisionNumber, frame.generationTaskId, frame.runId, JSON.stringify(frame.origin), JSON.stringify(frame.config.workflow ?? null), admission.slotKey);
  }
}
