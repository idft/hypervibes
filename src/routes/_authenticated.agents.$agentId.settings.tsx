import { createFileRoute } from "@tanstack/react-router";
import { AgentSettingsPage } from "../components/agent-settings-page.js";
import { useSuspenseQuery } from "@tanstack/react-query";
import { agentChatQueryOptions } from "../web/agent-queries.js";
import { gatewayQueryOptions } from "../web/gateway-queries.js";
import { riskQueryOptions } from "../web/risk-queries.js";

export const Route = createFileRoute("/_authenticated/agents/$agentId/settings")({
  loader: async ({ params, context }) => {
    await Promise.all([context.queryClient.fetchQuery(agentChatQueryOptions(params.agentId)), context.queryClient.ensureQueryData(riskQueryOptions(params.agentId)), context.queryClient.ensureQueryData(gatewayQueryOptions(params.agentId))]);
  },
  component: SettingsRoute,
});
function SettingsRoute() {
  const { agentId } = Route.useParams();
  const { data: chat } = useSuspenseQuery(agentChatQueryOptions(agentId));
  return <AgentSettingsPage key={agentId} initial={chat} />;
}