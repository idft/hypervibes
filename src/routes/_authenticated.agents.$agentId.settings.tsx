import { createFileRoute } from "@tanstack/react-router";
import { AgentSettingsPage } from "../components/agent-settings-page.js";
import { getAgentChat } from "../web/agents.js";
import { getRiskSettings } from "../web/risk.js";

export const Route = createFileRoute("/_authenticated/agents/$agentId/settings")({
  loader: async ({ params }) => ({ chat: await getAgentChat({ data: { id: params.agentId } }), risk: await getRiskSettings({ data: { agentId: params.agentId } }) }),
  component: SettingsRoute,
});
function SettingsRoute() {
  const { chat, risk } = Route.useLoaderData();
  if (!chat.ok) return <p role="alert" className="text-red-300">{chat.error}</p>;
  if (!risk.ok) return <p role="alert" className="text-red-300">{risk.error}</p>;
  return <AgentSettingsPage key={chat.data.agent.id} initial={chat.data} risk={risk.data} />;
}