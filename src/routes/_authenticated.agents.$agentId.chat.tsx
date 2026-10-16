import { createFileRoute } from "@tanstack/react-router";
import { AgentChatPage } from "../components/agent-chat-page.js";
import { getAgentChat } from "../web/agents.js";

export const Route = createFileRoute("/_authenticated/agents/$agentId/chat")({
  loader: ({ params }) => getAgentChat({ data: { id: params.agentId } }),
  component: ChatRoute,
});
function ChatRoute() {
  const result = Route.useLoaderData();
  return result.ok ? <AgentChatPage key={result.data.agent.id} initial={result.data} /> : <p role="alert" className="text-red-300">{result.error}</p>;
}
