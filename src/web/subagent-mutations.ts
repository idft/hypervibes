import type { QueryClient } from "@tanstack/react-query";
import type { SubagentSections } from "../agents/subagent-input.js";
import { cacheSubagent, invalidateSubagentLists } from "./subagent-queries.js";
import { runSubagent, saveSubagentSection, stopSubagentRun } from "./subagents.js";

/** Shared server/cache effects; callers retain drafts, feedback and retry IDs. */
export function subagentMutations(client: QueryClient, agentId: string) {
  return {
    async saveSection<K extends keyof SubagentSections>(input: { id: string; section: K; settings: SubagentSections[K] }) {
      const row = await saveSubagentSection({ data: { ...input, agentId } });
      await cacheSubagent(client, agentId, row);
      return row;
    },
    async run(input: { id: string; task: string; requestId: string }) {
      const run = await runSubagent({ data: { ...input, agentId } });
      await invalidateSubagentLists(client, agentId);
      return run;
    },
    async stop(runId: string) {
      await stopSubagentRun({ data: { agentId, runId } });
      await invalidateSubagentLists(client, agentId);
    },
  };
}
