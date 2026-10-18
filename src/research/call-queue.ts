/** Per-script FIFO admission. Closing rejects waiters; active operations own their cancellation. */
export class CallQueue {
  private active = 0;
  private readonly waiting: Array<{ start: () => void; reject: (reason: unknown) => void }> = [];
  private failure: { reason: unknown } | undefined;
  constructor(private readonly concurrency: number) {
    if (!Number.isSafeInteger(concurrency) || concurrency < 1) throw new Error("Invalid call concurrency");
  }
  get closed() { return this.failure !== undefined; }
  get saturated() { return this.active >= this.concurrency; }
  run<T>(operation: () => Promise<T>): Promise<T> {
    if (this.failure) return Promise.reject(this.failure.reason);
    return new Promise<T>((resolve, reject) => {
      const start = () => {
        this.active++;
        void Promise.resolve().then(() => {
          if (this.failure) throw this.failure.reason;
          return operation();
        }).then(resolve, reject).finally(() => {
          this.active--;
          if (!this.failure) this.waiting.shift()?.start();
        });
      };
      if (this.active < this.concurrency) start();
      else this.waiting.push({ start, reject });
    });
  }
  close(reason: unknown) {
    if (this.failure) return;
    this.failure = { reason };
    for (const waiter of this.waiting.splice(0)) waiter.reject(reason);
  }
}
