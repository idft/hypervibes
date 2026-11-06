import { WebSocketTransport } from "@nktkas/hyperliquid";

export interface TwapSnapshotReader {
  read(user: string, dex: string, signal: AbortSignal): Promise<{ data: unknown; receivedAt: number }>;
  close(): void;
}

type Snapshot = Awaited<ReturnType<TwapSnapshotReader["read"]>>;
type Subscription = Awaited<ReturnType<WebSocketTransport["subscribe"]>>;

/** Fresh events only: this socket never serves the dashboard or replays cached state. */
export function createTwapSnapshotReader(isTestnet: boolean): TwapSnapshotReader {
  let transport: WebSocketTransport | undefined;
  let closed = false;
  const pending = new Set<(error: Error) => void>();
  const unsubscribe = (subscription: Subscription) => {
    // SDK 0.33.3 detaches the local listener synchronously. The server's
    // acknowledgement must not extend the caller's deadline (including late handles).
    try { void subscription.unsubscribe().catch(() => {}); }
    catch { /* Cleanup failure cannot change the already settled read. */ }
  };

  return {
    async read(user, dex, signal) {
      signal.throwIfAborted();
      if (closed) throw new Error("TWAP snapshot reader is closed");
      transport ??= new WebSocketTransport({ isTestnet, resubscribe: false });
      const socket = transport.socket;
      const subscriptions = transport;
      return new Promise<Snapshot>((resolve, reject) => {
        const abort = new AbortController();
        let finished = false;
        let snapshot: Snapshot | undefined;
        let subscription: Subscription | undefined;
        const cleanup = () => {
          signal.removeEventListener("abort", cancelled);
          socket.removeEventListener("close", disconnected);
          socket.removeEventListener("error", disconnected);
          pending.delete(fail);
          // Aborting only stops a pending SDK confirmation. Confirmed handles
          // require explicit unsubscribe, even when their signal is aborted.
          abort.abort();
          if (subscription) unsubscribe(subscription);
        };
        const fail = (error: unknown) => {
          if (finished) return;
          finished = true;
          cleanup();
          reject(error);
        };
        const complete = () => {
          if (finished || !snapshot || !subscription) return;
          finished = true;
          cleanup();
          resolve(snapshot);
        };
        const cancelled = () => fail(signal.reason);
        const disconnected = () => fail(new Error("TWAP snapshot connection lost"));
        signal.addEventListener("abort", cancelled, { once: true });
        socket.addEventListener("close", disconnected);
        socket.addEventListener("error", disconnected);
        pending.add(fail);
        try {
          // Do not use the SDK's twapStates wrapper: it dereferences unvalidated
          // event.detail before calling the application listener. All reads on
          // this channel receive every frame, so route only after guarding it.
          const task = subscriptions.subscribe<unknown>("twapStates", { type: "twapStates", user, dex }, (event) => {
            if (finished) return;
            const data = event?.detail;
            if (typeof data !== "object" || data === null || Array.isArray(data)
              || !("user" in data) || typeof data.user !== "string"
              || !("dex" in data) || typeof data.dex !== "string") {
              fail(new Error("Malformed TWAP snapshot event envelope"));
              return;
            }
            if (data.user.toLowerCase() !== user.toLowerCase() || data.dex !== dex) return;
            snapshot = { data, receivedAt: Date.now() };
            complete();
          }, { signal: abort.signal, onError: fail });
          void task.then((handle) => {
            if (finished) { unsubscribe(handle); return; }
            subscription = handle;
            complete();
          }, fail);
        } catch (error) { fail(error); }
      });
    },
    close() {
      if (closed) return;
      closed = true;
      for (const fail of pending) fail(new Error("TWAP snapshot reader is closed"));
      transport?.close();
    },
  };
}
