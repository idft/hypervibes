import { createFileRoute } from "@tanstack/react-router";
import { createServerFn } from "@tanstack/react-start";
import { getRequestHeaders } from "@tanstack/react-start/server";
import { AgentLayout } from "../components/agent-layout.js";
import { getAgent } from "../web/agents.js";

/** Reads the browser-persisted navigation preference so SSR renders the correct rail size. */
const agentNavExpanded = createServerFn({ method: "GET" }).handler(() => {
  const cookie = getRequestHeaders().get("cookie") ?? "";
  return /(?:^|;\s*)hv-agent-nav-expanded=true(?:;|$)/.test(cookie);
});

export const Route = createFileRoute("/_authenticated/agents/$agentId")({
  loader: async ({ params }) => {
    const [result, navExpanded] = await Promise.all([getAgent({ data: { id: params.agentId } }), agentNavExpanded()]);
    return { result, navExpanded };
  },
  component: AgentRoute,
});

function AgentRoute() {
  const { result, navExpanded } = Route.useLoaderData();
  return result.ok ? <AgentLayout key={result.data.id} agent={result.data} initialExpanded={navExpanded} /> : <p role="alert" className="text-red-300">{result.error}</p>;
}
