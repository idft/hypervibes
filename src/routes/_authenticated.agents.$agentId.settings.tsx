import { createFileRoute } from "@tanstack/react-router";
import { AgentSettingsPage } from "../components/agent-settings-page.js";
import { getAgentChat } from "../web/agents.js";

export const Route = createFileRoute("/_authenticated/agents/$agentId/settings")({
  loader: ({ params }) => getAgentChat({ data: { id: params.agentId } }),
  component: SettingsRoute,
});
function SettingsRoute() {
  const result = Route.useLoaderData();
  return result.ok ? <AgentSettingsPage key={result.data.agent.id} initial={result.data} /> : <p role="alert" className="text-red-300">{result.error}</p>;
}
