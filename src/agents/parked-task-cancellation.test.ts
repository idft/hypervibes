import { awaitWithContext, BACKGROUND_CONTEXT as context, withAbortSignal } from "@earendil-works/chord/context";
import { openNodeSqliteDatabase } from "@earendil-works/chord/sqlite/node";
import { createModels } from "@earendil-works/pi-ai/models";
import { createRegistry, defineExtension, defineTask, Harness, type TaskId } from "@earendil-works/pi-durable";
import { SqliteStorage } from "@earendil-works/pi-durable/storage/sqlite";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, it, vi } from "vitest";

// This process must die without Harness.close(): graceful close deliberately does
// not consume the restart budget. All durable state is created through public APIs.
const childSource = `
import { BACKGROUND_CONTEXT as context } from '@earendil-works/chord/context';
import { openNodeSqliteDatabase } from '@earendil-works/chord/sqlite/node';
import { createModels } from '@earendil-works/pi-ai/models';
import { createRegistry, defineExtension, defineTask, Harness } from '@earendil-works/pi-durable';
import { SqliteStorage } from '@earendil-works/pi-durable/storage/sqlite';
process.on('message', () => {});
const task = defineTask({
  name: 'hv.cancellation-regression', version: 1,
  initial: () => ({ phase: 'effect' }),
  phases: { effect: async (record) => {
    process.send({ type: 'running', id: record.id });
    await new Promise(() => {});
  } },
  abort: async (_record, runtime, ctx) => {
    await runtime.commit(() => ({ status: 'terminal', outcome: { status: 'aborted' } }), ctx);
  },
});
const registry = createRegistry();
registry.install(defineExtension({ name: 'cancellation-regression', tasks: [task] }));
const database = await openNodeSqliteDatabase(process.env.CANCELLATION_TEST_DB);
const harness = await Harness.open(await SqliteStorage.open(database), {
  models: createModels(), registry, settings: { restartLimit: 1 },
}, context);
const root = await harness.root(context);
await root.commit(tx => tx.createTask(task, null, { ownership: { kind: 'conversation' } }), context);
harness.resume();
`;

it("settles a genuinely restart-parked task through abort without replay", async () => {
  const directory = await mkdtemp(join(tmpdir(), "hv-parked-cancellation-"));
  const path = join(directory, "tasks.sqlite");
  const child = spawn(process.execPath, ["--input-type=module", "--eval", childSource], {
    cwd: process.cwd(), env: { ...process.env, CANCELLATION_TEST_DB: path },
    stdio: ["ignore", "pipe", "pipe", "ipc"],
  });
  let stderr = "";
  child.stderr?.on("data", (chunk: Buffer) => { stderr += chunk.toString(); });
  const exited = once(child, "exit");
  let database: Awaited<ReturnType<typeof openNodeSqliteDatabase>> | undefined;
  let harness: Harness | undefined;
  try {
    const ready = new Promise<number>((resolve, reject) => {
      child.on("message", (message: unknown) => {
        if (typeof message === "object" && message !== null && "type" in message && message.type === "running" && "id" in message && typeof message.id === "number") resolve(message.id);
      });
      child.once("error", reject);
      child.once("exit", (code, signal) => reject(new Error(`Fixture exited before its barrier: ${code}/${signal}: ${stderr}`)));
    });
    const readiness = withAbortSignal(AbortSignal.timeout(10_000), context);
    const id = await awaitWithContext(ready, readiness) as TaskId<null>;
    // The IPC barrier runs inside the phase, after the running reservation is durable.
    child.kill("SIGKILL");
    expect(await exited).toEqual([null, "SIGKILL"]);

    const replay = vi.fn(async () => { throw new Error("Parked work must not replay"); });
    const abort = vi.fn();
    const task = defineTask<null, { phase: "effect" }, null>({
      name: "hv.cancellation-regression", version: 1,
      initial: () => ({ phase: "effect" }), phases: { effect: replay },
      abort: async (_record, runtime, ctx) => {
        abort();
        await runtime.commit(() => ({ status: "terminal", outcome: { status: "aborted" } }), ctx);
      },
    });
    const registry = createRegistry();
    registry.install(defineExtension({ name: "cancellation-regression", tasks: [task] }));
    database = await openNodeSqliteDatabase(path);
    harness = await Harness.open(await SqliteStorage.open(database), {
      models: createModels(), registry, settings: { restartLimit: 1 },
    }, context);
    const parked = await harness.inspect(context);
    expect(parked.scheduling).toBe("paused");
    expect(parked.tasks.find((task) => task.record.id === id)?.state).toMatchObject({ kind: "blocked", reason: "restart_limit" });
    expect(await harness.abortTask(id, context)).toBe("marked");
    expect((await harness.getTask(id, context))?.abortRequested).toBe(true);
    expect(abort).not.toHaveBeenCalled();
    expect((await harness.inspect(context)).scheduling).toBe("paused");

    // waitForTask resumes scheduling; abortTask alone only marks while paused.
    const cancellation = withAbortSignal(AbortSignal.timeout(10_000), context);
    const settled = await awaitWithContext(harness.waitForTask(id, cancellation), cancellation);
    expect(settled.state).toMatchObject({ status: "terminal", outcome: { status: "aborted" } });
    expect(replay).not.toHaveBeenCalled();
    expect(abort).toHaveBeenCalledTimes(1);
  } finally {
    if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
    await exited;
    await harness?.close(context);
    await database?.close();
    await rm(directory, { recursive: true, force: true });
  }
}, 30_000);
