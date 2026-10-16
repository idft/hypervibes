import { createFileRoute } from "@tanstack/react-router";
import { AgentDashboardPage } from "../components/agent-dashboard-page.js";
import { getAgentDashboard } from "../web/agents.js";

export const Route = createFileRoute("/_authenticated/agents/$agentId/")({
  loader: ({ params }) => getAgentDashboard({ data: { id: params.agentId } }),
  component: DashboardRoute,
});
function DashboardRoute() {
  const result = Route.useLoaderData();
  const { agentId } = Route.useParams();
  return result.ok ? <AgentDashboardPage key={agentId} id={agentId} initial={result.data} /> : <p role="alert" className="text-red-300">{result.error}</p>;
}
