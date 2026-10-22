import { IncomingMessage, ServerResponse } from "node:http";
import { Socket } from "node:net";
import { describe, expect, it } from "vitest";
import { StartRequest } from "./start-request.js";

function fixture() {
  const req = new IncomingMessage(new Socket());
  req.method = "GET"; req.url = "/"; req.headers.host = "localhost";
  const res = new ServerResponse(req);
  return { req, res, request: new StartRequest({ req, res, maxRequestBodySize: 1024 }) };
}

describe("Start HTTP disconnect handling", () => {
  it("classifies a disconnected client as cancellation before downstream listeners run", () => {
    const { res, request } = fixture();
    const signal = request.signal;
    expect(request.signal).toBe(signal);
    let observed: unknown;
    signal.addEventListener("abort", () => { observed = signal.reason; });
    res.emit("close");
    expect(signal.aborted).toBe(true);
    expect(observed).toBeInstanceOf(DOMException);
    expect(observed).toMatchObject({ name: "AbortError", status: 499 });
  });

  it("recognizes clients that disconnected before the signal was accessed", () => {
    const { res, request } = fixture();
    res.destroy();
    expect(request.signal.aborted).toBe(true);
    expect(request.signal.reason).toMatchObject({ name: "AbortError", status: 499 });
  });

  it("does not relabel unexpected transport failures as ordinary cancellation", () => {
    const { req, res, request } = fixture();
    const failure = new Error("Unexpected socket failure");
    Object.defineProperty(req, "errored", { value: failure });
    const signal = request.signal;
    res.emit("close");
    expect(signal.reason).toBe(failure);
    expect(signal.reason).not.toHaveProperty("status");
  });

  it("does not abort when a completed response closes", () => {
    const { res, request } = fixture();
    const signal = request.signal;
    res.end(); res.emit("close");
    expect(signal.aborted).toBe(false);
  });
});
