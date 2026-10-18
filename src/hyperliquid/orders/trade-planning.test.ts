import { describe, expect, it } from "vitest";
import type { FrontendOpenOrdersResponse, InfoClient } from "@nktkas/hyperliquid";
import { inspectProtection, verifyProtection } from "./protection.js";
import { preparePositionProtection, prepareProtectedEntry, protectionPrices, type TradingWorkflow } from "./trade-planning.js";
import { notionalWithin } from "./order-math.js";
const policy: TradingWorkflow = { version: 1, purpose: "trading", instruments: ["BTC"], automaticEntriesEnabled: false, requiredResearch: [], entryStrategy: "Ready", exitStrategy: "Invalidation", baseSizeByInstrument: { BTC: "0.001" }, maxEntryNotional: "101", takeProfitBps: 200, stopLossBps: 100 };
const info = { meta: async () => ({ universe: [{ name: "BTC", szDecimals: 5 }] }), allMids: async () => ({ BTC: "100000" }) } as unknown as InfoClient;
const signal = new AbortController().signal;
describe("exact protected-entry planning and honest protection inspection", () => {
  it("calculates from the guarded IOC string price and preserves explicit quantities without Number rounding", async () => {
    const draft = await prepareProtectedEntry(info, { instrument: "BTC", side: "buy", entry_style: "guarded_ioc" }, policy, false, signal);
    expect(draft.orders[0]).toMatchObject({ symbol: "BTC", price: "100500", size: "0.001", time_in_force: "ioc", take_profits: [{ trigger_price: "102510", size: "0.001" }], stop_losses: [{ trigger_price: "99495", size: "0.001" }] });
    expect(draft.validUntil - draft.observedAt).toBe(15000);
    expect(draft.quoteSource).toContain("not venue timestamp");
    expect(protectionPrices("100000", false, 5, 200, 100)).toEqual({ tp: "98000", sl: "101000" });
    expect(notionalWithin("9007199254740993.0000001", "0.0000001", "900719925.47409930000001")).toBe(true);
    expect(notionalWithin("9007199254740993.0000002", "0.0000001", "900719925.47409930000001")).toBe(false);
  });
  it("fails closed without automatic policy or explicit manual protection, and enforces sizing caps", async () => {
    await expect(prepareProtectedEntry(info, { instrument: "BTC", side: "buy", entry_style: "guarded_ioc", size: "0.001" }, null, false, signal)).rejects.toThrow("workflow");
    await expect(prepareProtectedEntry(info, { instrument: "BTC", side: "buy", entry_style: "guarded_ioc", size: "0.001" }, null, true, signal)).rejects.toThrow();
    await expect(prepareProtectedEntry(info, { instrument: "BTC", side: "buy", entry_style: "guarded_ioc", size: "0.002" }, policy, true, signal)).rejects.toThrow("cap");
    await expect(prepareProtectedEntry(info, { instrument: "BTC", side: "buy", entry_style: "guarded_ioc", take_profit_price: "110000" }, policy, false, signal)).rejects.toThrow("saved workflow");
    const manual = await prepareProtectedEntry(info, { instrument: "BTC", side: "buy", entry_style: "limit", limit_price: "100000", size: "0.001", take_profit_price: "110000", stop_loss_price: "90000" }, null, true, signal);
    expect(manual.orders[0]!.size).toBe("0.001");
    await expect(prepareProtectedEntry(info, { instrument: "BTC", side: "buy", entry_style: "guarded_ioc", size: "0.001\n", take_profit_price: "110000", stop_loss_price: "90000" }, null, true, signal)).rejects.toThrow();
  });
  it("plans actual partial-position protection without enlarging quantity, and refuses crossed levels", async () => {
    const positionInfo = { ...info, clearinghouseState: async () => ({ assetPositions: [{ position: { coin: "BTC", szi: "0.0005", entryPx: "100000" } }] }) } as unknown as InfoClient;
    const draft = await preparePositionProtection(positionInfo, { instrument: "BTC" }, policy, false, "0xaccount", signal);
    expect(draft.orders).toMatchObject([{ order_type: "take_profit", size: "0.0005", side: "sell", reduce_only: true, trigger_price: "102000" }, { order_type: "stop_loss", size: "0.0005", side: "sell", reduce_only: true, trigger_price: "99000" }]);
    await expect(preparePositionProtection({ ...positionInfo, allMids: async () => ({ BTC: "103000" }) } as unknown as InfoClient, { instrument: "BTC" }, policy, false, "0xaccount", signal)).rejects.toThrow("crossed");
    await expect(preparePositionProtection(positionInfo, { instrument: "BTC", stop_loss_price: "95000" }, policy, true, "0xaccount", signal)).rejects.toThrow("maximum loss");
  });
  it("uses newer individual leg quantity/status instead of stale frontend protection fields", async () => {
    const fields = { limitPx: "100000", timestamp: 1, triggerCondition: "Trigger condition", isPositionTpsl: true, tif: null, cloid: null };
    const orders: FrontendOpenOrdersResponse = [{ ...fields, coin: "BTC", oid: 1, side: "A", sz: "0.001", origSz: "0.001", orderType: "Take Profit Market", isTrigger: true, reduceOnly: true, triggerPx: "110000", children: [] }, { ...fields, coin: "BTC", oid: 2, side: "A", sz: "0.001", origSz: "0.001", orderType: "Stop Market", isTrigger: true, reduceOnly: true, triggerPx: "90000", children: [] }];
    const statuses = { orderStatus: async ({ oid }: { oid: number }) => ({ status: "order", order: { order: { ...orders.find((order) => order.oid === oid)!, sz: oid === 2 ? "0.0001" : "0.001" }, status: "open", statusTimestamp: 1 } }) } as unknown as InfoClient;
    const result = await verifyProtection(statuses, "0xaccount", [{ coin: "BTC", size: "0.001" }], orders, { BTC: "100000" }, signal);
    expect(result[0]!.covered).toBe(false);
    expect(result[0]!.stopLossOids).toEqual([]);
  });
  it("does not count native waiting-for-fill children, partial size, wrong side or stop-limit as full protection", () => {
    const leg = (oid: number, type: string, trigger: string, patch = {}) => ({ coin: "BTC", side: "A", sz: "0.001", origSz: "0.001", reduceOnly: true, isTrigger: true, orderType: type, oid, triggerPx: trigger, children: [], ...patch });
    const tp = leg(1, "Take Profit Market", "110000"), sl = leg(2, "Stop Market", "90000");
    const positions = [{ coin: "BTC", size: "0.001" }], quotes = { BTC: "100000" };
    const inspect = (rows: unknown[], statuses = new Map([[1, "open"], [2, "open"]])) => inspectProtection(positions, rows as FrontendOpenOrdersResponse, statuses, quotes)[0]!;
    expect(inspect([tp, sl]).covered).toBe(true);
    expect(inspect([tp, sl], new Map([[1, "waitingForFill"], [2, "waitingForFill"]])).covered).toBe(false);
    expect(inspect([{ ...tp, sz: "0.0005" }, sl]).covered).toBe(false);
    expect(inspect([tp, { ...sl, side: "B" }]).covered).toBe(false);
    expect(inspect([tp, { ...sl, orderType: "Stop Limit" }]).covered).toBe(false);
    expect(inspect([{ coin: "BTC", side: "B", sz: "0.001", isTrigger: false, reduceOnly: false, children: [tp, sl] }]).covered).toBe(false);
    expect(inspect([tp, { ...sl, triggerPx: "120000" }]).covered).toBe(false);
  });
});
