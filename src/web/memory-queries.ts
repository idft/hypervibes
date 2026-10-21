import { queryOptions } from "@tanstack/react-query";
import { getInsights, getMemory, listMemories } from "./memories.js";

export interface MemoryFilters {
  kind?: string; instrument?: string; topic?: string; producerSubagentId?: string;
  status?: string; query?: string; start?: number; end?: number;
  cursor?: { createdAt: number; ref: string };
}
export const memoriesQueryOptions = (agentId: string, filters: MemoryFilters = {}) => queryOptions({
  queryKey: ["memories", agentId, "list", filters],
  queryFn: () => listMemories({ data: { agentId, filters } }),
});
export const memoryQueryOptions = (agentId: string, ref: string) => queryOptions({
  queryKey: ["memories", agentId, "detail", ref],
  queryFn: () => getMemory({ data: { agentId, ref } }),
});
export const insightsQueryOptions = (agentId: string, scopeId = "", beforeVersion?: number) => queryOptions({
  queryKey: ["insights", agentId, scopeId, beforeVersion ?? null],
  queryFn: () => getInsights({ data: { agentId, scope: scopeId ? { kind: "worker", subagentId: scopeId } : { kind: "agent" }, history: true, ...(beforeVersion === undefined ? {} : { beforeVersion }) } }),
});
