import type { CodemodeExecuteOptions, CodemodeResult, CodemodeSandboxOptions } from "@earendil-works/pi-codemode";
import { afterEach, describe, expect, it, vi } from "vitest";
import { IndicatorRuntime } from "./runtime.js";

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}
interface ControlledWorker {
  options: CodemodeSandboxOptions;
  executionOptions?: CodemodeExecuteOptions;
  result: ReturnType<typeof deferred<CodemodeResult>>;
  exited: ReturnType<typeof deferred<void>>;
  release?: () => void;
}
const control = vi.hoisted(() => ({ workers: [] as ControlledWorker[], deferLaunch: false }));

// Separate script-result settlement from actual worker exit, including a launch
// still loading WASM. Real-worker numerical/burst coverage lives in runtime.test.ts.
vi.mock("@earendil-works/pi-codemode", () => ({
  CodemodeSandbox: class {
    readonly worker: ControlledWorker;
    constructor(options: CodemodeSandboxOptions) {
      this.worker = { options, result: deferred<CodemodeResult>(), exited: deferred<void>() };
      control.workers.push(this.worker);
    }
    async execute(_code: string, options: CodemodeExecuteOptions) {
      const worker = this.worker;
      worker.executionOptions = options;
      if (!control.deferLaunch) {
        worker.release = worker.options.acquireWorker?.();
        worker.options.onWorkerStarted?.(worker.exited.promise);
      }
      const abort = () => worker.result.resolve({ ok: false, error: { kind: "aborted", message: "Execution aborted" }, output: [], calls: [] });
      options.signal?.addEventListener("abort", abort, { once: true });
      if (options.signal?.aborted) abort();
      try { return await worker.result.promise; }
      finally { options.signal?.removeEventListener("abort", abort); }
    }
    close() { return Promise.resolve(); }
  },
}));

const request = { source: '//@version=6\nindicator("Queue test")\nplot(close)', coin: "BTC", pineTimeframe: "60", intervalMs: 3600_000,
  candles: [{ open: 1, high: 2, low: 0, close: 1, volume: 1, openTime: 0, closeTime: 3600_000 }], inputs: {} };
const runtimes: IndicatorRuntime[] = [];
async function create(limits: ConstructorParameters<typeof IndicatorRuntime>[0] = { workers: 1 }) {
  const runtime = new IndicatorRuntime(limits);
  runtimes.push(runtime);
  await runtime.initialize();
  return runtime;
}
const attempt = (runtime: IndicatorRuntime, agent = "a", signal?: AbortSignal) => runtime.execute(agent, request, signal).catch((error: unknown) => error);
const flush = async () => { for (let i = 0; i < 10; i++) await Promise.resolve(); };
function finish(worker: ControlledWorker) {
  worker.result.resolve({ ok: false, error: { kind: "script", message: "Test calculation finished" }, output: [], calls: [] });
}
function exit(worker: ControlledWorker) { worker.release?.(); worker.exited.resolve(); }
afterEach(async () => {
  const closing = runtimes.splice(0).map((runtime) => runtime.close());
  for (const worker of control.workers) exit(worker);
  await Promise.all(closing);
  control.workers.length = 0;
  control.deferLaunch = false;
  vi.useRealTimers();
});

