import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  createApp: vi.fn(),
  app: {
    listen: vi.fn(),
    close: vi.fn(),
    shutdown: vi.fn(),
    log: { error: vi.fn(), info: vi.fn() },
  },
}));

vi.mock("./web/app.js", () => ({ createApp: mocks.createApp }));
vi.mock("./config.js", () => ({
  loadConfig: () => ({ host: "127.0.0.1", port: 3003 }),
}));

import { main } from "./main.js";

// All server methods are mocked: these tests never open a socket or database.
describe("server entry point", () => {
  const signals = ["SIGINT", "SIGTERM"] as const;
  let originalListeners: Map<string, NodeJS.SignalsListener[]>;
  let originalExitCode: typeof process.exitCode;

  beforeEach(() => {
    vi.resetAllMocks();
    mocks.app.listen.mockResolvedValue(undefined);
    mocks.app.close.mockResolvedValue(undefined);
    mocks.app.shutdown.mockImplementation(() => mocks.app.close());
    mocks.createApp.mockResolvedValue(mocks.app);
    originalExitCode = process.exitCode;
    originalListeners = new Map(signals.map((signal) => [signal, process.listeners(signal)]));
  });
  afterEach(() => {
    for (const signal of signals) {
      for (const listener of process.listeners(signal)) {
        if (!originalListeners.get(signal)?.includes(listener)) process.off(signal, listener);
      }
    }
    process.exitCode = originalExitCode;
  });

  it("listens using config, then closes only once on repeated shutdown signals", async () => {
    await main(["--dev"]);
    expect(mocks.createApp).toHaveBeenCalledWith({ host: "127.0.0.1", port: 3003 }, { logger: true, development: true, forceShutdown: expect.any(AbortSignal) });
    expect(mocks.app.listen).toHaveBeenCalledWith({ host: "127.0.0.1", port: 3003 });
    const stop = process.listeners("SIGTERM").find((listener) =>
      !originalListeners.get("SIGTERM")?.includes(listener));
    expect(stop).toBeDefined();
    stop?.("SIGTERM");
    stop?.("SIGTERM");
    await vi.waitFor(() => {
      expect(mocks.app.shutdown).toHaveBeenCalledTimes(1);
      expect(mocks.app.close).toHaveBeenCalledTimes(1);
      expect(mocks.app.log.info).toHaveBeenLastCalledWith("Shutdown complete");
      expect(process.listeners("SIGTERM")).toEqual(originalListeners.get("SIGTERM"));
      expect(process.listeners("SIGINT")).toEqual(originalListeners.get("SIGINT"));
    });
  });

  it("logs the graceful wait and forces shutdown on a second signal", async () => {
    await main();
    const force = (mocks.createApp.mock.calls[0]![1] as { forceShutdown: AbortSignal }).forceShutdown;
    expect(force.aborted).toBe(false);
    let finishClose!: () => void;
    mocks.app.close.mockReturnValue(new Promise<void>((resolve) => { finishClose = resolve; }));
    const stop = process.listeners("SIGINT").find((listener) =>
      !originalListeners.get("SIGINT")?.includes(listener));
    stop?.("SIGINT");
    await vi.waitFor(() => expect(mocks.app.close).toHaveBeenCalledTimes(1));
    expect(mocks.app.log.info).toHaveBeenCalledWith("Shutdown signal received; waiting for in-flight agent jobs to complete");
    stop?.("SIGINT");
    expect(force.aborted).toBe(true);
    expect(mocks.app.log.info).toHaveBeenCalledWith("Second shutdown signal received; forcing shutdown");
    expect(mocks.app.close).toHaveBeenCalledTimes(1);
    expect(mocks.app.log.info).not.toHaveBeenCalledWith("Shutdown complete");
    finishClose();
    await vi.waitFor(() => expect(mocks.app.log.info).toHaveBeenLastCalledWith("Shutdown complete"));
  });

  it("closes resources and removes signal handlers when listening fails", async () => {
    const failure = new Error("address already in use");
    mocks.app.listen.mockRejectedValue(failure);
    await expect(main()).rejects.toBe(failure);
    expect(mocks.app.close).toHaveBeenCalledTimes(1);
    for (const signal of signals) {
      expect(process.listeners(signal)).toEqual(originalListeners.get(signal));
    }
  });

  it("propagates initialization failure without installing signal handlers", async () => {
    const failure = new Error("cannot open database");
    mocks.createApp.mockRejectedValue(failure);
    await expect(main()).rejects.toBe(failure);
    expect(mocks.app.listen).not.toHaveBeenCalled();
    for (const signal of signals) {
      expect(process.listeners(signal)).toEqual(originalListeners.get(signal));
    }
  });

  it("logs shutdown errors and sets a failing exit code", async () => {
    const failure = new Error("close failed");
    mocks.app.close.mockRejectedValue(failure);
    await main();
    const stop = process.listeners("SIGINT").find((listener) =>
      !originalListeners.get("SIGINT")?.includes(listener));
    stop?.("SIGINT");
    await vi.waitFor(() => {
      expect(mocks.app.log.error).toHaveBeenCalledWith(failure, "Shutdown failed");
      expect(mocks.app.log.info).not.toHaveBeenCalledWith("Shutdown complete");
      expect(process.exitCode).toBe(1);
      expect(process.listeners("SIGINT")).toEqual(originalListeners.get("SIGINT"));
    });
  });
});
