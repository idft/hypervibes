import { useQueryClient } from "@tanstack/react-query";
import type { AgentRow } from "../db/agent-schema.js";
import { agentQueryOptions, agentsQueryOptions } from "../web/agent-queries.js";
import { useAgentEvent } from "./agent-events.js";
import { riskQueryOptions } from "../web/risk-queries.js";
import type { DashboardSubagent } from "../agents/subagent-dashboard.js";
import { dashboardSubagentsQueryOptions, subagentsQueryOptions } from "../web/subagent-queries.js";
import { notificationSettingsQueryOptions } from "../web/notification-queries.js";

/** Keep controls, composers, and trading settings consistent across tabs. */
export function AgentLifecycleSubscriber({ agentId }: { agentId: string }) {
  const client = useQueryClient();
  useAgentEvent("agent", (event) => {
    const agent = JSON.parse(event.data) as AgentRow;
    if (agent.id !== agentId) return;
    void client.invalidateQueries({ queryKey: riskQueryOptions(agentId).queryKey });
    client.setQueryData(agentQueryOptions(agentId).queryKey, agent);
    client.setQueryData(agentsQueryOptions().queryKey, (previous) => previous?.map((item) => item.id === agentId ? { ...item, ...agent } : item));
    client.setQueryData<import("../agents/agent-service.js").AgentChat>(["agent-chat", agentId], (previous) => previous ? { ...previous, agent } : previous);
  });
  // The stream also publishes after configuration changes, including create/archive.
  useAgentEvent("subagents", (event) => {
    client.setQueryData(dashboardSubagentsQueryOptions(agentId).queryKey, JSON.parse(event.data) as DashboardSubagent[]);
    void client.invalidateQueries({ queryKey: subagentsQueryOptions(agentId).queryKey });
    void client.invalidateQueries({ queryKey: ["subagents", agentId, "detail"] });
  });
  useAgentEvent("notifications", () => {
    void client.invalidateQueries({ queryKey: notificationSettingsQueryOptions(agentId).queryKey });
  });
  return null;
}
