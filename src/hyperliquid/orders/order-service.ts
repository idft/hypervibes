import { ApiRequestError, ExchangeClient, InfoClient, type HttpTransport } from "@nktkas/hyperliquid";
import type { Context } from "@earendil-works/chord";
import type { ToolExecutionApi } from "@earendil-works/pi-durable";
import type { ExecutionContextService } from "../../agents/execution-context.js";
import { TradeAdmissionService, type OrderExecutionHandle } from "./trade-admission.js";
import { preparePositionProtection, prepareProtectedEntry, type PositionProtectionInput, type ProtectedEntryInput } from "./trade-planning.js";
import { verifyProtection } from "./protection.js";
import { compare } from "./order-math.js";
import { privateKeyToAccount } from "viem/accounts";
import type { SharedDatabase } from "../../db/shared-database.js";
import type { FastifyBaseLogger } from "fastify";
import { HyperliquidError } from "../errors.js";
import { assertStringAmounts, exchangeOid, instruments, instrument, isPerpetual, liveView, newView, prepareSubmit } from "./order-prepare.js";
import { OrderStore, active, type PreparedAction, type PreparedPayload, type RequestRow } from "./order-store.js";
import { reconciledView, responseViews, unknownViews } from "./order-outcomes.js";
import type { CancelAllInput, CancelInput, GetResult, ListInput, ListResult, OperationResult, OrderView, SubmitInput } from "./order-contracts.js";

export type TradingAuthority = { account: string; privateKey: string; vaultAddress?: `0x${string}`; builder: { b: `0x${string}`; f: number } };
export type OrderBindings = {
  account(userId: string, agentId: string): Promise<string | null>;
  authority(userId: string, agentId: string, signal: AbortSignal, needsBuilder: boolean): Promise<TradingAuthority>;
};

