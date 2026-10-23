import { awaitWithContext, BACKGROUND_CONTEXT as context, withAbortSignal } from "@earendil-works/chord/context";
import { createModels } from "@earendil-works/pi-ai/models";
import { createRegistry, defineExtension, defineTask, Harness, type TaskId } from "@earendil-works/pi-durable";
import { MemoryStorage } from "@earendil-works/pi-durable/storage/memory";
import { expect, it, vi } from "vitest";
import { cancelTaskScope } from "./cancel-task-scope.js";

it("rescans late background work through terminal owners without stopping unrelated tasks", async () => {
  const bounded = withAbortSignal(AbortSignal.timeout(5_000), context);
  let release!: () => void, entered!: () => void;
  const proceed = new Promise<void>((resolve) => { release = resolve; });
  const started = new Promise<void>((resolve) => { entered = resolve; });
  const leaf = defineTask<null, { phase: "sleep" }, null>({
    name: "hv.scope-leaf", version: 1, initial: () => ({ phase: "sleep" }),
    phases: { sleep: async (_task, runtime, ctx) => { await runtime.sleepUntil(runtime.now() + 60_000, { phase: "sleep" }, ctx); } },
    abort: async (_task, runtime, ctx) => { await runtime.commit(() => ({ status: "terminal", outcome: { status: "aborted" } }), ctx); },
  });
  let child: TaskId<null> | undefined;
  const owner = defineTask<null, { phase: "work" }, null>({
    name: "hv.scope-owner", version: 1, initial: () => ({ phase: "work" }),
    phases: { work: async (task, runtime, ctx) => {
      entered();
      await proceed;
      await runtime.commit(async (tx) => {
        const conversation = await tx.createConversation({ ownership: { kind: "task", taskId: task.id } });
        child = await tx.createTask(leaf, null, { conversationId: conversation.id, ownership: { kind: "conversation" }, background: true });
        return undefined;
      }, ctx);
      await runtime.commit(() => ({ status: "terminal", outcome: { status: "completed", result: null } }), ctx);
    } },
    abort: async (_task, runtime, ctx) => { await runtime.commit(() => ({ status: "terminal", outcome: { status: "aborted" } }), ctx); },
  });
  const registry = createRegistry();
  registry.install(defineExtension({ name: "scope-test", tasks: [leaf, owner] }));
  const harness = await Harness.open(new MemoryStorage(), { models: createModels(), registry }, context);
  try {
    const root = await harness.root(context);
    const target = await root.commit((tx) => tx.createTask(owner, null, { ownership: { kind: "conversation" } }), context);
    const unrelated = await root.commit((tx) => tx.createTask(leaf, null, { ownership: { kind: "conversation" }, background: true }), context);
    harness.resume();
    await awaitWithContext(started, bounded);
    const taskGraph = harness.taskGraph.bind(harness);
    vi.spyOn(harness, "taskGraph").mockImplementationOnce(async (ctx) => {
      const graph = await taskGraph(ctx);
      const snapshot = graph.value;
      expect(Object.keys(snapshot.tasks)).toHaveLength(2);
      // Create a background descendant after the cancellation snapshot, then let its owner settle.
      release();
      await harness.waitForTask(target, bounded);
      return new Proxy(graph, { get(state, key) {
        if (key === "value") return snapshot;
        if (key === "dispose") return () => graph.dispose();
        return Reflect.get(state, key, state) as unknown;
      } });
    });
    await cancelTaskScope(harness, [target], [], bounded);
    expect(child).toBeDefined();
    expect((await harness.getTask(child!, context))!.state).toMatchObject({ status: "terminal", outcome: { status: "aborted" } });
    expect((await harness.getTask(unrelated, context))!.state.status).not.toBe("terminal");
  } finally { release(); vi.restoreAllMocks(); await harness.close(context); }
});
