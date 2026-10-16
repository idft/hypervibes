import { createFileRoute } from "@tanstack/react-router";
import { AgentLayout } from "../components/agent-layout.js";
import { getAgent } from "../web/agents.js";

export const Route = createFileRoute("/_authenticated/agents/$agentId")({
  loader: ({ params }) => getAgent({ data: { id: params.agentId } }),
  component: AgentRoute,
});

function AgentRoute() {
  const result = Route.useLoaderData();
  return result.ok ? <AgentLayout key={result.data.id} agent={result.data} /> : <p role="alert" className="text-red-300">{result.error}</p>;
}
