import { createFileRoute } from "@tanstack/react-router";
import { AgentChartPage } from "../components/agent-chart-page.js";
import { getChartInstruments, getChartLayouts } from "../web/charts.js";
import { getAgentChat } from "../web/agents.js";
import { getModelChoices } from "../web/providers.js";

export const Route = createFileRoute("/_authenticated/agents/$agentId/chart")({
  loader: async ({ params }) => {
    const data = { id: params.agentId };
    const [instruments, layouts, chat, choices] = await Promise.all([getChartInstruments({ data }), getChartLayouts({ data }), getAgentChat({ data }), getModelChoices()]);
    return { instruments, layouts, chat, choices };
  },
  component: ChartRoute,
});

function ChartRoute() {
  const { instruments, layouts, chat, choices } = Route.useLoaderData();
  const { agentId } = Route.useParams();
  if (!instruments.ok) return <p role="alert" className="text-amber-200">{instruments.error}</p>;
  if (!layouts.ok) return <p role="alert" className="text-amber-200">{layouts.error}</p>;
  if (!chat.ok) return <p role="alert" className="text-amber-200">{chat.error}</p>;
  return <AgentChartPage key={agentId} id={agentId} instruments={instruments.data} layouts={layouts.data} initialChat={chat.data} choices={choices} />;
}