/** Application-owned intent/outcome gateway. Reconciliation NEVER submits orders. */
export class OrderService {
  readonly store: OrderStore;
  private readonly info: InfoClient;
  private readonly locks = new Map<string, Promise<unknown>>();
  private readonly tasks = new Set<Promise<unknown>>();
  private readonly shutdown = new AbortController();
  private timer?: ReturnType<typeof setTimeout>;
  private started = false;
  private stopping = false;
  private admission?: TradeAdmissionService;
  constructor(private readonly database: SharedDatabase, private readonly transport: HttpTransport, private readonly bindings: OrderBindings, private readonly log?: FastifyBaseLogger) {
    this.store = new OrderStore(database); this.info = new InfoClient({ transport });
  }
  installAdmission(execution: ExecutionContextService) { this.admission = new TradeAdmissionService(this.database, execution); }
  executionHandle(agentId: string, api: ToolExecutionApi, context: Context) {
    if (!this.admission) throw new HyperliquidError("Execution identity is not initialized; model orders are unavailable", 503);
    return this.admission.handle(agentId, api, context);
  }
  async plan(userId: string, agentId: string, input: ProtectedEntryInput, handle: OrderExecutionHandle | undefined, cancellation?: AbortSignal) {
    await this.account(userId, agentId);
    if (!this.admission) throw new HyperliquidError("Trade planning is not initialized", 503);
    this.admission.planning(agentId, handle);
    return prepareProtectedEntry(this.info, input, this.signal(cancellation));
  }
  async planProtection(userId: string, agentId: string, input: PositionProtectionInput, handle: OrderExecutionHandle | undefined, cancellation?: AbortSignal) {
    const account = await this.account(userId, agentId);
    if (!this.admission) throw new HyperliquidError("Trade planning is not initialized", 503);
    this.admission.planning(agentId, handle);
    return preparePositionProtection(this.info, input, account, this.signal(cancellation));
  }
  async protection(userId: string, agentId: string, cancellation?: AbortSignal) {
    const account = await this.account(userId, agentId), signal = this.signal(cancellation);
    const [state, orders, mids] = await Promise.all([this.info.clearinghouseState({ user: account as `0x${string}`, dex: "" }, signal), this.info.frontendOpenOrders({ user: account as `0x${string}`, dex: "" }, signal), this.info.allMids({ dex: "" }, signal)]);
    if (orders.length > 100 || state.assetPositions.length > 100) throw new HyperliquidError("More than 100 open orders/positions; protection coverage is over budget and must be reviewed manually", 409);
    const positions = await verifyProtection(this.info, account, state.assetPositions.map(({ position }) => ({ coin: position.coin, size: position.szi })), orders, mids, signal);
    return { observedAt: Date.now(), positions, warning: "Snapshot verification only. Native children waitingForFill do not protect partial positions. Scheduled model repair cannot guarantee zero unprotected time. Preserve known orders; do not flatten or cancel-all without authorization." };
  }
  recover() { return this.store.recover(); }
  start() {
    if (this.started || this.stopping) return;
    this.started = true;
    this.log?.info("Starting order reconciliation loop");
    const tick = async () => {
      try {
        for (const request of await this.store.pendingRequests()) {
          if (this.stopping) break;
          await this.lock(request.id, () => this.reconcile(request, this.signal()));
        }
      } catch (error) { this.log?.warn(error, "Order reconciliation pass failed; retrying on the next pass"); }
      finally { if (!this.stopping) { this.timer = setTimeout(() => { void this.track(tick()); }, 15_000); this.timer.unref(); } }
    };
    void this.track(tick());
  }
  async close() {
    this.stopping = true; clearTimeout(this.timer); this.shutdown.abort();
    await Promise.allSettled([...this.tasks]);
  }
  private signal(signal?: AbortSignal) { return AbortSignal.any([this.shutdown.signal, AbortSignal.timeout(15_000), ...(signal ? [signal] : [])]); }
  private track<T>(task: Promise<T>): Promise<T> {
    this.tasks.add(task);
    void task.finally(() => this.tasks.delete(task)).catch(() => {});
    return task;
  }
  private admit() { if (this.stopping) throw new HyperliquidError("Order service is closed", 503); }
  private lock<T>(key: string, run: () => Promise<T>): Promise<T> {
    const task = (this.locks.get(key) ?? Promise.resolve()).catch(() => {}).then(run);
    this.locks.set(key, task);
    void task.finally(() => { if (this.locks.get(key) === task) this.locks.delete(key); }).catch(() => {});
    return task;
  }
  private async account(userId: string, agentId: string) {
    this.admit();
    const address = await this.bindings.account(userId, agentId);
    if (!address || !/^0x[0-9a-fA-F]{40}$/.test(address)) throw new HyperliquidError("This Agent has no valid assigned trading account.", 409);
    return address.toLowerCase();
  }
  private requestKey(key: string) { if (typeof key !== "string" || !/^[a-zA-Z0-9_-]{1,120}$/.test(key)) throw new HyperliquidError("Use a stable request_id of 1–120 letters, digits, underscores or hyphens."); }
  submit(userId: string, agentId: string, input: SubmitInput, signal?: AbortSignal, execution?: OrderExecutionHandle) {
    assertStringAmounts(input);
    if (!Array.isArray(input.orders) || input.orders.length < 1 || input.orders.length > 20) throw new HyperliquidError("Submit 1–20 entry orders per request.");
    return this.mutate(userId, agentId, input.request_id, "submit", input, signal,
      (authority, abort) => prepareSubmit(this.info, input.request_id, input.orders, authority.builder, abort), execution);
  }
  cancel(userId: string, agentId: string, input: CancelInput, signal?: AbortSignal) {
    assertStringAmounts(input);
    if (!Array.isArray(input.orders) || input.orders.length < 1 || input.orders.length > 100) throw new HyperliquidError("Cancel 1–100 orders per request.");
    return this.mutate(userId, agentId, input.request_id, "cancel", input, signal,
      (authority, abort) => this.prepareCancel(authority.account, input.request_id, input.orders, abort));
  }
  cancelAll(userId: string, agentId: string, input: CancelAllInput, signal?: AbortSignal) {
    return this.mutate(userId, agentId, input.request_id, "cancel_all", input, signal, async (authority, abort) => {
      const orders = (await this.info.frontendOpenOrders({ user: authority.account as `0x${string}`, dex: "" }, abort))
        .filter((order) => isPerpetual(order.coin) && (!input.symbol || order.coin === input.symbol));
      if (orders.length > 100) throw new HyperliquidError("More than 100 open orders; cancel explicit bounded batches instead.");
      if (!orders.length) return [];
      return this.prepareCancel(authority.account, input.request_id, orders.map((order) => ({ symbol: order.coin, oid: String(order.oid) })), abort);
    });
  }
  private async prepareCancel(account: string, key: string, inputs: { symbol: string; oid: string }[], signal: AbortSignal): Promise<PreparedAction[]> {
    const meta = await instruments(this.info, signal);
    const seen = new Set<string>();
    const items: OrderView[] = [], cancels: { a: number; o: number }[] = [];
    for (const input of inputs) {
      const asset = instrument(meta, input.symbol).asset;
      const oid = exchangeOid(input.oid);
      if (seen.has(input.oid)) throw new HyperliquidError("Duplicate cancellation target.");
      seen.add(input.oid);
      // OIDs alone confer no authority. Check the assigned account and symbol,
      // including orders this account placed outside HyperVibes.
      const status = await this.info.orderStatus({ user: account as `0x${string}`, oid }, signal);
      if (status.status !== "order" || status.order.order.coin !== input.symbol || status.order.order.oid !== oid) throw new HyperliquidError("Cancellation target was not found on this Agent's account/symbol.", 409);
      const view = liveView(status.order.order, status.order.status, status.order.statusTimestamp);
      items.push({ ...view, id: newView(key, input.symbol, "cancel").id, requestId: key, kind: "cancel", cloid: null, commandStatus: "not_sent", source: "intent" });
      cancels.push({ a: asset, o: oid });
    }
    return [{ payload: { kind: "cancel", cancels }, items }];
  }
  private mutate(userId: string, agentId: string, key: string, kind: string, input: unknown, cancellation: AbortSignal | undefined, prepare: (authority: TradingAuthority, signal: AbortSignal) => Promise<PreparedAction[]>, execution?: OrderExecutionHandle): Promise<OperationResult> {
    this.admit(); this.requestKey(key);
    if (!this.started) return Promise.reject(new HyperliquidError("Order execution is not started", 503));
    return this.track(this.lock(agentId, async () => {
      const account = await this.account(userId, agentId);
      const signal = this.signal(cancellation);
      signal.throwIfAborted();
      let request = await this.store.request(agentId, key);
      let authority: TradingAuthority | undefined;
      if (request) this.store.matches(request, account, kind, input);
      else {
        if (kind === "submit" && (input as SubmitInput).orders.some((order) => !order.reduce_only && ["limit", "market"].includes(order.order_type)) && await this.store.hasUncertain(account)) {
          throw new HyperliquidError("An earlier command has an unknown outcome on this account. Reconcile it before adding exposure; do not retry with a new request_id.", 409);
        }
        authority = await this.bindings.authority(userId, agentId, signal, kind === "submit");
        if (authority.account !== account) throw new HyperliquidError("Account assignment changed", 409);
        const prepared = await prepare(authority, signal);
        if (kind === "submit" && this.admission) await this.admission.validate(agentId, this.info, account, signal, prepared, execution);
        request = await this.store.prepare(agentId, account, key, kind, input, prepared);
      }
      // The same request lock is also used by background reconciliation.
      const captured = request;
      return this.lock(captured.id, async () => {
        const actions = await this.store.actions(captured.id);
        let stop = actions.some((action) => action.state === "unknown" || action.state === "submitting")
          || (await this.store.allItems(captured.id)).some(({ view }) => (JSON.parse(view) as OrderView).commandStatus === "rejected");
        for (const action of actions) {
          if (action.state !== "prepared") continue;
          const views = (await this.store.items(action.id)).map(({ view }) => JSON.parse(view) as OrderView);
          if (stop || signal.aborted || Date.now() >= action.expiresAt) {
            await this.store.outcome(action.id, "not_sent", views.map((view) => ({ ...view, commandStatus: "not_sent", orderStatus: "not_sent", updatedAt: Date.now(), error: "Not sent: previous action uncertain/rejected, request expired, or execution aborted." })), "not_sent");
            continue;
          }
          authority ??= await this.bindings.authority(userId, agentId, signal, kind === "submit");
          if (authority.account !== captured.account) throw new HyperliquidError("Account assignment changed", 409);
          if (kind === "submit" && this.admission) {
            if (views.some((view) => !view.reduceOnly) && await this.store.hasUncertain(account)) throw new HyperliquidError("An earlier command has an unknown outcome on this account. Reconcile it before adding exposure.", 409);
            await this.admission.validate(agentId, this.info, account, signal, [{ payload: JSON.parse(action.payload) as PreparedPayload, items: views }], execution);
          }
          if (!(await this.store.begin(action))) continue;
          const payload = JSON.parse(action.payload) as PreparedPayload;
          let response: unknown;
          let uncertain = false;
          try {
            const client = new ExchangeClient({ transport: this.transport, wallet: privateKeyToAccount(authority.privateKey as `0x${string}`),
              nonceManager: (signer) => this.store.nonce(signer, action.id) });
            const options = { signal, vaultAddress: authority.vaultAddress, expiresAfter: action.expiresAt };
            response = payload.kind === "submit" ? await client.order({ orders: payload.orders, grouping: payload.grouping, builder: payload.builder }, options)
              : await client.cancel({ cancels: payload.cancels }, options);
          } catch (error) {
            if (error instanceof ApiRequestError) response = error.response;
            else uncertain = true;
          }
          const outcomes = uncertain ? unknownViews(views, "Submission outcome is unknown after transport/signing interruption. Do not resubmit; reconcile this request_id.") : responseViews(payload.kind, response, views);
          const unknown = outcomes.some((view) => view.commandStatus === "unknown");
          await this.store.outcome(action.id, unknown ? "unknown" : "done", outcomes, "exchange_response");
          stop = unknown || outcomes.some((view) => view.commandStatus === "rejected");
        }
        return this.result(captured);
      });
    }));
  }
  private async result(request: RequestRow): Promise<OperationResult> {
    const results = (await this.store.allItems(request.id)).map(({ view }) => JSON.parse(view) as OrderView);
    const warnings: string[] = [];
    if (request.kind === "cancel_all") warnings.push("Cancel-all targets only the original captured main-DEX perpetual orders, including any protective TP/SL. It does not sweep new orders or close positions.");
    if (results.some((view) => view.kind === "take_profit" || view.kind === "stop_loss")) warnings.push("TP/SL uses fixed-size reduce-only triggers. Native attached TP/SL activates on full parent fill; manually canceling a partially filled parent also cancels its attached protection. Inspect every leg; a rejected leg can leave exposure unprotected.");
    if (results.some((view) => view.filledSize && view.size && compare(view.filledSize, view.size) < 0)) warnings.push("Only part of the requested size was filled. filledSize is the executed quantity; a filled receipt does not imply the whole requested size traded. IOC remainders do not rest.");
    if (results.some((view) => view.commandStatus === "unknown")) warnings.push("Unknown is not failure. Reconcile using get_order_request/get_order; never retry with a new request_id.");
    return { operationId: request.id, requestId: request.requestKey, kind: request.kind, results, uncertain: results.some((view) => view.commandStatus === "unknown"), warnings };
  }
  private async reconcile(request: RequestRow, signal: AbortSignal) {
    for (const action of await this.store.actions(request.id)) {
      if (action.state === "prepared" || action.state === "not_sent") continue;
      const items = await this.store.items(action.id);
      const views: OrderView[] = [];
      for (const item of items) {
        const view = JSON.parse(item.view) as OrderView;
        if (!active(view)) { views.push(view); continue; }
        try {
          const oid = view.exchangeOid ? exchangeOid(view.exchangeOid) : view.cloid as `0x${string}` | null;
          if (!oid) { views.push(view); continue; }
          views.push(reconciledView(view, await this.info.orderStatus({ user: request.account as `0x${string}`, oid }, signal)));
        } catch {
          views.push({ ...view, updatedAt: Date.now(), error: "Order reconciliation unavailable; previous state retained." });
        }
      }
      await this.store.outcome(action.id, views.some((view) => view.commandStatus === "unknown") ? "unknown" : "done", views, "reconciliation");
    }
  }
  request(userId: string, agentId: string, key: string, cancellation?: AbortSignal): Promise<OperationResult> {
    this.admit(); this.requestKey(key);
    return this.track((async () => {
      const account = await this.account(userId, agentId);
      const request = await this.store.request(agentId, key);
      if (!request || request.account !== account) throw new HyperliquidError("Order request not found", 404);
      return this.lock(request.id, async () => { await this.reconcile(request, this.signal(cancellation)); return this.result(request); });
    })());
  }
  list(userId: string, agentId: string, input: ListInput, cancellation?: AbortSignal): Promise<ListResult> {
    this.admit();
    return this.track((async () => {
      const account = await this.account(userId, agentId);
      const source = input.source ?? "open", limit = input.limit ?? 50, offset = input.offset ?? 0;
      if (!Number.isInteger(limit) || limit < 1 || limit > 100 || !Number.isInteger(offset) || offset < 0 || offset > 100000) throw new HyperliquidError("Invalid pagination.");
      let items: OrderView[];
      if (source === "local") {
        items = (await this.store.local(agentId, account)).map(({ view }) => JSON.parse(view) as OrderView);
      }
      else if (source === "open") items = (await this.info.frontendOpenOrders({ user: account as `0x${string}`, dex: "" }, this.signal(cancellation))).filter((order) => isPerpetual(order.coin)).map((order) => liveView(order));
      else throw new HyperliquidError("Invalid order source.");
      items = items.filter((item) => (!input.symbol || item.symbol === input.symbol) && (!input.status || item.orderStatus === input.status));
      return { items: items.slice(offset, offset + limit), nextOffset: offset + limit < items.length ? offset + limit : null, source, observedAt: Date.now(),
        coverage: source === "open" ? "Current main-DEX perpetual open orders, including externally placed orders. Pagination is over a changing live snapshot." : "Locally recorded submissions from the latest 500 requests only; not full exchange history. States may lag; get_order refreshes a selected order." };
    })());
  }
  get(userId: string, agentId: string, id: string, includeEvents = false, cancellation?: AbortSignal): Promise<GetResult> {
    this.admit();
    return this.track((async () => {
      const account = await this.account(userId, agentId);
      const found = await this.store.find(agentId, account, id);
      if (found && found.request.account !== account) throw new HyperliquidError("Order not found", 404);
      if (found) {
        return this.lock(found.request.id, async () => {
          await this.reconcile(found.request, this.signal(cancellation));
          const updated = await this.store.find(agentId, account, found.item.id);
          return { order: updated ? JSON.parse(updated.item.view) as OrderView : null, events: includeEvents ? await this.store.events(found.item.id) : [], error: null, observedAt: Date.now() };
        });
      }
      const oid = /^0x[0-9a-fA-F]{32}$/.test(id) ? id as `0x${string}` : exchangeOid(id);
      const response = await this.info.orderStatus({ user: account as `0x${string}`, oid }, this.signal(cancellation));
      if (response.status !== "order") return { order: null, events: [], error: "Order not found; absence is not proof of rejection, fill, or cancellation.", observedAt: Date.now() };
      if (!isPerpetual(response.order.order.coin)) throw new HyperliquidError("Only main-DEX perpetual orders are supported.");
      return { order: liveView(response.order.order, response.order.status, response.order.statusTimestamp), events: [], error: null, observedAt: Date.now() };
    })());
  }
}
