import { beforeEach, describe, expect, it, vi } from "vitest";
const mocks = vi.hoisted(() => ({ transports: [] as { socket: EventTarget & { readyState: number }; close: ReturnType<typeof vi.fn> }[], subscriptions: [] as { unsubscribe: ReturnType<typeof vi.fn> }[] }));
vi.mock("@nktkas/hyperliquid", () => {
  class WebSocketTransport {
    socket = Object.assign(new EventTarget(), { readyState: 1 });
    close = vi.fn();
    constructor() { mocks.transports.push(this); }
  }
  const subscribe = async () => { const subscription = { unsubscribe: vi.fn(async () => {}) }; mocks.subscriptions.push(subscription); return subscription; };
  class SubscriptionClient {
    candle = subscribe;
    clearinghouseState = subscribe;
    spotState = subscribe;
    openOrders = subscribe;
  }
  return { WebSocketTransport, SubscriptionClient };
});
import { SharedHyperliquidFeeds } from "./shared-feeds.js";

describe("App-owned exchange socket", () => {
  beforeEach(() => { mocks.transports.length = 0; mocks.subscriptions.length = 0; });
  it("lazily shares account and candle transport and explicitly unsubscribes without closing other feeds", async () => {
    const feeds = new SharedHyperliquidFeeds();
    expect(mocks.transports).toHaveLength(0);
    const events = { connected: vi.fn(), disconnected: vi.fn(), failed: vi.fn(), update: vi.fn() };
    const account = feeds.account(`0x${"12".repeat(20)}`, events);
    const candle = feeds.candle({ coin: "BTC", interval: "1h" }, events);
    await Promise.resolve();
    expect(mocks.transports).toHaveLength(1);
    expect(mocks.subscriptions).toHaveLength(4);
    await candle.close();
    expect(mocks.subscriptions[3]!.unsubscribe).toHaveBeenCalledOnce();
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
