import { createFileRoute } from "@tanstack/react-router";
import { AgentDashboardPage } from "../components/agent-dashboard-page.js";
import { useQuery, useSuspenseQuery } from "@tanstack/react-query";
import { agentDashboardQueryOptions } from "../web/agent-queries.js";
import { dashboardSubagentsQueryOptions } from "../web/subagent-queries.js";

export const Route = createFileRoute("/_authenticated/agents/$agentId/")({
  loader: async ({ params, context }) => {
    await Promise.all([
      context.queryClient.fetchQuery(agentDashboardQueryOptions(params.agentId)),
      // Worker activity is optional: preserve the account dashboard on failure.
      context.queryClient.prefetchQuery(dashboardSubagentsQueryOptions(params.agentId)),
    ]);
  },
  component: DashboardRoute,
});
function DashboardRoute() {
  const { agentId } = Route.useParams();
  const { data: dashboard } = useSuspenseQuery(agentDashboardQueryOptions(agentId));
  const { data: subagents, error } = useQuery(dashboardSubagentsQueryOptions(agentId));
  return <AgentDashboardPage key={agentId} id={agentId} initial={dashboard} subagents={subagents} subagentsError={error?.message} />;
}
