import { createFileRoute } from "@tanstack/react-router";
import { AgentSettingsPage } from "../components/agent-settings-page.js";
import { getAgentChat } from "../web/agents.js";
import { getModelChoices } from "../web/providers.js";

export const Route = createFileRoute("/_authenticated/agents/$agentId/settings")({
  loader: async ({ params }) => ({ result: await getAgentChat({ data: { id: params.agentId } }), choices: await getModelChoices() }),
  component: SettingsRoute,
});
function SettingsRoute() {
  const { result, choices } = Route.useLoaderData();
  return result.ok ? <AgentSettingsPage key={result.data.agent.id} initial={result.data} choices={choices} /> : <p role="alert" className="text-red-300">{result.error}</p>;
}
