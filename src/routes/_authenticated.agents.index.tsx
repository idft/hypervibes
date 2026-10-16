import { createFileRoute } from "@tanstack/react-router";
import { AgentsPage } from "../components/agents-page.js";
import { createAgent, listAgents } from "../web/agents.js";

export const Route = createFileRoute("/_authenticated/agents/")({
  loader: () => listAgents(),
  component: AgentsRoute,
});

function AgentsRoute() {
  return <AgentsPage agents={Route.useLoaderData()} create={(name, requestId) => createAgent({ data: { name, requestId } })} />;
}
