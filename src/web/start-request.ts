import { NodeRequest } from "srvx/node";

/** Preserve srvx's streaming/body limits while classifying ordinary disconnects
 * as HTTP cancellations, not unexpected failures in Start's error handler. */
export class StartRequest extends NodeRequest {
  private disconnectSignal?: AbortSignal;

  override get signal(): AbortSignal {
    if (!this.disconnectSignal) {
      const original = super.signal;
      const controller = new AbortController();
      const abort = () => {
        const reason: unknown = original.reason;
        controller.abort(reason instanceof DOMException && reason.name === "AbortError"
          ? Object.assign(new DOMException("Client closed request", "AbortError"), { status: 499 })
          : reason);
      };
      this.disconnectSignal = controller.signal;
      if (original.aborted) abort();
      else original.addEventListener("abort", abort, { once: true });
    }
    return this.disconnectSignal;
  }
}
