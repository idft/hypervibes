import { queryOptions, type QueryClient } from "@tanstack/react-query";
import type { SubagentRow } from "../db/subagent-schema.js";
import { getDashboardSubagents, getSubagent, getSubagentRun, listSubagents } from "./subagents.js";

export const subagentsQueryOptions = (agentId: string) => queryOptions({
  queryKey: ["subagents", agentId, "list"],
  queryFn: () => listSubagents({ data: { agentId } }),
});
export const subagentQueryOptions = (agentId: string, id: string) => queryOptions({
  queryKey: ["subagents", agentId, "detail", id],
  queryFn: () => getSubagent({ data: { agentId, id } }),
});
export const dashboardSubagentsQueryOptions = (agentId: string) => queryOptions({
  queryKey: ["subagents", agentId, "dashboard"],
  queryFn: () => getDashboardSubagents({ data: { agentId } }),
  staleTime: 0,
  refetchOnMount: false,
});
export const subagentRunQueryOptions = (agentId: string, id: string, runId: string) => queryOptions({
  queryKey: ["subagents", agentId, "run", id, runId],
  queryFn: () => getSubagentRun({ data: { agentId, id, runId } }),
  // The loader fetches a fresh seed; SSE owns the mounted transcript.
  staleTime: 0,
  refetchOnMount: false,
});
export type SubagentDetail = Awaited<ReturnType<typeof getSubagent>>;

export async function invalidateSubagentLists(client: QueryClient, agentId: string) {
  await Promise.all([
    client.invalidateQueries({ queryKey: subagentsQueryOptions(agentId).queryKey }),
    client.invalidateQueries({ queryKey: dashboardSubagentsQueryOptions(agentId).queryKey, refetchType: "none" }),
    client.invalidateQueries({ queryKey: ["subagents", agentId, "detail"], refetchType: "none" }),
    client.invalidateQueries({ queryKey: ["agent-chat", agentId], refetchType: "none" }),
  ]);
}

/** Save responses are authoritative definitions, never copies of a local draft. */
export async function cacheSubagent(client: QueryClient, agentId: string, row: SubagentRow) {
  client.setQueryData(subagentQueryOptions(agentId, row.id).queryKey, (current) => current ? { ...current, definition: row } : undefined);
  client.setQueryData(subagentsQueryOptions(agentId).queryKey, (current) => current?.map((entry) => entry.id === row.id ? row : entry));
  await invalidateSubagentLists(client, agentId);
}
