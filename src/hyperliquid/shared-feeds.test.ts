import { beforeEach, describe, expect, it, vi } from "vitest";
const mocks = vi.hoisted(() => ({
  transports: [] as { socket: EventTarget & { readyState: number }; close: ReturnType<typeof vi.fn> }[],
  subscriptions: [] as {
    method: string; params: Record<string, unknown>; listener: (event: unknown) => void;
    unsubscribe: ReturnType<typeof vi.fn>;
  }[],
}));
vi.mock("@nktkas/hyperliquid", () => {
  class WebSocketTransport {
    socket = Object.assign(new EventTarget(), { readyState: 1 });
    close = vi.fn();
    constructor() { mocks.transports.push(this); }
  }
  const subscribe = (method: string) => async (params: Record<string, unknown>, listener: (event: unknown) => void) => {
    const subscription = { method, params, listener, unsubscribe: vi.fn(async () => {}) };
    mocks.subscriptions.push(subscription);
    return subscription;
  };
  class SubscriptionClient {
    candle = subscribe("candle");
    clearinghouseState = subscribe("clearinghouseState");
    spotState = subscribe("spotState");
    openOrders = subscribe("openOrders");
    webData3 = subscribe("webData3");
  }
  return { WebSocketTransport, SubscriptionClient };
});
import { SharedHyperliquidFeeds } from "./shared-feeds.js";

describe("App-owned exchange socket", () => {
  beforeEach(() => { mocks.transports.length = 0; mocks.subscriptions.length = 0; });
  it("uses explicit webData3 abstraction, isolates account callbacks, and never infers unified mode", async () => {
    const feeds = new SharedHyperliquidFeeds();
    const address = `0x${"ab".repeat(20)}`;
    const other = `0x${"cd".repeat(20)}`;
    const events = { connected: vi.fn(), disconnected: vi.fn(), failed: vi.fn(), update: vi.fn() };
    const otherEvents = { connected: vi.fn(), disconnected: vi.fn(), failed: vi.fn(), update: vi.fn() };
    const account = feeds.account(address, events);
    const otherAccount = feeds.account(other, otherEvents);
    await Promise.resolve();
    expect(mocks.transports).toHaveLength(1);
    expect(mocks.subscriptions).toHaveLength(8);
    const mode = mocks.subscriptions.find((subscription) => subscription.method === "webData3" && subscription.params.user === address)!;
    const otherMode = mocks.subscriptions.find((subscription) => subscription.method === "webData3" && subscription.params.user === other)!;
    expect(mode.params).toEqual({ user: address });
    for (const abstraction of ["unifiedAccount", "disabled", "portfolioMargin"]) {
      mode.listener({ userState: { user: address.toUpperCase(), abstraction }, perpDexStates: [] });
      expect(events.update).toHaveBeenLastCalledWith("mode", { accountAbstraction: abstraction });
    }
    expect(otherEvents.update).not.toHaveBeenCalled();
    const before = events.update.mock.calls.length;
    mode.listener({ userState: { user: other, abstraction: "unifiedAccount" }, perpDexStates: [] });
    expect(events.update).toHaveBeenCalledTimes(before);
    otherMode.listener({ userState: { user: other, abstraction: "disabled" }, perpDexStates: [] });
    expect(otherEvents.update).toHaveBeenCalledExactlyOnceWith("mode", { accountAbstraction: "disabled" });
    for (const dexAbstractionEnabled of [true, false]) {
      mode.listener({ userState: { user: address, dexAbstractionEnabled }, perpDexStates: [{ totalVaultEquity: "100" }] });
      expect(events.update).toHaveBeenLastCalledWith("mode", { accountAbstraction: null });
    }
    // Repeated omitted abstraction must not start an HTTP repair loop.
    expect(events.update).toHaveBeenCalledTimes(before + 1);
    mode.listener({ userState: { user: address }, perpDexStates: [] });
    expect(events.update).toHaveBeenCalledTimes(before + 1);
    mode.listener({ userState: { user: address, abstraction: "disabled" }, perpDexStates: [] });
    expect(events.update).toHaveBeenCalledTimes(before + 2);
    mode.listener({ userState: { user: address, abstraction: "disabled" }, perpDexStates: [] });
    expect(events.update).toHaveBeenCalledTimes(before + 2);
    expect(events.update.mock.calls.every(([section]) => section === "mode")).toBe(true);
    mocks.transports[0]!.socket.dispatchEvent(new Event("close"));
    mocks.transports[0]!.socket.dispatchEvent(new Event("open"));
    mode.listener({ userState: { user: address, abstraction: "disabled" }, perpDexStates: [] });
    expect(events.update).toHaveBeenCalledTimes(before + 3);
    expect(events.update).toHaveBeenLastCalledWith("mode", { accountAbstraction: "disabled" });
    await account.close();
    expect(mocks.subscriptions.filter((subscription) => subscription.params.user === address).every((subscription) => subscription.unsubscribe.mock.calls.length === 1)).toBe(true);
    expect(mocks.subscriptions.filter((subscription) => subscription.params.user === other).every((subscription) => subscription.unsubscribe.mock.calls.length === 0)).toBe(true);
    await otherAccount.close();
    feeds.close();
  });
  it("lazily shares account and candle transport and explicitly unsubscribes without closing other feeds", async () => {
    const feeds = new SharedHyperliquidFeeds();
    expect(mocks.transports).toHaveLength(0);
    const events = { connected: vi.fn(), disconnected: vi.fn(), failed: vi.fn(), update: vi.fn() };
    const account = feeds.account(`0x${"12".repeat(20)}`, events);
    const candle = feeds.candle({ coin: "BTC", interval: "1h" }, events);
    await Promise.resolve();
    expect(mocks.transports).toHaveLength(1);
    expect(mocks.subscriptions).toHaveLength(5);
    expect(mocks.subscriptions.map((subscription) => subscription.method).sort()).toEqual(["candle", "clearinghouseState", "openOrders", "spotState", "webData3"]);
    await candle.close();
    expect(mocks.subscriptions.find((subscription) => subscription.method === "candle")!.unsubscribe).toHaveBeenCalledOnce();
    expect(mocks.subscriptions[0]!.unsubscribe).not.toHaveBeenCalled();
    expect(mocks.transports[0]!.close).not.toHaveBeenCalled();
    await account.close();
    expect(mocks.subscriptions.every((subscription) => subscription.unsubscribe.mock.calls.length === 1)).toBe(true);
    const calls = events.disconnected.mock.calls.length;
    mocks.transports[0]!.socket.dispatchEvent(new Event("close"));
    expect(events.disconnected).toHaveBeenCalledTimes(calls);
    feeds.close();
    expect(mocks.transports[0]!.close).toHaveBeenCalledOnce();
    expect(() => feeds.candle({ coin: "BTC", interval: "1h" }, events)).toThrow("closed");
  });
});
