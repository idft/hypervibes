import { createFileRoute } from "@tanstack/react-router";
import { AgentsPage } from "../components/agents-page.js";
import { listAgents } from "../web/agents.js";

export const Route = createFileRoute("/_authenticated/agents/")({
  loader: () => listAgents(),
  component: AgentsRoute,
});

function AgentsRoute() {
  return <AgentsPage agents={Route.useLoaderData()} />;
}