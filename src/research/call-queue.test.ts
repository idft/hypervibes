import { describe, expect, it } from "vitest";
import { CallQueue } from "./call-queue.js";

const deferred = () => { let resolve!: () => void; const promise = new Promise<void>((done) => { resolve = done; }); return { promise, resolve }; };

describe("per-script call queue", () => {
  it("admits FIFO and never runs more than four operations", async () => {
    const queue = new CallQueue(4), barrier = deferred(), entered = deferred();
    let active = 0, peak = 0;
    const order: number[] = [];
    const work = Array.from({ length: 12 }, (_, id) => queue.run(async () => {
      order.push(id); active++; peak = Math.max(peak, active);
      if (active === 4) entered.resolve();
      await barrier.promise; active--; return id;
    }));
    await entered.promise;
    expect(order).toEqual([0, 1, 2, 3]);
    barrier.resolve();
    expect(await Promise.all(work)).toEqual(Array.from({ length: 12 }, (_, id) => id));
    expect(order).toEqual(Array.from({ length: 12 }, (_, id) => id));
    expect(peak).toBe(4);
  });
  it("rejects waiters on close and never promotes them when active work finishes", async () => {
    const queue = new CallQueue(1), barrier = deferred(), entered = deferred();
    const first = queue.run(async () => { entered.resolve(); await barrier.promise; });
    await entered.promise;
    let dispatched = false;
    const waiting = queue.run(async () => { dispatched = true; });
    const rejected = expect(waiting).rejects.toThrow("deadline");
    queue.close(new Error("deadline")); await rejected;
    barrier.resolve(); await first;
    await expect(queue.run(async () => {})).rejects.toThrow("deadline");
    expect(dispatched).toBe(false);
  });
  it("does not dispatch reserved work if closed before its microtask starts", async () => {
    const queue = new CallQueue(4);
    let dispatched = false;
    const work = queue.run(async () => { dispatched = true; });
    queue.close(new Error("cancelled"));
    await expect(work).rejects.toThrow("cancelled");
    expect(dispatched).toBe(false);
  });
  it("releases capacity after operation failures", async () => {
    const queue = new CallQueue(1);
    const first = queue.run(async () => { throw new Error("tool failed"); });
    const second = queue.run(async () => "next");
    await expect(first).rejects.toThrow("tool failed");
    expect(await second).toBe("next");
  });
});
