import { getEventListeners } from "node:events";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  transports: [] as { options: unknown; socket: EventTarget; close: ReturnType<typeof vi.fn> }[],
  calls: [] as {
    channel: string;
    params: { type: string; user: string; dex: string };
    listener: (data: unknown) => void;
    frame: (event: CustomEvent<unknown>) => void;
    options: { signal: AbortSignal; onError: (error: Error) => void };
    acknowledge: () => void;
    reject: (error: Error) => void;
    unsubscribe: ReturnType<typeof vi.fn>;
  }[],
  synchronousError: false,
}));
vi.mock("@nktkas/hyperliquid", () => ({
  WebSocketTransport: class {
    socket = new EventTarget();
    close = vi.fn();
    constructor(public options: unknown) { mocks.transports.push(this); }
    subscribe(
      channel: string,
      params: { type: string; user: string; dex: string },
      frame: (event: CustomEvent<unknown>) => void,
      options: { signal: AbortSignal; onError: (error: Error) => void },
    ) {
      if (mocks.synchronousError) throw new Error("Invalid subscription");
      return new Promise<{ unsubscribe: () => Promise<void> }>((resolve, reject) => {
        const unsubscribe = vi.fn(async () => {});
        const listener = (data: unknown) => frame(new CustomEvent(channel, { detail: data }));
        mocks.calls.push({ channel, params, listener, frame, options, acknowledge: () => resolve({ unsubscribe }), reject, unsubscribe });
      });
    }
  },
}));
import { createTwapSnapshotReader, type TwapSnapshotReader } from "./twap-snapshot.js";

const user = `0x${"ab".repeat(20)}`;
const event = { user, dex: "", states: [] };
const readers: TwapSnapshotReader[] = [];
function reader(isTestnet = false) {
  const result = createTwapSnapshotReader(isTestnet);
  readers.push(result);
  return result;
}
function expectClean(signal: AbortSignal) {
  expect(getEventListeners(signal, "abort")).toHaveLength(0);
  for (const { socket } of mocks.transports) {
    expect(getEventListeners(socket, "close")).toHaveLength(0);
    expect(getEventListeners(socket, "error")).toHaveLength(0);
  }
}

