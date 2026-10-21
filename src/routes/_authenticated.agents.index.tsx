import { createFileRoute } from "@tanstack/react-router";
import { AgentsPage } from "../components/agents-page.js";
import { useSuspenseQuery } from "@tanstack/react-query";
import { agentsQueryOptions } from "../web/agent-queries.js";
import { setupQueryOptions } from "../web/provider-queries.js";

export const Route = createFileRoute("/_authenticated/agents/")({
  loader: async ({ context }) => {
    await Promise.all([context.queryClient.ensureQueryData(agentsQueryOptions()), context.queryClient.ensureQueryData(setupQueryOptions())]);
  },
  component: AgentsRoute,
});

function AgentsRoute() {
  const { data: agents, error } = useSuspenseQuery(agentsQueryOptions());
  const { data: setup } = useSuspenseQuery(setupQueryOptions());
  return <>{error && <p role="alert" className="mb-4 text-sm text-red-300">{error.message}</p>}<AgentsPage agents={agents} setup={setup} /></>;
}