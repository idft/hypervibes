import { BACKGROUND_CONTEXT as context } from "@earendil-works/chord/context";
import { defineExtension } from "@earendil-works/pi-durable";
import { describe, expect, it, vi } from "vitest";
import { createSubagentExtension, RunReceipt } from "./subagent-extension.js";

type Service = Parameters<typeof createSubagentExtension>[0];
type Execution = Parameters<typeof createSubagentExtension>[4];
type Runner = ReturnType<typeof createSubagentExtension>["runner"]["definition"];
type Runtime = Parameters<Runner["phases"]["execute"]>[1];

function fixture() {
  const run = { id: "run", task: "Observe", createdAt: 1000, status: "running", source: "delegation", result: "Evidence", config: { timeoutSeconds: 10, name: "Worker" } };
  const service = {
    internalRun: vi.fn(async () => run), started: vi.fn(async () => {}),
    finish: vi.fn(async () => {}), reported: vi.fn(async () => {}),
    reportWhenReady: vi.fn(async () => false),
  };
  const runner = createSubagentExtension(service as unknown as Service, "agent", defineExtension({ name: "test" }), [], {} as Execution).runner.definition;
  const wait = vi.fn(async () => ({ status: "done", type: "input", answer: 1 }));
  const child = { submit: vi.fn(async () => ({ wait })), abort: vi.fn(async () => {}) };
  const cancel = vi.fn();
  let fire = () => {};
  const runtime = {
    now: vi.fn(() => 4000), conversationId: 1, signal: new AbortController().signal,
    conversation: vi.fn(async () => child), commit: vi.fn(async () => {}),
    sleepUntil: vi.fn(async () => {}),
    setTimer: vi.fn((callback: () => void) => { fire = callback; return cancel; }),
  };
  const executeTask = { id: 2, input: { runId: "run" }, state: { checkpoint: { phase: "execute", child: 3 } } } as Parameters<Runner["phases"]["execute"]>[0];
  const execute = () => runner.phases.execute(executeTask, runtime as unknown as Runtime, context);
  return { runner, service, run, runtime, child, wait, cancel, execute, fire: () => fire() };
}

describe("subagent runner Durable 1.4 compatibility", () => {
  it("preserves the task version and checkpoint shapes", () => {
    const { runner } = fixture();
    expect(runner.version).toBe(1);
    expect(runner.initial({ runId: "run" })).toEqual({ phase: "start" });
    expect(runner.initial({ runId: "run", reportOnly: true })).toEqual({ phase: "report" });
    expect(RunReceipt.definition.checkpointWhen({ taskId: null }, [], { deltasSinceBase: 0 })).toBe(false);
  });

  it("disarms the fixed admission deadline after completion", async () => {
    const f = fixture();
    await f.execute();
    expect(f.runtime.setTimer).toHaveBeenCalledWith(expect.any(Function), 7000);
    expect(f.cancel).toHaveBeenCalledOnce();
    expect(f.service.finish).toHaveBeenCalledWith("run", "completed", "");
    expect(f.child.abort).not.toHaveBeenCalled();
  });

  it("disarms the deadline when the submission wait rejects", async () => {
    const f = fixture();
    const error = new Error("Invocation ended");
    f.wait.mockRejectedValueOnce(error);
    await expect(f.execute()).rejects.toBe(error);
    expect(f.cancel).toHaveBeenCalledOnce();
    expect(f.service.finish).not.toHaveBeenCalled();
  });

  it.each([[10, "10-second"], [60, "1-minute"], [900, "15-minute"]] as const)("reports the configured %s-second limit when the deadline wins", async (timeoutSeconds, timeLimit) => {
    const f = fixture();
    f.run.config.timeoutSeconds = timeoutSeconds;
    f.wait.mockImplementationOnce(() => new Promise(() => {}));
    const executing = f.execute();
    await vi.waitFor(() => expect(f.runtime.setTimer).toHaveBeenCalledOnce());
    f.fire();
    await executing;
    expect(f.cancel).toHaveBeenCalledOnce();
    expect(f.child.abort).toHaveBeenCalledWith(context, { background: true });
    expect(f.service.finish).toHaveBeenCalledWith("run", "failed", `Run stopped after reaching its ${timeLimit} time limit. The agent had not finished.`);
  });

  it("does not submit an assignment whose deadline has passed", async () => {
    const f = fixture();
    f.runtime.now.mockReturnValue(11000);
    await f.execute();
    expect(f.child.submit).not.toHaveBeenCalled();
    expect(f.runtime.setTimer).not.toHaveBeenCalled();
    expect(f.child.abort).toHaveBeenCalledOnce();
  });

  it("commits a durable report retry and returns without marking delivery", async () => {
    const f = fixture();
    const task = { id: 2, input: { runId: "run" }, state: { checkpoint: { phase: "report" } } } as Parameters<Runner["phases"]["report"]>[0];
    await f.runner.phases.report(task, f.runtime as unknown as Runtime, context);
    expect(f.runtime.sleepUntil).toHaveBeenCalledWith(34000, { phase: "report" }, context);
    expect(f.service.reported).not.toHaveBeenCalled();
    expect(f.runtime.commit).not.toHaveBeenCalled();
    expect(f.runtime.setTimer).not.toHaveBeenCalled();
  });
});