describe("fresh TWAP snapshot reader", () => {
  beforeEach(() => { mocks.transports.length = 0; mocks.calls.length = 0; mocks.synchronousError = false; });
  afterEach(() => { for (const value of readers.splice(0)) value.close(); vi.restoreAllMocks(); });

  it.each([false, true])("lazily creates one dedicated socket with isTestnet=%s", async (isTestnet) => {
    const value = reader(isTestnet);
    expect(mocks.transports).toHaveLength(0);
    const signal = new AbortController().signal;
    const first = value.read(user, "", signal);
    const second = value.read(user, "xyz", signal);
    expect(mocks.transports).toHaveLength(1);
    expect(mocks.transports[0]!.options).toEqual({ isTestnet, resubscribe: false });
    expect(mocks.calls.map(({ channel, params }) => ({ channel, params }))).toEqual([
      { channel: "twapStates", params: { type: "twapStates", user, dex: "" } },
      { channel: "twapStates", params: { type: "twapStates", user, dex: "xyz" } },
    ]);
    for (const call of mocks.calls) { call.listener({ ...event, dex: call.params.dex }); call.acknowledge(); }
    await Promise.all([first, second]);
    expectClean(signal);
    value.close(); value.close();
    expect(mocks.transports[0]!.close).toHaveBeenCalledOnce();
  });

  it("requires acknowledgement and retains the event receipt time", async () => {
    const now = vi.spyOn(Date, "now").mockReturnValue(100);
    const signal = new AbortController().signal;
    const result = reader().read(user, "", signal);
    const settled = vi.fn();
    void result.then(settled);
    const call = mocks.calls[0]!;
    call.listener(event);
    await Promise.resolve();
    expect(settled).not.toHaveBeenCalled();
    now.mockReturnValue(500);
    call.acknowledge();
    expect(await result).toEqual({ data: event, receivedAt: 100 });
    expect(call.unsubscribe).toHaveBeenCalledOnce();
    expect(call.options.signal.aborted).toBe(true);
    expectClean(signal);
  });

  it("requires an event after acknowledgement and returns raw data", async () => {
    const value = reader();
    const signal = new AbortController().signal;
    const result = value.read(user, "", signal);
    const call = mocks.calls[0]!;
    call.acknowledge();
    await Promise.resolve();
    expect(call.unsubscribe).not.toHaveBeenCalled();
    const raw = { ...event, states: "malformed: validated by account-status" };
    call.listener(raw);
    expect((await result).data).toBe(raw);
    expectClean(signal);
  });

  it("does not replay a prior event to another read of the same account and DEX", async () => {
    const value = reader();
    const controller = new AbortController();
    const first = value.read(user, "", controller.signal);
    mocks.calls[0]!.listener(event);
    mocks.calls[0]!.acknowledge();
    await first;
    const second = value.read(user, "", controller.signal);
    mocks.calls[1]!.acknowledge();
    await Promise.resolve();
    const failure = expect(second).rejects.toThrow("deadline");
    controller.abort(new Error("deadline"));
    await failure;
    expect(mocks.calls[1]!.unsubscribe).toHaveBeenCalledOnce();
    expectClean(controller.signal);
  });

  it.each([false, true])("rejects caller abort promptly with acknowledgement=%s", async (acknowledged) => {
    const controller = new AbortController();
    const result = reader().read(user, "", controller.signal);
    const call = mocks.calls[0]!;
    if (acknowledged) { call.acknowledge(); await Promise.resolve(); }
    else call.listener(event);
    const failure = expect(result).rejects.toThrow("deadline");
    controller.abort(new Error("deadline"));
    await failure;
    expect(call.options.signal.aborted).toBe(true);
    expectClean(controller.signal);
    // An injected transport can ignore abort and return a handle later.
    if (!acknowledged) { call.acknowledge(); await Promise.resolve(); }
    expect(call.unsubscribe).toHaveBeenCalledOnce();
    call.listener(event);
  });

  it("rejects subscription failure even after receiving an event", async () => {
    const signal = new AbortController().signal;
    const result = reader().read(user, "", signal);
    const failure = expect(result).rejects.toThrow("subscription refused");
    mocks.calls[0]!.listener(event);
    mocks.calls[0]!.reject(new Error("subscription refused"));
    await failure;
    expect(mocks.calls[0]!.options.signal.aborted).toBe(true);
    expectClean(signal);
  });

  it("handles synchronous subscription errors without retaining listeners", async () => {
    mocks.synchronousError = true;
    const signal = new AbortController().signal;
    await expect(reader().read(user, "", signal)).rejects.toThrow("Invalid subscription");
    expectClean(signal);
  });

  it("rejects confirmed subscription errors", async () => {
    const signal = new AbortController().signal;
    const result = reader().read(user, "", signal);
    const failure = expect(result).rejects.toThrow("subscription lost");
    const call = mocks.calls[0]!;
    call.acknowledge();
    await Promise.resolve();
    call.options.onError(new Error("subscription lost"));
    await failure;
    expect(call.unsubscribe).toHaveBeenCalledOnce();
    expectClean(signal);
  });

  it.each(["close", "error"])("rejects socket %s before acknowledgement and cleans late handles", async (type) => {
    const signal = new AbortController().signal;
    const result = reader().read(user, "", signal);
    const failure = expect(result).rejects.toThrow("connection lost");
    mocks.transports[0]!.socket.dispatchEvent(new Event(type));
    await failure;
    mocks.calls[0]!.acknowledge();
    await Promise.resolve();
    expect(mocks.calls[0]!.unsubscribe).toHaveBeenCalledOnce();
    expectClean(signal);
  });

  it.each(["pending", "rejected", "throws"])("does not await %s unsubscribe cleanup", async (behavior) => {
    const signal = new AbortController().signal;
    const result = reader().read(user, "", signal);
    const call = mocks.calls[0]!;
    call.unsubscribe.mockImplementation(() => {
      if (behavior === "throws") throw new Error("cleanup failed");
      if (behavior === "rejected") return Promise.reject(new Error("cleanup failed"));
      return new Promise<void>(() => {});
    });
    call.listener(event); call.acknowledge();
    expect((await result).data).toBe(event);
    expectClean(signal);
  });

  it("close rejects all outstanding reads, aborts subscriptions, and prevents reuse", async () => {
    const value = reader();
    const signal = new AbortController().signal;
    const first = value.read(user, "", signal);
    const second = value.read(user, "xyz", signal);
    const failures = [expect(first).rejects.toThrow("closed"), expect(second).rejects.toThrow("closed")];
    mocks.calls[0]!.acknowledge();
    await Promise.resolve();
    value.close();
    await Promise.all(failures);
    expect(mocks.calls.every(({ options }) => options.signal.aborted)).toBe(true);
    mocks.calls[1]!.acknowledge();
    await Promise.resolve();
    expect(mocks.calls.every(({ unsubscribe }) => unsubscribe.mock.calls.length === 1)).toBe(true);
    expectClean(signal);
    await expect(value.read(user, "", signal)).rejects.toThrow("closed");
    expect(mocks.transports[0]!.close).toHaveBeenCalledOnce();
  });

  it.each([
    null, undefined, [], "invalid", {}, { user }, { dex: "" },
    { user: null, dex: "" }, { user, dex: null },
  ])("rejects malformed event data %j without throwing from the callback", async (data) => {
    const signal = new AbortController().signal;
    const result = reader().read(user, "", signal);
    const failure = expect(result).rejects.toThrow("Malformed TWAP snapshot event envelope");
    const call = mocks.calls[0]!;
    call.acknowledge();
    await Promise.resolve();
    expect(() => call.frame(new CustomEvent("twapStates", { detail: data }))).not.toThrow();
    await failure;
    expect(call.unsubscribe).toHaveBeenCalledOnce();
    expectClean(signal);
  });

  it("rejects an event with no detail before acknowledgement and cleans its late handle", async () => {
    const signal = new AbortController().signal;
    const result = reader().read(user, "", signal);
    const failure = expect(result).rejects.toThrow("Malformed TWAP snapshot event envelope");
    const call = mocks.calls[0]!;
    expect(() => call.frame(new CustomEvent("twapStates"))).not.toThrow();
    await failure;
    expect(call.options.signal.aborted).toBe(true);
    call.acknowledge();
    await Promise.resolve();
    expect(call.unsubscribe).toHaveBeenCalledOnce();
    expectClean(signal);
  });

  it("routes shared channel frames only to the requested account and DEX", async () => {
    const value = reader();
    const signal = new AbortController().signal;
    const first = value.read(user, "", signal);
    const second = value.read(user, "xyz", signal);
    for (const call of mocks.calls) call.acknowledge();
    await Promise.resolve();
    const publish = (data: unknown) => {
      const frame = new CustomEvent("twapStates", { detail: data });
      for (const call of mocks.calls) call.frame(frame);
    };
    publish({ ...event, user: `0x${"cd".repeat(20)}` });
    publish({ ...event, dex: "unrelated" });
    expect(mocks.calls.every(({ unsubscribe }) => unsubscribe.mock.calls.length === 0)).toBe(true);
    const xyz = { ...event, dex: "xyz" };
    publish(xyz);
    expect((await second).data).toBe(xyz);
    expect(mocks.calls[0]!.unsubscribe).not.toHaveBeenCalled();
    const main = { ...event, user: user.toUpperCase() };
    publish(main);
    expect((await first).data).toBe(main);
    expect(mocks.calls.every(({ unsubscribe }) => unsubscribe.mock.calls.length === 1)).toBe(true);
    expectClean(signal);
  });

  it("does not open a socket for a cancelled read or an unused closed reader", async () => {
    const cancelled = AbortSignal.abort(new Error("cancelled"));
    await expect(reader().read(user, "", cancelled)).rejects.toThrow("cancelled");
    const value = reader();
    value.close();
    await expect(value.read(user, "", new AbortController().signal)).rejects.toThrow("closed");
    expect(mocks.transports).toHaveLength(0);
  });
});
