import { queryOptions, type QueryClient } from "@tanstack/react-query";
import { getAgent, getAgentChat, getAgentChatHistory, getAgentDashboard, getTradingAccountChoices, listAgents } from "./agents.js";

export const agentsQueryOptions = () => queryOptions({
  queryKey: ["agents"],
  queryFn: () => listAgents(),
});
export const agentQueryOptions = (id: string) => queryOptions({
  queryKey: ["agent", id],
  queryFn: () => getAgent({ data: { id } }),
});
export const agentChatQueryOptions = (id: string) => queryOptions({
  queryKey: ["agent-chat", id],
  queryFn: () => getAgentChat({ data: { id } }),
  // SSE owns the live transcript. Loaders fetch a fresh mount seed.
  staleTime: 0,
  refetchOnMount: false,
});
export const agentHistoryQueryOptions = (id: string, before: number | undefined, context: string) => queryOptions({
  // Never dedupe a pre-reset or unmounted transcript's in-flight page into a new context.
  queryKey: ["agent-history", id, context, before ?? null],
  queryFn: () => getAgentChatHistory({ data: { id, before } }),
  staleTime: 0,
  gcTime: 0,
});
export const agentDashboardQueryOptions = (id: string) => queryOptions({
  queryKey: ["agent-dashboard", id],
  queryFn: () => getAgentDashboard({ data: { id } }),
  staleTime: 0,
  refetchOnMount: false,
});
export const tradingAccountsQueryOptions = () => queryOptions({
  queryKey: ["trading-accounts"],
  queryFn: () => getTradingAccountChoices(),
});

export async function invalidateAgentCollection(client: QueryClient) {
  await Promise.all([
    client.invalidateQueries({ queryKey: agentsQueryOptions().queryKey }),
    client.invalidateQueries({ queryKey: tradingAccountsQueryOptions().queryKey }),
    client.invalidateQueries({ queryKey: ["account"] }),
    client.invalidateQueries({ queryKey: ["setup"] }),
  ]);
}
