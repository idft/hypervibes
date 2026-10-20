import { createFileRoute } from "@tanstack/react-router";
import { AgentDashboardPage } from "../components/agent-dashboard-page.js";
import { getAgentDashboard } from "../web/agents.js";
import { getDashboardSubagents } from "../web/subagents.js";

export const Route = createFileRoute("/_authenticated/agents/$agentId/")({
  loader: async ({ params }) => {
    const [dashboard, subagents] = await Promise.all([
      getAgentDashboard({ data: { id: params.agentId } }),
      getDashboardSubagents({ data: { agentId: params.agentId } }),
    ]);
    return { dashboard, subagents };
  },
  component: DashboardRoute,
});
function DashboardRoute() {
  const { dashboard: result, subagents } = Route.useLoaderData();
  const { agentId } = Route.useParams();
  return result.ok ? <AgentDashboardPage key={agentId} id={agentId} initial={result.data} subagents={subagents.ok ? subagents.data : []} subagentsError={subagents.ok ? undefined : subagents.error} /> : <p role="alert" className="text-red-300">{result.error}</p>;
}
