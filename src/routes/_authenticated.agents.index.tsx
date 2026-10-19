import { createFileRoute } from "@tanstack/react-router";
import { AgentsPage } from "../components/agents-page.js";
import { listAgents } from "../web/agents.js";
import { getSetupState } from "../web/setup.js";

export const Route = createFileRoute("/_authenticated/agents/")({
  loader: async () => {
    const [agents, setup] = await Promise.all([listAgents(), getSetupState()]);
    return { agents, setup };
  },
  component: AgentsRoute,
});

function AgentsRoute() {
  const { agents, setup } = Route.useLoaderData();
  return <AgentsPage agents={agents} setup={setup} />;
}