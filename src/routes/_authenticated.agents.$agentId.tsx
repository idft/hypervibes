import { createFileRoute } from "@tanstack/react-router";
import { createServerFn } from "@tanstack/react-start";
import { getRequestHeaders } from "@tanstack/react-start/server";
import { AgentLayout } from "../components/agent-layout.js";
import { useSuspenseQuery } from "@tanstack/react-query";
import { agentQueryOptions } from "../web/agent-queries.js";

/** Reads the browser-persisted navigation preference so SSR renders the correct rail size. */
const agentNavExpanded = createServerFn({ method: "GET" }).handler(() => {
  const cookie = getRequestHeaders().get("cookie") ?? "";
  return /(?:^|;\s*)hv-agent-nav-expanded=true(?:;|$)/.test(cookie);
});

export const Route = createFileRoute("/_authenticated/agents/$agentId")({
  loader: async ({ params, context }) => {
    const [, navExpanded] = await Promise.all([context.queryClient.ensureQueryData(agentQueryOptions(params.agentId)), agentNavExpanded()]);
    return { navExpanded };
  },
  component: AgentRoute,
});

function AgentRoute() {
  const { navExpanded } = Route.useLoaderData();
  const { agentId } = Route.useParams();
  const { data: agent } = useSuspenseQuery(agentQueryOptions(agentId));
  return <AgentLayout key={agent.id} agent={agent} initialExpanded={navExpanded} />;
}
