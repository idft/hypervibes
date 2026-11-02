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

/** Section saves own only their fields; SSE refetches own the full definition. */
export async function cacheSubagent(client: QueryClient, agentId: string, row: SubagentRow, fields?: readonly (keyof SubagentRow)[]) {
  const merge = (current: SubagentRow): SubagentRow => {
    if (!fields) return row;
    // A later complete snapshot also owns the fields saved by this request.
    if (current.updatedAt > row.updatedAt) return current;
    return { ...current, ...Object.fromEntries(fields.map((key) => [key, row[key]])), updatedAt: row.updatedAt };
  };
  client.setQueryData(subagentQueryOptions(agentId, row.id).queryKey, (current) => current ? { ...current, definition: merge(current.definition) } : undefined);
  client.setQueryData(subagentsQueryOptions(agentId).queryKey, (current) => current?.map((entry) => entry.id === row.id ? merge(entry) : entry));
  await invalidateSubagentLists(client, agentId);
  const detail = client.getQueryData(subagentQueryOptions(agentId, row.id).queryKey)?.definition;
  const listed = client.getQueryData(subagentsQueryOptions(agentId).queryKey)?.find((entry) => entry.id === row.id);
  return [detail, listed].reduce<SubagentRow>((latest, candidate) => candidate && candidate.updatedAt >= latest.updatedAt ? candidate : latest, row);
}
