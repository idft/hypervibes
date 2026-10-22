import { IndicatorError } from "./validation.js";

/** Global FIFO permits. The owner releases a permit only when its worker exits. */
export class IndicatorWorkerQueue {
  private active = 0;
  private readonly waiting: { start: () => void; cancel: () => void }[] = [];
  constructor(private readonly concurrency: number, private readonly maxWaiting: number) {}

  acquire(signal: AbortSignal): Promise<() => void> {
    signal.throwIfAborted();
    if (this.active >= this.concurrency && this.waiting.length >= this.maxWaiting) {
      throw new IndicatorError("Indicator queue is full; retry later", 429, "indicator_busy");
    }
    return new Promise((resolve, reject) => {
      const waiter = {
        start: () => {
          signal.removeEventListener("abort", waiter.cancel);
          this.active++;
          let released = false;
          resolve(() => {
            if (released) return;
            released = true;
            this.active--;
            this.waiting.shift()?.start();
          });
        },
        cancel: () => {
          const index = this.waiting.indexOf(waiter);
          if (index < 0) return;
          this.waiting.splice(index, 1);
          signal.removeEventListener("abort", waiter.cancel);
          reject(signal.reason);
        },
      };
      if (this.active < this.concurrency) waiter.start();
      else {
        this.waiting.push(waiter);
        signal.addEventListener("abort", waiter.cancel, { once: true });
      }
    });
  }
}
