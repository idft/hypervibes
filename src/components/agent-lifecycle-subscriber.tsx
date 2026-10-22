import { useQueryClient } from "@tanstack/react-query";
import type { AgentRow } from "../db/agent-schema.js";
import { agentQueryOptions, agentsQueryOptions } from "../web/agent-queries.js";
import { useAgentEvent } from "./agent-events.js";

/** Keep controls and composers consistent when another tab disables the agent. */
export function AgentLifecycleSubscriber({ agentId }: { agentId: string }) {
  const client = useQueryClient();
  useAgentEvent("agent", (event) => {
    const agent = JSON.parse(event.data) as AgentRow;
    if (agent.id !== agentId) return;
    client.setQueryData(agentQueryOptions(agentId).queryKey, agent);
    client.setQueryData(agentsQueryOptions().queryKey, (previous) => previous?.map((item) => item.id === agentId ? { ...item, ...agent } : item));
    client.setQueryData<import("../agents/agent-service.js").AgentChat>(["agent-chat", agentId], (previous) => previous ? { ...previous, agent } : previous);
  });
  return null;
}
