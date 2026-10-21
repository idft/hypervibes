import { createFileRoute } from "@tanstack/react-router";
import { AgentChatPage } from "../components/agent-chat-page.js";
import { agentChatQueryOptions } from "../web/agent-queries.js";
import { useSuspenseQuery } from "@tanstack/react-query";
import { modelChoicesQueryOptions } from "../web/provider-queries.js";

export const Route = createFileRoute("/_authenticated/agents/$agentId/chat")({
  loader: async ({ params, context }) => {
    await Promise.all([context.queryClient.fetchQuery(agentChatQueryOptions(params.agentId)), context.queryClient.ensureQueryData(modelChoicesQueryOptions())]);
  },
  component: ChatRoute,
});
function ChatRoute() {
  const { agentId } = Route.useParams();
  const { data: chat } = useSuspenseQuery(agentChatQueryOptions(agentId));
  const { data: choices } = useSuspenseQuery(modelChoicesQueryOptions());
  return <AgentChatPage key={agentId} initial={chat} choices={choices} />;
}