import { notifyManager } from "@tanstack/react-query";

// Awaited React act() callbacks observe microtasks, not Query's default timers.
notifyManager.setScheduler((callback) => queueMicrotask(callback));
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

// jsdom does not implement the native dialog lifecycle.
if (typeof HTMLDialogElement !== "undefined") {
  Object.defineProperties(HTMLDialogElement.prototype, {
    showModal: { configurable: true, value(this: HTMLDialogElement) { this.open = true; } },
    close: { configurable: true, value(this: HTMLDialogElement) { this.open = false; } },
  });
}
