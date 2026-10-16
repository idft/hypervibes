import { SubscriptionClient, WebSocketTransport } from "@nktkas/hyperliquid";
import type { CandleEvent } from "@nktkas/hyperliquid/api/subscription";
import { openAccountFeed, type AccountFeedFactory } from "./live-dashboard.js";
import type { ChartSelection } from "./chart-contracts.js";

export interface CandleFeedEvents {
  connected(): void;
  disconnected(): void;
  update(candle: CandleEvent): void;
  failed(): void;
}
export type CandleFeedFactory = (selection: ChartSelection, events: CandleFeedEvents) => { close(): Promise<void> };

/** A single lazy, app-owned exchange socket shared by account and candle feeds. */
export class SharedHyperliquidFeeds {
  private transport?: WebSocketTransport;
  private stopped = false;
  private socket() {
    if (this.stopped) throw new Error("Exchange feeds are closed");
    return this.transport ??= new WebSocketTransport();
  }
  readonly account: AccountFeedFactory = (address, events) => openAccountFeed(address, events, this.socket());
  readonly candle: CandleFeedFactory = (selection, events) => {
    const transport = this.socket();
    const abort = new AbortController();
    const connected = () => events.connected();
    const disconnected = () => events.disconnected();
    transport.socket.addEventListener("open", connected);
    transport.socket.addEventListener("close", disconnected);
    if (transport.socket.readyState === 1) queueMicrotask(() => { if (!abort.signal.aborted) connected(); });
    const task = new SubscriptionClient({ transport }).candle(selection, (candle) => { if (!abort.signal.aborted) events.update(candle); }, { signal: abort.signal, onError: () => { if (!abort.signal.aborted) events.failed(); } });
    void task.catch(() => { if (!abort.signal.aborted) events.failed(); });
    return { async close() {
      transport.socket.removeEventListener("open", connected);
      transport.socket.removeEventListener("close", disconnected);
      abort.abort();
      const [result] = await Promise.allSettled([task]);
      if (result?.status === "fulfilled") await result.value.unsubscribe().catch(() => {});
    } };
  };
  close() { this.stopped = true; this.transport?.close(); }
}