describe("indicator runtime queue lifecycle", () => {
  it("holds capacity until actual exit, not script completion or sandbox close", async () => {
    const runtime = await create();
    const first = attempt(runtime);
    await flush();
    finish(control.workers[0]!);
    expect(await first).toMatchObject({ message: expect.stringContaining("Test calculation finished") });
    const second = attempt(runtime);
    await flush();
    expect(control.workers).toHaveLength(1);
    exit(control.workers[0]!);
    await flush();
    expect(control.workers).toHaveLength(2);
    finish(control.workers[1]!); exit(control.workers[1]!);
    await second;
  });
  it("cancels waiting calls without launching a worker or consuming the next permit", async () => {
    const runtime = await create();
    const first = attempt(runtime);
    const controller = new AbortController();
    const cancelled = attempt(runtime, "a", controller.signal);
    const next = attempt(runtime);
    await flush();
    controller.abort(new Error("Caller cancelled"));
    expect(await cancelled).toMatchObject({ message: "Caller cancelled" });
    expect(control.workers).toHaveLength(1);
    finish(control.workers[0]!); exit(control.workers[0]!);
    await first; await flush();
    expect(control.workers).toHaveLength(2);
    finish(control.workers[1]!); exit(control.workers[1]!);
    await next;
  });
  it("applies the deadline to queue wait and recovers after timed-out workers exit", async () => {
    const runtime = await create({ workers: 1, timeoutMs: 100 });
    vi.useFakeTimers();
    const first = attempt(runtime);
    await flush();
    await vi.advanceTimersByTimeAsync(50);
    const waiting = attempt(runtime);
    await vi.advanceTimersByTimeAsync(100);
    expect(await first).toMatchObject({ code: "indicator_timeout" });
    expect(await waiting).toMatchObject({ code: "indicator_timeout", status: 408 });
    expect(control.workers).toHaveLength(1);
    exit(control.workers[0]!);
    const next = attempt(runtime);
    await flush();
    expect(control.workers).toHaveLength(2);
    finish(control.workers[1]!); exit(control.workers[1]!);
    await next;
  });
  it("subtracts queue waiting from the VM execution deadline", async () => {
    const runtime = await create({ workers: 1, timeoutMs: 100 });
    vi.useFakeTimers();
    const first = attempt(runtime);
    await flush();
    const waiting = attempt(runtime);
    await vi.advanceTimersByTimeAsync(60);
    finish(control.workers[0]!); exit(control.workers[0]!);
    await first; await flush();
    expect(control.workers).toHaveLength(2);
    expect(control.workers[1]!.executionOptions?.timeoutMs).toBe(40);
    await vi.advanceTimersByTimeAsync(40);
    expect(await waiting).toMatchObject({ code: "indicator_timeout" });
    exit(control.workers[1]!);
  });
  it("deletes an Agent's active and waiting calls without waiting for another Agent", async () => {
    const runtime = await create();
    const first = attempt(runtime, "a"), waiting = attempt(runtime, "a"), other = attempt(runtime, "b");
    await flush();
    let deleted = false;
    const deletion = runtime.abortAgent("a").then(() => { deleted = true; });
    expect(await first).toMatchObject({ message: "Agent deleted" });
    expect(await waiting).toMatchObject({ message: "Agent deleted" });
    await flush();
    expect(deleted).toBe(false);
    exit(control.workers[0]!);
    await deletion; await flush();
    expect(control.workers).toHaveLength(2);
    expect(await attempt(runtime, "a")).toMatchObject({ status: 503 });
    finish(control.workers[1]!); exit(control.workers[1]!);
    await other;
  });
  it("cancels queued work on shutdown and waits for actual worker exit", async () => {
    const runtime = await create();
    const first = attempt(runtime), waiting = attempt(runtime);
    await flush();
    let closed = false;
    const closing = runtime.close().then(() => { closed = true; });
    expect(await first).toMatchObject({ message: "Indicator runtime closed" });
    expect(await waiting).toMatchObject({ message: "Indicator runtime closed" });
    expect(control.workers).toHaveLength(1);
    expect(closed).toBe(false);
    exit(control.workers[0]!);
    await closing;
    expect(await attempt(runtime)).toMatchObject({ status: 503 });
  });
  it("does not start a late WASM launch after cancellation released its permit", async () => {
    const runtime = await create();
    control.deferLaunch = true;
    const controller = new AbortController();
    const cancelled = attempt(runtime, "a", controller.signal);
    await flush();
    controller.abort(new Error("Stopped before launch"));
    await cancelled;
    control.deferLaunch = false;
    const next = attempt(runtime);
    await flush();
    expect(control.workers).toHaveLength(2);
    expect(() => control.workers[0]!.options.acquireWorker?.()).toThrow("Stopped before launch");
    finish(control.workers[1]!); exit(control.workers[1]!);
    await next;
  });
  it("bounds the queue without rejecting ordinary capacity contention", async () => {
    const runtime = await create({ workers: 1, queuedCalls: 1 });
    const first = attempt(runtime), waiting = attempt(runtime), overflow = attempt(runtime);
    expect(await overflow).toMatchObject({ status: 429, code: "indicator_busy", message: "Indicator queue is full; retry later" });
    await flush();
    finish(control.workers[0]!); exit(control.workers[0]!);
    await first; await flush();
    finish(control.workers[1]!); exit(control.workers[1]!);
    await waiting;
  });
});
