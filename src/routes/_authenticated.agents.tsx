import { createFileRoute } from "@tanstack/react-router";
import { AgentsPage } from "../components/agents-page.js";
import { getRuntimeStatus } from "../web/status.js";

export const Route = createFileRoute("/_authenticated/agents")({
  loader: () => getRuntimeStatus(),
  component: AgentsRoute,
});

function AgentsRoute() {
  return <AgentsPage status={Route.useLoaderData()} />;
}
