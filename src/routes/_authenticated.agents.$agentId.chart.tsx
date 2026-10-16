import { createFileRoute } from "@tanstack/react-router";
import { AgentChartPage } from "../components/agent-chart-page.js";
import { getChartInstruments } from "../web/charts.js";

export const Route = createFileRoute("/_authenticated/agents/$agentId/chart")({
  loader: ({ params }) => getChartInstruments({ data: { id: params.agentId } }),
  component: ChartRoute,
});

function ChartRoute() {
  const result = Route.useLoaderData();
  const { agentId } = Route.useParams();
  return result.ok ? <AgentChartPage key={agentId} id={agentId} instruments={result.data} /> : <p role="alert" className="text-amber-200">{result.error}</p>;
}
