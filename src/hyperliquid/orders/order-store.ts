import { and, asc, desc, eq, or } from "drizzle-orm";
import { randomUUID } from "node:crypto";
import { hvExchangeNonces, hvOrderActions, hvOrderEvents, hvOrderItems, hvOrderRequests } from "../../db/order-schema.js";
import { applicationDatabase, type SharedDatabase } from "../../db/shared-database.js";
import { HyperliquidError } from "../errors.js";
import type { OrderView } from "./order-contracts.js";
import type { ExchangeClient } from "@nktkas/hyperliquid";

export type WireOrder = Parameters<ExchangeClient["order"]>[0]["orders"][number];
export type PreparedPayload =
  | { kind: "submit"; orders: WireOrder[]; grouping: "na" | "normalTpsl"; builder: { b: `0x${string}`; f: number } }
  | { kind: "cancel"; cancels: { a: number; o: number }[] };
export type PreparedAction = { payload: PreparedPayload; items: OrderView[] };
export type RequestRow = typeof hvOrderRequests.$inferSelect;
export type ActionRow = typeof hvOrderActions.$inferSelect;
export type ItemRow = typeof hvOrderItems.$inferSelect;

import type { SqliteExecutor } from "@earendil-works/pi-durable/storage/sqlite";
import { canonical } from "../../canonical.js";
export { canonical } from "../../canonical.js";
export function active(view: OrderView): boolean {
  return view.commandStatus === "unknown" || ["open", "waitingForFill", "waitingForTrigger", "triggered"].includes(view.orderStatus);
}
export class OrderStore {
  private readonly db;
  constructor(private readonly database: SharedDatabase) { this.db = applicationDatabase(database); }
  request(agentId: string, key: string) { return this.db.select().from(hvOrderRequests).where(and(eq(hvOrderRequests.agentId, agentId), eq(hvOrderRequests.requestKey, key))).get(); }
  actions(id: string) { return this.db.select().from(hvOrderActions).where(eq(hvOrderActions.requestId, id)).orderBy(asc(hvOrderActions.ordinal)).all(); }
  items(actionId: string) { return this.db.select().from(hvOrderItems).where(eq(hvOrderItems.actionId, actionId)).orderBy(asc(hvOrderItems.ordinal)).all(); }
  async allItems(requestId: string) { return (await Promise.all((await this.actions(requestId)).map((action) => this.items(action.id)))).flat(); }
  async prepare(agentId: string, account: string, key: string, kind: string, input: unknown, actions: PreparedAction[], audit?: (tx: SqliteExecutor, row: RequestRow) => Promise<void>) {
    return this.database.appTransaction(async (db, tx) => {
      const existing = await db.select().from(hvOrderRequests).where(and(eq(hvOrderRequests.agentId, agentId), eq(hvOrderRequests.requestKey, key))).get();
      if (existing) {
        this.matches(existing, account, kind, input);
        return existing;
      }
      const now = Date.now();
      const row: RequestRow = { id: randomUUID(), agentId, account, requestKey: key, kind, input: canonical(input), createdAt: now };
      await db.insert(hvOrderRequests).values(row).run();
      for (const [ordinal, action] of actions.entries()) {
        const actionId = randomUUID();
        await db.insert(hvOrderActions).values({ id: actionId, requestId: row.id, ordinal, payload: JSON.stringify(action.payload), state: "prepared", expiresAt: now + 60_000, updatedAt: now }).run();
        for (const [index, view] of action.items.entries()) {
          await db.insert(hvOrderItems).values({ id: view.id, actionId, ordinal: index, symbol: view.symbol, cloid: view.cloid, oid: view.exchangeOid, active: 0, view: JSON.stringify(view), updatedAt: now }).run();
          await db.insert(hvOrderEvents).values({ itemId: view.id, source: "intent", time: now, data: JSON.stringify(view) }).run();
        }
      }
      await audit?.(tx, row);
      return row;
    });
  }
  audit(id: string) { return this.database.get<{ decision_number: number | null; origin: string }>("SELECT decision_number,origin FROM hv_order_execution_contexts WHERE request_id=?", id); }
  audits(agentId: string, account: string) { return this.database.all<{ request_key: string; decision_number: number | null; origin: string | null }>("SELECT r.request_key,c.decision_number,c.origin FROM hv_order_requests r LEFT JOIN hv_order_execution_contexts c ON c.request_id=r.id AND c.agent_id=r.agent_id WHERE r.agent_id=? AND r.account=? AND r.kind='submit' ORDER BY r.created_at DESC LIMIT 500", agentId, account); }
  matches(row: RequestRow, account: string, kind: string, input: unknown) {
    if (row.account !== account || row.kind !== kind || row.input !== canonical(input)) throw new HyperliquidError("This request_id was already used with different arguments or an account assignment.", 409);
  }
  async begin(action: ActionRow): Promise<boolean> {
    return this.database.appTransaction(async (db) => {
      const now = Date.now();
      const rows = await db.update(hvOrderActions).set({ state: "submitting", updatedAt: now })
        .where(and(eq(hvOrderActions.id, action.id), eq(hvOrderActions.state, "prepared"))).returning().all();
      if (!rows.length) return false;
      for (const item of await db.select().from(hvOrderItems).where(eq(hvOrderItems.actionId, action.id)).all()) {
        const view: OrderView = { ...JSON.parse(item.view) as OrderView, commandStatus: "unknown", orderStatus: "unknown", updatedAt: now, error: "Submission started; venue outcome not yet committed." };
        await db.update(hvOrderItems).set({ active: 1, view: JSON.stringify(view), updatedAt: now }).where(eq(hvOrderItems.id, item.id)).run();
        await db.insert(hvOrderEvents).values({ itemId: item.id, source: "submission_intent", time: now, data: JSON.stringify(view) }).run();
      }
      return true;
    });
  }
  async hasUncertain(account: string) {
    const row = await this.db.select({ id: hvOrderActions.id }).from(hvOrderActions).innerJoin(hvOrderRequests, eq(hvOrderRequests.id, hvOrderActions.requestId))
      .where(and(eq(hvOrderRequests.account, account), or(eq(hvOrderActions.state, "unknown"), eq(hvOrderActions.state, "submitting")))).get();
    return Boolean(row);
  }
  async outcome(actionId: string, state: string, views: OrderView[], source: string) {
    await this.database.appTransaction(async (db) => {
      await db.update(hvOrderActions).set({ state, updatedAt: Date.now() }).where(eq(hvOrderActions.id, actionId)).run();
      for (const view of views) {
        const previous = await db.select().from(hvOrderItems).where(eq(hvOrderItems.id, view.id)).get();
        if (!previous || previous.actionId !== actionId) throw new Error("Order item identity changed");
        const old = JSON.parse(previous.view) as OrderView;
        await db.update(hvOrderItems).set({ oid: view.exchangeOid, view: JSON.stringify(view), active: active(view) ? 1 : 0, updatedAt: view.updatedAt }).where(eq(hvOrderItems.id, view.id)).run();
        // Polling timestamps do not produce an unbounded event log of identical states.
        const comparable = (v: OrderView) => ({ ...v, updatedAt: 0, source: "", venueTimestamp: null });
        if (canonical(comparable(old)) !== canonical(comparable(view))) await db.insert(hvOrderEvents).values({ itemId: view.id, source, time: Date.now(), data: JSON.stringify(view) }).run();
      }
    });
  }
  async nonce(signer: string, actionId?: string) {
    return this.database.appTransaction(async (db) => {
      signer = signer.toLowerCase();
      const previous = await db.select().from(hvExchangeNonces).where(eq(hvExchangeNonces.signer, signer)).get();
      const nonce = Math.max(Date.now(), (previous?.nonce ?? 0) + 1);
      await db.insert(hvExchangeNonces).values({ signer, nonce }).onConflictDoUpdate({ target: hvExchangeNonces.signer, set: { nonce } }).run();
      if (actionId) await db.update(hvOrderActions).set({ nonce }).where(eq(hvOrderActions.id, actionId)).run();
      return nonce;
    });
  }
  async recover() {
    for (const action of await this.db.select().from(hvOrderActions).where(eq(hvOrderActions.state, "submitting")).all()) {
      const views = (await this.items(action.id)).map(({ view }) => ({ ...JSON.parse(view) as OrderView, commandStatus: "unknown" as const, orderStatus: "unknown", source: "restart", updatedAt: Date.now(), error: "Process stopped during submission. Reconciliation required; do not resubmit." }));
      await this.outcome(action.id, "unknown", views, "restart");
    }
  }
  async pendingRequests(limit = 20) {
    const rows = await this.db.select({ request: hvOrderRequests }).from(hvOrderRequests)
      .innerJoin(hvOrderActions, eq(hvOrderActions.requestId, hvOrderRequests.id))
      .innerJoin(hvOrderItems, eq(hvOrderItems.actionId, hvOrderActions.id))
      .where(eq(hvOrderItems.active, 1)).orderBy(asc(hvOrderItems.updatedAt)).limit(limit).all();
    return [...new Map(rows.map(({ request }) => [request.id, request])).values()];
  }
  async local(agentId: string, account: string) {
    // Bounded local history scan. Live open orders are queried separately.
    const requests = await this.db.select().from(hvOrderRequests).where(and(eq(hvOrderRequests.agentId, agentId), eq(hvOrderRequests.account, account), eq(hvOrderRequests.kind, "submit"))).orderBy(desc(hvOrderRequests.createdAt)).limit(500).all();
    return (await Promise.all(requests.map((request) => this.allItems(request.id)))).flat();
  }
  async find(agentId: string, account: string, id: string) {
    const rows = await this.db.select({ item: hvOrderItems, request: hvOrderRequests }).from(hvOrderItems)
      .innerJoin(hvOrderActions, eq(hvOrderActions.id, hvOrderItems.actionId)).innerJoin(hvOrderRequests, eq(hvOrderRequests.id, hvOrderActions.requestId))
      .where(and(eq(hvOrderRequests.agentId, agentId), eq(hvOrderRequests.account, account), or(eq(hvOrderItems.id, id), eq(hvOrderItems.cloid, id), eq(hvOrderItems.oid, id)))).orderBy(desc(hvOrderRequests.createdAt)).get();
    return rows;
  }
  events(id: string) { return this.db.select({ id: hvOrderEvents.id, source: hvOrderEvents.source, time: hvOrderEvents.time, data: hvOrderEvents.data }).from(hvOrderEvents).where(eq(hvOrderEvents.itemId, id)).orderBy(desc(hvOrderEvents.id)).limit(100).all(); }
}
