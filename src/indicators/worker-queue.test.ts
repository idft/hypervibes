import { describe, expect, it } from "vitest";
import { IndicatorWorkerQueue } from "./worker-queue.js";

const signal = () => new AbortController().signal;

describe("indicator worker permits", () => {
  it("queues excess calls FIFO and never exceeds global concurrency", async () => {
    const queue = new IndicatorWorkerQueue(2, 4);
    const first = await queue.acquire(signal()), second = await queue.acquire(signal());
    const admitted: number[] = [];
    const third = queue.acquire(signal()).then((release) => { admitted.push(3); return release; });
    const fourth = queue.acquire(signal()).then((release) => { admitted.push(4); return release; });
    await Promise.resolve();
    expect(admitted).toEqual([]);
    first();
    const releaseThird = await third;
    expect(admitted).toEqual([3]);
    second();
    const releaseFourth = await fourth;
    expect(admitted).toEqual([3, 4]);
    releaseThird(); releaseFourth();
  });
  it("bounds the waiting queue and admits new requests after cancellation", async () => {
    const queue = new IndicatorWorkerQueue(1, 1);
    const first = await queue.acquire(signal());
    const controller = new AbortController();
    const waiting = expect(queue.acquire(controller.signal)).rejects.toThrow("cancelled");
    expect(() => queue.acquire(signal())).toThrow("Indicator queue is full");
    controller.abort(new Error("cancelled"));
    await waiting;
    const replacement = queue.acquire(signal());
    first();
    (await replacement)();
  });
  it("removes an aborted waiter without blocking or reordering the remaining calls", async () => {
    const queue = new IndicatorWorkerQueue(1, 3);
    const release = await queue.acquire(signal());
    const controller = new AbortController();
    const aborted = expect(queue.acquire(controller.signal)).rejects.toThrow("deleted");
    const next = queue.acquire(signal());
    controller.abort(new Error("deleted"));
    await aborted;
    release();
    (await next)();
  });
  it("releases each permit only once", async () => {
    const queue = new IndicatorWorkerQueue(1, 2);
    const release = await queue.acquire(signal());
    const next = queue.acquire(signal());
    const controller = new AbortController();
    const last = expect(queue.acquire(controller.signal)).rejects.toThrow("still waiting");
    release(); release();
    const releaseNext = await next;
    controller.abort(new Error("still waiting"));
    await last;
    releaseNext();
  });
  it("rejects already-aborted calls without consuming capacity", async () => {
    const queue = new IndicatorWorkerQueue(1, 1);
    expect(() => queue.acquire(AbortSignal.abort(new Error("stopped")))).toThrow("stopped");
    (await queue.acquire(signal()))();
  });
});
