import type { Context } from "@earendil-works/chord/context";
import type { Harness, TaskId } from "@earendil-works/pi-durable";

/** Cancel through public APIs, including late background descendants and reused conversations. */
export async function cancelTaskScope(harness: Harness, taskIds: readonly number[], conversationIds: readonly number[], context: Context): Promise<void> {
  const roots = new Set(taskIds);
  const conversations = new Set(conversationIds);
  while (true) {
    context.abortSignal?.throwIfAborted();
    const attached = await harness.taskGraph(context);
    const nodes = Object.values(attached.value.tasks);
    attached.dispose();
    // A terminal owner can disappear from the live graph while background work survives.
    // Follow public records rather than assuming that every ancestor is still live.
    const scoped = await harness.commit(async (tx) => {
      const known = new Map<number, boolean>();
      const belongs = async (id: number, seen = new Set<number>()): Promise<boolean> => {
        if (roots.has(id)) return true;
        const cached = known.get(id);
        if (cached !== undefined) return cached;
        if (seen.has(id)) return false;
        seen.add(id);
        const task = await tx.task(id as TaskId);
        if (!task) return false;
        let result: boolean;
        if (task.owner !== undefined) result = await belongs(task.owner, seen);
        else if (conversations.has(task.conversationId)) result = true;
        else {
          const conversation = await tx.conversation(task.conversationId);
          result = conversation?.owner !== undefined && await belongs(conversation.owner.taskId, seen);
        }
        known.set(id, result);
        return result;
      };
      const result = [];
      for (const node of nodes) if (await belongs(node.id)) result.push(node);
      return result;
    }, context);
    if (!scoped.length) return;
    // Mark each task and join its run invocation. A running harness can process marks between calls.
    for (const task of scoped) await harness.abortTask(task.id, context);
    await Promise.all(scoped.map((task) => harness.waitForTask(task.id, context)));
    // Re-scan after joins: a cooperative invocation can create background work before it exits.
  }
}
